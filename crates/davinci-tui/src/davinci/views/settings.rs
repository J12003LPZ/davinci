//! Reference-style tabbed configuration surface. All values are supplied by
//! DaVinci's existing settings owner. Rendering never changes configuration.
use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::{
    app::Flow,
    model::{Choice, Entry, Model},
    ui::{self, span},
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Search,
    List,
    Tabs,
}

pub const PINNED_DETAIL_ROWS: usize = 7;
const TABS: [&str; 4] = ["Status", "Config", "Usage", "Stats"];

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
    [
        "General",
        "Display",
        "Autocomplete",
        "Agent behavior",
        "Network",
        "Advanced",
    ][rank.min(5)]
}
pub fn visible_indices(model: &Model) -> Vec<usize> {
    model
        .settings_rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| {
            super::picker::matches(
                &model.settings_query,
                &[
                    &row.label,
                    &row.key,
                    &row.description,
                    group_label(group_rank(&row.key)),
                ],
            )
            .then_some(i)
        })
        .collect()
}
fn line(model: &Model, text: impl Into<String>, muted: bool) -> Line<'static> {
    Line::from(span(
        ui::clip_ellipsis(&text.into(), model.width),
        if muted {
            model.theme.muted
        } else {
            model.theme.text
        },
    ))
}
fn tabs(model: &Model) -> Line<'static> {
    let mut spans = vec![
        span("   ", model.theme.text),
        ui::span_strong("Settings  ", model.theme.primary, &model.theme),
    ];
    let short = model.width < 50;
    for (i, label) in TABS.iter().enumerate() {
        if short && i != model.settings_tab {
            continue;
        }
        let selected = i == model.settings_tab;
        let mut style = Style::default().fg(model.theme.text);
        if selected {
            style = style.add_modifier(Modifier::BOLD);
            if model.settings_focus == Focus::Tabs {
                style = style.fg(model.theme.primary);
            }
        }
        spans.push(Span::styled(format!("{label}   "), style));
    }
    if short {
        spans.push(span("← →", model.theme.muted));
    }
    Line::from(ui::truncate_run(spans, model.width))
}
fn search_box(model: &Model) -> Vec<Line<'static>> {
    let width = model.width.saturating_sub(6);
    let inner = width.saturating_sub(2);
    let color = if model.settings_focus == Focus::Search {
        model.theme.primary
    } else {
        model.theme.border
    };
    let query = if model.settings_query.is_empty() {
        "Search settings…"
    } else {
        &model.settings_query
    };
    let content = ui::clip_ellipsis(&format!(" ⌕ {query}"), inner);
    let used = unicode_width::UnicodeWidthStr::width(content.as_str());
    [
        format!("   ╭{}╮", "─".repeat(usize::from(inner))),
        format!(
            "   │{content}{}│",
            " ".repeat(usize::from(inner).saturating_sub(used))
        ),
        format!("   ╰{}╯", "─".repeat(usize::from(inner))),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, text)| {
        if i != 1 {
            Line::from(span(ui::clip_ellipsis(&text, model.width), color))
        } else {
            let mut spans = vec![
                span("   │", color),
                span(content.clone(), model.theme.muted),
            ];
            spans.push(span(
                format!("{}│", " ".repeat(usize::from(inner).saturating_sub(used))),
                color,
            ));
            Line::from(ui::truncate_run(spans, model.width))
        }
    })
    .collect()
}
fn setting_row(model: &Model, index: usize) -> Line<'static> {
    let row = &model.settings_rows[index];
    let selected = model.settings_focus == Focus::List && model.settings_index == index;
    let label_width = 41.min(model.width.saturating_sub(20));
    let label = ui::clip_ellipsis(&row.label, label_width);
    let occupied = unicode_width::UnicodeWidthStr::width(label.as_str());
    let marker = if selected { "   ❯ " } else { "     " };
    let color = if selected {
        model.theme.primary
    } else {
        model.theme.text
    };
    Line::from(ui::truncate_run(
        vec![
            span(marker, color),
            span(label, color),
            span(
                " ".repeat(usize::from(label_width).saturating_sub(occupied) + 2),
                color,
            ),
            span(row.value.clone(), color),
        ],
        model.width,
    ))
}
fn facts(model: &Model) -> Vec<Line<'static>> {
    let mut values: Vec<(String, String)> = Vec::new();
    match model.settings_tab {
        0 => {
            values.extend([
                ("Version".into(), env!("CARGO_PKG_VERSION").into()),
                ("Working directory".into(), model.cwd.clone()),
                ("Branch".into(), model.branch.clone()),
                ("Model".into(), model.model_name.clone()),
                ("Effort".into(), model.thinking_level.clone()),
                (
                    "Permission mode".into(),
                    model.permission_label().to_string(),
                ),
                ("Configuration".into(), model.config_path.clone()),
            ]);
        }
        2 => {
            values.push(("Context used".into(), format!("{} tokens", model.context.0)));
            values.push((
                "Context limit".into(),
                format!("{} tokens", model.context.1),
            ));
            if let Some(run) = &model.graph_run {
                values.push(("Graph run cost".into(), run.cost.clone()));
                values.push(("Graph run elapsed".into(), run.elapsed.clone()));
            }
            values.push((
                "Accounting".into(),
                "Only locally reported usage is shown; account balances are not available.".into(),
            ));
        }
        3 => {
            values.push(("Scope".into(), "Current conversation".into()));
            values.push((
                "User messages".into(),
                model
                    .transcript
                    .iter()
                    .filter(|e| matches!(e, Entry::User(_)))
                    .count()
                    .to_string(),
            ));
            values.push((
                "Tool calls".into(),
                model
                    .transcript
                    .iter()
                    .filter(|e| matches!(e, Entry::Tool { .. }))
                    .count()
                    .to_string(),
            ));
            values.push((
                "Reported failures".into(),
                model
                    .transcript
                    .iter()
                    .filter(|e| matches!(e, Entry::Failure { .. }))
                    .count()
                    .to_string(),
            ));
            values.push((
                "Foreground activity".into(),
                if model.running { "Working" } else { "Idle" }.into(),
            ));
        }
        _ => {}
    }
    values
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .flat_map(|(k, v)| {
            ui::wrap(&format!("{k}: {v}"), model.width.saturating_sub(3))
                .into_iter()
                .map(|text| line(model, format!("   {text}"), false))
                .collect::<Vec<_>>()
        })
        .collect()
}
/// Full setting metadata is opt-in; the default list keeps reference density.
pub fn details(model: &Model) -> Vec<Line<'static>> {
    let Some(row) = model.settings_rows.get(model.settings_index) else {
        return Vec::new();
    };
    let scope = if row.project { "project" } else { "user" };
    [
        row.label.clone(),
        format!("Current: {}", row.value),
        row.description.clone(),
        format!("Scope: {scope} · {}", row.key),
        format!("Choices: {}", row.values.join(" · ")),
        row.note.clone(),
    ]
    .into_iter()
    .filter(|text| !text.is_empty())
    .flat_map(|text| ui::wrap(&text, model.width.saturating_sub(3)))
    .map(|text| line(model, format!("   {text}"), false))
    .collect()
}

fn prefix(model: &Model) -> Vec<Line<'static>> {
    let mut rows = vec![
        line(model, "▔".repeat(usize::from(model.width)), true),
        tabs(model),
        ui::blank(),
    ];
    if model.settings_tab == 1 {
        rows.extend(search_box(model));
        rows.push(ui::blank());
    }
    rows
}
/// Full row list, also used by keyboard/window tests. Source order is preserved.
pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let mut rows = prefix(model);
    if model.settings_details {
        rows.extend(details(model));
        return rows;
    }
    if model.settings_tab != 1 {
        rows.extend(facts(model));
        return rows;
    }
    let indices = visible_indices(model);
    if indices.is_empty() {
        rows.push(line(
            model,
            if model.settings_rows.is_empty() {
                "   No settings to show."
            } else {
                "   No matching settings. Backspace to edit the search."
            },
            true,
        ));
    } else {
        rows.extend(indices.into_iter().map(|i| setting_row(model, i)));
    }
    rows
}
pub fn screen_height(model: &Model) -> usize {
    (lines(model).len() + 3)
        .min(usize::from(model.height).saturating_sub(2))
        .max(1)
}
pub fn screen(model: &Model, height: usize) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    let mut rows = prefix(model);
    let notices = model
        .section_notice
        .as_ref()
        .map(|text| ui::wrap(text, model.width.saturating_sub(3)))
        .unwrap_or_default();
    let notice_rows = notices.len().min(2);
    let room = height.saturating_sub(rows.len() + 3 + notice_rows);
    if model.settings_details {
        let items = details(model);
        rows.extend(ui::window(
            items,
            room,
            model.section_offset.unwrap_or(0),
            &model.theme,
        ));
    } else if model.settings_tab == 1 {
        let indices = visible_indices(model);
        let selected = indices
            .iter()
            .position(|i| *i == model.settings_index)
            .unwrap_or(0);
        let anchor = model
            .section_offset
            .unwrap_or(selected)
            .min(indices.len().saturating_sub(1));
        let capacity = room
            .saturating_sub(usize::from(indices.len() > room))
            .max(1);
        let start = anchor
            .saturating_sub(capacity.saturating_sub(1))
            .min(indices.len().saturating_sub(capacity));
        rows.extend(
            indices
                .iter()
                .skip(start)
                .take(capacity)
                .map(|i| setting_row(model, *i)),
        );
        let remaining = indices.len().saturating_sub(start + capacity);
        if remaining > 0 {
            rows.push(line(model, format!("   ↓ {remaining} more below"), true));
        }
        if indices.is_empty() {
            rows.push(line(
                model,
                if model.settings_rows.is_empty() {
                    "   No settings to show."
                } else {
                    "   No matching settings. Backspace to edit the search."
                },
                true,
            ));
        }
    } else {
        rows.extend(facts(model).into_iter().take(room));
    }
    for text in notices.into_iter().take(notice_rows) {
        let mut row = line(model, format!("   {text}"), false);
        for span in &mut row.spans {
            span.style.fg = Some(model.theme.warning);
        }
        rows.push(row);
    }
    rows.push(ui::blank());
    let controls = if model.settings_details {
        "   PgUp/PgDn read · Enter/Esc back"
    } else {
        match model.settings_focus {
            Focus::Search => "   Type to filter · Enter/↓ to select · ↑ to tabs · Esc to clear",
            Focus::Tabs => "   ←/→ to switch · ↓ to select · Esc to close",
            Focus::List => "   Enter/Space to change · / search · F1 details · Esc close",
        }
    };
    rows.push(line(model, controls, true));
    rows.truncate(height);
    rows
}
/// The settings surface owns search, tabs, selection and close. Nothing here
/// writes settings: the existing host applies the returned stable source choice.
pub fn handle_key(model: &mut Model, key: KeyEvent) -> Flow {
    if key.kind == KeyEventKind::Release
        || key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return Flow::Continue;
    }
    if key.code == KeyCode::F(1) && model.settings_tab == 1 {
        model.settings_details = !model.settings_details;
        model.section_offset = None;
        return Flow::Continue;
    }
    if model.settings_details {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                model.settings_details = false;
                model.settings_focus = Focus::List;
                model.section_offset = None;
            }
            KeyCode::PageDown | KeyCode::Down => {
                let by = if key.code == KeyCode::Down {
                    1
                } else {
                    usize::from(model.height.saturating_sub(10)).max(1)
                };
                model.section_offset = Some(
                    model
                        .section_offset
                        .unwrap_or(0)
                        .saturating_add(by)
                        .min(details(model).len().saturating_sub(1)),
                );
            }
            KeyCode::PageUp | KeyCode::Up => {
                let by = if key.code == KeyCode::Up {
                    1
                } else {
                    usize::from(model.height.saturating_sub(10)).max(1)
                };
                model.section_offset = Some(model.section_offset.unwrap_or(0).saturating_sub(by));
            }
            _ => {}
        }
        return Flow::Continue;
    }
    match key.code {
        KeyCode::Esc => {
            if model.settings_tab == 1 && model.settings_focus == Focus::Search {
                model.settings_query.clear();
                model.settings_focus = Focus::List;
            } else {
                model.close();
            }
        }
        KeyCode::Up if model.settings_focus == Focus::Search => model.settings_focus = Focus::Tabs,
        KeyCode::Down if model.settings_focus == Focus::Tabs => {
            model.settings_focus = Focus::Search
        }
        KeyCode::Left | KeyCode::Right if model.settings_focus == Focus::Tabs => {
            model.settings_tab = crate::davinci::model::wrap_index(
                model.settings_tab,
                if key.code == KeyCode::Left { -1 } else { 1 },
                TABS.len(),
            );
            model.section_offset = None;
        }
        KeyCode::Tab => model.settings_focus = Focus::Tabs,
        KeyCode::Enter | KeyCode::Down if model.settings_focus == Focus::Search => {
            model.settings_focus = Focus::List
        }
        KeyCode::Enter | KeyCode::Char(' ')
            if model.settings_focus == Focus::List && model.settings_tab == 1 =>
        {
            let indices = visible_indices(model);
            if indices.contains(&model.settings_index)
                && model
                    .settings_rows
                    .get(model.settings_index)
                    .is_some_and(|r| !r.values.is_empty())
            {
                return Flow::Choose(Choice::Setting(model.settings_index));
            }
        }
        KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
            if model.settings_focus == Focus::List =>
        {
            let indices = visible_indices(model);
            let amount = if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
                isize::try_from(model.height.saturating_sub(10))
                    .unwrap_or(1)
                    .max(1)
            } else {
                1
            };
            let delta = if matches!(key.code, KeyCode::Up | KeyCode::PageUp) {
                -amount
            } else {
                amount
            };
            if model.settings_index == indices.first().copied().unwrap_or(0)
                && key.code == KeyCode::Up
            {
                model.settings_focus = Focus::Search;
            } else {
                model.settings_index = super::picker::step(&indices, model.settings_index, delta);
            }
            model.section_offset = None;
        }
        KeyCode::Char('/') if model.settings_tab == 1 && model.settings_focus != Focus::Search => {
            model.settings_focus = Focus::Search
        }
        KeyCode::Char(ch) if model.settings_tab == 1 => {
            model.settings_focus = Focus::Search;
            model.settings_query.push(ch);
            reselect(model);
        }
        KeyCode::Backspace if model.settings_tab == 1 => {
            use unicode_segmentation::UnicodeSegmentation;
            let end = model
                .settings_query
                .grapheme_indices(true)
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            model.settings_query.truncate(end);
            reselect(model);
        }
        _ => {}
    }
    Flow::Continue
}
fn reselect(model: &mut Model) {
    let indices = visible_indices(model);
    if !indices.contains(&model.settings_index) {
        if let Some(first) = indices.first() {
            model.settings_index = *first;
        }
    }
    model.section_offset = None;
}
pub fn chrome(model: &Model) -> SheetChrome {
    SheetChrome {
        header_right: vec![span(
            format!("{} settings", model.settings_rows.len()),
            model.theme.muted,
        )],
        hints: vec![hint(&model.theme, "Type to filter")],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..Default::default()
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
        assert!(!text(&model).contains("Choices: on · off"));
        let detail = details(&model)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(detail.contains("Choices: on · off"));
        assert!(!text(&model).contains("Choices: sse"));
    }

    #[test]
    fn only_focus_expands_and_its_scope_and_note_are_retained() {
        let mut model = model(80);
        model.settings_details = true;
        assert!(text(&model).contains("Compact before"));
        assert!(!text(&model).contains("Preferred transport"));
        model.settings_index = 1;
        let text = text(&model);
        assert!(text.contains("Preferred transport"));
        assert!(text.contains("Scope: project · transport"));
        assert!(text.contains("Provider dependent"));
        assert!(text.contains("Current: websocket-cached"));
        assert!(!text.contains("Compact before"));
        model.settings_details = false;
        model.settings_focus = Focus::List;
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

        // Categories remain searchable metadata, not vertical clutter.
        for category in [
            "General",
            "Display",
            "Autocomplete",
            "Agent behavior",
            "Network",
            "Advanced",
        ] {
            model.settings_query = category.into();
            assert!(!visible_indices(&model).is_empty(), "{category}");
        }
        model.settings_query.clear();
        model.settings_focus = Focus::List;
        let rows = lines(&model);
        let focus = ui::focused_row(&rows).unwrap();
        let search = rows
            .iter()
            .position(|row| row.to_string().contains("Search settings…"))
            .unwrap();
        assert!(search < focus);
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
