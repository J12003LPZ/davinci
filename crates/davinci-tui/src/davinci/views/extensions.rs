//! `/plugins`, `/skills` and `/mcp`: install, find and manage extensions.
//!
//! Each command opens the manager on one kind. Its views: Installed (what
//! the host found, with the actions each row allows), Discover (a search box
//! over marketplaces and online directories) and, for plugins, Marketplaces.
//! The host fills every list and decides every action; this view only draws.
//! No TypeScript counterpart.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::{ExtensionRow, ExtensionTab, ExtensionView, ExtensionsSheet, Model};
use crate::davinci::theme::State;
use crate::davinci::ui::{section_detail, section_row, span};
use ratatui::text::{Line, Span};

fn view_count(sheet: &ExtensionsSheet, view: ExtensionView) -> Option<usize> {
    match view {
        ExtensionView::Installed => Some(sheet.current_rows().len()),
        ExtensionView::Discover => None,
        ExtensionView::Marketplaces => Some(sheet.marketplaces.len()),
    }
}

fn view_bar(model: &Model, sheet: &ExtensionsSheet) -> Line<'static> {
    let th = &model.theme;
    let mut spans: Vec<Span<'static>> = vec![span("   ", th.muted)];
    for (i, view) in sheet.tab.views().iter().enumerate() {
        if i > 0 {
            spans.push(span("   ", th.muted));
        }
        let label = match view_count(sheet, *view) {
            Some(count) => format!("{} ({count})", view.label()),
            None => view.label().to_string(),
        };
        if *view == sheet.view {
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
            "No plugins installed. Press → for Discover to find one, or adopt your \
             Claude Code / Codex plugins with `davinci plugin import`."
        }
        ExtensionTab::Skills => "No skills installed. Press → for Discover to find one.",
        ExtensionTab::Mcp => "No MCP servers configured. Press → for Discover to find one.",
    }
}

/// What the Discover search covers, said in the empty search box.
fn search_placeholder(tab: ExtensionTab) -> &'static str {
    match tab {
        ExtensionTab::Plugins => "Search plugins in your marketplaces",
        ExtensionTab::Skills => "Search skills in your marketplaces and on skills.sh",
        ExtensionTab::Mcp => "Search the MCP Registry",
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let Some(sheet) = &model.extension_manager else {
        return section_detail(width, th, "Nothing to manage.");
    };
    let mut rows = vec![view_bar(model, sheet), Line::default()];
    if let Some(notice) = sheet.notice.as_deref().filter(|text| !text.is_empty()) {
        for line in notice.lines() {
            rows.extend(colored(section_detail(width, th, line), th.muted));
        }
        rows.push(Line::default());
    }
    match sheet.view {
        ExtensionView::Installed => installed_lines(model, sheet, &mut rows),
        ExtensionView::Discover => discover_lines(model, sheet, &mut rows),
        ExtensionView::Marketplaces => marketplace_lines(model, sheet, &mut rows),
    }
    rows
}

fn push_row(model: &Model, rows: &mut Vec<Line<'static>>, item: &ExtensionRow, selected: bool) {
    let th = &model.theme;
    let width = model.width;
    rows.push(section_row(width, th, selected, &item.title, &item.status));
    if !item.detail.is_empty() {
        rows.extend(section_detail(width, th, &item.detail));
    }
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
}

fn installed_lines(model: &Model, sheet: &ExtensionsSheet, rows: &mut Vec<Line<'static>>) {
    let th = &model.theme;
    let items = sheet.current_rows();
    if items.is_empty() {
        rows.extend(section_detail(model.width, th, empty_text(sheet.tab)));
        return;
    }
    let selected = sheet.index();
    for (i, item) in items.iter().enumerate() {
        push_row(model, rows, item, i == selected);
        if i == selected {
            if let Some(action) = sheet.armed_here() {
                rows.extend(colored(
                    section_detail(model.width, th, &confirm_warning(sheet.tab, action, item)),
                    th.warning,
                ));
            }
        }
    }
}

fn discover_lines(model: &Model, sheet: &ExtensionsSheet, rows: &mut Vec<Line<'static>>) {
    let th = &model.theme;
    let width = model.width;
    let discover = &sheet.discover;
    let search = if discover.query.is_empty() {
        vec![
            span("   ⌕ ", th.primary),
            span(search_placeholder(sheet.tab), th.muted),
        ]
    } else {
        vec![
            span("   ⌕ ", th.primary),
            span(discover.query.clone(), th.text),
            span("▏", th.primary),
        ]
    };
    rows.push(Line::from(crate::davinci::ui::truncate_run(search, width)));
    let status = if discover.searching {
        Some("Searching…".to_string())
    } else {
        discover.message.clone()
    };
    if let Some(status) = status {
        rows.extend(colored(section_detail(width, th, &status), th.muted));
    }
    rows.push(Line::default());
    if discover.results.is_empty() {
        if !discover.searching {
            let text = if discover.query.is_empty() {
                "Type to search."
            } else {
                "Nothing matches."
            };
            rows.extend(section_detail(width, th, text));
        }
        return;
    }
    let selected = discover.index();
    for (i, item) in discover.results.iter().enumerate() {
        push_row(model, rows, item, i == selected);
        if i == selected && discover.armed_here() {
            rows.extend(colored(
                section_detail(
                    width,
                    th,
                    &format!(
                        "Press enter again to install {} ({}). Esc cancels.",
                        item.title, item.status
                    ),
                ),
                th.warning,
            ));
        }
    }
}

fn marketplace_lines(model: &Model, sheet: &ExtensionsSheet, rows: &mut Vec<Line<'static>>) {
    let th = &model.theme;
    let width = model.width;
    if let Some(input) = &sheet.marketplace_input {
        rows.push(Line::from(crate::davinci::ui::truncate_run(
            vec![
                span("   Add marketplace: ", th.primary),
                span(input.clone(), th.text),
                span("▏", th.primary),
            ],
            width,
        )));
        rows.extend(colored(
            section_detail(
                width,
                th,
                "owner/repo on GitHub, a git URL, or a local folder. Enter adds it.",
            ),
            th.muted,
        ));
        rows.push(Line::default());
    }
    if sheet.marketplaces.is_empty() {
        rows.extend(section_detail(
            width,
            th,
            "No marketplaces yet. Press a to add one, for example \
             anthropics/claude-plugins-official.",
        ));
        return;
    }
    let selected = sheet.marketplace_index();
    for (i, item) in sheet.marketplaces.iter().enumerate() {
        push_row(model, rows, item, i == selected);
        let armed = sheet
            .armed
            .as_ref()
            .is_some_and(|(name, _)| i == selected && *name == item.key);
        if armed {
            rows.extend(colored(
                section_detail(
                    width,
                    th,
                    &format!(
                        "Press y to remove the marketplace {}. Installed plugins stay. \
                         Any other key cancels.",
                        item.title
                    ),
                ),
                th.warning,
            ));
        }
    }
}

fn confirm_warning(tab: ExtensionTab, action: &str, item: &ExtensionRow) -> String {
    if action == "approve" {
        return format!(
            "Press y to let the hooks of {} (listed above) run shell commands on this machine. \
             Any other key cancels.",
            item.title
        );
    }
    let what = match tab {
        ExtensionTab::Plugins => "uninstall this plugin",
        ExtensionTab::Skills => "move this skill to the DaVinci trash",
        ExtensionTab::Mcp => "remove this server from its mcp.json",
    };
    format!("Press y to {what} ({}). Any other key cancels.", item.title)
}

/// The keys the selected installed row allows, in the order the hint row
/// shows them. While an action waits for confirmation only `y` does anything.
pub fn row_hints(item: Option<&ExtensionRow>, armed: Option<&str>) -> Vec<&'static str> {
    if let Some(action) = armed {
        return vec![if action == "approve" {
            "y approve hooks"
        } else {
            "y confirm delete"
        }];
    }
    let mut out = vec!["←→ view", "↑↓ move"];
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
        out.push("d delete");
    }
    out
}

fn discover_hints(sheet: &ExtensionsSheet) -> Vec<&'static str> {
    if sheet.discover.armed_here() {
        return vec!["enter install", "esc cancel"];
    }
    let mut out = vec!["type to search", "↑↓ move"];
    if sheet
        .discover
        .current()
        .is_some_and(|row| row.status != "installed")
    {
        out.push("enter install");
    }
    out.push("←→ view");
    out
}

fn marketplace_hints(sheet: &ExtensionsSheet) -> Vec<&'static str> {
    if sheet.marketplace_input.is_some() {
        return vec!["enter add", "esc cancel"];
    }
    if sheet.armed.is_some() {
        return vec!["y confirm remove"];
    }
    let mut out = vec!["←→ view", "↑↓ move", "a add"];
    if let Some(row) = sheet.current_marketplace() {
        if row.can_update {
            out.push("u update");
        }
        if row.can_delete {
            out.push("d remove");
        }
    }
    out
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let (header, hints) = match &model.extension_manager {
        Some(sheet) => {
            let hints = match sheet.view {
                ExtensionView::Installed => row_hints(sheet.current(), sheet.armed_here()),
                ExtensionView::Discover => discover_hints(sheet),
                ExtensionView::Marketplaces => marketplace_hints(sheet),
            };
            (sheet.tab.label().to_string(), hints)
        }
        None => (String::new(), row_hints(None, None)),
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
        model::{DiscoverState, ExtensionsSheet},
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
            marketplaces: vec![ExtensionRow {
                can_update: true,
                can_delete: true,
                ..row("superpowers-marketplace", "davinci")
            }],
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

    fn hints(m: &Model) -> Vec<String> {
        chrome(m)
            .hints
            .iter()
            .flat_map(|h| h.iter().map(|s| s.content.to_string()))
            .collect()
    }

    #[test]
    fn installed_view_draws_its_rows_and_the_views_of_its_kind() {
        let m = model(100);
        let drawn = text(&lines(&m));
        for value in [
            "[Installed (2)]",
            "Discover",
            "Marketplaces (1)",
            "superpowers@superpowers-marketplace",
            "detail of caveman@caveman",
            "hooks changed, approval needed",
        ] {
            assert!(drawn.contains(value), "missing {value}:\n{drawn}");
        }
        assert_eq!(ui::focused_row(&lines(&m)), Some(2));
        let header: String = chrome(&m)
            .header_right
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert_eq!(header, "Plugins");
    }

    #[test]
    fn skills_and_mcp_have_no_marketplaces_view() {
        let mut m = model(100);
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.tab = ExtensionTab::Skills;
        let drawn = text(&lines(&m));
        assert!(drawn.contains("[Installed (1)]"), "{drawn}");
        assert!(!drawn.contains("Marketplaces"), "{drawn}");
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.switch_view(1);
        assert_eq!(sheet.view, ExtensionView::Discover);
        sheet.switch_view(1);
        assert_eq!(sheet.view, ExtensionView::Installed);
    }

    #[test]
    fn hints_name_only_the_actions_the_row_allows() {
        let mut m = model(100);
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
    fn discover_shows_the_search_box_results_and_the_armed_install() {
        let mut m = model(100);
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.tab = ExtensionTab::Skills;
        sheet.view = ExtensionView::Discover;
        let drawn = text(&lines(&m));
        assert!(
            drawn.contains("Search skills in your marketplaces and on skills.sh"),
            "{drawn}"
        );
        assert!(drawn.contains("Type to search."), "{drawn}");
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.discover = DiscoverState {
            query: "pdf".into(),
            results: vec![
                ExtensionRow {
                    key: "skills.sh:anthropics/skills/pdf".into(),
                    title: "pdf".into(),
                    status: "anthropics/skills".into(),
                    detail: "Read and fill PDF forms".into(),
                    note: Some("205,751 installs".into()),
                    ..ExtensionRow::default()
                },
                ExtensionRow {
                    key: "local:x".into(),
                    title: "docx".into(),
                    status: "installed".into(),
                    ..ExtensionRow::default()
                },
            ],
            message: Some("skills.sh: offline".into()),
            ..DiscoverState::default()
        };
        let drawn = text(&lines(&m));
        for value in [
            "⌕ pdf",
            "Read and fill PDF forms",
            "205,751 installs",
            "skills.sh: offline",
        ] {
            assert!(drawn.contains(value), "missing {value}:\n{drawn}");
        }
        assert!(hints(&m).contains(&"enter install".to_string()));
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.discover.armed = Some("skills.sh:anthropics/skills/pdf".into());
        assert!(text(&lines(&m)).contains("Press enter again to install pdf (anthropics/skills)"));
        assert_eq!(hints(&m), ["enter install", "esc cancel"]);
        // An installed result offers no install.
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.discover.armed = None;
        sheet.discover.move_selection(1);
        assert!(!hints(&m).contains(&"enter install".to_string()));
    }

    #[test]
    fn marketplaces_view_adds_updates_and_confirms_removal() {
        let mut m = model(100);
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.view = ExtensionView::Marketplaces;
        assert_eq!(
            hints(&m),
            ["←→ view", "↑↓ move", "a add", "u update", "d remove"]
        );
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.marketplace_input = Some("anthropics/skills".into());
        let drawn = text(&lines(&m));
        assert!(
            drawn.contains("Add marketplace: anthropics/skills"),
            "{drawn}"
        );
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.marketplace_input = None;
        sheet.armed = Some(("superpowers-marketplace".into(), "remove"));
        assert!(text(&lines(&m)).contains("Press y to remove the marketplace"));
    }

    #[test]
    fn armed_delete_warns_and_narrow_widths_stay_bounded() {
        let mut m = model(100);
        m.extension_manager.as_mut().unwrap().armed =
            Some(("superpowers@superpowers-marketplace".into(), "delete"));
        assert!(text(&lines(&m)).contains("Press y to uninstall this plugin"));
        assert_eq!(hints(&m), vec!["y confirm delete".to_string()]);
        // Armed on a row that is not selected: nothing is shown.
        m.extension_manager.as_mut().unwrap().armed = Some(("caveman@caveman".into(), "delete"));
        assert!(!text(&lines(&m)).contains("Press y"));
        for view in [
            ExtensionView::Installed,
            ExtensionView::Discover,
            ExtensionView::Marketplaces,
        ] {
            for width in [0, 1, 20, 40, 80, 120] {
                let mut m = model(width);
                let sheet = m.extension_manager.as_mut().unwrap();
                sheet.view = view;
                sheet.discover.query = "a long query that will not fit".into();
                sheet.marketplace_input = Some("owner/repository-name".into());
                for row in lines(&m) {
                    assert!(ui::run_width(&row.spans) <= width, "{view:?} {width}");
                }
            }
        }
    }
}
