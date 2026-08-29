//! InteractiveMode: TUI host + slash commands + AgentSession.

use crate::commands;
use crate::extensions::{self, Extension};
use crate::login::{self, LoginStart};
use crate::login_dialog::{LoginDialog, LoginDialogAction};
use crate::session_runtime::{to_json_event, SessionRuntime};
use crate::settings;
use crate::slash::{self, BUILTIN_SLASH_COMMANDS};
use crate::trust_selector::{TrustSelector, TrustSelectorAction};
use pi_agent::ThinkingLevel;
use pi_session::{default_sessions_root, discover_sessions};
use pi_tui::component::Component;
use pi_tui::{
    default_keybindings, disable_mouse, disable_raw_input, enable_mouse, enable_raw_input,
    enter_alt_screen, layout_transcript_and_dock, leave_alt_screen, set_title, ChatView, Editor,
    Key, Markdown, Node, Overlay, OverlayOptions, SelectList, SettingsList, Tui, TuiAltScreen,
    TuiAltScreenOptions, TuiMode, ViewportScroll, ViewportScrollOptions,
};
use serde_json::json;
use std::io::{self, BufRead, Read, Write};

pub struct InteractiveMode {
    pub runtime: SessionRuntime,
    pub tui: Tui,
    pub chat: ChatView,
    pub editor: Editor,
    pub theme: String,
    pub discovered: Vec<Extension>,
    pub auto_approve: bool,
    pub last_json_events: Vec<serde_json::Value>,
    trust_selector: Option<TrustSelector>,
    login_dialog: Option<LoginDialog>,
    pub alt: Option<TuiAltScreen>,
    transcript: Option<Node>,
    dock: Option<Node>,
}

impl InteractiveMode {
    pub fn new(runtime: SessionRuntime, mode: TuiMode, discovered: Vec<Extension>) -> Self {
        let mut tui = Tui::new(mode, 80, 24);
        let mut chat = ChatView::default();
        chat.push(
            "system",
            format!(
                "pi {}  {}/{}",
                crate::args::VERSION,
                runtime.provider,
                runtime.model_id
            ),
        );
        if let Some(path) = &runtime.session_path {
            chat.push("session", path.display().to_string());
        }
        tui.add_child_lines(chat.render(80));
        let theme = runtime.theme.clone();
        let (alt, transcript, dock) = if mode == TuiMode::Fullscreen {
            chat.semantic_zones = true;
            let transcript = Node::text(chat.render(80).into_iter().collect::<Vec<_>>().join("\n"));
            let scroll = ViewportScroll::new(
                transcript.clone(),
                ViewportScrollOptions {
                    follow_end: true,
                    primary: true,
                    ..ViewportScrollOptions::default()
                },
            );
            let dock = Node::text("editor");
            let mut alt = TuiAltScreen::new(80, 24, TuiAltScreenOptions::default());
            alt.set_layout_root(layout_transcript_and_dock(scroll, dock.clone()));
            (Some(alt), Some(transcript), Some(dock))
        } else {
            (None, None, None)
        };
        Self {
            runtime,
            tui,
            chat,
            editor: Editor::default(),
            theme,
            discovered,
            auto_approve: true,
            last_json_events: Vec::new(),
            trust_selector: None,
            login_dialog: None,
            alt,
            transcript,
            dock,
        }
    }

    pub fn footer_lines(&self) -> Vec<String> {
        let mut line = format!(
            " {}/{}  think:{}  msgs:{}  theme:{}  /help",
            self.runtime.provider,
            self.runtime.model_id,
            self.runtime.thinking.as_str(),
            self.runtime.messages.len(),
            self.theme
        );
        let statuses = self.runtime.ui.status_line();
        if !statuses.is_empty() {
            line.push_str("  ");
            line.push_str(&statuses);
        }
        if let Some((_, message)) = self.runtime.ui.notifications.last() {
            line.push_str("  ");
            line.push_str(message);
        }
        vec![line]
    }

    pub fn apply_ui_request(&mut self, request: &serde_json::Value) {
        match request.get("method").and_then(|v| v.as_str()) {
            Some("setTitle") => {
                if let Some(title) = request.get("title").and_then(|v| v.as_str()) {
                    let _ = set_title(&mut io::stdout(), title);
                    if let Some(name) = self.runtime.ui.title.clone() {
                        self.chat.push("title", name);
                    }
                }
            }
            Some("setStatus") => {}
            Some("setWidget") => {
                for line in self.runtime.ui.widget_lines("aboveEditor") {
                    self.chat.push("widget", line);
                }
            }
            Some("notify") => {
                if let Some(message) = request.get("message").and_then(|v| v.as_str()) {
                    self.chat.push(
                        request
                            .get("notifyType")
                            .and_then(|v| v.as_str())
                            .unwrap_or("info"),
                        message,
                    );
                }
            }
            Some("set_editor_text") => {
                if let Some(text) = request.get("text").and_then(|v| v.as_str()) {
                    self.editor.buffer = text.to_string();
                    self.editor.cursor = text.chars().count();
                }
            }
            Some("select") => {
                let options = request
                    .get("options")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                self.show_selector(
                    request
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("select"),
                    options,
                );
            }
            Some("confirm") => {
                self.show_selector(
                    request
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("confirm"),
                    vec!["Yes".into(), "No".into()],
                );
            }
            Some("input" | "editor") => {
                if let Some(prefill) = request
                    .get("prefill")
                    .or_else(|| request.get("placeholder"))
                    .and_then(|v| v.as_str())
                {
                    self.editor.buffer = prefill.to_string();
                    self.editor.cursor = prefill.chars().count();
                }
            }
            _ => {}
        }
    }

    pub fn redraw(&mut self, force: bool) -> String {
        if self.alt.is_some() {
            let width = self.alt.as_ref().map(|alt| alt.columns).unwrap_or(80);
            if let Some(transcript) = &self.transcript {
                transcript.set_text(self.chat.render(width).join("\n"));
            }
            if let Some(dock) = &self.dock {
                let mut dock_lines = self.footer_lines();
                dock_lines.extend(
                    self.runtime
                        .ui
                        .widget_lines("aboveEditor")
                        .into_iter()
                        .map(|line| format!(" {line}")),
                );
                if let Some(dialog) = &self.login_dialog {
                    dock_lines.extend(dialog.render().lines().map(str::to_string));
                } else {
                    dock_lines.extend(self.editor.render(width));
                }
                dock.set_text(dock_lines.join("\n"));
            }
            let alt = self.alt.as_mut().expect("fullscreen alt");
            alt.request_render();
            return alt.writes.last().cloned().unwrap_or_default();
        }
        self.tui.clear_children();
        let mut lines = self.chat.render(self.tui.columns);
        lines.extend(
            self.runtime
                .ui
                .widget_lines("aboveEditor")
                .into_iter()
                .map(|line| format!(" {line}")),
        );
        lines.extend(self.footer_lines());
        lines.extend(
            self.runtime
                .ui
                .widget_lines("belowEditor")
                .into_iter()
                .map(|line| format!(" {line}")),
        );
        if let Some(dialog) = &self.login_dialog {
            let _ = (dialog.focused(), dialog.input_value());
            lines.extend(dialog.render().lines().map(str::to_string));
        } else {
            lines.extend(self.editor.render(self.tui.columns));
        }
        self.tui.add_child_lines(lines);
        self.tui.render_now(force)
    }

    pub fn show_selector(&mut self, title: &str, items: Vec<String>) {
        let list = SelectList::new(items);
        let overlay = Overlay::new(title, list.filtered());
        self.tui.hide_all_overlays();
        self.tui.show_overlay(
            overlay.render(40),
            OverlayOptions {
                width: Some(40),
                ..OverlayOptions::default()
            },
        );
    }

    pub fn handle_slash(&mut self, cmd: &str, args: &str) -> Result<bool, String> {
        match cmd {
            "quit" | "exit" => return Ok(true),
            "help" => {
                let mut body: Vec<String> = BUILTIN_SLASH_COMMANDS
                    .iter()
                    .map(|(name, desc)| format!("/{name}  {desc}"))
                    .collect();
                for command in &self.runtime.registry.commands {
                    body.push(format!(
                        "/{}  {}",
                        command.name,
                        command
                            .description
                            .as_deref()
                            .unwrap_or("extension command")
                    ));
                }
                self.tui.show_overlay(
                    Overlay::new("help", body).render(80),
                    OverlayOptions {
                        width: Some(72),
                        ..OverlayOptions::default()
                    },
                );
                self.chat.push("help", "slash commands overlay");
            }
            "hotkeys" => {
                let mut keys: Vec<String> = default_keybindings()
                    .into_iter()
                    .map(|k| format!("{}  {}", k.key, k.action))
                    .collect();
                for shortcut in &self.runtime.registry.shortcuts {
                    keys.push(format!(
                        "{}  {}  ({})",
                        shortcut.shortcut,
                        shortcut
                            .description
                            .as_deref()
                            .unwrap_or("extension shortcut"),
                        shortcut.path.display()
                    ));
                }
                self.tui.show_overlay(
                    Overlay::new("hotkeys", keys).render(40),
                    OverlayOptions::default(),
                );
            }
            "settings" => {
                let list = SettingsList {
                    items: vec![
                        ("auto compact".into(), self.runtime.auto_compact),
                        ("auto retry".into(), self.runtime.auto_retry),
                        ("auto approve tools".into(), self.auto_approve),
                    ],
                    selected: 0,
                };
                self.tui.show_overlay(
                    Overlay::new("settings", list.render(40)).render(40),
                    OverlayOptions::default(),
                );
            }
            "model" => {
                if args.is_empty() {
                    let items = self
                        .runtime
                        .available_models()
                        .into_iter()
                        .take(40)
                        .map(|m| {
                            format!(
                                "{}/{}",
                                m["provider"].as_str().unwrap_or(""),
                                m["id"].as_str().unwrap_or("")
                            )
                        })
                        .collect();
                    self.show_selector("model", items);
                } else if let Some((provider, id)) = args.split_once('/') {
                    self.runtime.set_model(provider, id);
                    self.chat.push("system", format!("model {provider}/{id}"));
                } else {
                    let provider = self.runtime.provider.clone();
                    self.runtime.set_model(&provider, args);
                    self.chat.push("system", format!("model {args}"));
                }
            }
            "scoped-models" => {
                if args.is_empty() {
                    self.show_selector("scoped-models", self.runtime.scoped_models.clone());
                } else {
                    self.runtime.scoped_models = args
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    self.chat.push(
                        "system",
                        format!("scoped {}", self.runtime.scoped_models.join(",")),
                    );
                }
            }
            "thinking" => {
                if let Some(level) = ThinkingLevel::parse(args) {
                    self.runtime.set_thinking(level);
                } else {
                    self.runtime.set_thinking(self.runtime.thinking.cycle());
                }
                self.chat.push(
                    "system",
                    format!("thinking {}", self.runtime.thinking.as_str()),
                );
            }
            "tree" => {
                let tree = self.runtime.tree();
                let lines = tree["tree"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|n| {
                        format!(
                            "{} {}",
                            n["id"].as_str().unwrap_or("-"),
                            n["label"].as_str().unwrap_or("")
                        )
                    })
                    .collect();
                self.show_selector("tree", lines);
            }
            "session" => {
                self.chat.push("session", self.runtime.stats().to_string());
            }
            "name" => {
                if !args.is_empty() {
                    self.runtime.set_name(args);
                    self.chat.push("system", format!("named {args}"));
                }
            }
            "export" => {
                match self
                    .runtime
                    .export_html(if args.is_empty() { None } else { Some(args) })
                {
                    Ok(path) => self
                        .chat
                        .push("system", format!("exported {}", path.display())),
                    Err(err) => self.chat.push("error", err),
                }
            }
            "import" => {
                if args.is_empty() {
                    self.chat.push("error", "usage: /import <path.jsonl>");
                } else if let Err(err) = self.runtime.switch_session(args) {
                    self.chat.push("error", err);
                } else {
                    self.chat.push("system", format!("imported {args}"));
                }
            }
            "share" => {
                let html = self.runtime.export_html(None);
                let jsonl = self.runtime.session_path.clone();
                match html {
                    Ok(html_path) => {
                        let token = if self.runtime.provider == "radius" {
                            self.runtime.api_key.as_deref()
                        } else {
                            None
                        };
                        let jsonl_path = jsonl.unwrap_or_else(|| html_path.clone());
                        match crate::share::share_session(&jsonl_path, &html_path, token, None) {
                            Ok(shared) => {
                                let mut msg = format!("Share URL: {}", shared.preview_url);
                                if !shared.gist_url.is_empty() {
                                    msg.push_str(&format!("\nGist: {}", shared.gist_url));
                                }
                                self.chat.push("system", msg);
                            }
                            Err(err) => self.chat.push("error", err),
                        }
                    }
                    Err(err) => self
                        .chat
                        .push("error", format!("Failed to export session: {err}")),
                }
            }
            "copy" => match self.runtime.last_assistant() {
                Some(text) => match crate::clipboard::copy_to_clipboard(&text) {
                    Ok(()) => self
                        .chat
                        .push("system", "Copied last agent message to clipboard"),
                    Err(err) => self.chat.push("error", err),
                },
                None => self.chat.push("error", "No agent messages to copy yet."),
            },
            "changelog" => {
                let markdown =
                    crate::changelog::render_changelog(&crate::changelog::changelog_path());
                self.chat.push("system", "What's New");
                self.chat.push("changelog", markdown);
            }
            "fork" => {
                if args.is_empty() {
                    let msgs = self.runtime.fork_user_messages();
                    let items = msgs
                        .into_iter()
                        .map(|m| {
                            format!(
                                "{} {}",
                                m["entryId"].as_str().unwrap_or("-"),
                                m["text"].as_str().unwrap_or("")
                            )
                        })
                        .collect();
                    self.show_selector("fork", items);
                } else if let Err(err) = self.runtime.fork(args) {
                    self.chat.push("error", err);
                }
            }
            "clone" => {
                if let Err(err) = self.runtime.clone_current() {
                    self.chat.push("error", err);
                } else {
                    self.chat.push("system", "cloned session");
                }
            }
            "trust" => {
                let cwd = self.runtime.cwd.clone();
                let agent = commands::agent_dir();
                if args.is_empty() {
                    let trusted = crate::package_manager::project_is_trusted(
                        None,
                        crate::package_manager::ProjectTrustMode::Full,
                    );
                    let selector = TrustSelector::new(&cwd, &agent, trusted);
                    self.show_selector(
                        "Project trust",
                        settings::project_trust_options(&cwd, false)
                            .iter()
                            .map(|option| option.label.clone())
                            .collect(),
                    );
                    self.chat.push("system", selector.render());
                    self.trust_selector = Some(selector);
                } else if let Some(trusted) =
                    settings::apply_project_trust_selection(&agent, &cwd, args, false)
                {
                    self.trust_selector = None;
                    self.chat.push(
                        "system",
                        format!(
                            "Saved trust decision: {}. Restart {} for this to take effect.",
                            if trusted { "trusted" } else { "untrusted" },
                            crate::args::APP_NAME
                        ),
                    );
                } else {
                    self.chat
                        .push("error", format!("Unknown trust option: {args}"));
                }
            }
            "login" => match login::begin_interactive_login(&commands::agent_dir(), args) {
                Ok(LoginStart::Message(message)) => self.chat.push("system", message),
                Ok(LoginStart::Dialog(dialog)) => {
                    self.chat.push("system", dialog.render());
                    self.login_dialog = Some(*dialog);
                }
                Err(err) => self.chat.push("error", err),
            },
            "logout" => match login::handle_logout_command(&commands::agent_dir(), args) {
                Ok(message) => self.chat.push("system", message),
                Err(err) => self.chat.push("error", err),
            },
            "new" => {
                let events = self.runtime.new_session(None)?;
                self.chat = ChatView::default();
                self.chat.push("system", "new session");
                if events.is_empty() {
                    let title = self.runtime.ui.set_title("pi");
                    self.apply_ui_request(&title);
                } else {
                    for event in events {
                        self.apply_ui_request(&event);
                    }
                }
            }
            "compact" => {
                let data = self
                    .runtime
                    .compact(if args.is_empty() { None } else { Some(args) });
                self.chat.push("system", data.to_string());
            }
            "resume" => {
                if args.is_empty() {
                    let sessions = discover_sessions(
                        &default_sessions_root(),
                        Some(&self.runtime.cwd.to_string_lossy()),
                    )
                    .unwrap_or_default();
                    let items = sessions
                        .into_iter()
                        .map(|s| {
                            format!(
                                "{} {}",
                                s.id,
                                s.name.unwrap_or_else(|| s.path.display().to_string())
                            )
                        })
                        .collect();
                    self.show_selector("resume", items);
                } else if let Err(err) = self.runtime.switch_session(args) {
                    self.chat.push("error", err);
                }
            }
            "reload" => {
                self.runtime.bus.clear();
                extensions::attach_extensions(&self.runtime.bus, &self.discovered);
                self.runtime.bind_extensions();
                self.chat.push(
                    "system",
                    format!("reloaded ({} channels)", self.runtime.bus.channel_count()),
                );
            }
            other => {
                if self.runtime.registry.command(other).is_some() {
                    match self.runtime.invoke_registered_command(other, args) {
                        Ok(events) => {
                            for event in events {
                                self.apply_ui_request(&event);
                            }
                            let extra = self.runtime.take_extension_turn_events();
                            self.show_agent_events(extra);
                            self.chat.push("system", format!("/{other}"));
                        }
                        Err(err) => self.chat.push("error", err),
                    }
                } else {
                    self.chat.push("system", format!("/{other} {args}"));
                }
            }
        }
        Ok(false)
    }

    pub fn submit_prompt(&mut self, text: &str) -> Result<(), String> {
        let user = self
            .runtime
            .transform_markdown(text, "user", false, self.tui.columns.max(1));
        self.chat.push("user", user);
        let events = self.runtime.prompt(text, vec![])?;
        self.last_json_events = events.iter().map(to_json_event).collect();
        self.show_agent_events(events);
        Ok(())
    }

    fn show_agent_events(&mut self, events: Vec<pi_agent::AgentEvent>) {
        for event in events {
            match event {
                pi_agent::AgentEvent::Message { message } => {
                    let transformed = self.runtime.transform_markdown(
                        &message.content,
                        "assistant",
                        false,
                        self.tui.columns.max(1),
                    );
                    let md = Markdown::new(&transformed);
                    for line in md.render(self.tui.columns) {
                        self.chat.push("assistant", line);
                    }
                }
                pi_agent::AgentEvent::ToolStart { name, .. } => {
                    self.chat.push("tool", format!("▶ {name}"));
                }
                pi_agent::AgentEvent::ToolEnd { name, output, .. } => {
                    self.chat.push("tool", format!("■ {name}\n{output}"));
                }
                pi_agent::AgentEvent::Error { message } => self.chat.push("error", message),
                pi_agent::AgentEvent::Compaction { summary } => {
                    self.chat.push("system", format!("compacted: {summary}"));
                }
                _ => {}
            }
        }
        for (kind, line) in self.runtime.take_custom_lines() {
            self.chat.push(&kind, line);
        }
    }

    pub fn handle_line(&mut self, line: &str) -> Result<bool, String> {
        if let Some(alt) = &mut self.alt {
            if alt.handle_input(line) {
                return Ok(false);
            }
        }
        if self.login_dialog.is_some() {
            if line == "\u{1b}" || line.eq_ignore_ascii_case("escape") {
                let _ = self.apply_login_dialog_key(&Key::Escape);
                return Ok(false);
            }
            if line.contains("\x1b[200~") {
                if let Some(dialog) = self.login_dialog.as_mut() {
                    dialog.set_focused(true);
                    match dialog.handle_input(line) {
                        LoginDialogAction::Submitted(value) => {
                            let dialog = self.login_dialog.take().expect("login dialog");
                            match login::complete_login_dialog(
                                &commands::agent_dir(),
                                &dialog,
                                &value,
                            ) {
                                Ok(message) => self.chat.push("system", message),
                                Err(err) => self.chat.push("error", err),
                            }
                        }
                        LoginDialogAction::Cancelled => {
                            self.login_dialog = None;
                            self.chat.push("system", login::login_cancelled_message());
                        }
                        LoginDialogAction::Continue => {
                            let rendered = self.login_dialog.as_ref().unwrap().render();
                            self.chat.push("system", rendered);
                        }
                    }
                }
                return Ok(false);
            }
            if let Some(dialog) = self.login_dialog.as_mut() {
                dialog.set_focused(true);
                for ch in line.chars() {
                    let _ = dialog.handle_key(&Key::Char(ch));
                }
            }
            let _ = self.apply_login_dialog_key(&Key::Enter);
            return Ok(false);
        }
        if let Some((_, hit)) = self.tui.hit_test_mouse(line) {
            self.runtime
                .bus
                .emit("mouse", json!({"overlay": hit, "raw": line}));
            return Ok(false);
        }
        let key = pi_tui::parse_key(line);
        match key {
            Key::Ctrl('c') => return Ok(true),
            Key::Ctrl('p') => {
                if let Some(data) = self.runtime.cycle_model() {
                    self.chat.push("system", data.to_string());
                }
                return Ok(false);
            }
            Key::Ctrl('t') => {
                self.runtime.set_thinking(self.runtime.thinking.cycle());
                self.chat.push(
                    "system",
                    format!("thinking {}", self.runtime.thinking.as_str()),
                );
                return Ok(false);
            }
            Key::Escape => {
                self.tui.hide_all_overlays();
                return Ok(false);
            }
            _ => {}
        }
        if let Some((cmd, args)) = slash::parse_slash(line) {
            return self.handle_slash(cmd, args);
        }
        if !line.trim().is_empty() {
            self.submit_prompt(line)?;
            if !self.runtime.ui.editor_text.is_empty() {
                self.editor.buffer = self.runtime.ui.editor_text.clone();
                self.editor.cursor = self.editor.buffer.chars().count();
            }
        }
        Ok(false)
    }

    fn try_extension_shortcut(&mut self, key: &Key) -> Result<bool, String> {
        let id = key_id(key);
        if crate::extensions::is_reserved_shortcut(&id) {
            return Ok(false);
        }
        if self
            .runtime
            .registry
            .shortcuts
            .iter()
            .any(|shortcut| shortcut.shortcut == id)
        {
            match self.runtime.invoke_shortcut(&id) {
                Ok(events) => {
                    for event in events {
                        self.apply_ui_request(&event);
                    }
                    let extra = self.runtime.take_extension_turn_events();
                    self.show_agent_events(extra);
                    self.chat.push("system", format!("shortcut {id}"));
                    return Ok(true);
                }
                Err(err) => self.chat.push("error", err),
            }
        }
        Ok(false)
    }

    fn apply_login_dialog_key(&mut self, key: &Key) -> bool {
        let Some(dialog) = self.login_dialog.as_mut() else {
            return false;
        };
        match dialog.handle_key(key) {
            LoginDialogAction::Continue => {
                let rendered = dialog.render();
                self.chat.push("system", rendered);
                true
            }
            LoginDialogAction::Submitted(value) => {
                let dialog = self.login_dialog.take().expect("login dialog");
                match login::complete_login_dialog(&commands::agent_dir(), &dialog, &value) {
                    Ok(message) => self.chat.push("system", message),
                    Err(err) => self.chat.push("error", err),
                }
                true
            }
            LoginDialogAction::Cancelled => {
                self.login_dialog = None;
                self.chat.push("system", login::login_cancelled_message());
                true
            }
        }
    }

    fn apply_trust_selector_key(&mut self, key: &Key) -> bool {
        let Some(selector) = self.trust_selector.as_mut() else {
            return false;
        };
        match selector.handle_key(key) {
            TrustSelectorAction::Continue => {
                let rendered = selector.render();
                self.chat.push("system", rendered);
                true
            }
            TrustSelectorAction::Selected(option) => {
                settings::apply_trust_option(&commands::agent_dir(), &option);
                settings::remember_session_trust(&self.runtime.cwd, option.trusted);
                self.trust_selector = None;
                self.tui.hide_all_overlays();
                self.chat.push(
                    "system",
                    format!(
                        "Saved trust decision: {}. Restart {} for this to take effect.",
                        if option.trusted {
                            "trusted"
                        } else {
                            "untrusted"
                        },
                        crate::args::APP_NAME
                    ),
                );
                true
            }
            TrustSelectorAction::Cancelled => {
                self.trust_selector = None;
                self.tui.hide_all_overlays();
                true
            }
        }
    }

    pub fn handle_key(&mut self, key: &Key) -> Result<bool, String> {
        if self.apply_login_dialog_key(key) {
            return Ok(false);
        }
        if self.apply_trust_selector_key(key) {
            return Ok(false);
        }
        if self.try_extension_shortcut(key)? {
            return Ok(false);
        }
        match key {
            Key::Ctrl('c') => return Ok(true),
            Key::Ctrl('p') => {
                if let Some(data) = self.runtime.cycle_model() {
                    self.chat.push("system", data.to_string());
                }
            }
            Key::Ctrl('t') => {
                self.runtime.set_thinking(self.runtime.thinking.cycle());
                self.chat.push(
                    "system",
                    format!("thinking {}", self.runtime.thinking.as_str()),
                );
            }
            Key::Escape => self.tui.hide_all_overlays(),
            Key::Enter => {
                let text = std::mem::take(&mut self.editor.buffer);
                self.editor.cursor = 0;
                if self.handle_line(&text)? {
                    return Ok(true);
                }
            }
            other => self.editor.handle_key(other),
        }
        Ok(false)
    }
}

fn key_id(key: &Key) -> String {
    match key {
        Key::Ctrl(c) => format!("ctrl+{c}"),
        Key::Char(c) => c.to_lowercase().to_string(),
        Key::Enter => "enter".into(),
        Key::Escape => "escape".into(),
        Key::Tab => "tab".into(),
        Key::Backspace => "backspace".into(),
        Key::Left => "left".into(),
        Key::Right => "right".into(),
        Key::Up => "up".into(),
        Key::Down => "down".into(),
        Key::Unknown(raw) => raw.to_ascii_lowercase(),
    }
}

fn decode_raw_key(bytes: &[u8]) -> Option<Key> {
    if bytes.is_empty() {
        return None;
    }
    match bytes[0] {
        0x03 => Some(Key::Ctrl('c')),
        0x10 => Some(Key::Ctrl('p')),
        0x14 => Some(Key::Ctrl('t')),
        0x0d | 0x0a => Some(Key::Enter),
        0x7f | 0x08 => Some(Key::Backspace),
        0x09 => Some(Key::Tab),
        0x1b => Some(pi_tui::parse_key(&String::from_utf8_lossy(bytes))),
        b if b.is_ascii_graphic() || b == b' ' => Some(Key::Char(b as char)),
        _ => None,
    }
}

pub fn run_interactive(mut mode: InteractiveMode, fullscreen: bool) -> Result<i32, String> {
    let mut stdout = io::stdout();
    if let Some(alt) = mode.alt.as_mut() {
        alt.start();
        write!(stdout, "{}", alt.writes.last().cloned().unwrap_or_default()).ok();
        enable_raw_input().ok();
    } else if fullscreen {
        enter_alt_screen(&mut stdout).ok();
        enable_mouse(&mut stdout).ok();
        enable_raw_input().ok();
    }
    print!("{}", mode.redraw(true));
    println!();
    if mode.runtime.messages.is_empty() {
        println!("Type a prompt, or /help. Ctrl+C to quit.");
    } else {
        let initial = mode
            .runtime
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .map(|m| m.content.clone())
            .collect::<Vec<_>>();
        for text in initial {
            mode.submit_prompt(&text)?;
            print!("{}", mode.redraw(false));
        }
    }
    let raw = fullscreen || std::env::var("PI_TUI_RAW").is_ok();
    if raw {
        let mut stdin = io::stdin();
        let mut pending = Vec::new();
        loop {
            let mut buf = [0u8; 64];
            let n = stdin.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            pending.extend_from_slice(&buf[..n]);
            let raw_str = String::from_utf8_lossy(&pending).to_string();
            if mode.tui.hit_test_mouse(&raw_str).is_some() {
                let _ = mode.handle_line(&raw_str)?;
                pending.clear();
            } else if let Some(key) = decode_raw_key(&pending) {
                pending.clear();
                if mode.handle_key(&key)? {
                    break;
                }
            } else if pending.len() > 16 {
                pending.clear();
            }
            print!("{}", mode.redraw(false));
            let _ = stdout.flush();
        }
    } else {
        for line in io::stdin().lock().lines() {
            let line = line.map_err(|e| e.to_string())?;
            if mode.handle_line(&line)? {
                break;
            }
            print!("{}", mode.redraw(false));
            let _ = stdout.flush();
        }
    }
    if let Some(alt) = mode.alt.as_mut() {
        alt.stop();
        write!(stdout, "{}", alt.writes.last().cloned().unwrap_or_default()).ok();
        disable_raw_input().ok();
    } else if fullscreen {
        disable_raw_input().ok();
        disable_mouse(&mut stdout).ok();
        leave_alt_screen(&mut stdout).ok();
    }
    Ok(0)
}
