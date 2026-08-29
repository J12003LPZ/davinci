//! Interactive raw-mode session matching TS `ProcessTerminal` + `tui-alt-screen.ts`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::autocomplete::{apply_completion, suggestions, SlashCommandSpec};
use crate::chrome::ChatChrome;
use crate::first_time::{FirstTimeAction, FirstTimeSetup};
use crate::image::{delete_all_kitty_images, delete_kitty_image, encode_kitty};
use crate::keys::decode_kitty_printable;
use crate::login_dialog::{LoginDialog, LoginDialogAction};
use crate::mermaid::MermaidMode;
use crate::mouse::{parse_mouse_sgr, MouseKind, MOUSE_DISABLE, MOUSE_ENABLE};
use crate::overlay::Overlay;
use crate::render::Component;
use crate::scoped_models::{EnabledIds, ScopedModel, ScopedModelsAction, ScopedModelsSelector};
use crate::settings::{SettingItem, SettingsList};
use crate::themes::Theme;
use crate::tool_card::ToolCard;
use crate::tree::{FilterMode, SessionTreeNode, TreeAction, TreeSelector};
use crate::{SelectList, ALT_BUFFER_ENTER, ALT_BUFFER_LEAVE};

pub const DOUBLE_ESCAPE_MS: u64 = 500;

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
    CycleSetting,
    OpenTree,
    OpenFork,
    OpenScopedModels,
    OpenLogin,
    SelectTreeEntry(String),
    RunBash(String),
    CloseOverlay,
    FirstTimeSubmit {
        theme: String,
        share_analytics: bool,
    },
    FirstTimeSkip,
    LoginCancelled,
    LoginSubmit(String),
    PersistScopedModels(EnabledIds),
    ChangeScopedModels(EnabledIds),
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
    pub double_escape_action: DoubleEscapeAction,
    pub slash_commands: Vec<SlashCommandSpec>,
    pub cwd: PathBuf,
    pub login_providers: Vec<String>,
    pub autocomplete_max_visible: usize,
    pub tree_filter_mode: FilterMode,
    pub mermaid_mode: MermaidMode,
    pub enabled_model_ids: EnabledIds,
    last_escape: Option<Instant>,
    next_image_id: u32,
    paste_buf: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubleEscapeAction {
    Tree,
    Fork,
    None,
}

impl DoubleEscapeAction {
    pub fn parse(value: &str) -> Self {
        match value {
            "fork" => Self::Fork,
            "none" => Self::None,
            _ => Self::Tree,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tree => "tree",
            Self::Fork => "fork",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    None,
    Model,
    Session,
    Settings,
    Tree,
    ScopedModels,
    Login,
    FirstTime,
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
            double_escape_action: DoubleEscapeAction::Tree,
            slash_commands: Vec::new(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            login_providers: Vec::new(),
            autocomplete_max_visible: 8,
            tree_filter_mode: FilterMode::Default,
            mermaid_mode: MermaidMode::Streaming,
            enabled_model_ids: None,
            last_escape: None,
            next_image_id: 1,
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
        self.open_settings_list(SettingsList::new(
            items
                .into_iter()
                .map(|label| SettingItem {
                    id: label.clone(),
                    label,
                    description: None,
                    current_value: String::new(),
                    values: Vec::new(),
                })
                .collect(),
            self.autocomplete_max_visible,
        ));
    }

    pub fn open_settings_list(&mut self, list: SettingsList) {
        self.overlay_kind = OverlayKind::Settings;
        self.chrome.settings_list = Some(list);
        self.chrome.selector = None;
        self.chrome.status = "Settings".into();
    }

    pub fn open_first_time_setup(&mut self, detected_theme: &str, app_name: &str) {
        self.overlay_kind = OverlayKind::FirstTime;
        self.chrome.first_time = Some(FirstTimeSetup::new(detected_theme, app_name));
        self.chrome.status = "First-time setup".into();
    }

    pub fn open_login_dialog(
        &mut self,
        provider_id: &str,
        provider_name: Option<&str>,
        title: Option<&str>,
    ) {
        self.overlay_kind = OverlayKind::Login;
        self.chrome.login_dialog = Some(LoginDialog::new(provider_id, provider_name, title));
        self.chrome.status = format!("Login to {provider_id}");
    }

    pub fn open_tree_overlay(&mut self, roots: Vec<SessionTreeNode>, leaf_id: Option<String>) {
        self.overlay_kind = OverlayKind::Tree;
        self.chrome.tree = Some(TreeSelector::new(
            roots,
            leaf_id,
            self.autocomplete_max_visible.max(8),
            self.tree_filter_mode,
        ));
        self.chrome.status = "Session Tree".into();
    }

    pub fn open_scoped_models(&mut self, models: Vec<ScopedModel>) {
        self.overlay_kind = OverlayKind::ScopedModels;
        self.chrome.scoped_models = Some(ScopedModelsSelector::new(
            models,
            self.enabled_model_ids.clone(),
        ));
        self.chrome.status = "Model Configuration".into();
    }

    pub fn close_overlays(&mut self) {
        self.chrome.selector = None;
        self.chrome.settings_list = None;
        self.chrome.first_time = None;
        self.chrome.login_dialog = None;
        self.chrome.tree = None;
        self.chrome.scoped_models = None;
        self.overlay_kind = OverlayKind::None;
        self.chrome.status.clear();
    }

    fn overlay_open(&self) -> bool {
        self.chrome.first_time.is_some()
            || self.chrome.login_dialog.is_some()
            || self.chrome.tree.is_some()
            || self.chrome.scoped_models.is_some()
            || self.chrome.selector.is_some()
            || self.chrome.settings_list.is_some()
    }

    pub fn push_tool_card(&mut self, card: ToolCard) {
        if let Some(existing) = self
            .chrome
            .tool_cards
            .iter_mut()
            .find(|item| item.tool_call_id == card.tool_call_id)
        {
            *existing = card;
        } else {
            self.chrome.tool_cards.push(card);
        }
    }

    pub fn finish_tool_card(
        &mut self,
        tool_call_id: &str,
        result: &serde_json::Value,
        is_error: bool,
    ) {
        if let Some(card) = self
            .chrome
            .tool_cards
            .iter_mut()
            .find(|item| item.tool_call_id == tool_call_id)
        {
            card.finish(result, is_error);
            for (data, _mime) in card.image_payloads() {
                self.place_kitty_image(&data, None);
            }
        }
    }

    pub fn place_kitty_image(&mut self, base64_data: &str, rows: Option<u32>) -> u32 {
        let image_id = self.next_image_id;
        self.next_image_id = self.next_image_id.saturating_add(1);
        let sequence = encode_kitty(
            base64_data,
            Some(40),
            rows.or(Some(1)),
            Some(image_id),
            false,
        );
        self.chrome.transcript.push("image", sequence);
        image_id
    }

    pub fn remove_kitty_image(&self, image_id: u32) -> String {
        delete_kitty_image(image_id)
    }

    pub fn refresh_autocomplete(&mut self, force_path: bool) {
        let text = self.chrome.editor.buffer.clone();
        let found = suggestions(
            &text,
            &self.slash_commands,
            &self.models,
            &self.thinking_levels,
            &self.login_providers,
            &self.cwd,
            force_path,
        );
        if let Some(mut found) = found {
            found.items.truncate(self.autocomplete_max_visible);
            self.chrome.autocomplete_selected = 0;
            self.chrome.autocomplete = Some(found);
        } else {
            self.chrome.autocomplete = None;
        }
    }

    pub fn accept_autocomplete(&mut self) -> bool {
        let Some(suggestions) = self.chrome.autocomplete.clone() else {
            return false;
        };
        let Some(item) = suggestions.items.get(self.chrome.autocomplete_selected) else {
            return false;
        };
        let cursor = self.chrome.editor.cursor;
        self.chrome.editor.buffer = apply_completion(
            &self.chrome.editor.buffer,
            cursor,
            &suggestions.prefix,
            item,
        );
        self.chrome.editor.cursor = self.chrome.editor.buffer.len();
        self.chrome.autocomplete = None;
        true
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
        if trimmed == "/tree" {
            return SessionAction::OpenTree;
        }
        if trimmed == "/scoped-models" {
            return SessionAction::OpenScopedModels;
        }
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
        if let Some(action) = self.handle_special_overlay(data) {
            return action;
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
                self.chrome.tool_cards.clear();
                let _ = delete_all_kitty_images();
                SessionAction::Clear
            }
            "\x1b" => self.handle_escape(),
            "\t" => self.handle_tab(),
            " " if self.chrome.settings_list.is_some() || self.chrome.scoped_models.is_some() => {
                if let Some(settings) = &mut self.chrome.settings_list {
                    settings.cycle();
                }
                SessionAction::CycleSetting
            }
            "\r" => self.handle_enter(),
            "\n" => {
                if self.overlay_open() {
                    self.handle_enter()
                } else {
                    self.chrome.editor.insert_str("\n");
                    SessionAction::None
                }
            }
            "\x1b[A" => {
                self.move_overlay(-1);
                SessionAction::None
            }
            "\x1b[B" => {
                self.move_overlay(1);
                SessionAction::None
            }
            other => self.handle_printable(other),
        }
    }

    fn handle_tab(&mut self) -> SessionAction {
        if self.accept_autocomplete() {
            return SessionAction::None;
        }
        self.refresh_autocomplete(true);
        if self
            .chrome
            .autocomplete
            .as_ref()
            .is_some_and(|items| items.items.len() == 1)
        {
            self.accept_autocomplete();
        }
        SessionAction::None
    }

    fn handle_special_overlay(&mut self, data: &str) -> Option<SessionAction> {
        if self.chrome.first_time.is_some() {
            let action = self
                .chrome
                .first_time
                .as_mut()
                .expect("first-time overlay")
                .handle_key(data);
            return Some(match action {
                FirstTimeAction::PreviewTheme(theme) => {
                    if let Some(found) = crate::builtin_themes()
                        .into_iter()
                        .find(|item| item.name == theme)
                    {
                        self.chrome.theme = found;
                    }
                    SessionAction::None
                }
                FirstTimeAction::None => SessionAction::None,
                FirstTimeAction::Submit(result) => {
                    self.close_overlays();
                    SessionAction::FirstTimeSubmit {
                        theme: result.theme,
                        share_analytics: result.share_analytics,
                    }
                }
                FirstTimeAction::Cancel => {
                    self.close_overlays();
                    SessionAction::FirstTimeSkip
                }
            });
        }
        if let Some(login) = &mut self.chrome.login_dialog {
            return Some(match login.handle_key(data) {
                LoginDialogAction::None => SessionAction::None,
                LoginDialogAction::Cancel => {
                    self.close_overlays();
                    SessionAction::LoginCancelled
                }
                LoginDialogAction::Submit(value) => SessionAction::LoginSubmit(value),
            });
        }
        if let Some(tree) = &mut self.chrome.tree {
            return Some(match tree.handle_key(data) {
                TreeAction::None => SessionAction::None,
                TreeAction::Select(id) => {
                    self.close_overlays();
                    SessionAction::SelectTreeEntry(id)
                }
                TreeAction::Cancel => {
                    self.close_overlays();
                    SessionAction::CloseOverlay
                }
            });
        }
        if let Some(scoped) = &mut self.chrome.scoped_models {
            return Some(match scoped.handle_key(data) {
                ScopedModelsAction::None => SessionAction::None,
                ScopedModelsAction::Change(ids) => {
                    self.enabled_model_ids = ids.clone();
                    SessionAction::ChangeScopedModels(ids)
                }
                ScopedModelsAction::Persist(ids) => {
                    self.enabled_model_ids = ids.clone();
                    SessionAction::PersistScopedModels(ids)
                }
                ScopedModelsAction::Cancel => {
                    self.close_overlays();
                    SessionAction::CloseOverlay
                }
            });
        }
        None
    }

    fn move_overlay(&mut self, delta: isize) {
        if let Some(setup) = &mut self.chrome.first_time {
            let key = if delta < 0 { "\x1b[A" } else { "\x1b[B" };
            let _ = setup.handle_key(key);
        } else if let Some(tree) = &mut self.chrome.tree {
            let key = if delta < 0 { "\x1b[A" } else { "\x1b[B" };
            let _ = tree.handle_key(key);
        } else if let Some(scoped) = &mut self.chrome.scoped_models {
            let key = if delta < 0 { "\x1b[A" } else { "\x1b[B" };
            let _ = scoped.handle_key(key);
        } else if let Some(settings) = &mut self.chrome.settings_list {
            settings.move_by(delta);
        } else if let Some(selector) = &mut self.chrome.selector {
            selector.move_by(delta);
        } else if let Some(suggestions) = &self.chrome.autocomplete {
            let len = suggestions.items.len() as isize;
            if len > 0 {
                self.chrome.autocomplete_selected =
                    (self.chrome.autocomplete_selected as isize + delta).rem_euclid(len) as usize;
            }
        }
    }

    fn handle_escape(&mut self) -> SessionAction {
        if self.chrome.autocomplete.is_some() {
            self.chrome.autocomplete = None;
            return SessionAction::None;
        }
        if self.overlay_open() {
            self.close_overlays();
            return SessionAction::CloseOverlay;
        }
        if self.chrome.editor.buffer.trim().is_empty()
            && self.double_escape_action != DoubleEscapeAction::None
        {
            let now = Instant::now();
            if self.last_escape.is_some_and(|prev| {
                now.duration_since(prev) < Duration::from_millis(DOUBLE_ESCAPE_MS)
            }) {
                self.last_escape = None;
                return match self.double_escape_action {
                    DoubleEscapeAction::Tree => SessionAction::OpenTree,
                    DoubleEscapeAction::Fork => SessionAction::OpenFork,
                    DoubleEscapeAction::None => SessionAction::Abort,
                };
            }
            self.last_escape = Some(now);
            return SessionAction::None;
        }
        self.aborted = true;
        SessionAction::Abort
    }

    fn handle_enter(&mut self) -> SessionAction {
        if self.accept_autocomplete() {
            return SessionAction::None;
        }
        if let Some(settings) = &self.chrome.settings_list {
            if let Some(item) = settings.selected_item() {
                let value = format!("{}={}", item.id, item.current_value);
                self.chrome.settings_list = None;
                self.overlay_kind = OverlayKind::None;
                self.chrome.status.clear();
                return SessionAction::SelectSetting(value);
            }
            self.chrome.settings_list = None;
            self.overlay_kind = OverlayKind::None;
            return SessionAction::CloseOverlay;
        }
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
                    OverlayKind::Tree => SessionAction::SelectTreeEntry(item),
                    OverlayKind::ScopedModels | OverlayKind::Login | OverlayKind::FirstTime => {
                        SessionAction::CloseOverlay
                    }
                };
            }
            self.chrome.selector = None;
            self.overlay_kind = OverlayKind::None;
            return SessionAction::CloseOverlay;
        }
        let submitted = self.chrome.editor.submit();
        if let Some(command) = submitted.trim_start().strip_prefix('!') {
            return SessionAction::RunBash(command.trim().to_string());
        }
        if submitted == "/scoped-models" {
            return SessionAction::OpenScopedModels;
        }
        if submitted == "/tree" {
            return SessionAction::OpenTree;
        }
        if submitted == "/login" || submitted.starts_with("/login ") {
            return SessionAction::Submit(submitted);
        }
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
        self.refresh_autocomplete(false);
        SessionAction::None
    }

    fn cycle_ids(&self) -> Vec<String> {
        match &self.enabled_model_ids {
            None => self.models.clone(),
            Some(ids) => {
                let filtered: Vec<String> = ids
                    .iter()
                    .filter(|id| self.models.iter().any(|model| model == *id))
                    .cloned()
                    .collect();
                if filtered.is_empty() {
                    self.models.clone()
                } else {
                    filtered
                }
            }
        }
    }

    fn cycle_model(&mut self) {
        let ids = self.cycle_ids();
        if ids.is_empty() {
            return;
        }
        let current = self.current_model().unwrap_or_default().to_string();
        let position = ids.iter().position(|id| id == &current).unwrap_or(0);
        let next = ids[(position + 1) % ids.len()].clone();
        if let Some(index) = self.models.iter().position(|model| model == &next) {
            self.model_index = index;
        }
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
        session.chrome.editor.handle_input("busy");
        assert_eq!(session.handle_bytes("\x1b"), SessionAction::Abort);
        assert!(session.aborted);
        session.chrome.editor.buffer.clear();
        session.double_escape_action = DoubleEscapeAction::Tree;
        session.last_escape = Some(Instant::now());
        assert_eq!(session.handle_bytes("\x1b"), SessionAction::OpenTree);
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
        session.open_settings_list(SettingsList::new(
            vec![SettingItem {
                id: "double-escape-action".into(),
                label: "Double-escape action".into(),
                description: None,
                current_value: "tree".into(),
                values: vec!["tree".into(), "fork".into(), "none".into()],
            }],
            8,
        ));
        assert_eq!(session.handle_bytes(" "), SessionAction::CycleSetting);
        assert_eq!(
            session.handle_bytes("\r"),
            SessionAction::SelectSetting("double-escape-action=fork".into())
        );
        let mut card =
            crate::tool_card::ToolCard::start("bash", "t1", serde_json::json!({"command": "ls"}));
        card.finish(&serde_json::json!({"content": "ok"}), false);
        session.push_tool_card(card);
        session.place_kitty_image("QQ==", Some(1));
        assert!(session
            .chrome
            .transcript
            .lines
            .iter()
            .any(|line| line.role == "image" && line.text.contains("a=T,f=100,q=2")));
        session.slash_commands = vec![crate::autocomplete::SlashCommandSpec {
            name: "model".into(),
            description: "Select model".into(),
            argument_hint: Some("<provider/model>".into()),
        }];
        session.chrome.editor.buffer = "/mo".into();
        session.chrome.editor.cursor = 3;
        assert_eq!(session.handle_bytes("\t"), SessionAction::None);
        assert!(session.chrome.editor.buffer.starts_with("/model"));
        session.chrome.autocomplete = None;
        session.chrome.selector = None;
        session.chrome.settings_list = None;
        session.overlay_kind = OverlayKind::None;
        session.chrome.editor.buffer.clear();
        session.chrome.editor.cursor = 0;
        session.chrome.editor.handle_input("hi");
        assert_eq!(session.handle_bytes("\n"), SessionAction::None);
        assert!(session.chrome.editor.buffer.contains('\n'));
        session.chrome.editor.buffer = "!echo ok".into();
        assert_eq!(
            session.handle_bytes("\r"),
            SessionAction::RunBash("echo ok".into())
        );
        session.chrome.editor.buffer = "hi".into();
        session.chrome.editor.cursor = 2;
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
        session.open_first_time_setup("dark", "pi");
        assert!(session
            .render_frame()
            .contains("Welcome to pi, the minimal coding agent."));
        assert_eq!(session.handle_bytes("\x1b"), SessionAction::FirstTimeSkip);
        session.open_login_dialog("openai", None, None);
        if let Some(dialog) = &mut session.chrome.login_dialog {
            dialog.show_auth("https://example.test", None);
        }
        assert!(session.render_frame().contains("Login to openai"));
        assert_eq!(session.handle_bytes("\x1b"), SessionAction::LoginCancelled);
        session.open_tree_overlay(
            crate::build_session_tree(vec![crate::SessionTreeEntry::message(
                "u1", None, "user", "hello",
            )]),
            Some("u1".into()),
        );
        assert!(session.render_frame().contains("Session Tree"));
        assert_eq!(
            session.handle_bytes("\r"),
            SessionAction::SelectTreeEntry("u1".into())
        );
        session.open_scoped_models(vec![crate::ScopedModel {
            provider: "faux".into(),
            id: "one".into(),
            name: "One".into(),
        }]);
        assert!(session.render_frame().contains("Model Configuration"));
        assert_eq!(session.handle_bytes("\x1b"), SessionAction::CloseOverlay);
    }
}
