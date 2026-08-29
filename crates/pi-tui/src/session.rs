//! Interactive raw-mode session matching TS `ProcessTerminal` + `tui-alt-screen.ts`.

use crate::chrome::ChatChrome;
use crate::keys::decode_kitty_printable;
use crate::mouse::{parse_mouse_sgr, MouseKind, MOUSE_DISABLE, MOUSE_ENABLE};
use crate::overlay::Overlay;
use crate::render::Component;
use crate::themes::Theme;
use crate::{SelectList, ALT_BUFFER_ENTER, ALT_BUFFER_LEAVE};

pub const DISABLE_AUTOWRAP: &str = "\x1b[?7l";
pub const ENABLE_AUTOWRAP: &str = "\x1b[?7h";
pub const BRACKETED_PASTE_ENABLE: &str = "\x1b[?2004h";
pub const BRACKETED_PASTE_DISABLE: &str = "\x1b[?2004l";
/// TS `KITTY_KEYBOARD_PROTOCOL_QUERY` with flags 7: `\x1b[>7u\x1b[?u\x1b[c`
pub const KITTY_KEYBOARD_QUERY: &str = "\x1b[>7u\x1b[?u\x1b[c";
pub const KITTY_KEYBOARD_DISABLE: &str = "\x1b[<u";
pub const BEGIN_SYNCHRONIZED_OUTPUT: &str = "\x1b[?2026h";
pub const END_SYNCHRONIZED_OUTPUT: &str = "\x1b[?2026l";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionAction {
    None,
    Submit(String),
    Abort,
    Quit,
    CycleModel,
    CycleThinking,
    Clear,
    OpenModel,
    SelectModel(String),
    SelectSession(String),
    SelectSetting(String),
    CloseOverlay,
}

#[derive(Debug, Clone)]
pub struct InteractiveSession {
    pub chrome: ChatChrome,
    pub models: Vec<String>,
    pub model_index: usize,
    pub thinking_levels: Vec<String>,
    pub thinking_index: usize,
    pub aborted: bool,
    pub width: usize,
    pub overlay_kind: OverlayKind,
    paste_buf: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    None,
    Model,
    Session,
    Settings,
}

impl InteractiveSession {
    pub fn new(theme: Theme, title: impl Into<String>, models: Vec<String>) -> Self {
        Self {
            chrome: ChatChrome::new(theme, title),
            models,
            model_index: 0,
            thinking_levels: vec![
                "off".into(),
                "minimal".into(),
                "low".into(),
                "medium".into(),
                "high".into(),
            ],
            thinking_index: 0,
            aborted: false,
            width: 80,
            overlay_kind: OverlayKind::None,
            paste_buf: None,
        }
    }

    pub fn enter_sequences(fullscreen: bool) -> String {
        let mut out = String::new();
        if fullscreen {
            out.push_str(ALT_BUFFER_ENTER);
            out.push_str(DISABLE_AUTOWRAP);
        }
        out.push_str(MOUSE_ENABLE);
        out.push_str(BRACKETED_PASTE_ENABLE);
        out.push_str(KITTY_KEYBOARD_QUERY);
        out
    }

    pub fn leave_sequences(fullscreen: bool) -> String {
        let mut out = String::new();
        out.push_str(KITTY_KEYBOARD_DISABLE);
        out.push_str(BRACKETED_PASTE_DISABLE);
        out.push_str(MOUSE_DISABLE);
        if fullscreen {
            out.push_str(ENABLE_AUTOWRAP);
            out.push_str(ALT_BUFFER_LEAVE);
        }
        out
    }

    pub fn current_model(&self) -> Option<&str> {
        self.models.get(self.model_index).map(String::as_str)
    }

    pub fn current_thinking(&self) -> &str {
        self.thinking_levels
            .get(self.thinking_index)
            .map(String::as_str)
            .unwrap_or("off")
    }

    pub fn open_model_overlay(&mut self) {
        self.overlay_kind = OverlayKind::Model;
        self.chrome.selector = Some(SelectList::new(self.models.clone()));
        self.chrome.status = "Select model".into();
    }

    pub fn open_session_overlay(&mut self, sessions: Vec<String>) {
        self.overlay_kind = OverlayKind::Session;
        self.chrome.selector = Some(SelectList::new(sessions));
        self.chrome.status = "Select session".into();
    }

    pub fn open_settings_overlay(&mut self, items: Vec<String>) {
        self.overlay_kind = OverlayKind::Settings;
        self.chrome.selector = Some(SelectList::new(items));
        self.chrome.status = "Settings".into();
    }

    pub fn render_frame(&self) -> String {
        let mut lines = self.chrome.render(self.width);
        if let Some(selector) = &self.chrome.selector {
            let overlay = Overlay::new("model", Box::new(selector.clone()));
            lines.extend(overlay.render(self.width));
        }
        format!(
            "{BEGIN_SYNCHRONIZED_OUTPUT}{}{END_SYNCHRONIZED_OUTPUT}",
            lines.join("\n")
        )
    }

    pub fn handle_line(&mut self, line: &str) -> SessionAction {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "/model"
            || (trimmed.starts_with("/model ") && trimmed["/model ".len()..].is_empty())
        {
            self.open_model_overlay();
            return SessionAction::OpenModel;
        }
        if let Some(rest) = trimmed.strip_prefix("/model ") {
            if !rest.is_empty() {
                return SessionAction::SelectModel(rest.to_string());
            }
        }
        self.chrome.editor.handle_input(trimmed);
        let submitted = self.chrome.editor.submit();
        if submitted.is_empty() {
            SessionAction::None
        } else {
            SessionAction::Submit(submitted)
        }
    }

    pub fn handle_bytes(&mut self, data: &str) -> SessionAction {
        if data.is_empty() {
            return SessionAction::None;
        }
        if let Some(mut buf) = self.paste_buf.take() {
            if let Some(end) = data.find("\x1b[201~") {
                buf.push_str(&data[..end]);
                self.chrome.editor.handle_input(&buf);
                let rest = &data[end + "\x1b[201~".len()..];
                if rest.is_empty() {
                    return SessionAction::None;
                }
                return self.handle_bytes(rest);
            }
            buf.push_str(data);
            self.paste_buf = Some(buf);
            return SessionAction::None;
        }
        if let Some(rest) = data.strip_prefix("\x1b[200~") {
            if let Some(end) = rest.find("\x1b[201~") {
                self.chrome.editor.handle_input(&rest[..end]);
                return self.handle_bytes(&rest[end + "\x1b[201~".len()..]);
            }
            self.paste_buf = Some(rest.to_string());
            return SessionAction::None;
        }
        if let Some(mouse) = parse_mouse_sgr(data) {
            if mouse.kind == MouseKind::Down || mouse.kind == MouseKind::ScrollDown {
                self.chrome.apply_mouse(mouse.y, self.width);
            }
            if mouse.kind == MouseKind::ScrollUp {
                if let Some(selector) = &mut self.chrome.selector {
                    selector.move_by(-1);
                }
            }
            if mouse.kind == MouseKind::ScrollDown {
                if let Some(selector) = &mut self.chrome.selector {
                    selector.move_by(1);
                }
            }
            return SessionAction::None;
        }
        if let Some(text) = decode_kitty_printable(data) {
            return self.handle_printable(&text);
        }
        match data {
            "\x03" => SessionAction::Quit,
            "\x10" => {
                self.cycle_model();
                SessionAction::CycleModel
            }
            "\x14" => {
                self.cycle_thinking();
                SessionAction::CycleThinking
            }
            "\x0c" => {
                self.chrome.transcript.lines.clear();
                SessionAction::Clear
            }
            "\x1b" => self.handle_escape(),
            "\r" | "\n" => self.handle_enter(),
            "\x1b[A" => {
                if let Some(selector) = &mut self.chrome.selector {
                    selector.move_by(-1);
                }
                SessionAction::None
            }
            "\x1b[B" => {
                if let Some(selector) = &mut self.chrome.selector {
                    selector.move_by(1);
                }
                SessionAction::None
            }
            other => self.handle_printable(other),
        }
    }

    fn handle_escape(&mut self) -> SessionAction {
        if self.chrome.selector.is_some() {
            self.chrome.selector = None;
            self.overlay_kind = OverlayKind::None;
            self.chrome.status.clear();
            return SessionAction::CloseOverlay;
        }
        self.aborted = true;
        SessionAction::Abort
    }

    fn handle_enter(&mut self) -> SessionAction {
        if let Some(selector) = &self.chrome.selector {
            if let Some(item) = selector.selected_item() {
                let item = item.to_string();
                let kind = self.overlay_kind;
                self.chrome.selector = None;
                self.overlay_kind = OverlayKind::None;
                self.chrome.status.clear();
                return match kind {
                    OverlayKind::Session => SessionAction::SelectSession(item),
                    OverlayKind::Settings => SessionAction::SelectSetting(item),
                    OverlayKind::Model | OverlayKind::None => {
                        if let Some(index) = self.models.iter().position(|m| m == &item) {
                            self.model_index = index;
                        }
                        SessionAction::SelectModel(item)
                    }
                };
            }
            self.chrome.selector = None;
            self.overlay_kind = OverlayKind::None;
            return SessionAction::CloseOverlay;
        }
        let submitted = self.chrome.editor.submit();
        if submitted == "/model" {
            self.open_model_overlay();
            return SessionAction::OpenModel;
        }
        if let Some(rest) = submitted.strip_prefix("/model ") {
            if !rest.is_empty() {
                return SessionAction::SelectModel(rest.to_string());
            }
            self.open_model_overlay();
            return SessionAction::OpenModel;
        }
        if submitted.is_empty() {
            SessionAction::None
        } else {
            SessionAction::Submit(submitted)
        }
    }

    fn handle_printable(&mut self, data: &str) -> SessionAction {
        if self.chrome.selector.is_some() {
            self.chrome.handle_input(data);
            return SessionAction::None;
        }
        self.chrome.editor.handle_input(data);
        SessionAction::None
    }

    fn cycle_model(&mut self) {
        if self.models.is_empty() {
            return;
        }
        self.model_index = (self.model_index + 1) % self.models.len();
        if let Some(model) = self.current_model() {
            self.chrome.status = format!("model={model}");
        }
    }

    fn cycle_thinking(&mut self) {
        self.thinking_index = (self.thinking_index + 1) % self.thinking_levels.len();
        self.chrome.status = format!("thinking={}", self.current_thinking());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin_themes;

    #[test]
    fn enter_leave_match_ts_sequences() {
        let enter = InteractiveSession::enter_sequences(true);
        assert!(enter.contains(ALT_BUFFER_ENTER));
        assert!(enter.contains("\x1b[?1003h"));
        assert!(enter.contains("\x1b[?1004h"));
        assert!(enter.contains(BRACKETED_PASTE_ENABLE));
        assert!(enter.contains(KITTY_KEYBOARD_QUERY));
        let leave = InteractiveSession::leave_sequences(true);
        assert!(leave.contains(KITTY_KEYBOARD_DISABLE));
        assert!(leave.contains(BRACKETED_PASTE_DISABLE));
        assert!(leave.contains(MOUSE_DISABLE));
        assert!(leave.contains(ALT_BUFFER_LEAVE));
    }

    #[test]
    fn handles_keys_overlays_mouse_and_kitty_without_tty() {
        let theme = builtin_themes().into_iter().next().expect("theme");
        let mut session = InteractiveSession::new(
            theme,
            "pi 0.84.4",
            vec!["google/gemini".into(), "anthropic/sonnet".into()],
        );
        assert_eq!(session.handle_bytes("\x10"), SessionAction::CycleModel);
        assert_eq!(session.current_model(), Some("anthropic/sonnet"));
        assert_eq!(session.handle_bytes("\x14"), SessionAction::CycleThinking);
        assert_eq!(session.current_thinking(), "minimal");
        assert_eq!(session.handle_bytes("\x1b"), SessionAction::Abort);
        assert!(session.aborted);
        session.aborted = false;
        assert_eq!(session.handle_line("/model"), SessionAction::OpenModel);
        assert!(session.chrome.selector.is_some());
        assert!(session.render_frame().contains('┌'));
        assert_eq!(session.handle_bytes("\x1b[B"), SessionAction::None);
        assert_eq!(
            session.handle_bytes("\r"),
            SessionAction::SelectModel("anthropic/sonnet".into())
        );
        assert!(session.chrome.selector.is_none());
        session.open_model_overlay();
        assert_eq!(session.handle_bytes("\x1b"), SessionAction::CloseOverlay);
        session.open_session_overlay(vec!["abc".into(), "def".into()]);
        assert_eq!(
            session.handle_bytes("\r"),
            SessionAction::SelectSession("abc".into())
        );
        session.open_settings_overlay(vec!["theme: dark".into()]);
        assert_eq!(
            session.handle_bytes("\r"),
            SessionAction::SelectSetting("theme: dark".into())
        );
        session.chrome.editor.handle_input("hi");
        assert_eq!(
            session.handle_bytes("\r"),
            SessionAction::Submit("hi".into())
        );
        assert_eq!(session.handle_bytes("\x03"), SessionAction::Quit);
        assert_eq!(session.handle_bytes("\x1b[<0;1;2M"), SessionAction::None);
        assert_eq!(session.handle_bytes("\u{1b}[57399u"), SessionAction::None);
        assert!(session.chrome.editor.buffer.contains('0'));
        assert_eq!(
            session.handle_bytes("\x1b[200~pasted\x1b[201~"),
            SessionAction::None
        );
        assert!(session.chrome.editor.buffer.contains("pasted"));
        assert!(session.render_frame().contains(BEGIN_SYNCHRONIZED_OUTPUT));
    }
}
