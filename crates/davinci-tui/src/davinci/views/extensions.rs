//! `/plugins`, `/skills` and `/mcp`: install, find and manage extensions.
//!
//! Each command opens the manager on one kind. Its views: Installed (what
//! the host found, with the actions each row allows), Discover (a search box
//! over marketplaces and online directories) and, for plugins, Marketplaces.
//! The host fills every list and decides every action; this view only draws.
//! No TypeScript counterpart.
//!
//! Layout, top to bottom (the tab bar and search box stay pinned while the
//! list scrolls; every row is separated from the next by a blank line):
//!
//! ```text
//!  Installed (3)  Discover  Marketplaces (1)      ✗ 1 error · ! 1 needs auth   2/3
//!
//!   ● code-review@claude-plugins-official · enabled · v1.0.0
//!     from davinci · 1 command · 1 agent · hooks approved
//!
//! ❯ ! superpowers@superpowers-marketplace  ! needs auth · enabled
//!     from Claude Code · v6.2.0 · 14 skills
//!     401 from the server; run /login for it.
//! ```
//!
//! The selected row shows two description lines and its note; the rest clip
//! their description to one line. Skills never carry a problem badge.

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::{ExtensionRow, ExtensionTab, ExtensionView, ExtensionsSheet, Model};
use crate::davinci::theme::{State, Theme};
use crate::davinci::ui::{
    self, clip_ellipsis, pad, span, truncate_run, wrap, SELECTION_BAR, UNSELECTED_BAR,
};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

/// The widest the manager draws; a wider terminal leaves the rest empty so
/// names and descriptions stay close enough to read as one row.
const MAX_WIDTH: u16 = 112;
/// `❯ ` + state glyph + space: where a row's name, and everything under it, starts.
const INDENT: u16 = 4;
/// The most header rows the list scroll keeps pinned on a short terminal.
const MAX_PINNED: usize = 8;

/// Which list a row belongs to, so its glyph says the right thing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Installed,
    Discover,
    Marketplace,
}

fn width_of(model: &Model) -> u16 {
    model.width.min(MAX_WIDTH)
}

fn fit(spans: Vec<Span<'static>>, width: u16) -> Line<'static> {
    Line::from(truncate_run(spans, width))
}

fn view_count(sheet: &ExtensionsSheet, view: ExtensionView) -> Option<usize> {
    match view {
        ExtensionView::Installed => Some(sheet.current_rows().len()),
        ExtensionView::Discover => None,
        ExtensionView::Marketplaces => Some(sheet.marketplaces.len()),
    }
}

/// `n/total` for the open list, so a long registry says where you are.
fn position(sheet: &ExtensionsSheet) -> Option<String> {
    let (index, total) = match sheet.view {
        ExtensionView::Installed => (sheet.index(), sheet.current_rows().len()),
        ExtensionView::Discover => (sheet.discover.index(), sheet.discover.results.len()),
        ExtensionView::Marketplaces => (sheet.marketplace_index(), sheet.marketplaces.len()),
    };
    (total > 0).then(|| format!("{}/{total}", index + 1))
}

/// What is wrong with a row, if anything. Skills are instructions, not
/// processes, so they never carry one; plugins and MCP servers do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Problem {
    Error,
    Auth,
    Attention,
}

impl Problem {
    fn of(item: &ExtensionRow, tab: ExtensionTab) -> Option<Self> {
        if tab == ExtensionTab::Skills {
            return None;
        }
        match item.state {
            State::Failed | State::Attention => {}
            _ => return None,
        }
        let text = format!(
            "{} {}",
            item.status,
            item.note.as_deref().unwrap_or_default()
        )
        .to_lowercase();
        let auth = [
            "401",
            "403",
            "unauthor",
            "forbidden",
            "auth",
            "login",
            "token",
        ]
        .iter()
        .any(|needle| text.contains(needle));
        Some(match (item.state, auth) {
            (_, true) => Self::Auth,
            (State::Failed, false) => Self::Error,
            _ => Self::Attention,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Auth => "needs auth",
            Self::Attention => "needs attention",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Error => State::Failed.glyph(),
            Self::Auth | Self::Attention => State::Attention.glyph(),
        }
    }

    fn color(self, th: &Theme) -> Color {
        match self {
            Self::Error => th.error,
            Self::Auth | Self::Attention => th.warning,
        }
    }
}

/// `2 errors · 1 needs auth` over everything installed of this kind, so a
/// problem is visible from Discover too.
fn problem_summary(model: &Model, sheet: &ExtensionsSheet) -> Vec<Span<'static>> {
    let th = &model.theme;
    let mut out = Vec::new();
    for kind in [Problem::Error, Problem::Auth, Problem::Attention] {
        let n = sheet
            .current_rows()
            .iter()
            .filter(|row| Problem::of(row, sheet.tab) == Some(kind))
            .count();
        if n == 0 {
            continue;
        }
        if !out.is_empty() {
            out.push(span(" · ", th.muted));
        }
        let label = match (kind, n) {
            (Problem::Error, 1) => "1 error".to_string(),
            (Problem::Error, n) => format!("{n} errors"),
            (kind, n) => format!("{n} {}", kind.label()),
        };
        out.push(span(format!("{} {label}", kind.glyph()), kind.color(th)));
    }
    out
}

/// One row of tabs; the open one is a filled pill, as in Claude Code's
/// plugin manager. Problems and the list position sit flush right.
fn tab_bar(model: &Model, sheet: &ExtensionsSheet) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = width_of(model);
    let mut labels: Vec<Span<'static>> = vec![pad(1, None)];
    for view in sheet.tab.views() {
        let name = match view_count(sheet, *view) {
            Some(count) => format!("{} ({count})", view.label()),
            None => view.label().to_string(),
        };
        if *view == sheet.view {
            labels.push(Span::styled(
                format!(" {name} "),
                Style::default().fg(th.background).bg(th.text),
            ));
        } else {
            labels.push(span(format!(" {name} "), th.muted));
        }
    }
    let mut right = problem_summary(model, sheet);
    if let Some(text) = position(sheet) {
        if !right.is_empty() {
            right.push(span("  ", th.muted));
        }
        right.push(span(text, th.muted));
    }
    right.push(pad(2, None));
    vec![ui::spread(width, labels, right)]
}

/// A rounded box with `inner` on the left and `right` flush right.
fn boxed(
    model: &Model,
    inner: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = width_of(model);
    let edge = th.cc().permission;
    let outer = width.saturating_sub(4);
    if outer < 8 {
        return vec![fit(inner, width)];
    }
    let room = outer - 4;
    let wall = |text: &str| span(text, edge);
    let mut middle = vec![pad(2, None), wall("│"), pad(1, None)];
    middle.extend(ui::spread(room, inner, right).spans);
    middle.extend([pad(1, None), wall("│")]);
    let bar = "─".repeat(usize::from(outer - 2));
    vec![
        fit(vec![pad(2, None), wall(&format!("╭{bar}╮"))], width),
        fit(middle, width),
        fit(vec![pad(2, None), wall(&format!("╰{bar}╯"))], width),
    ]
}

fn indented(model: &Model, text: &str, color: Color) -> Vec<Line<'static>> {
    let width = width_of(model);
    if text.is_empty() || width == 0 {
        return Vec::new();
    }
    let lead = 2.min(width.saturating_sub(1));
    wrap(text, width.saturating_sub(lead))
        .into_iter()
        .map(|text| fit(vec![pad(lead, None), span(text, color)], width))
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

/// Everything above the list: it stays on screen while the list scrolls.
fn header(model: &Model, sheet: &ExtensionsSheet) -> Vec<Line<'static>> {
    let th = &model.theme;
    let mut rows = tab_bar(model, sheet);
    rows.push(Line::default());
    if let Some(notice) = sheet.notice.as_deref().filter(|text| !text.is_empty()) {
        for line in notice.lines() {
            rows.extend(indented(model, line, th.muted));
        }
        rows.push(Line::default());
    }
    match sheet.view {
        ExtensionView::Installed => {}
        ExtensionView::Discover => {
            let discover = &sheet.discover;
            let mut inner = vec![span("⌕ ", th.primary)];
            if discover.query.is_empty() {
                inner.push(span(search_placeholder(sheet.tab), th.muted));
            } else {
                inner.push(span(discover.query.clone(), th.text));
                inner.push(span("▏", th.primary));
            }
            let right = if discover.searching {
                vec![span("searching…", th.primary)]
            } else if discover.results.is_empty() {
                Vec::new()
            } else {
                let n = discover.results.len();
                vec![span(
                    format!("{n} result{}", if n == 1 { "" } else { "s" }),
                    th.muted,
                )]
            };
            rows.extend(boxed(model, inner, right));
            if let Some(message) = discover.message.as_deref().filter(|m| !m.is_empty()) {
                rows.extend(indented(model, message, th.muted));
            }
            rows.push(Line::default());
        }
        ExtensionView::Marketplaces => {
            if let Some(input) = &sheet.marketplace_input {
                let inner = vec![
                    span("+ ", th.primary),
                    span("Add marketplace: ", th.primary),
                    span(input.clone(), th.text),
                    span("▏", th.primary),
                ];
                rows.extend(boxed(model, inner, Vec::new()));
                rows.extend(indented(
                    model,
                    "owner/repo on GitHub, a git URL, or a local folder. Enter adds it.",
                    th.muted,
                ));
                rows.push(Line::default());
            }
        }
    }
    rows
}

/// How many leading rows the sheet keeps on screen while the list scrolls.
pub fn pinned_rows(model: &Model) -> usize {
    model
        .extension_manager
        .as_ref()
        .map_or(0, |sheet| header(model, sheet).len().min(MAX_PINNED))
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let Some(sheet) = &model.extension_manager else {
        return indented(model, "Nothing to manage.", th.muted);
    };
    let mut rows = header(model, sheet);
    match sheet.view {
        ExtensionView::Installed => installed_lines(model, sheet, &mut rows),
        ExtensionView::Discover => discover_lines(model, sheet, &mut rows),
        ExtensionView::Marketplaces => marketplace_lines(model, sheet, &mut rows),
    }
    rows
}

/// The state glyph and its colour. A search result says whether it can be
/// installed; every other row says how healthy it is.
fn glyph(item: &ExtensionRow, kind: Kind, th: &Theme) -> (&'static str, Color) {
    if kind == Kind::Discover {
        return if item.status == "installed" {
            (State::Done.glyph(), th.success)
        } else {
            ("+", th.primary)
        };
    }
    match item.state {
        State::Failed => (State::Failed.glyph(), th.error),
        State::Attention => (State::Attention.glyph(), th.warning),
        State::Skipped => ("○", th.muted),
        _ => ("●", th.success),
    }
}

fn status_color(word: &str, th: &Theme) -> Color {
    match word {
        "enabled" | "connected" | "installed" | "approved" => th.success,
        "failed" | "error" | "unreachable" => th.error,
        "featured" => th.primary,
        _ => th.muted,
    }
}

/// The name with its qualifier dimmed: `name@marketplace`, `owner/name`.
fn title_spans(title: &str, selected: bool, th: &Theme) -> Vec<Span<'static>> {
    let cc = th.cc();
    let title = title.strip_prefix("io.github.").unwrap_or(title);
    let name_color = if selected { cc.permission } else { th.text };
    let dim = th.muted;
    if let Some(at) = title.find('@') {
        vec![
            span(title[..at].to_string(), name_color),
            span(title[at..].to_string(), dim),
        ]
    } else if let Some(slash) = title.rfind('/') {
        vec![
            span(title[..=slash].to_string(), dim),
            span(title[slash + 1..].to_string(), name_color),
        ]
    } else {
        vec![span(title.to_string(), name_color)]
    }
}

fn note_color(item: &ExtensionRow, th: &Theme) -> Color {
    match item.state {
        State::Failed => th.error,
        State::Attention => th.warning,
        _ => th.muted,
    }
}

/// One row, three lines with a blank under it: name, then `· status · source`
/// in muted ink, then one clipped description. A problem is badged on the
/// first line and never folded away. The selected row also shows its note and
/// any pending confirmation, indented under the description.
fn card(
    model: &Model,
    item: &ExtensionRow,
    kind: Kind,
    selected: bool,
    confirm: Option<String>,
) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let width = width_of(model);
    let body = width.saturating_sub(INDENT);
    let tab = model
        .extension_manager
        .as_ref()
        .map_or(ExtensionTab::Plugins, |sheet| sheet.tab);
    let problem = Problem::of(item, tab);
    // Skills are instructions: whatever state a host reports, they are fine.
    let mut healthy;
    let item =
        if tab == ExtensionTab::Skills && matches!(item.state, State::Failed | State::Attention) {
            healthy = item.clone();
            healthy.state = State::Done;
            &healthy
        } else {
            item
        };
    let (mark, mark_color) = match problem {
        Some(problem) => (problem.glyph(), problem.color(th)),
        None => glyph(item, kind, th),
    };
    let mut line = vec![
        span(
            if selected {
                SELECTION_BAR
            } else {
                UNSELECTED_BAR
            },
            if selected { cc.permission } else { cc.inactive },
        ),
        span(mark, mark_color),
        pad(1, None),
    ];
    line.extend(title_spans(&item.title, selected, th));
    if let Some(problem) = problem {
        line.push(span(
            format!("  {} {}", problem.glyph(), problem.label()),
            problem.color(th),
        ));
    }
    let mut parts = item.status.split(" · ").filter(|part| !part.is_empty());
    if let Some(first) = parts.next() {
        line.push(span(" · ", th.muted));
        line.push(span(first.to_string(), status_color(first, th)));
        for part in parts {
            line.push(span(format!(" · {part}"), th.muted));
        }
    }
    let mut rows = vec![fit(line, width)];

    let lead = || pad(INDENT.min(width), None);
    let indent_rows = |text: &str, ink: Color, limit: usize| -> Vec<Line<'static>> {
        text.lines()
            .flat_map(|line| wrap(line, body))
            .take(limit)
            .map(|text| fit(vec![lead(), span(text, ink)], width))
            .collect()
    };
    let first = item.detail.lines().next().unwrap_or("");
    if selected {
        rows.extend(indent_rows(&item.detail, th.text, 2));
    } else if !first.is_empty() {
        rows.push(fit(
            vec![lead(), span(clip_ellipsis(first, body), th.muted)],
            width,
        ));
    }
    if let Some(note) = item.note.as_deref().filter(|n| !n.is_empty()) {
        let ink = note_color(item, th);
        if selected {
            rows.extend(indent_rows(note, ink, 3));
        } else if problem.is_some() {
            let first = note.lines().next().unwrap_or("");
            rows.push(fit(
                vec![lead(), span(clip_ellipsis(first, body), ink)],
                width,
            ));
        }
    }
    if let Some(warning) = confirm {
        rows.extend(indent_rows(&warning, th.warning, 3));
    }
    rows.push(Line::default());
    rows
}

fn installed_lines(model: &Model, sheet: &ExtensionsSheet, rows: &mut Vec<Line<'static>>) {
    let th = &model.theme;
    let items = sheet.current_rows();
    if items.is_empty() {
        rows.extend(indented(model, empty_text(sheet.tab), th.muted));
        return;
    }
    let selected = sheet.index();
    for (i, item) in items.iter().enumerate() {
        let confirm = (i == selected)
            .then(|| sheet.armed_here())
            .flatten()
            .map(|action| confirm_warning(sheet.tab, action, item));
        rows.extend(card(model, item, Kind::Installed, i == selected, confirm));
    }
}

fn discover_lines(model: &Model, sheet: &ExtensionsSheet, rows: &mut Vec<Line<'static>>) {
    let th = &model.theme;
    let discover = &sheet.discover;
    if discover.results.is_empty() {
        if !discover.searching {
            let text = if discover.query.is_empty() {
                "Type to search."
            } else {
                "Nothing matches."
            };
            rows.extend(indented(model, text, th.muted));
        }
        return;
    }
    let selected = discover.index();
    for (i, item) in discover.results.iter().enumerate() {
        let confirm = (i == selected && discover.armed_here()).then(|| {
            format!(
                "Press enter again to install {} ({}). Esc cancels.",
                item.title, item.status
            )
        });
        rows.extend(card(model, item, Kind::Discover, i == selected, confirm));
    }
}

fn marketplace_lines(model: &Model, sheet: &ExtensionsSheet, rows: &mut Vec<Line<'static>>) {
    let th = &model.theme;
    if sheet.marketplaces.is_empty() {
        rows.extend(indented(
            model,
            "No marketplaces yet. Press a to add one, for example \
             anthropics/claude-plugins-official.",
            th.muted,
        ));
        return;
    }
    let selected = sheet.marketplace_index();
    for (i, item) in sheet.marketplaces.iter().enumerate() {
        let armed = sheet
            .armed
            .as_ref()
            .is_some_and(|(name, _)| i == selected && *name == item.key);
        let confirm = armed.then(|| {
            format!(
                "Press y to remove the marketplace {}. Installed plugins stay. \
                 Any other key cancels.",
                item.title
            )
        });
        rows.extend(card(model, item, Kind::Marketplace, i == selected, confirm));
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
            "Installed (2)",
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
    fn each_manager_is_titled_by_its_kind() {
        let mut m = model(100);
        m.screen = crate::davinci::model::Screen::Extensions;
        for (tab, title) in [
            (ExtensionTab::Plugins, "Plugins"),
            (ExtensionTab::Skills, "Skills"),
            (ExtensionTab::Mcp, "MCP servers"),
        ] {
            m.extension_manager.as_mut().unwrap().tab = tab;
            assert_eq!(super::super::sheet::sheet_title(&m), title);
        }
    }

    #[test]
    fn skills_and_mcp_have_no_marketplaces_view() {
        let mut m = model(100);
        let sheet = m.extension_manager.as_mut().unwrap();
        sheet.tab = ExtensionTab::Skills;
        let drawn = text(&lines(&m));
        assert!(drawn.contains("Installed (1)"), "{drawn}");
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
