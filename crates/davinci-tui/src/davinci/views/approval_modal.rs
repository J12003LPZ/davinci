//! Approval modal view and input classification contract.
//!
//! Upstream reference: vendor/davinci (RPC mode approvals and interactive prompts).
//! Context precedes the answers; focus does not imply approval, and the runtime
//! owns every decision.

use crate::davinci::model::Model;
use ratatui::text::Line;

pub fn approval_key(open: bool, key: &str) -> &'static str {
    if !open {
        return "delegate";
    }
    match key {
        "escape" => "deny",
        "enter" => "confirm",
        "up" => "previous",
        "down" => "next",
        "1" | "2" | "3" | "4" | "5" => "select_number",
        _ => "consume",
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    crate::davinci::views::ask::lines(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f01_modal_traps_mode_key() {
        assert_eq!(approval_key(true, "shift_tab"), "consume");
        assert_eq!(approval_key(true, "escape"), "deny");
        assert_eq!(approval_key(true, "enter"), "confirm");
        assert_eq!(approval_key(false, "shift_tab"), "delegate");
    }

    #[test]
    fn f01_modal_key_navigation_mappings() {
        assert_eq!(approval_key(true, "up"), "previous");
        assert_eq!(approval_key(true, "down"), "next");
        for n in ["1", "2", "3", "4", "5"] {
            assert_eq!(approval_key(true, n), "select_number");
        }
        assert_eq!(approval_key(true, "other"), "consume");
        assert_eq!(approval_key(false, "enter"), "delegate");
        assert_eq!(approval_key(false, "escape"), "delegate");
    }
}
