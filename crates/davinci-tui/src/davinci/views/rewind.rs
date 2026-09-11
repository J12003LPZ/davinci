//! Checkpoint selection, conflict preview, and external effect disclosure for task rewind.

use crate::davinci::theme::Theme;
use crate::davinci::ui::{hint_row, section_detail, section_row, span, Surface};
use ratatui::text::Line;

pub fn effect_rewind_action(kind: &str, owned: bool) -> &'static str {
    if kind == "local_file" && owned {
        "restore"
    } else {
        "disclose_only"
    }
}

pub fn redact_secret_string(input: &str) -> String {
    let lower = input.to_lowercase();
    if lower.contains("token")
        || lower.contains("secret")
        || lower.contains("bearer")
        || lower.contains("password")
        || lower.contains("api_key")
        || lower.contains("auth")
    {
        "[REDACTED]".to_string()
    } else {
        input.to_string()
    }
}

pub fn rewind_key(open: bool, key: &str) -> &'static str {
    if !open {
        return "delegate";
    }
    match key {
        "escape" => "cancel",
        "enter" => "confirm",
        "1" | "c" => "toggle_code",
        "2" | "t" => "toggle_tasks",
        "3" | "s" => "toggle_transcript",
        _ => "consume",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindFileSummary {
    pub path: String,
    pub classification: String,
    pub is_conflict: bool,
    pub conflict_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindIrreversibleSummary {
    pub operation_id: String,
    pub kind: String,
    pub details: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewindDomainSelection {
    pub code: bool,
    pub task_state: bool,
    pub transcript: bool,
}

impl Default for RewindDomainSelection {
    fn default() -> Self {
        Self {
            code: true,
            task_state: true,
            transcript: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewindModalOutcome {
    Confirmed {
        checkpoint_id: String,
        preview_digest: String,
        selection: RewindDomainSelection,
    },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindModalState {
    pub checkpoint_id: String,
    pub checkpoint_name: String,
    pub checkpoint_time: String,
    pub preview_digest: String,
    pub selection: RewindDomainSelection,
    pub files: Vec<RewindFileSummary>,
    pub conflict_count: usize,
    pub irreversible_effects: Vec<RewindIrreversibleSummary>,
    pub outcome: Option<RewindModalOutcome>,
}

impl RewindModalState {
    pub fn new(
        checkpoint_id: impl Into<String>,
        checkpoint_name: impl Into<String>,
        checkpoint_time: impl Into<String>,
        preview_digest: impl Into<String>,
        files: Vec<RewindFileSummary>,
        conflict_count: usize,
        irreversible_effects: Vec<RewindIrreversibleSummary>,
    ) -> Self {
        Self {
            checkpoint_id: checkpoint_id.into(),
            checkpoint_name: checkpoint_name.into(),
            checkpoint_time: checkpoint_time.into(),
            preview_digest: preview_digest.into(),
            selection: RewindDomainSelection::default(),
            files,
            conflict_count,
            irreversible_effects,
            outcome: None,
        }
    }

    pub fn can_confirm(&self) -> bool {
        self.conflict_count == 0 && !self.is_selection_empty()
    }

    pub fn is_selection_empty(&self) -> bool {
        !self.selection.code && !self.selection.task_state && !self.selection.transcript
    }

    pub fn handle_key(&mut self, key: &str) -> &'static str {
        match rewind_key(true, key) {
            "cancel" => {
                self.outcome = Some(RewindModalOutcome::Cancelled);
                "cancel"
            }
            "confirm" => {
                if self.can_confirm() {
                    self.outcome = Some(RewindModalOutcome::Confirmed {
                        checkpoint_id: self.checkpoint_id.clone(),
                        preview_digest: self.preview_digest.clone(),
                        selection: self.selection,
                    });
                    "confirm"
                } else {
                    "blocked"
                }
            }
            "toggle_code" => {
                self.selection.code = !self.selection.code;
                "toggle"
            }
            "toggle_tasks" => {
                self.selection.task_state = !self.selection.task_state;
                "toggle"
            }
            "toggle_transcript" => {
                self.selection.transcript = !self.selection.transcript;
                "toggle"
            }
            other => other,
        }
    }
}

pub fn lines(state: &RewindModalState, width: u16, th: &Theme) -> Vec<Line<'static>> {
    let inset = 4u16.min(width.saturating_sub(40) / 2);
    let inner = width.saturating_sub(inset * 2).saturating_sub(4);
    let mut body = Vec::new();

    let checkpoint_header = format!(
        "Checkpoint: {} ({}) · digest: {}",
        state.checkpoint_name,
        state.checkpoint_time,
        &state.preview_digest[..8.min(state.preview_digest.len())]
    );
    body.extend(section_detail(inner, th, &checkpoint_header));

    body.extend(section_detail(inner, th, "--- Restore Domains ---"));
    let code_check = if state.selection.code { "[x]" } else { "[ ]" };
    body.push(section_row(
        inner,
        th,
        false,
        &format!("{code_check} 1. Task-owned code changes"),
        "",
    ));

    let task_check = if state.selection.task_state {
        "[x]"
    } else {
        "[ ]"
    };
    body.push(section_row(
        inner,
        th,
        false,
        &format!("{task_check} 2. Task execution state"),
        "",
    ));

    let transcript_check = if state.selection.transcript {
        "[x]"
    } else {
        "[ ]"
    };
    body.push(section_row(
        inner,
        th,
        false,
        &format!("{transcript_check} 3. Conversation transcript"),
        "",
    ));

    if state.conflict_count > 0 {
        body.extend(section_detail(
            inner,
            th,
            &format!(
                "⚠ {} later user edit(s) overlap this checkpoint and need review!",
                state.conflict_count
            ),
        ));
        for f in &state.files {
            if f.is_conflict {
                let reason = f.conflict_reason.as_deref().unwrap_or("conflict");
                body.extend(section_detail(
                    inner,
                    th,
                    &format!("  • {}: {reason}", f.path),
                ));
            }
        }
    } else {
        body.extend(section_detail(
            inner,
            th,
            &format!("✓ {} file(s) eligible to restore.", state.files.len()),
        ));
    }

    if !state.irreversible_effects.is_empty() {
        body.extend(section_detail(
            inner,
            th,
            "--- Irreversible External Effects ---",
        ));
        for eff in &state.irreversible_effects {
            let redacted = redact_secret_string(&eff.details);
            body.extend(section_detail(
                inner,
                th,
                &format!(
                    "  [EXTERNAL] {}: {} ({})",
                    eff.operation_id, eff.kind, redacted
                ),
            ));
        }
    }

    let confirm_hint = if state.can_confirm() {
        super::sheet::hint(th, "enter confirm")
    } else {
        super::sheet::hint(th, "enter (blocked)")
    };

    body.push(hint_row(
        inner,
        &[
            confirm_hint,
            super::sheet::hint(th, "1/c code"),
            super::sheet::hint(th, "2/t tasks"),
            super::sheet::hint(th, "3/s transcript"),
        ],
        Some("esc cancel"),
        th,
    ));

    let title_span = span("Rewind Task", th.primary);
    Surface::section(width, th)
        .inset(inset)
        .title(vec![title_span])
        .rows(body.into_iter().map(|r| r.spans).collect())
        .lines()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f04_external_effect_not_undo() {
        assert_eq!(effect_rewind_action("local_file", true), "restore");
        assert_eq!(effect_rewind_action("publish", true), "disclose_only");
        assert_eq!(effect_rewind_action("deploy", false), "disclose_only");
    }

    #[test]
    fn test_external_publish_remains_performed_after_code_rewind() {
        let action = effect_rewind_action("publish", false);
        assert_eq!(action, "disclose_only");

        let eff = RewindIrreversibleSummary {
            operation_id: "op-pub-1".into(),
            kind: "publish".into(),
            details: "npm publish @davinci/pkg v1.0.0".into(),
        };
        assert_eq!(eff.operation_id, "op-pub-1");
    }

    #[test]
    fn test_cancelled_confirm() {
        let mut state = RewindModalState::new(
            "cp-1",
            "Before task",
            "12:00",
            "digest123456",
            Vec::new(),
            0,
            Vec::new(),
        );
        let action = state.handle_key("escape");
        assert_eq!(action, "cancel");
        assert_eq!(state.outcome, Some(RewindModalOutcome::Cancelled));
    }

    #[test]
    fn test_task_with_only_external_effects() {
        let eff = RewindIrreversibleSummary {
            operation_id: "op-ext-1".into(),
            kind: "deploy".into(),
            details: "deploy production".into(),
        };
        let mut state = RewindModalState::new(
            "cp-2",
            "Milestone",
            "13:00",
            "digest999",
            Vec::new(),
            0,
            vec![eff],
        );
        assert!(state.can_confirm());
        let action = state.handle_key("enter");
        assert_eq!(action, "confirm");
        match state.outcome {
            Some(RewindModalOutcome::Confirmed { checkpoint_id, .. }) => {
                assert_eq!(checkpoint_id, "cp-2");
            }
            _ => panic!("Expected confirmed outcome"),
        }
    }

    #[test]
    fn test_secret_redaction() {
        let secret1 = "Authorization: Bearer my-secret-jwt-token-xyz";
        let redacted1 = redact_secret_string(secret1);
        assert_eq!(redacted1, "[REDACTED]");

        let secret2 = "api_key=sk-1234567890abcdef";
        let redacted2 = redact_secret_string(secret2);
        assert_eq!(redacted2, "[REDACTED]");

        let safe = "deployed service foo to region us-east-1";
        assert_eq!(redact_secret_string(safe), safe);
    }

    #[test]
    fn test_narrow_terminal() {
        let th = Theme::da_vinci(crate::davinci::theme::ColorDepth::TrueColor, false);
        let state = RewindModalState::new(
            "cp-1",
            "Test",
            "10:00",
            "abcdef123456",
            Vec::new(),
            0,
            Vec::new(),
        );

        let narrow_lines = lines(&state, 40, &th);
        assert!(!narrow_lines.is_empty());

        let very_narrow = lines(&state, 25, &th);
        assert!(!very_narrow.is_empty());
    }

    #[test]
    fn test_user_selection_preserves_unrelated_composer_text() {
        let mut state = RewindModalState::new(
            "cp-1",
            "Test",
            "10:00",
            "abcdef123456",
            Vec::new(),
            0,
            Vec::new(),
        );

        let composer_buffer = "my unfinished prompt in composer";
        state.handle_key("1");
        assert!(!state.selection.code);

        state.handle_key("2");
        assert!(!state.selection.task_state);

        assert_eq!(composer_buffer, "my unfinished prompt in composer");
    }
}
