use std::fmt;

use ratatui::text::Line;
use zeroize::{Zeroize, Zeroizing};

use crate::davinci::model::Model;
use crate::davinci::ui::{self, section_detail, span, Surface};

pub const MAX_SECRET_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretInputStatus {
    EnterKey,
    Validating,
}

pub struct SecretInputState {
    buffer: Zeroizing<String>,
    pub status: SecretInputStatus,
}

impl Clone for SecretInputState {
    fn clone(&self) -> Self {
        Self {
            buffer: Zeroizing::new(self.buffer.to_string()),
            status: self.status,
        }
    }
}

impl fmt::Debug for SecretInputState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretInputState")
            .field("length", &self.buffer.chars().count())
            .field("status", &self.status)
            .finish()
    }
}

impl SecretInputState {
    pub fn new() -> Self {
        Self {
            buffer: Zeroizing::new(String::new()),
            status: SecretInputStatus::EnterKey,
        }
    }

    pub fn insert_text(&mut self, text: &str) {
        if self.status != SecretInputStatus::EnterKey {
            return;
        }
        for character in text.chars() {
            if character.is_control() {
                continue;
            }
            let next_bytes = self.buffer.len().saturating_add(character.len_utf8());
            if next_bytes > MAX_SECRET_BYTES {
                break;
            }
            self.buffer.push(character);
        }
    }

    pub fn backspace(&mut self) {
        if self.status == SecretInputStatus::EnterKey {
            self.buffer.pop();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn masked_len(&self) -> usize {
        self.buffer.chars().count()
    }

    pub fn begin_validation(&mut self) -> Option<String> {
        if self.is_empty() || self.status != SecretInputStatus::EnterKey {
            return None;
        }
        self.status = SecretInputStatus::Validating;
        let candidate = self.buffer.to_string();
        self.buffer.zeroize();
        Some(candidate)
    }

    pub fn cancel(&mut self) {
        self.buffer.zeroize();
        self.status = SecretInputStatus::EnterKey;
    }
}

impl Default for SecretInputState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let Some(state) = model.secret_input.as_ref() else {
        return Vec::new();
    };
    let inner = model
        .width
        .saturating_sub(model.overlay_inset() * 2)
        .saturating_sub(4);
    let masked = "•".repeat(state.masked_len());
    let status = match state.status {
        SecretInputStatus::EnterKey => {
            "Paste is accepted; voice input is disabled while this field has focus."
        }
        SecretInputStatus::Validating => "Validating credential…",
    };
    let mut body = Vec::new();
    body.extend(section_detail(
        inner,
        th,
        "Enter the TypeSafe API key. It will be validated before saving.",
    ));
    body.extend(section_detail(inner, th, &format!("Key: {masked}")));
    body.extend(section_detail(inner, th, status));
    body.push(ui::hint_row(
        inner,
        &[
            vec![span("enter validate", th.muted)],
            vec![span("esc cancel", th.muted)],
        ],
        Some("ctrl+c cancel"),
        th,
    ));
    Surface::section(model.width, th)
        .inset(model.overlay_inset())
        .title(vec![span("TypeSafe API key", th.primary)])
        .rows(body.into_iter().map(|line| line.spans).collect())
        .lines()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_input_is_masked_and_bounded() {
        let mut state = SecretInputState::new();
        state.insert_text(&"🔑".repeat(MAX_SECRET_BYTES));
        assert!(state.masked_len() > 0);
        assert!(state.buffer.len() <= MAX_SECRET_BYTES);
        let debug = format!("{state:?}");
        assert!(!debug.contains('🔑'));
    }

    #[test]
    fn submit_zeroizes_internal_buffer() {
        let mut state = SecretInputState::new();
        state.insert_text("candidate");
        let candidate = state.begin_validation().expect("candidate");
        assert_eq!(candidate, "candidate");
        assert_eq!(state.masked_len(), 0);
        assert_eq!(state.status, SecretInputStatus::Validating);
    }
}
