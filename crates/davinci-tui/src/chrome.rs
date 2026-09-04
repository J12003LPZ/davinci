use crate::autocomplete::AutocompleteSuggestions;
use crate::editor::Editor;
use crate::extension_ui::{
    ExtensionConfirm, ExtensionEditor, ExtensionInput, ExtensionProgress, ExtensionSelector,
    ExtensionWidget, WidgetPlacement,
};
use crate::first_time::FirstTimeSetup;
use crate::footer::{format_pwd_line, truncate_to_width};
use crate::loaded_resources::{ExpandableText, LoadedResources};
use crate::login_dialog::LoginDialog;
use crate::model_selector::ModelSelector;
use crate::oauth_selector::OAuthSelector;
use crate::overlay::{composite_overlay_lines, OverlayOptions};
use crate::render::Component;
use crate::scoped_models::ScopedModelsSelector;
use crate::session_selector::SessionSelector;
use crate::settings::SettingsList;
use crate::settings_submenu::SettingsSubmenu;
use crate::themes::{builtin_themes, glyphs, Theme};
use crate::thinking_selector::ThinkingSelector;
use crate::tool_card::ToolCard;
use crate::transcript::Transcript;
use crate::tree::TreeSelector;
use crate::trust_selector::TrustSelector;
use crate::SelectList;

/// Fullscreen / regular chat chrome used by interactive mode.
#[derive(Debug, Clone)]
pub struct ChatChrome {
    pub transcript: Transcript,
    pub editor: Editor,
    pub selector: Option<SelectList>,
    pub model_selector: Option<ModelSelector>,
    pub thinking_selector: Option<ThinkingSelector>,
    pub trust_selector: Option<TrustSelector>,
    pub settings_list: Option<SettingsList>,
    pub settings_submenu: Option<SettingsSubmenu>,
    pub session_selector: Option<SessionSelector>,
    pub first_time: Option<FirstTimeSetup>,
    pub login_dialog: Option<LoginDialog>,
    pub oauth_selector: Option<OAuthSelector>,
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
    /// `provider/model` shown on the right of the status bar.
    pub footer_model: Option<String>,
    /// `(used, window)` context tokens; rendered as a proportion meter.
    pub footer_context: Option<(u64, u64)>,
    /// `(files, added, removed)` session change stats (`Δ3 +42 -11`).
    pub footer_delta: Option<(u64, u64, u64)>,
    pub tools_expanded: bool,
    /// Dim hint shown in the empty composer.
    pub composer_placeholder: String,
    pub startup_header: Option<ExpandableText>,
    pub loaded_resources: LoadedResources,
    pub available_themes: Vec<Theme>,
    pub terminal_input_registered: bool,
}

impl ChatChrome {
    pub fn new(theme: Theme, title: impl Into<String>) -> Self {
        let mut chrome = Self {
            transcript: Transcript::default(),
            editor: Editor::new(),
            selector: None,
            model_selector: None,
            thinking_selector: None,
            trust_selector: None,
            settings_list: None,
            settings_submenu: None,
            session_selector: None,
            first_time: None,
            login_dialog: None,
            oauth_selector: None,
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
            footer_model: None,
            footer_context: None,
            footer_delta: None,
            tools_expanded: false,
            composer_placeholder: "What shall we construct?".into(),
            startup_header: None,
            loaded_resources: LoadedResources::default(),
            available_themes: builtin_themes(),
            terminal_input_registered: false,
        };
        chrome.transcript.theme = chrome.theme.clone();
        chrome
    }

    pub fn get_theme_name(&self) -> &str {
        &self.theme.name
    }

    pub fn get_all_themes(&self) -> Vec<(String, Option<String>)> {
        self.available_themes
            .iter()
            .map(|theme| (theme.name.clone(), None))
            .collect()
    }

    pub fn set_theme_by_name(&mut self, name: &str) -> Result<(), String> {
        if let Some(theme) = self
            .available_themes
            .iter()
            .find(|theme| theme.name == name)
            .cloned()
            .or_else(|| {
                builtin_themes()
                    .into_iter()
                    .find(|theme| theme.name == name)
            })
        {
            self.theme = theme.clone();
            self.transcript.theme = theme;
            Ok(())
        } else {
            Err(format!("Theme not found: {name}"))
        }
    }

    pub fn set_theme_instance(&mut self, theme: Theme) {
        if let Some(existing) = self
            .available_themes
            .iter_mut()
            .find(|item| item.name == theme.name)
        {
            *existing = theme.clone();
        } else {
            self.available_themes.push(theme.clone());
        }
        self.transcript.theme = theme.clone();
        self.theme = theme;
    }

    pub fn set_tools_expanded(&mut self, expanded: bool) {
        self.tools_expanded = expanded;
        self.transcript.tools_expanded = expanded;
        self.loaded_resources.set_expanded(expanded);
        if let Some(header) = &mut self.startup_header {
            header.set_expanded(expanded);
        }
        for card in &mut self.tool_cards {
            card.expanded = expanded;
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
            "setTheme" => {
                if let Some(theme) = call.get("theme").filter(|value| value.is_object()) {
                    if let Ok(parsed) = serde_json::from_value::<Theme>(theme.clone()) {
                        self.set_theme_instance(parsed);
                    } else if let Some(name) = theme.get("name").and_then(|value| value.as_str()) {
                        let _ = self.set_theme_by_name(name);
                    }
                } else if let Some(name) = call.get("name").and_then(|value| value.as_str()) {
                    let _ = self.set_theme_by_name(name);
                }
            }
            "setToolsExpanded" => {
                self.set_tools_expanded(
                    call.get("expanded")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                );
            }
            "onTerminalInput" => {
                self.terminal_input_registered = true;
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

impl ChatChrome {
    /// Transcript / header / tool cards (TS `documentContainer` + chat).
    pub fn render_document(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(header) = &self.startup_header {
            lines.extend(header.current().lines().map(str::to_string));
            lines.push(String::new());
        } else if !self.quiet_startup || self.extension_header.is_some() {
            lines.push(format!(
                "{} {}",
                self.theme.fg("primary", glyphs::AGENT),
                self.theme.fg("muted", &self.title)
            ));
        }
        if let Some(header) = &self.extension_header {
            lines.extend(header.iter().cloned());
        }
        lines.extend(self.loaded_resources.render(width));
        // One blank line of air between the startup block and the transcript.
        if !self.transcript.lines.is_empty() && lines.last().is_some_and(|line| !line.is_empty()) {
            lines.push(String::new());
        }
        lines.extend(self.transcript.render(width));
        for card in &self.tool_cards {
            for line in card.render(width) {
                lines.push(crate::transcript::style_glyph_line(&self.theme, &line));
            }
        }
        lines
    }

    /// Editor, overlays, status, and footer (TS dock under the transcript ScrollView).
    pub fn render_dock(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
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
        if let Some(setup) = &self.first_time {
            lines.push(String::new());
            lines.extend(setup.render(width));
        } else if let Some(login) = &self.login_dialog {
            lines.push(String::new());
            lines.extend(login.render(width));
        } else if let Some(selector) = &self.oauth_selector {
            lines.push(String::new());
            lines.extend(selector.render(width));
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
        } else if let Some(selector) = &self.thinking_selector {
            lines.push(String::new());
            lines.extend(selector.render(width));
        } else if let Some(selector) = &self.trust_selector {
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
                lines.extend(self.render_composer(width));
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
        lines.extend(self.render_status_bar(width));
        if let Some(footer) = &self.extension_footer {
            lines.extend(footer.iter().cloned());
        }
        lines
    }

    /// Composer (spec §6): copper top rule, `›` prompt, keybind hints below.
    fn render_composer(&self, width: usize) -> Vec<String> {
        let raw = self.editor.render(width.saturating_sub(2));
        if raw.len() < 2 {
            return raw;
        }
        let theme = &self.theme;
        let mut lines = Vec::with_capacity(raw.len() + 1);
        lines.push(theme.fg("primary", &"─".repeat(width)));
        let content_count = raw.len().saturating_sub(2);
        let placeholder = (self.editor.buffer.is_empty() && !self.composer_placeholder.is_empty())
            .then(|| theme.fg("dim", &self.composer_placeholder));
        for (index, line) in raw.iter().skip(1).take(content_count).enumerate() {
            if index == 0 {
                let mut line = format!(
                    "{} {}",
                    theme.fg("primary", glyphs::PROMPT),
                    line.trim_end()
                );
                if let Some(hint) = &placeholder {
                    line.push(' ');
                    line.push_str(hint);
                }
                lines.push(line);
            } else {
                lines.push(format!("  {line}"));
            }
        }
        lines.push(theme.fg("border", &"─".repeat(width)));
        if width >= 70 {
            lines.push(theme.fg(
                "dim",
                "  enter send · shift+enter newline · tab complete · esc cancel",
            ));
        }
        lines
    }

    /// Status bar (spec §6): `dir (branch) · Δn +a -d` left, model + context
    /// meter right. Falls back to the legacy pwd/stats lines when the
    /// structured fields are unset.
    fn render_status_bar(&self, width: usize) -> Vec<String> {
        let Some(cwd) = &self.footer_cwd else {
            return Vec::new();
        };
        let theme = &self.theme;
        if self.footer_model.is_none() && self.footer_context.is_none() {
            let pwd = format_pwd_line(
                cwd,
                self.footer_home.as_deref(),
                self.footer_branch.as_deref(),
                self.footer_session_name.as_deref(),
            );
            let mut lines = vec![truncate_to_width(&pwd, width, "...")];
            if let Some(stats) = &self.footer_stats {
                lines.push(truncate_to_width(stats, width, "..."));
            }
            return lines;
        }
        let sep = format!(" {} ", theme.fg("border", "·"));
        let pwd = crate::footer::format_cwd_for_footer(cwd, self.footer_home.as_deref());
        let mut left_plain = pwd.clone();
        let mut left = theme.fg("muted", &pwd);
        if let Some(branch) = self.footer_branch.as_deref().filter(|b| !b.is_empty()) {
            left_plain.push_str(&format!(" · {branch}"));
            left.push_str(&sep);
            left.push_str(&theme.fg("secondary", branch));
        }
        if let Some((files, added, removed)) = self.footer_delta {
            if files > 0 {
                let plain = format!("Δ{files} +{added} -{removed}");
                left_plain.push_str(&format!(" · {plain}"));
                left.push_str(&sep);
                left.push_str(&theme.fg("primary", &format!("Δ{files}")));
                left.push_str(&theme.fg("success", &format!(" +{added}")));
                left.push_str(&theme.fg("error", &format!(" -{removed}")));
            }
        }
        let mut right_plain = String::new();
        let mut right = String::new();
        if let Some(model) = self.footer_model.as_deref() {
            right_plain.push_str(model);
            right.push_str(&theme.fg("muted", model));
        }
        if let Some((used, window)) = self.footer_context {
            let counts = format!(
                "{}/{}",
                format_tokens_short(used),
                format_tokens_short(window)
            );
            let meter_cells = if width >= 100 { 16 } else { 8 };
            if !right_plain.is_empty() {
                right_plain.push_str(" · ");
                right.push_str(&sep);
            }
            right_plain.push_str(&format!("{} {counts}", "─".repeat(meter_cells)));
            right.push_str(&theme.meter(used, window, meter_cells));
            right.push(' ');
            right.push_str(&theme.fg("muted", &counts));
        }
        let pad = width
            .saturating_sub(crate::render::visible_width(&left_plain))
            .saturating_sub(crate::render::visible_width(&right_plain))
            .max(2);
        let line = format!("{left}{}{right}", " ".repeat(pad));
        vec![crate::ansi::truncate_to_width(
            &line,
            width.max(8),
            "…",
            false,
        )]
    }
}

/// `47k` / `1.2m` token counts (spec §9: numbers always carry unit and cap).
pub fn format_tokens_short(count: u64) -> String {
    if count >= 1_000_000 {
        let millions = count as f64 / 1_000_000.0;
        if millions.fract() < 0.05 {
            format!("{}m", millions as u64)
        } else {
            format!("{millions:.1}m")
        }
    } else if count >= 1_000 {
        format!("{}k", count / 1_000)
    } else {
        count.to_string()
    }
}

impl Component for ChatChrome {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = self.render_document(width);
        lines.extend(self.render_dock(width));
        lines
    }

    fn handle_input(&mut self, data: &str) {
        if let Some(setup) = &mut self.first_time {
            setup.handle_input(data);
        } else if let Some(login) = &mut self.login_dialog {
            login.handle_input(data);
        } else if let Some(selector) = &mut self.oauth_selector {
            let _ = selector.handle_key(data);
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
        } else if let Some(selector) = &mut self.thinking_selector {
            let _ = selector.handle_key(data);
        } else if let Some(selector) = &mut self.trust_selector {
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
