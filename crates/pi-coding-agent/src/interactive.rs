//! InteractiveMode: TUI host + slash commands + AgentSession.

use crate::commands;
use crate::extensions::{self, Extension};
use crate::session_runtime::{to_json_event, SessionRuntime};
use crate::settings;
use crate::slash::{self, BUILTIN_SLASH_COMMANDS};
use pi_agent::ThinkingLevel;
use pi_ai::AuthStorage;
use pi_session::{default_sessions_root, discover_sessions};
use pi_tui::component::Component;
use pi_tui::{
    default_keybindings, disable_mouse, disable_raw_input, enable_mouse, enable_raw_input,
    enter_alt_screen, leave_alt_screen, set_title, ChatView, Editor, Key, Markdown, Overlay,
    OverlayOptions, SelectList, SettingsList, Tui, TuiMode,
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
        Self {
            runtime,
            tui,
            chat,
            editor: Editor::default(),
            theme: "default".into(),
            discovered,
            auto_approve: true,
            last_json_events: Vec::new(),
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
        lines.extend(self.editor.render(self.tui.columns));
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
                let body = BUILTIN_SLASH_COMMANDS
                    .iter()
                    .map(|(name, desc)| format!("/{name}  {desc}"))
                    .collect();
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
                let keys = default_keybindings()
                    .into_iter()
                    .map(|k| format!("{}  {}", k.key, k.action))
                    .collect();
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
                self.chat.push(
                    "system",
                    "Share: export HTML then create a secret GitHub gist (gh gist create --secret)",
                );
            }
            "copy" => match self.runtime.last_assistant() {
                Some(text) => self
                    .chat
                    .push("system", format!("copied {} chars", text.len())),
                None => self.chat.push("error", "no assistant message"),
            },
            "changelog" => {
                self.chat.push(
                    "system",
                    format!(
                        "pi {} changelog: see vendor/pi CHANGELOG",
                        crate::args::VERSION
                    ),
                );
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
                settings::save_trust(&commands::agent_dir(), &self.runtime.cwd);
                self.chat
                    .push("system", format!("trusted {}", self.runtime.cwd.display()));
            }
            "login" => {
                if let Some(url) = pi_ai::authorize_url(args, "http://127.0.0.1:8765/cb", "pi") {
                    self.chat.push("system", format!("Open: {url}"));
                } else {
                    self.chat.push("error", "usage: /login <provider>");
                }
            }
            "logout" => {
                if let Ok(mut store) =
                    pi_ai::FileAuthStorage::open(commands::agent_dir().join("auth.json"))
                {
                    let _ = store.delete(args);
                }
                self.chat.push("system", format!("logged out {args}"));
            }
            "new" => {
                self.runtime.new_session(None)?;
                self.chat = ChatView::default();
                self.chat.push("system", "new session");
                let title = self.runtime.ui.set_title("pi");
                self.apply_ui_request(&title);
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
                self.chat.push(
                    "system",
                    format!("reloaded ({} channels)", self.runtime.bus.channel_count()),
                );
            }
            other => self.chat.push("system", format!("/{other} {args}")),
        }
        Ok(false)
    }

    pub fn submit_prompt(&mut self, text: &str) -> Result<(), String> {
        self.chat.push("user", text);
        let events = self.runtime.prompt(text, vec![])?;
        self.last_json_events = events.iter().map(to_json_event).collect();
        for event in events {
            match event {
                pi_agent::AgentEvent::Message { message } => {
                    let md = Markdown::new(&message.content);
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
        Ok(())
    }

    pub fn handle_line(&mut self, line: &str) -> Result<bool, String> {
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

    pub fn handle_key(&mut self, key: &Key) -> Result<bool, String> {
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
    if fullscreen {
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
    if fullscreen {
        disable_raw_input().ok();
        disable_mouse(&mut stdout).ok();
        leave_alt_screen(&mut stdout).ok();
    }
    Ok(0)
}
