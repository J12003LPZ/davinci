//! The frame every command sheet shares: header facts, status segments, hint
//! row and composer mode, one descriptor per sheet (design.md §11).
//!
//! A sheet's view owns its own descriptor (`<view>::chrome`); this module is
//! the dispatch the header, status bar and shell read, so the seventeen
//! sheets agree on one frame without each drawing it.
//!
//! Mirrors the frame every artboard of `docs/ui/Pi TUI Instruments.dc.html`
//! shares.

use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, Screen};
use crate::davinci::theme::Theme;
use crate::davinci::ui::{self, span};

use super::{
    agents, cogitator, compact, context_inspector, diff, export, governor, graph_run, keys, login,
    mcp, officina, permissions, recovery, resume, securitas, settings, task_board, thinking, tree,
    trust, vectors, workflows,
};

/// What sits under a sheet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Composer {
    /// No composer under this sheet.
    #[default]
    Hidden,
    /// The composer with this placeholder (or the command that opened it).
    Prompt(&'static str),
    /// The same, for a placeholder built from live state (`/graph-view t6`
    /// naming the worker that is actually running).
    PromptOwned(String),
    /// The composer is drawn but takes no input; the text sits in the dim ramp.
    Disabled(&'static str),
}

/// One sheet's frame.
#[derive(Debug, Clone, Default)]
pub struct SheetChrome {
    /// Header right run; empty means `cwd │ branch │ model`.
    pub header_right: Vec<Span<'static>>,
    /// Third segment of `mode · branch · third`.
    pub status_third: Option<Vec<Span<'static>>>,
    /// Status bar right run; `None` is the context meter.
    pub status_right: Option<Vec<Span<'static>>>,
    /// Hints joined by ` │ `.
    pub hints: Vec<Vec<Span<'static>>>,
    /// `esc close`, `esc cancel`, `esc done`, `esc leave it`; `None` draws no
    /// hint row.
    pub escape: Option<&'static str>,
    pub composer: Composer,
    /// `> /command` echoed as the first body row.
    pub echo: Option<String>,
}

/// A `│`-separated header run. Empty parts are skipped, so a fact that is not
/// known live drops its segment rather than showing a blank (design.md §11).
pub fn facts(theme: &Theme, parts: Vec<Vec<Span<'static>>>) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    for part in parts {
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(span(" │ ", theme.border));
        }
        out.extend(part);
    }
    out
}

/// `label ━━━◸ ─── used/cap`, twelve cells of meter as the status bar draws
/// it. A meter never shows a number without its cap (design.md §9).
pub fn status_meter(
    theme: &Theme,
    label: &str,
    fraction: f64,
    used: &str,
    cap: &str,
) -> Vec<Span<'static>> {
    let mut run = vec![span(format!("{label} "), theme.muted)];
    run.extend(ui::meter(fraction, 12, theme, None));
    run.push(span(format!(" {used}/{cap}"), theme.muted));
    run
}

/// A hint, in readable secondary ink.
pub fn hint(theme: &Theme, text: &str) -> Vec<Span<'static>> {
    vec![span(text, theme.muted)]
}

/// A secondary explanatory hint. Only describe actions the input owner supports.
pub fn hint_dim(theme: &Theme, text: &str) -> Vec<Span<'static>> {
    vec![span(text, theme.dim().border)]
}

/// The sheet's descriptor, or `None` for the transcript, Disegno, Grafo,
/// Memoria recall, Mensura and the Codex split, which keep their own frame.
pub fn chrome(model: &Model) -> Option<SheetChrome> {
    if model.overlay.is_some() {
        return None;
    }
    let mut chrome = match model.screen {
        Screen::Agent => return None,
        Screen::Plan | Screen::Grafo | Screen::Memoria | Screen::Mensura => SheetChrome::default(),
        Screen::Models => cogitator::chrome(model),
        Screen::Settings => settings::chrome(model),
        Screen::Thinking => thinking::chrome(model),
        Screen::Login => login::chrome(model),
        Screen::Keys => keys::chrome(model),
        Screen::Resume => resume::chrome(model),
        Screen::Tree => tree::chrome(model),
        Screen::Compact => compact::chrome(model),
        Screen::Export => export::chrome(model),
        Screen::GraphRun => graph_run::chrome(model),
        Screen::Vectors => vectors::chrome(model),
        Screen::Governor => governor::chrome(model),
        Screen::Securitas => securitas::chrome(model),
        Screen::Trust => trust::chrome(model),
        Screen::Officina => officina::chrome(model),
        Screen::Recovery => recovery::chrome(model),
        Screen::Diff => diff::chrome(model),
        Screen::Mcp => mcp::chrome(model),
        Screen::Permissions => permissions::chrome(model),
        Screen::Workflows => workflows::chrome(model),
        Screen::TaskBoard => task_board::chrome(model),
        Screen::Agents => agents::chrome(model),
        Screen::ContextInspector => context_inspector::chrome(model),
    };
    // These surfaces own the keyboard. A drawn-but-inert composer is misleading.
    if model.screen != Screen::GraphRun {
        chrome.composer = Composer::Hidden;
    }
    chrome.echo = None;
    chrome.escape = Some("esc close");
    if model.screen == Screen::GraphRun {
        return Some(chrome);
    }
    if model.screen == Screen::TaskBoard {
        // The Background sheet uses Claude's selection/view vocabulary rather
        // than the generic scrolling footer.
        chrome.hints = vec![
            hint(&model.theme, "↑/↓ to select"),
            hint(&model.theme, "Enter to view"),
        ];
        chrome.escape = Some("Esc to close");
        return Some(chrome);
    }
    let action = match model.screen {
        Screen::Models | Screen::Thinking => Some("enter select"),
        Screen::Settings => Some("enter change"),
        Screen::Login => Some("enter configure"),
        Screen::Resume => Some("enter resume"),
        Screen::Tree => Some("enter switch"),
        Screen::Trust => Some("enter decide"),
        Screen::Permissions => Some(
            if model
                .permission_rows
                .get(model.permission_index)
                .is_some_and(|row| row.kind == "rule")
            {
                "enter remove rule"
            } else {
                "enter set mode"
            },
        ),
        _ => None,
    };
    let picking = matches!(
        model.screen,
        Screen::Models
            | Screen::Settings
            | Screen::Thinking
            | Screen::Login
            | Screen::Resume
            | Screen::Tree
            | Screen::Permissions
            | Screen::Diff
            | Screen::Securitas
    );
    chrome.hints = vec![hint(
        &model.theme,
        if picking {
            "↑↓ move"
        } else {
            "↑↓ scroll"
        },
    )];
    if let Some(action) = action {
        chrome.hints.push(hint(
            &model.theme,
            if model.section_offset.is_some() {
                "enter back"
            } else {
                action
            },
        ));
    }
    chrome.hints.push(hint(&model.theme, "pgup/pgdn page"));
    Some(chrome)
}

/// The hint row for this sheet, if it has one.
pub fn hint_row(model: &Model, chrome: &SheetChrome) -> Option<Line<'static>> {
    chrome
        .escape
        .map(|esc| ui::hint_row(model.width, &chrome.hints, Some(esc), &model.theme))
}

/// Human-readable section names; domain-specific content keeps its own structure.
pub fn title(screen: Screen) -> &'static str {
    match screen {
        Screen::Agent => "Conversation",
        Screen::Plan => "Plan",
        Screen::Grafo => "Code graph",
        Screen::Memoria => "Memory recall",
        Screen::Mensura => "Context budget",
        Screen::Models => "Select model",
        Screen::Settings => "Settings",
        Screen::Thinking => "Model thinking level",
        Screen::Login => "Providers",
        Screen::Keys => "Help  General   Commands   Custom commands",
        Screen::Resume => "Resume session",
        Screen::Tree => "Session tree",
        Screen::Compact => "Compaction",
        Screen::Export => "Export session",
        Screen::GraphRun => "Graph run",
        Screen::Vectors => "Vector memory",
        Screen::Governor => "Output governor",
        Screen::Securitas => "Security report",
        Screen::Trust => "Project trust",
        Screen::Officina => "Reload",
        Screen::Recovery => "Recovery",
        Screen::Diff => "Review changes",
        Screen::Mcp => "MCP servers",
        Screen::Permissions => {
            "Permissions  Recently denied   Allow   Ask   Deny   Auto mode   Workspace"
        }
        Screen::Workflows => "Workflows",
        Screen::TaskBoard => "Background",
        Screen::Agents => "Agents",
        Screen::ContextInspector => "Context & Memory Inspector",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};

    fn theme() -> Theme {
        Theme::da_vinci(ColorDepth::TrueColor, false)
    }

    fn text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn facts_are_bar_separated_and_skip_what_is_unknown() {
        let th = theme();
        let run = facts(
            &th,
            vec![
                vec![span("7 files", th.text)],
                Vec::new(),
                vec![span("+145 -127", th.success)],
            ],
        );
        assert_eq!(text(&run), "7 files │ +145 -127");
    }

    #[test]
    fn a_status_meter_names_its_unit_and_cap() {
        let th = theme();
        let run = status_meter(&th, "disk", 0.15, "1.2G", "8G");
        let t = text(&run);
        assert!(t.starts_with("disk "), "{t}");
        assert!(t.ends_with(" 1.2G/8G"), "{t}");
        assert!(t.contains('◸'), "{t}");
    }

    #[test]
    fn the_transcript_has_no_sheet_chrome() {
        let m = Model::new(theme(), 100, 44, true);
        assert!(chrome(&m).is_none());
    }
}
