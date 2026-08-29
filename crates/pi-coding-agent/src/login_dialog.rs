//! Login dialog that replaces the editor, matching TypeScript `LoginDialogComponent`.

use pi_ai::DeviceCodeInfo;
use pi_tui::keys::Key;
use pi_tui::widgets::{Input, InputAction, CURSOR_MARKER};
use pi_tui::Component;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginDialogAction {
    Continue,
    Submitted(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginDialogKind {
    ApiKey,
    OauthPaste,
    Device,
}

#[derive(Debug, Clone)]
pub struct LoginDialog {
    pub title: String,
    pub provider_id: String,
    pub kind: LoginDialogKind,
    content: Vec<String>,
    input: Input,
    input_visible: bool,
    submitted_value: Option<String>,
    cancel_hint: Option<String>,
    pub oauth_state: Option<String>,
}

impl LoginDialog {
    pub fn new(provider_id: &str, provider_name: &str, kind: LoginDialogKind) -> Self {
        let mut input = Input {
            focused: true,
            ..Input::default()
        };
        input.focused = true;
        Self {
            title: format!("Login to {provider_name}"),
            provider_id: provider_id.to_string(),
            kind,
            content: Vec::new(),
            input,
            input_visible: false,
            submitted_value: None,
            cancel_hint: None,
            oauth_state: None,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn focused(&self) -> bool {
        self.input.focused
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.input.focused = focused;
    }

    pub fn input_value(&self) -> &str {
        self.input.get_value()
    }

    pub fn show_auth(&mut self, url: &str, instructions: Option<&str>) {
        self.content.clear();
        self.content.push(String::new());
        self.content.push(osc_link(url, url));
        let hint = click_hint();
        self.content.push(osc_link(url, hint));
        if let Some(instructions) = instructions {
            self.content.push(String::new());
            self.content.push(instructions.to_string());
        }
    }

    pub fn show_device_code(&mut self, info: &DeviceCodeInfo) {
        self.kind = LoginDialogKind::Device;
        self.content.clear();
        self.content.push(String::new());
        self.content
            .push(osc_link(&info.verification_uri, &info.verification_uri));
        self.content
            .push(osc_link(&info.verification_uri, click_hint()));
        self.content.push(String::new());
        self.content.push(format!("Enter code: {}", info.user_code));
    }

    pub fn show_manual_input(&mut self, prompt: &str) {
        self.input.set_value("");
        self.input.focused = true;
        self.input_visible = true;
        self.content.push(String::new());
        self.content.push(prompt.to_string());
        self.cancel_hint = Some("(escape to cancel)".into());
    }

    pub fn show_prompt(&mut self, message: &str, placeholder: Option<&str>) {
        self.input.set_value("");
        self.input.focused = true;
        self.input_visible = true;
        if let Some(placeholder) = placeholder {
            self.input.placeholder = format!("e.g., {placeholder}");
        }
        self.content.push(String::new());
        self.content.push(message.to_string());
        if let Some(placeholder) = placeholder {
            self.content.push(format!("e.g., {placeholder}"));
        }
        self.cancel_hint = Some("(escape to cancel, enter to submit)".into());
    }

    pub fn show_details(&mut self, lines: &[String]) {
        self.content.clear();
        self.content.push(String::new());
        self.content.extend(lines.iter().cloned());
    }

    pub fn show_info(&mut self, message: &str, links: &[(String, String)], show_close_hint: bool) {
        self.content.push(String::new());
        self.content.push(message.to_string());
        for (label, url) in links {
            let text = if label.is_empty() {
                url.clone()
            } else {
                format!("{label}: {url}")
            };
            self.content.push(osc_link(url, &text));
        }
        if show_close_hint {
            self.content.push(String::new());
            self.cancel_hint = Some("(escape to close)".into());
        }
    }

    pub fn show_waiting(&mut self, message: &str) {
        self.content.push(String::new());
        self.content.push(message.to_string());
        self.cancel_hint = Some("(escape to cancel)".into());
    }

    pub fn show_progress(&mut self, message: &str) {
        self.content.push(message.to_string());
    }

    pub fn render(&self) -> String {
        let mut lines = vec!["─".repeat(40), self.title.clone()];
        lines.extend(self.content.iter().cloned());
        if let Some(value) = &self.submitted_value {
            lines.push(format!("> {value}"));
        } else if self.input_visible {
            let rendered = self.input.render(80);
            if rendered.is_empty() {
                lines.push(if self.input.focused {
                    format!("{CURSOR_MARKER}\x1b[7m \x1b[27m")
                } else {
                    String::new()
                });
            } else {
                lines.extend(rendered);
            }
        }
        if let Some(hint) = &self.cancel_hint {
            lines.push(hint.clone());
        }
        lines.push("─".repeat(40));
        lines.join("\n") + "\n"
    }

    pub fn handle_key(&mut self, key: &Key) -> LoginDialogAction {
        if matches!(key, Key::Escape) {
            return LoginDialogAction::Cancelled;
        }
        if !self.input_visible {
            return LoginDialogAction::Continue;
        }
        match self.input.handle_key(key) {
            InputAction::Submit => {
                let value = self.input.get_value().to_string();
                self.submitted_value = Some(value.clone());
                self.input_visible = false;
                LoginDialogAction::Submitted(value)
            }
            InputAction::Escape => LoginDialogAction::Cancelled,
            InputAction::Continue => LoginDialogAction::Continue,
        }
    }
}

fn click_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+click to open"
    } else {
        "Ctrl+click to open"
    }
}

fn osc_link(url: &str, text: &str) -> String {
    format!("\x1b]8;;{url}\x07{text}\x1b]8;;\x07")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_typescript_login_dialog_chrome() {
        let mut dialog = LoginDialog::new("anthropic", "Anthropic", LoginDialogKind::OauthPaste);
        dialog.show_auth(
            "https://claude.ai/oauth/authorize",
            Some("Complete login in your browser. If the browser is on another machine, paste the final redirect URL here."),
        );
        dialog.show_manual_input(
            "Complete login in your browser, or paste the authorization code / redirect URL here:",
        );
        let rendered = dialog.render();
        assert!(rendered.contains("Login to Anthropic"));
        assert!(rendered.contains("https://claude.ai/oauth/authorize"));
        assert!(rendered.contains("Ctrl+click to open") || rendered.contains("Cmd+click to open"));
        assert!(rendered.contains("escape to cancel"));
        assert!(rendered.contains(CURSOR_MARKER));
        assert!(dialog.focused());
    }

    #[test]
    fn device_code_and_waiting_match_typescript() {
        let mut dialog =
            LoginDialog::new("github-copilot", "GitHub Copilot", LoginDialogKind::Device);
        dialog.show_device_code(&DeviceCodeInfo {
            device_code: "dev".into(),
            user_code: "ABCD-1234".into(),
            verification_uri: "https://github.com/login/device".into(),
            interval_seconds: Some(5.0),
            expires_in_seconds: 900.0,
        });
        dialog.show_waiting("Waiting for authentication...");
        let rendered = dialog.render();
        assert!(rendered.contains("Enter code: ABCD-1234"));
        assert!(rendered.contains("https://github.com/login/device"));
        assert!(rendered.contains("Waiting for authentication..."));
        assert!(rendered.contains("escape to cancel"));
    }

    #[test]
    fn input_submit_and_escape_replace_editor() {
        let mut dialog = LoginDialog::new("anthropic", "Anthropic", LoginDialogKind::ApiKey);
        dialog.show_prompt("Enter API key", None);
        assert_eq!(
            dialog.handle_key(&Key::Char('s')),
            LoginDialogAction::Continue
        );
        assert_eq!(
            dialog.handle_key(&Key::Char('k')),
            LoginDialogAction::Continue
        );
        match dialog.handle_key(&Key::Enter) {
            LoginDialogAction::Submitted(value) => assert_eq!(value, "sk"),
            other => panic!("{other:?}"),
        }
        assert!(dialog.render().contains("> sk"));
        let mut dialog = LoginDialog::new("anthropic", "Anthropic", LoginDialogKind::ApiKey);
        dialog.show_prompt("Enter API key", None);
        assert_eq!(
            dialog.handle_key(&Key::Escape),
            LoginDialogAction::Cancelled
        );
    }
}
