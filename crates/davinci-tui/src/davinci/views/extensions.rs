//! `/plugin` — manage installed plugins, skills and MCP servers.
//!
//! One tab per kind. The host fills the rows and decides which actions each
//! row allows (`ExtensionRow::can_*`); this view draws them and names only
//! the keys that apply to the selected row. No TypeScript counterpart.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::{ExtensionRow, ExtensionTab, ExtensionsSheet, Model};
use crate::davinci::theme::State;
use crate::davinci::ui::{section_detail, section_row, span};
use ratatui::text::{Line, Span};

fn tab_bar(model: &Model, sheet: &ExtensionsSheet) -> Line<'static> {
    let th = &model.theme;
    let mut spans: Vec<Span<'static>> = vec![span("   ", th.muted)];
    for (i, tab) in ExtensionTab::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(span("   ", th.muted));
        }
        let label = format!("{} ({})", tab.label(), sheet.rows(*tab).len());
        if *tab == sheet.tab {
            spans.push(span(format!("[{label}]"), model.theme.cc().permission));
        } else {
            spans.push(span(label, th.muted));
        }
    }
    Line::from(crate::davinci::ui::truncate_run(spans, model.width))
}

fn colored(rows: Vec<Line<'static>>, color: ratatui::style::Color) -> Vec<Line<'static>> {
    rows.into_iter()
        .map(|mut row| {
            for span in &mut row.spans {
                span.style.fg = Some(color);
            }
            row
        })
        .collect()
}

fn empty_text(tab: ExtensionTab) -> &'static str {
    match tab {
        ExtensionTab::Plugins => {
            "No plugins installed. Run /plugin import to adopt Claude Code or Codex plugins, \
             or /plugin browse to see marketplaces."
        }
        ExtensionTab::Skills => "No skills found.",
        ExtensionTab::Mcp => "No MCP servers configured.",
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(sheet) = &model.extension_manager else {
        return section_detail(width, th, "Nothing to manage.");
    };
    let mut rows = vec![tab_bar(model, sheet), Line::default()];
    if let Some(notice) = sheet.notice.as_deref().filter(|text| !text.is_empty()) {
        for line in notice.lines() {
            rows.extend(colored(section_detail(width, th, line), th.muted));
        }
        rows.push(Line::default());
    }
    let items = sheet.current_rows();
    if items.is_empty() {
        rows.extend(section_detail(width, th, empty_text(sheet.tab)));
        return rows;
    }
    let selected = sheet.index();
    for (i, item) in items.iter().enumerate() {
        rows.push(section_row(
            width,
            th,
            i == selected,
            &item.title,
            &item.status,
        ));
        rows.extend(section_detail(width, th, &item.detail));
        if let Some(note) = &item.note {
            let color = match item.state {
                State::Failed => th.error,
                State::Attention => th.warning,
                _ => th.muted,
            };
            for line in note.lines() {
                rows.extend(colored(section_detail(width, th, line), color));
            }
        }
        if i == selected && sheet.armed_delete.as_deref() == Some(item.key.as_str()) {
            rows.extend(colored(
                section_detail(width, th, &delete_warning(sheet.tab, item)),
                th.warning,
            ));
        }
    }
    rows
}

fn delete_warning(tab: ExtensionTab, item: &ExtensionRow) -> String {
    let what = match tab {
        ExtensionTab::Plugins => "uninstall this plugin",
        ExtensionTab::Skills => "move this skill's folder to the DaVinci trash",
        ExtensionTab::Mcp => "remove this server from its mcp.json",
    };
    format!(
        "Press d again to {what} ({}). Any other key cancels.",
        item.title
    )
}

/// The keys the selected row allows, in the order the hint row shows them.
pub fn row_hints(item: Option<&ExtensionRow>, armed: bool) -> Vec<&'static str> {
    let mut out = vec!["←→ tab", "↑↓ move"];
    let Some(item) = item else {
        return out;
    };
    out.push("enter details");
    if item.can_update {
        out.push("u update");
    }
    if item.can_toggle {
        out.push(if item.status == "disabled" {
            "e enable"
        } else {
            "e disable"
        });
    }
    if item.can_approve {
        out.push("a approve hooks");
    }
    if item.can_revoke {
        out.push("r revoke hooks");
    }
    if item.can_delete {
        out.push(if armed {
            "d confirm delete"
        } else {
            "d delete"
        });
    }
    out
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let (header, hints) = match &model.extension_manager {
        Some(sheet) => {
            let item = sheet.current();
            let armed =
                item.is_some_and(|item| sheet.armed_delete.as_deref() == Some(item.key.as_str()));
            (
                format!(
                    "{} plugins · {} skills · {} MCP",
                    sheet.plugins.len(),
                    sheet.skills.len(),
                    sheet.mcp.len()
                ),
                row_hints(item, armed),
            )
        }
        None => (String::new(), row_hints(None, false)),
    };
    SheetChrome {
        header_right: vec![span(header, th.muted)],
        hints: hints.into_iter().map(|text| hint(th, text)).collect(),
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::ExtensionsSheet,
        theme::{ColorDepth, Theme},
        ui,
    };

    fn row(key: &str, status: &str) -> ExtensionRow {
        ExtensionRow {
            key: key.into(),
            title: key.into(),
            status: status.into(),
            detail: format!("detail of {key}"),
            can_toggle: true,
            can_delete: true,
            ..ExtensionRow::default()
        }
    }

    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            24,
            false,
        );
        m.extension_manager = Some(ExtensionsSheet {
            plugins: vec![
                row("superpowers@superpowers-marketplace", "enabled"),
                ExtensionRow {
                    note: Some("hooks changed, approval needed".into()),
                    state: State::Attention,
                    can_approve: true,
                    can_update: true,
                    ..row("caveman@caveman", "disabled")
                },
            ],
            skills: vec![row("pelican-facts", "user")],
            ..ExtensionsSheet::default()
        });
        m
    }

    fn text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn tabs_counts_rows_and_notes_are_drawn() {
        let m = model(100);
        let drawn = text(&lines(&m));
        for value in [
            "[Plugins (2)]",
            "Skills (1)",
            "MCP servers (0)",
            "superpowers@superpowers-marketplace",
            "detail of caveman@caveman",
            "hooks changed, approval needed",
        ] {
            assert!(drawn.contains(value), "missing {value}:\n{drawn}");
        }
        assert_eq!(ui::focused_row(&lines(&m)), Some(2));
    }

    #[test]
    fn switching_tabs_keeps_each_selection_and_empty_tabs_explain() {
        let mut m = model(100);
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.move_selection(1);
        sheet.switch_tab(1);
        assert_eq!(sheet.tab, ExtensionTab::Skills);
        assert_eq!(sheet.index(), 0);
        sheet.switch_tab(1);
        assert!(text(&lines(&m)).contains("No MCP servers configured."));
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.switch_tab(1);
        assert_eq!(sheet.tab, ExtensionTab::Plugins);
        assert_eq!(sheet.current().unwrap().key, "caveman@caveman");
    }

    #[test]
    fn hints_name_only_the_actions_the_row_allows() {
        let mut m = model(100);
        let hints = |m: &Model| {
            chrome(m)
                .hints
                .iter()
                .flat_map(|h| h.iter().map(|s| s.content.to_string()))
                .collect::<Vec<_>>()
        };
        let first = hints(&m);
        assert!(first.contains(&"e disable".to_string()));
        assert!(!first.iter().any(|h| h.starts_with("a ")));
        m.extension_manager.as_mut().unwrap().move_selection(1);
        let second = hints(&m);
        assert!(second.contains(&"e enable".to_string()));
        assert!(second.contains(&"a approve hooks".to_string()));
        assert!(second.contains(&"u update".to_string()));
    }

    #[test]
    fn armed_delete_warns_and_narrow_widths_stay_bounded() {
        let mut m = model(100);
        m.extension_manager.as_mut().unwrap().armed_delete =
            Some("superpowers@superpowers-marketplace".into());
        assert!(text(&lines(&m)).contains("Press d again to uninstall this plugin"));
        for width in [0, 1, 20, 40, 80, 120] {
            for row in lines(&model(width)) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
