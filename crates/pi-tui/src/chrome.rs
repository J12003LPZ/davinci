use crate::autocomplete::AutocompleteSuggestions;
use crate::editor::Editor;
use crate::extension_ui::{
    ExtensionConfirm, ExtensionEditor, ExtensionInput, ExtensionProgress, ExtensionSelector,
    ExtensionWidget, WidgetPlacement,
};
use crate::first_time::FirstTimeSetup;
use crate::footer::{format_pwd_line, truncate_to_width};
use crate::login_dialog::LoginDialog;
use crate::model_selector::ModelSelector;
use crate::overlay::{composite_overlay_lines, OverlayOptions};
use crate::render::Component;
use crate::scoped_models::ScopedModelsSelector;
use crate::session_selector::SessionSelector;
use crate::settings::SettingsList;
use crate::settings_submenu::SettingsSubmenu;
use crate::themes::Theme;
use crate::tool_card::ToolCard;
use crate::transcript::Transcript;
use crate::tree::TreeSelector;
use crate::SelectList;

/// Fullscreen / regular chat chrome used by interactive mode.
#[derive(Debug, Clone)]
pub struct ChatChrome {
    pub transcript: Transcript,
    pub editor: Editor,
    pub selector: Option<SelectList>,
    pub model_selector: Option<ModelSelector>,
    pub settings_list: Option<SettingsList>,
    pub settings_submenu: Option<SettingsSubmenu>,
    pub session_selector: Option<SessionSelector>,
    pub first_time: Option<FirstTimeSetup>,
    pub login_dialog: Option<LoginDialog>,
    pub tree: Option<TreeSelector>,
    pub scoped_models: Option<ScopedModelsSelector>,
    pub tool_cards: Vec<ToolCard>,
    pub autocomplete: Option<AutocompleteSuggestions>,
    pub autocomplete_selected: usize,
    pub theme: Theme,
    pub status: String,
    pub title: String,
    pub widgets_above: Vec<ExtensionWidget>,
    pub widgets_below: Vec<ExtensionWidget>,
    pub extension_header: Option<Vec<String>>,
    pub extension_footer: Option<Vec<String>>,
    pub extension_statuses: Vec<(String, String)>,
    pub working_message: Option<String>,
    pub working_visible: bool,
    pub working_indicator_frames: Vec<String>,
    pub working_indicator_interval_ms: Option<u64>,
    pub hidden_thinking_label: Option<String>,
    pub extension_selector: Option<ExtensionSelector>,
    pub extension_input: Option<ExtensionInput>,
    pub extension_editor: Option<ExtensionEditor>,
    pub extension_confirm: Option<ExtensionConfirm>,
    pub extension_progress: Option<ExtensionProgress>,
    pub custom_editor_lines: Option<Vec<String>>,
    pub custom_overlay_lines: Option<Vec<String>>,
    pub custom_overlay_path: Option<String>,
    pub custom_overlay_command: Option<String>,
    pub custom_overlay_snapshot: Option<serde_json::Value>,
    pub custom_overlay_composite: bool,
    pub custom_overlay_options: Option<OverlayOptions>,
    pub quiet_startup: bool,
    pub footer_cwd: Option<String>,
    pub footer_home: Option<String>,
    pub footer_branch: Option<String>,
    pub footer_session_name: Option<String>,
    pub footer_stats: Option<String>,
}

impl ChatChrome {
    pub fn new(theme: Theme, title: impl Into<String>) -> Self {
        Self {
            transcript: Transcript::default(),
            editor: Editor::new(),
            selector: None,
            model_selector: None,
            settings_list: None,
            settings_submenu: None,
            session_selector: None,
            first_time: None,
            login_dialog: None,
            tree: None,
            scoped_models: None,
            tool_cards: Vec::new(),
            autocomplete: None,
            autocomplete_selected: 0,
            theme,
            status: String::new(),
            title: title.into(),
            widgets_above: Vec::new(),
            widgets_below: Vec::new(),
            extension_header: None,
            extension_footer: None,
            extension_statuses: Vec::new(),
            working_message: None,
            working_visible: true,
            working_indicator_frames: Vec::new(),
            working_indicator_interval_ms: None,
            hidden_thinking_label: None,
            extension_selector: None,
            extension_input: None,
            extension_editor: None,
            extension_confirm: None,
            extension_progress: None,
            custom_editor_lines: None,
            custom_overlay_lines: None,
            custom_overlay_path: None,
            custom_overlay_command: None,
            custom_overlay_snapshot: None,
            custom_overlay_composite: false,
            custom_overlay_options: None,
            quiet_startup: false,
            footer_cwd: None,
            footer_home: None,
            footer_branch: None,
            footer_session_name: None,
            footer_stats: None,
        }
    }

    pub fn set_widget(&mut self, widget: ExtensionWidget) {
        self.widgets_above.retain(|item| item.key != widget.key);
        self.widgets_below.retain(|item| item.key != widget.key);
        match widget.placement {
            WidgetPlacement::AboveEditor => self.widgets_above.push(widget),
            WidgetPlacement::BelowEditor => self.widgets_below.push(widget),
        }
    }

    pub fn clear_widget(&mut self, key: &str) {
        self.widgets_above.retain(|item| item.key != key);
        self.widgets_below.retain(|item| item.key != key);
    }

    pub fn set_extension_status(&mut self, key: &str, text: Option<&str>) {
        self.extension_statuses.retain(|(id, _)| id != key);
        if let Some(text) = text {
            if !text.is_empty() {
                self.extension_statuses
                    .push((key.to_string(), text.to_string()));
            }
        }
    }

    pub fn apply_ui_call(&mut self, call: &serde_json::Value) {
        let op = call
            .get("op")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        match op {
            "setWidget" => {
                let key = call
                    .get("key")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                if call.get("lines").is_none() && call.get("content").is_none() {
                    self.clear_widget(key);
                    return;
                }
                let lines = call
                    .get("lines")
                    .and_then(|value| value.as_array())
                    .map(|lines| {
                        lines
                            .iter()
                            .filter_map(|line| line.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let placement = WidgetPlacement::parse(
                    call.get("placement")
                        .and_then(|value| value.as_str())
                        .unwrap_or("aboveEditor"),
                );
                self.set_widget(ExtensionWidget::new(key, lines, placement));
            }
            "setStatus" => {
                let key = call
                    .get("key")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let text = call.get("text").and_then(|value| value.as_str());
                self.set_extension_status(key, text);
            }
            "setHeader" => {
                self.extension_header =
                    call.get("lines")
                        .and_then(|value| value.as_array())
                        .map(|lines| {
                            lines
                                .iter()
                                .filter_map(|line| line.as_str().map(str::to_string))
                                .collect()
                        });
            }
            "setFooter" => {
                self.extension_footer =
                    call.get("lines")
                        .and_then(|value| value.as_array())
                        .map(|lines| {
                            lines
                                .iter()
                                .filter_map(|line| line.as_str().map(str::to_string))
                                .collect()
                        });
            }
            "notify" => {
                if let Some(message) = call.get("message").and_then(|value| value.as_str()) {
                    let kind = call
                        .get("type")
                        .and_then(|value| value.as_str())
                        .unwrap_or("info");
                    self.status = format!("{kind}: {message}");
                }
            }
            "setWorkingMessage" => {
                self.working_message = call
                    .get("message")
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
            }
            "setWorkingVisible" => {
                self.working_visible = call
                    .get("visible")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true);
            }
            "setWorkingIndicator" => {
                if call.get("options").is_none()
                    || call.get("options").is_some_and(|value| value.is_null())
                {
                    self.working_indicator_frames.clear();
                    self.working_indicator_interval_ms = None;
                } else {
                    self.working_indicator_frames = call
                        .get("options")
                        .and_then(|value| value.get("frames"))
                        .and_then(|value| value.as_array())
                        .map(|frames| {
                            frames
                                .iter()
                                .filter_map(|frame| frame.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    self.working_indicator_interval_ms = call
                        .get("options")
                        .and_then(|value| value.get("intervalMs"))
                        .and_then(|value| value.as_u64());
                }
            }
            "setHiddenThinkingLabel" => {
                self.hidden_thinking_label = call
                    .get("label")
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
            }
            "setTitle" => {
                if let Some(title) = call.get("title").and_then(|value| value.as_str()) {
                    self.title = title.to_string();
                }
            }
            "setEditorText" => {
                if let Some(text) = call.get("text").and_then(|value| value.as_str()) {
                    self.editor.buffer.clear();
                    self.editor.cursor = 0;
                    self.editor.handle_input(text);
                }
            }
            "pasteToEditor" => {
                if let Some(text) = call.get("text").and_then(|value| value.as_str()) {
                    self.editor.handle_input(text);
                }
            }
            "setEditorComponent" => {
                if call.get("enabled").and_then(|value| value.as_bool()) == Some(false) {
                    self.custom_editor_lines = None;
                }
            }
            _ => {}
        }
    }

    pub fn apply_mouse(&mut self, y: u16, viewport: usize) {
        if let Some(selector) = &mut self.selector {
            if (y as usize) < selector.items.len() {
                selector.selected = y as usize;
            }
        } else {
            self.transcript.scroll_by(0, viewport);
        }
    }
}

impl Component for ChatChrome {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.quiet_startup || self.extension_header.is_some() {
            lines.push(format!("{}  theme={}", self.title, self.theme.name));
        }
        if let Some(header) = &self.extension_header {
            lines.extend(header.iter().cloned());
        }
        if !self.status.is_empty() {
            lines.push(self.status.clone());
        }
        if !self.extension_statuses.is_empty() {
            lines.push(
                self.extension_statuses
                    .iter()
                    .map(|(key, text)| format!("{key}: {text}"))
                    .collect::<Vec<_>>()
                    .join(" · "),
            );
        }
        if self.working_visible {
            if let Some(frame) = self.working_indicator_frames.first() {
                if !frame.is_empty() {
                    lines.push(frame.clone());
                }
            } else if let Some(working) = &self.working_message {
                lines.push(working.clone());
            }
        }
        lines.extend(self.transcript.render(width));
        for card in &self.tool_cards {
            lines.extend(card.render(width));
        }
        if let Some(setup) = &self.first_time {
            lines.push(String::new());
            lines.extend(setup.render(width));
        } else if let Some(login) = &self.login_dialog {
            lines.push(String::new());
            lines.extend(login.render(width));
        } else if let Some(tree) = &self.tree {
            lines.push(String::new());
            lines.extend(tree.render(width));
        } else if let Some(scoped) = &self.scoped_models {
            lines.push(String::new());
            lines.extend(scoped.render(width));
        } else if let Some(submenu) = &self.settings_submenu {
            lines.push(String::new());
            lines.extend(submenu.render(width));
        } else if let Some(sessions) = &self.session_selector {
            lines.push(String::new());
            lines.extend(sessions.render(width));
        } else if let Some(settings) = &self.settings_list {
            lines.push(String::new());
            lines.extend(settings.render(width));
        } else if let Some(selector) = &self.model_selector {
            lines.push(String::new());
            lines.extend(selector.render(width));
        } else if let Some(selector) = &self.selector {
            lines.push(String::new());
            lines.extend(selector.render(width));
        } else if let Some(selector) = &self.extension_selector {
            lines.push(String::new());
            lines.extend(selector.render(width));
        } else if let Some(input) = &self.extension_input {
            lines.push(String::new());
            lines.extend(input.render(width));
        } else if let Some(editor) = &self.extension_editor {
            lines.push(String::new());
            lines.extend(editor.render(width));
        } else if let Some(confirm) = &self.extension_confirm {
            lines.push(String::new());
            lines.extend(confirm.render(width));
        } else if let Some(progress) = &self.extension_progress {
            lines.push(String::new());
            lines.extend(progress.render(width));
        } else if let Some(custom) = &self.custom_overlay_lines {
            if self.custom_overlay_composite {
                let mut base = Vec::new();
                for widget in &self.widgets_above {
                    base.extend(widget.lines.iter().cloned());
                }
                if let Some(editor) = &self.custom_editor_lines {
                    base.extend(editor.iter().cloned());
                } else {
                    base.extend(self.editor.render(width));
                }
                for widget in &self.widgets_below {
                    base.extend(widget.lines.iter().cloned());
                }
                let options = self.custom_overlay_options.clone().unwrap_or_default();
                let height = base.len().max(custom.len()).max(24);
                lines.extend(composite_overlay_lines(
                    &base, custom, &options, width, height,
                ));
            } else {
                lines.push(String::new());
                lines.extend(custom.iter().cloned());
            }
        } else {
            for widget in &self.widgets_above {
                lines.extend(widget.lines.iter().cloned());
            }
            if let Some(custom) = &self.custom_editor_lines {
                lines.extend(custom.iter().cloned());
            } else {
                lines.extend(self.editor.render(width));
            }
            for widget in &self.widgets_below {
                lines.extend(widget.lines.iter().cloned());
            }
            if let Some(suggestions) = &self.autocomplete {
                for (index, item) in suggestions.items.iter().enumerate() {
                    let prefix = if index == self.autocomplete_selected {
                        "> "
                    } else {
                        "  "
                    };
                    let desc = item
                        .description
                        .as_deref()
                        .map(|value| format!("  {value}"))
                        .unwrap_or_default();
                    lines.push(format!("{prefix}{}{desc}", item.label));
                }
            }
        }
        if let Some(cwd) = &self.footer_cwd {
            let pwd = format_pwd_line(
                cwd,
                self.footer_home.as_deref(),
                self.footer_branch.as_deref(),
                self.footer_session_name.as_deref(),
            );
            lines.push(truncate_to_width(&pwd, width, "..."));
            if let Some(stats) = &self.footer_stats {
                lines.push(truncate_to_width(stats, width, "..."));
            }
        }
        if let Some(footer) = &self.extension_footer {
            lines.extend(footer.iter().cloned());
        }
        lines
    }

    fn handle_input(&mut self, data: &str) {
        if let Some(setup) = &mut self.first_time {
            setup.handle_input(data);
        } else if let Some(login) = &mut self.login_dialog {
            login.handle_input(data);
        } else if let Some(tree) = &mut self.tree {
            tree.handle_input(data);
        } else if let Some(scoped) = &mut self.scoped_models {
            scoped.handle_input(data);
        } else if let Some(submenu) = &mut self.settings_submenu {
            submenu.handle_input(data);
        } else if let Some(sessions) = &mut self.session_selector {
            sessions.handle_input(data);
        } else if let Some(settings) = &mut self.settings_list {
            settings.handle_input(data);
        } else if let Some(selector) = &mut self.model_selector {
            let _ = selector.handle_key(data);
        } else if let Some(selector) = &mut self.selector {
            selector.query.push_str(data);
        } else if let Some(selector) = &mut self.extension_selector {
            selector.handle_input(data);
        } else if let Some(input) = &mut self.extension_input {
            input.handle_input(data);
        } else if let Some(editor) = &mut self.extension_editor {
            editor.handle_input(data);
        } else if let Some(confirm) = &mut self.extension_confirm {
            confirm.handle_input(data);
        } else if let Some(progress) = &mut self.extension_progress {
            progress.handle_input(data);
        } else {
            self.editor.handle_input(data);
        }
    }

    fn invalidate(&mut self) {
        self.editor.invalidate();
        self.transcript.invalidate();
    }
}
