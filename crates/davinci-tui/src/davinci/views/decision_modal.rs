//! Structured decision modal view and input classification contract.
//!
//! Upstream reference: vendor/davinci (clarification and structured user decisions).
//! Exclusive input ownership prevents accidental grants or unverified execution.

use crate::davinci::theme::Theme;
use crate::davinci::ui::{hint_row, section_detail, section_row, span, Surface};
use ratatui::text::Line;

pub fn commits_decision(event: &str, answer_valid: bool) -> bool {
    event == "enter" && answer_valid
}

pub fn decision_key(open: bool, key: &str, is_custom_mode: bool) -> &'static str {
    if !open {
        return "delegate";
    }
    match key {
        "shift_tab" => "consume", // Traps permission mode cycle
        "escape" => "cancel",
        "enter" => "confirm",
        "up" => "previous",
        "down" => "next",
        "ctrl_d" | "d" if !is_custom_mode => "defer",
        "ctrl_i" | "i" if !is_custom_mode => "inspect",
        "tab" => "toggle_custom",
        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" if !is_custom_mode => "select_number",
        _ => {
            if is_custom_mode {
                "edit"
            } else {
                "consume"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionModalOption {
    pub id: String,
    pub label: String,
    pub explanation: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionResult {
    Choice(String),
    Custom(String),
    Defer,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionModalState {
    pub request_id: String,
    pub question_id: String,
    pub plan_revision: u64,
    pub title: String,
    pub question: String,
    pub materiality: String,
    pub evidence_refs: Vec<String>,
    pub options: Vec<DecisionModalOption>,
    pub allow_custom: bool,
    pub custom_only: bool,
    pub selected_index: usize,
    pub custom_text: String,
    pub custom_cursor: usize,
    pub custom_focused: bool,
    pub inspecting_evidence: bool,
    pub submitted: bool,
    pub outcome: Option<DecisionResult>,
}

impl DecisionModalState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_id: impl Into<String>,
        question_id: impl Into<String>,
        plan_revision: u64,
        title: impl Into<String>,
        question: impl Into<String>,
        materiality: impl Into<String>,
        evidence_refs: Vec<String>,
        options: Vec<DecisionModalOption>,
        allow_custom: bool,
        custom_only: bool,
    ) -> Self {
        let custom_focused = custom_only || (options.is_empty() && allow_custom);
        Self {
            request_id: request_id.into(),
            question_id: question_id.into(),
            plan_revision,
            title: title.into(),
            question: question.into(),
            materiality: materiality.into(),
            evidence_refs,
            options,
            allow_custom,
            custom_only,
            selected_index: 0,
            custom_text: String::new(),
            custom_cursor: 0,
            custom_focused,
            inspecting_evidence: false,
            submitted: false,
            outcome: None,
        }
    }

    pub fn select_number(&mut self, number: usize) {
        if number > 0 && number <= self.options.len() {
            self.selected_index = number - 1;
            self.custom_focused = false;
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.custom_only {
            self.custom_focused = true;
            return;
        }
        if self.custom_focused {
            if delta < 0 && !self.options.is_empty() {
                self.custom_focused = false;
                self.selected_index = self.options.len() - 1;
            }
            return;
        }
        if self.options.is_empty() {
            if self.allow_custom {
                self.custom_focused = true;
            }
            return;
        }
        let max_idx = self.options.len() - 1;
        if delta > 0 && self.selected_index == max_idx && self.allow_custom {
            self.custom_focused = true;
        } else {
            let next = (self.selected_index as isize + delta).clamp(0, max_idx as isize);
            self.selected_index = next as usize;
        }
    }

    pub fn toggle_custom(&mut self) {
        if self.allow_custom && !self.custom_only {
            self.custom_focused = !self.custom_focused;
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if self.custom_cursor <= self.custom_text.len() {
            self.custom_text.insert(self.custom_cursor, c);
            self.custom_cursor += c.len_utf8();
        }
    }

    pub fn backspace(&mut self) {
        if self.custom_cursor > 0 {
            let prev = self.custom_text[..self.custom_cursor]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            self.custom_text.drain(prev..self.custom_cursor);
            self.custom_cursor = prev;
        }
    }

    pub fn toggle_inspect(&mut self) {
        self.inspecting_evidence = !self.inspecting_evidence;
    }

    pub fn is_valid(&self) -> bool {
        if self.submitted {
            return false;
        }
        if self.custom_focused || self.custom_only {
            !self.custom_text.trim().is_empty()
        } else {
            self.selected_index < self.options.len()
        }
    }

    pub fn commit(&mut self) -> Option<DecisionResult> {
        if self.submitted || !self.is_valid() {
            return None;
        }
        self.submitted = true;
        let res = if self.custom_focused || self.custom_only {
            DecisionResult::Custom(self.custom_text.trim().to_string())
        } else {
            DecisionResult::Choice(self.options[self.selected_index].id.clone())
        };
        self.outcome = Some(res.clone());
        Some(res)
    }

    pub fn defer(&mut self) -> Option<DecisionResult> {
        if self.submitted {
            return None;
        }
        self.submitted = true;
        self.outcome = Some(DecisionResult::Defer);
        Some(DecisionResult::Defer)
    }

    pub fn cancel(&mut self) -> Option<DecisionResult> {
        if self.submitted {
            return None;
        }
        self.submitted = true;
        self.outcome = Some(DecisionResult::Cancel);
        Some(DecisionResult::Cancel)
    }
}

pub fn lines(state: &DecisionModalState, width: u16, th: &Theme) -> Vec<Line<'static>> {
    let inset = 4u16.min(width.saturating_sub(40) / 2);
    let inner = width.saturating_sub(inset * 2).saturating_sub(4);
    let mut body = Vec::new();

    body.extend(section_detail(inner, th, &state.question));
    body.extend(section_detail(
        inner,
        th,
        &format!("Materiality: {}", state.materiality),
    ));

    if state.inspecting_evidence {
        body.extend(section_detail(inner, th, "--- Supporting Evidence ---"));
        if state.evidence_refs.is_empty() {
            body.extend(section_detail(inner, th, "No evidence refs attached"));
        } else {
            for ev in &state.evidence_refs {
                body.extend(section_detail(inner, th, &format!("• {ev}")));
            }
        }
        body.extend(section_detail(inner, th, "---------------------------"));
    }

    if !state.custom_only {
        for (idx, opt) in state.options.iter().enumerate() {
            let focused = !state.custom_focused && idx == state.selected_index;
            let mut label = format!("{}. {}", idx + 1, opt.label);
            if opt.recommended {
                label.push_str(" [Recommended]");
            }
            body.push(section_row(inner, th, focused, &label, ""));
            if focused {
                body.extend(section_detail(inner, th, &opt.explanation));
            }
        }
    }

    if state.allow_custom {
        let focused = state.custom_focused;
        let mut custom_display = if state.custom_text.is_empty() {
            "(Type custom response...)".to_string()
        } else {
            state.custom_text.clone()
        };
        if focused {
            custom_display.push('▏');
        }
        body.push(section_row(
            inner,
            th,
            focused,
            &format!("Custom: {custom_display}"),
            "",
        ));
    }

    let inspect_hint = if state.inspecting_evidence {
        "i hide evidence"
    } else {
        "i inspect evidence"
    };

    body.push(hint_row(
        inner,
        &[
            super::sheet::hint(th, "enter confirm"),
            super::sheet::hint(th, "d defer"),
            super::sheet::hint(th, "↑↓/1-N select"),
            super::sheet::hint(th, inspect_hint),
        ],
        Some("esc cancel"),
        th,
    ));

    let title_span = span(format!("Question · {}", state.title), th.primary);
    Surface::section(width, th)
        .inset(inset)
        .title(vec![title_span])
        .rows(body.into_iter().map(|r| r.spans).collect())
        .lines()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};

    #[test]
    fn f02_enter_required() {
        assert!(!commits_decision("highlight", true));
        assert!(!commits_decision("enter", false));
        assert!(commits_decision("enter", true));
    }

    #[test]
    fn f02_arrow_and_digit_selection_never_answer() {
        let mut state = DecisionModalState::new(
            "req-1",
            "q-1",
            1,
            "Persistence",
            "Which database?",
            "Architectural boundary",
            vec!["db.rs".into()],
            vec![
                DecisionModalOption {
                    id: "sqlite".into(),
                    label: "SQLite".into(),
                    explanation: "Local file".into(),
                    recommended: true,
                },
                DecisionModalOption {
                    id: "memory".into(),
                    label: "Memory".into(),
                    explanation: "RAM only".into(),
                    recommended: false,
                },
            ],
            true,
            false,
        );

        // Digit selection changes selected_index, but does NOT submit or answer
        state.select_number(2);
        assert_eq!(state.selected_index, 1);
        assert!(!state.submitted);

        // Arrow movement changes selection, never answers
        state.move_selection(-1);
        assert_eq!(state.selected_index, 0);
        assert!(!state.submitted);

        // Explicit Enter commits
        assert!(commits_decision("enter", state.is_valid()));
        let result = state.commit();
        assert_eq!(result, Some(DecisionResult::Choice("sqlite".into())));
        assert!(state.submitted);
    }

    #[test]
    fn f02_enter_repeats_only_once() {
        let mut state = DecisionModalState::new(
            "req-1",
            "q-1",
            1,
            "Persistence",
            "Which database?",
            "Materiality",
            vec![],
            vec![DecisionModalOption {
                id: "sqlite".into(),
                label: "SQLite".into(),
                explanation: "Local file".into(),
                recommended: false,
            }],
            false,
            false,
        );

        let first = state.commit();
        assert_eq!(first, Some(DecisionResult::Choice("sqlite".into())));

        // Second Enter attempt repeats once and returns None
        let second = state.commit();
        assert_eq!(second, None);
        assert!(!state.is_valid());
    }

    #[test]
    fn f02_esc_cancels_and_defer_is_distinct() {
        let mut cancel_state = DecisionModalState::new(
            "req-1",
            "q-1",
            1,
            "Question",
            "Body",
            "Materiality",
            vec![],
            vec![],
            false,
            false,
        );
        assert_eq!(cancel_state.cancel(), Some(DecisionResult::Cancel));
        assert_eq!(cancel_state.cancel(), None);

        let mut defer_state = DecisionModalState::new(
            "req-2",
            "q-2",
            1,
            "Question",
            "Body",
            "Materiality",
            vec![],
            vec![],
            false,
            false,
        );
        assert_eq!(defer_state.defer(), Some(DecisionResult::Defer));
        assert_eq!(defer_state.defer(), None);
    }

    #[test]
    fn f02_resize_preserves_custom_draft() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let mut state = DecisionModalState::new(
            "req-1",
            "q-1",
            1,
            "Custom question",
            "Enter custom",
            "Materiality",
            vec!["spec.rs".into()],
            vec![],
            true,
            true,
        );

        state.insert_char('H');
        state.insert_char('i');
        assert_eq!(state.custom_text, "Hi");

        // Render at width 80
        let lines_80 = lines(&state, 80, &theme);
        assert!(!lines_80.is_empty());

        // Resize to width 120 and re-render
        let lines_120 = lines(&state, 120, &theme);
        assert!(!lines_120.is_empty());

        // Custom text draft is completely preserved across resize
        assert_eq!(state.custom_text, "Hi");
        assert_eq!(state.commit(), Some(DecisionResult::Custom("Hi".into())));
    }

    #[test]
    fn f02_recommendation_is_visibly_marked_but_never_auto_submitted() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let state = DecisionModalState::new(
            "req-1",
            "q-1",
            1,
            "Approach",
            "Which approach?",
            "Performance impact",
            vec!["core.rs".into()],
            vec![DecisionModalOption {
                id: "fast".into(),
                label: "Fast path".into(),
                explanation: "Optimized".into(),
                recommended: true,
            }],
            false,
            false,
        );

        let rendered_lines = lines(&state, 100, &theme);
        let text: String = rendered_lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(
            text.contains("[Recommended]"),
            "Recommendation must be visibly marked in text"
        );
        assert!(!state.submitted);
    }

    #[test]
    fn f02_modal_traps_permission_cycle_key() {
        assert_eq!(decision_key(true, "shift_tab", false), "consume");
        assert_eq!(decision_key(true, "escape", false), "cancel");
        assert_eq!(decision_key(true, "enter", false), "confirm");
        assert_eq!(decision_key(true, "d", false), "defer");
        assert_eq!(decision_key(true, "i", false), "inspect");
        assert_eq!(decision_key(false, "shift_tab", false), "delegate");
    }
}
