//! Semantic code navigation and rename review TUI views matching spec.

use ratatui::text::Line;

use crate::davinci::theme::Theme;
use crate::davinci::ui::{section_detail, section_row};

/// View state for semantic symbol inspection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticSymbolView {
    pub symbol: String,
    pub definition: Option<String>,
    pub references_count: usize,
    pub references: Vec<String>,
    pub diagnostics_summary: String,
}

/// View state for rename preview and review.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenameReviewView {
    pub old_name: String,
    pub new_name: String,
    pub total_edits: usize,
    pub affected_files: Vec<(String, usize)>,
    pub confirmed: bool,
}

/// Renders the semantic symbol view matching the source spec:
/// ```text
/// Symbol: Agent::cycle_permission_mode
/// Definition   crates/davinci-agent/src/lib.rs:742
/// References   8
///   davinci_interactive.rs:418
///   main.rs:1261
///   model.rs:773
///   ...
/// Diagnostics  0 errors · 1 warning
/// [Enter] Open   [r] References   [n] Rename preview
/// ```
pub fn render_symbol_lines(
    view: &SemanticSymbolView,
    width: u16,
    th: &Theme,
) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    rows.push(section_row(
        width,
        th,
        true,
        &format!("Symbol: {}", view.symbol),
        "",
    ));

    if let Some(ref def) = view.definition {
        rows.extend(section_detail(width, th, &format!("Definition   {}", def)));
    } else {
        rows.extend(section_detail(width, th, "Definition   [unavailable]"));
    }

    rows.extend(section_detail(
        width,
        th,
        &format!("References   {}", view.references_count),
    ));
    for r in &view.references {
        rows.extend(section_detail(width, th, &format!("  {}", r)));
    }

    rows.extend(section_detail(
        width,
        th,
        &format!("Diagnostics  {}", view.diagnostics_summary),
    ));

    rows.extend(section_detail(
        width,
        th,
        "[Enter] Open   [r] References   [n] Rename preview",
    ));

    rows
}

/// Renders the rename preview view.
pub fn render_rename_review_lines(
    review: &RenameReviewView,
    width: u16,
    th: &Theme,
) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    rows.push(section_row(
        width,
        th,
        true,
        &format!("Rename: {} -> {}", review.old_name, review.new_name),
        "",
    ));

    rows.extend(section_detail(
        width,
        th,
        &format!(
            "Total edits: {} across {} file(s)",
            review.total_edits,
            review.affected_files.len()
        ),
    ));

    for (file, count) in &review.affected_files {
        rows.extend(section_detail(
            width,
            th,
            &format!("  {} ({} edit(s))", file, count),
        ));
    }

    rows.extend(section_detail(
        width,
        th,
        "[Enter] Confirm & Apply   [Esc] Cancel",
    ));

    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::ColorDepth;

    #[test]
    fn test_render_symbol_lines() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let view = SemanticSymbolView {
            symbol: "Agent::cycle_permission_mode".into(),
            definition: Some("crates/davinci-agent/src/lib.rs:742".into()),
            references_count: 8,
            references: vec!["davinci_interactive.rs:418".into(), "main.rs:1261".into()],
            diagnostics_summary: "0 errors · 1 warning".into(),
        };

        let lines = render_symbol_lines(&view, 80, &theme);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_rename_review_lines() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let review = RenameReviewView {
            old_name: "old_symbol".into(),
            new_name: "new_symbol".into(),
            total_edits: 3,
            affected_files: vec![("crates/lib.rs".into(), 2), ("crates/main.rs".into(), 1)],
            confirmed: false,
        };

        let lines = render_rename_review_lines(&review, 80, &theme);
        assert!(!lines.is_empty());
    }
}
