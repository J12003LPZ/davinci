//! `/settings`: current values first, with details for the focused setting.
//! Behavior remains in `davinci_interactive::cycle_setting`; the reusable
//! settings contract mirrors vendor/davinci/packages/tui/src/components/settings-list.ts.

use ratatui::text::Line;

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::Model;
use crate::davinci::ui::{self, section_detail, section_row, span};

pub fn group_rank(key: &str) -> usize {
    match key {
        "autocompact" | "autocompact-threshold" => 0,
        "show-images"
        | "image-width-cells"
        | "auto-resize-images"
        | "block-images"
        | "show-hardware-cursor"
        | "editor-padding"
        | "output-padding"
        | "clear-on-shrink"
        | "terminal-progress"
        | "hide-thinking"
        | "show-tool-output"
        | "mermaid-rendering"
        | "collapse-changelog"
        | "quiet-startup"
        | "tui-mode"
        | "fullscreen-exit-output"
        | "fullscreen-scrollbar"
        | "fullscreen-copy-on-select"
        | "theme" => 1,
        "skill-commands" | "autocomplete-max-visible" => 2,
        "steering-mode"
        | "follow-up-mode"
        | "decision-intelligence"
        | "typesafe-api-key"
        | "default-project-trust"
        | "double-escape-action"
        | "tree-filter-mode"
        | "model-thinking" => 3,
        "transport" | "http-idle-timeout" | "cache-miss-notices" => 4,
        _ => 5,
    }
}

fn group_label(rank: usize) -> &'static str {
    match rank {
        0 => "General",
        1 => "Display",
        2 => "Autocomplete",
        3 => "Agent behavior",
        4 => "Network",
        _ => "Advanced",
    }
}

pub const PINNED_DETAIL_ROWS: usize = 5;

fn detail_line(model: &Model, text: impl Into<String>, primary: bool) -> Line<'static> {
    let th = &model.theme;
    let width = model.width;
    let lead = 3.min(width);
    let text = ui::clip_ellipsis(&text.into(), width.saturating_sub(lead));
    let content = if primary {
        ui::span_strong(text, th.primary, th)
    } else {
        span(text, th.muted)
    };
    ui::indent(lead, vec![content])
}

/// Visible source indices, including keys and descriptions in the search corpus.
pub fn visible_indices(model: &Model) -> Vec<usize> {
    model
        .settings_rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            super::picker::matches(
                &model.settings_query,
                &[
                    &row.label,
                    &row.key,
                    &row.description,
                    group_label(group_rank(&row.key)),
                ],
            )
            .then_some(index)
        })
        .collect()
}

/// Compact configuration panel. It occupies only its content, not the entire terminal.
pub fn screen(model: &Model, height: usize) -> Vec<Line<'static>> {
    let mut rows = lines(model);
    let anchor = model
        .section_offset
        .or_else(|| ui::focused_row(&rows))
        .unwrap_or(0);
    let room = height.saturating_sub(3);
    let pinned = PINNED_DETAIL_ROWS
        .min(rows.len())
        .min(room.saturating_sub(1));
    let mut visible = rows.drain(..pinned).collect::<Vec<_>>();
    visible.extend(ui::window(
        rows,
        room.saturating_sub(pinned),
        anchor.saturating_sub(pinned),
        &model.theme,
    ));
    rows = visible;
    let mut out = vec![ui::indent(
        2.min(model.width),
        vec![ui::paper_label("Settings", &model.theme, true)],
    )];
    out.extend(rows);
    out.push(Line::from(span(
        ui::clip_ellipsis(
            "  Type to filter · ↑↓ move · enter change · esc close",
            model.width,
        ),
        model.theme.muted,
    )));
    out.push(ui::blank());
    out.truncate(height);
    out
}

pub fn screen_height(model: &Model) -> usize {
    (lines(model).len() + 3).min((model.height as usize * 3 / 4).max(12))
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    if model.settings_rows.is_empty() {
        return section_detail(
            width,
            th,
            "No settings to show. Close this view and retry /settings.",
        );
    }

    let indices = visible_indices(model);
    if indices.is_empty() {
        return vec![
            detail_line(model, format!("Search: {}", model.settings_query), true),
            detail_line(
                model,
                "No matching settings. Backspace to edit the search.",
                false,
            ),
        ];
    }
    let selected = if indices.contains(&model.settings_index) {
        model.settings_index
    } else {
        indices[0]
    };
    let focused = &model.settings_rows[selected];
    let scope = if focused.project { "project" } else { "user" };
    let mut tail = Vec::new();
    if !focused.note.is_empty() {
        tail.push(focused.note.clone());
    }
    if !focused.values.is_empty() {
        tail.push(format!("Choices: {}", focused.values.join(" · ")));
    }

    let mut rows = vec![
        detail_line(
            model,
            if model.settings_query.is_empty() {
                "Search settings…".into()
            } else {
                format!("Search: {}", model.settings_query)
            },
            true,
        ),
        detail_line(model, format!("Current: {}", focused.value), false),
        detail_line(model, focused.description.clone(), false),
        detail_line(model, format!("Scope: {scope} · {}", focused.key), false),
        detail_line(model, tail.join(" · "), false),
    ];

    let mut previous_group = None;
    for index in indices {
        let setting = &model.settings_rows[index];
        let rank = group_rank(&setting.key);
        if previous_group != Some(rank) {
            rows.extend(ui::section_heading(width, th, group_label(rank)));
            previous_group = Some(rank);
        }
        rows.push(section_row(
            width,
            th,
            index == selected,
            &setting.label,
            &setting.value,
        ));
    }
    rows
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let can_change = model
        .settings_rows
        .get(model.settings_index)
        .is_some_and(|row| !row.values.is_empty());
    let mut hints = vec![hint(th, "↑↓ move")];
    if can_change {
        hints.push(hint(th, "enter change"));
    }
    SheetChrome {
        header_right: vec![span(
            format!("{} settings", model.settings_rows.len()),
            th.muted,
        )],
        status_third: Some(vec![span("settings", th.muted)]),
        hints,
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::SettingRow,
        theme::{ColorDepth, Theme},
        ui,
    };

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        model.settings_rows = vec![
            SettingRow {
                label: "Auto-compact".into(),
                value: "on".into(),
                values: vec!["on".into(), "off".into()],
                description: "Compact before context overflows.".into(),
                key: "autocompact".into(),
                ..SettingRow::default()
            },
            SettingRow {
                label: "Transport".into(),
                value: "websocket-cached".into(),
                project: true,
                values: vec!["sse".into(), "websocket-cached".into()],
                description: "Preferred transport.".into(),
                key: "transport".into(),
                note: "Provider dependent".into(),
            },
        ];
        model
    }

    fn text(model: &Model) -> String {
        lines(model)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_setting_names_its_current_value_without_a_ramp_of_chips() {
        let model = model(80);
        for setting in &model.settings_rows {
            assert!(lines(&model)
                .iter()
                .any(|r| r.to_string().contains(&setting.label)
                    && r.to_string().contains(&setting.value)));
        }
        assert!(text(&model).contains("Choices: on · off"));
        assert!(!text(&model).contains("Choices: sse"));
    }

    #[test]
    fn only_focus_expands_and_its_scope_and_note_are_retained() {
        let mut model = model(80);
        assert!(text(&model).contains("Compact before"));
        assert!(!text(&model).contains("Preferred transport"));
        model.settings_index = 1;
        let text = text(&model);
        assert!(text.contains("Preferred transport"));
        assert!(text.contains("Scope: project · transport"));
        assert!(text.contains("Provider dependent"));
        assert!(text.contains("Current: websocket-cached"));
        assert!(!text.contains("Compact before"));
        assert!(lines(&model)[ui::focused_row(&lines(&model)).unwrap()]
            .to_string()
            .contains("Transport"));
    }

    #[test]
    fn settings_are_grouped_and_focused_details_stay_above_the_list() {
        let mut model = model(100);
        model.settings_rows.extend([
            SettingRow {
                label: "Theme".into(),
                value: "dark".into(),
                key: "theme".into(),
                ..SettingRow::default()
            },
            SettingRow {
                label: "Autocomplete max items".into(),
                value: "5".into(),
                key: "autocomplete-max-visible".into(),
                ..SettingRow::default()
            },
            SettingRow {
                label: "Steering mode".into(),
                value: "one-at-a-time".into(),
                key: "steering-mode".into(),
                ..SettingRow::default()
            },
            SettingRow {
                label: "Warnings".into(),
                value: "configure".into(),
                key: "warnings".into(),
                ..SettingRow::default()
            },
        ]);
        model.settings_rows.sort_by_key(|row| group_rank(&row.key));
        model.settings_index = model
            .settings_rows
            .iter()
            .position(|row| row.key == "autocompact")
            .unwrap();

        let rows = lines(&model);
        let drawn = rows.iter().map(Line::to_string).collect::<Vec<_>>();
        let joined = drawn.join("\n");
        for heading in [
            "General",
            "Display",
            "Autocomplete",
            "Agent behavior",
            "Network",
            "Advanced",
        ] {
            assert!(joined.contains(heading), "missing {heading}: {joined}");
        }
        let focus = ui::focused_row(&rows).unwrap();
        let detail = drawn
            .iter()
            .position(|row| row.contains("Search settings…"))
            .expect("focused detail header");
        assert!(
            detail < focus,
            "details must not expand underneath the selected row"
        );
    }

    #[test]
    fn inert_settings_do_not_advertise_enter_as_a_working_change() {
        let mut model = model(80);
        model.settings_rows[0].values.clear();
        let chrome = chrome(&model);
        assert!(chrome
            .hints
            .iter()
            .flatten()
            .all(|span| !span.content.contains("enter change")));
    }

    #[test]
    fn empty_and_narrow_sheets_are_readable_and_do_not_invent_paths_or_controls() {
        let mut empty = model(80);
        empty.settings_rows.clear();
        assert!(text(&empty).contains("No settings to show"));
        assert!(chrome(&empty)
            .header_right
            .iter()
            .all(|s| !s.content.contains("tab")));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut model = model(width);
            model.settings_rows[0].label = "设置 café 🦀 long setting".into();
            for row in lines(&model) {
                assert!(ui::run_width(&row.spans) <= width, "{width}: {row:?}");
            }
            assert!(!text(&model).contains("%USERPROFILE%"));
        }
    }
}
