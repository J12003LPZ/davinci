//! Login dialog matching TS `login-dialog.ts`.

use crate::render::Component;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthInfoLink {
    pub label: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeInfo {
    pub verification_uri: String,
    pub user_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginDialogAction {
    None,
    Cancel,
    Submit(String),
}

#[derive(Debug, Clone)]
pub struct LoginDialog {
    pub title: String,
    pub lines: Vec<String>,
    pub input: String,
    pub input_enabled: bool,
    pub cancelled: bool,
    pub complete_message: Option<String>,
    pub opened_browser: Option<String>,
}

impl LoginDialog {
    pub fn new(
        provider_id: &str,
        provider_name: Option<&str>,
        title_override: Option<&str>,
    ) -> Self {
        let provider_name = provider_name.unwrap_or(provider_id);
        let title = title_override
            .map(str::to_string)
            .unwrap_or_else(|| format!("Login to {provider_name}"));
        Self {
            title,
            lines: Vec::new(),
            input: String::new(),
            input_enabled: false,
            cancelled: false,
            complete_message: None,
            opened_browser: None,
        }
    }

    pub fn click_hint() -> &'static str {
        if cfg!(target_os = "macos") {
            "Cmd+click to open"
        } else {
            "Ctrl+click to open"
        }
    }

    pub fn osc8(url: &str, text: &str) -> String {
        format!("\x1b]8;;{url}\x07{text}\x1b]8;;\x07")
    }

    pub fn show_auth(&mut self, url: &str, instructions: Option<&str>) {
        self.lines.clear();
        self.lines.push(Self::osc8(url, url));
        self.lines.push(Self::osc8(url, Self::click_hint()));
        if let Some(instructions) = instructions {
            self.lines.push(instructions.to_string());
        }
        self.opened_browser = Some(crate::open_browser::open_browser(url));
    }

    pub fn show_device_code(&mut self, info: &DeviceCodeInfo) {
        self.lines.clear();
        self.lines
            .push(Self::osc8(&info.verification_uri, &info.verification_uri));
        self.lines
            .push(Self::osc8(&info.verification_uri, Self::click_hint()));
        self.lines.push(format!("Enter code: {}", info.user_code));
    }

    pub fn show_manual_input(&mut self, prompt: &str) {
        self.input.clear();
        self.input_enabled = true;
        self.lines.push(prompt.to_string());
        self.lines.push("(escape to cancel)".into());
    }

    pub fn show_prompt(&mut self, message: &str, placeholder: Option<&str>) {
        self.input.clear();
        self.input_enabled = true;
        self.lines.push(message.to_string());
        if let Some(placeholder) = placeholder {
            self.lines.push(format!("e.g., {placeholder}"));
        }
        self.lines
            .push("(escape to cancel, enter to submit)".into());
    }

    pub fn show_details(&mut self, lines: &[String]) {
        self.lines.clear();
        self.lines.extend(lines.iter().cloned());
    }

    pub fn show_info(&mut self, message: &str, links: &[AuthInfoLink], show_close_hint: bool) {
        self.lines.push(message.to_string());
        for link in links {
            let text = match &link.label {
                Some(label) => format!("{}: {}", label, link.url),
                None => link.url.clone(),
            };
            self.lines.push(Self::osc8(&link.url, &text));
        }
        if show_close_hint {
            self.lines.push("(escape to close)".into());
        }
    }

    pub fn show_waiting(&mut self, message: &str) {
        self.lines.push(message.to_string());
        self.lines.push("(escape to cancel)".into());
    }

    pub fn show_progress(&mut self, message: &str) {
        self.lines.push(message.to_string());
    }

    pub fn cancel(&mut self) -> LoginDialogAction {
        self.cancelled = true;
        self.complete_message = Some("Login cancelled".into());
        LoginDialogAction::Cancel
    }

    pub fn handle_key(&mut self, data: &str) -> LoginDialogAction {
        if data == "\x1b" {
            return self.cancel();
        }
        if !self.input_enabled {
            return LoginDialogAction::None;
        }
        match data {
            "\r" | "\n" => {
                let value = self.input.clone();
                self.input_enabled = false;
                self.lines.push(format!("> {value}"));
                LoginDialogAction::Submit(value)
            }
            "\x7f" | "\x08" => {
                self.input.pop();
                LoginDialogAction::None
            }
            other if other.chars().all(|ch| !ch.is_control()) => {
                self.input.push_str(other);
                LoginDialogAction::None
            }
            _ => LoginDialogAction::None,
        }
    }
}

impl Component for LoginDialog {
    fn render(&self, _width: usize) -> Vec<String> {
        let mut lines = vec![self.title.clone()];
        lines.extend(self.lines.iter().cloned());
        if self.input_enabled {
            lines.push(format!("> {}", self.input));
        }
        if let Some(message) = &self.complete_message {
            lines.push(message.clone());
        }
        lines
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_dialog_matches_ts_chrome() {
        let mut dialog = LoginDialog::new("openai", None, None);
        assert_eq!(dialog.title, "Login to openai");
        std::env::set_var("PI_OPEN_BROWSER_DRY_RUN", "1");
        dialog.show_auth("https://example.test/auth", Some("Open the URL"));
        let rendered = dialog.render(80).join("\n");
        assert!(rendered.contains(
            "\x1b]8;;https://example.test/auth\x07https://example.test/auth\x1b]8;;\x07"
        ));
        assert!(rendered.contains(LoginDialog::click_hint()));
        assert!(rendered.contains("Ctrl+click to open") || rendered.contains("Cmd+click to open"));
        assert!(rendered.contains("Open the URL"));
        std::env::set_var("PI_OPEN_BROWSER_DRY_RUN", "1");
        let mut launched = LoginDialog::new("openai", None, None);
        launched.show_auth("https://example.test/auth", None);
        assert!(launched
            .opened_browser
            .as_deref()
            .is_some_and(|cmd| cmd.contains("https://example.test/auth")));
        std::env::remove_var("PI_OPEN_BROWSER_DRY_RUN");
        dialog.show_device_code(&DeviceCodeInfo {
            verification_uri: "https://example.test/device".into(),
            user_code: "ABCD-1234".into(),
        });
        assert!(dialog
            .render(80)
            .join("\n")
            .contains("Enter code: ABCD-1234"));
        dialog.show_waiting("Waiting for authorization...");
        dialog.show_progress("exchanging code");
        dialog.show_info(
            "Visit the docs",
            &[AuthInfoLink {
                label: Some("Docs".into()),
                url: "https://example.test/docs".into(),
            }],
            true,
        );
        let info = dialog.render(80).join("\n");
        assert!(info.contains("Visit the docs"));
        assert!(info.contains("escape to close"));
        dialog.show_manual_input("Paste the redirect URL");
        assert_eq!(
            dialog.handle_key("pi-fixture-code"),
            LoginDialogAction::None
        );
        assert_eq!(
            dialog.handle_key("\r"),
            LoginDialogAction::Submit("pi-fixture-code".into())
        );
        let mut cancel = LoginDialog::new("anthropic", Some("Anthropic"), Some("Custom title"));
        assert_eq!(cancel.title, "Custom title");
        assert_eq!(cancel.handle_key("\x1b"), LoginDialogAction::Cancel);
        assert_eq!(cancel.complete_message.as_deref(), Some("Login cancelled"));
    }
}
