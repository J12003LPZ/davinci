//! TS `vendor/pi/packages/tui/src/terminal.ts` helpers (Shift+Enter, ESC timeout, Kitty negotiation).

pub const NATIVE_SHIFT_ENTER_SEQUENCE: &str = "\x1b[13;2u";
pub const DEFAULT_ESCAPE_TIMEOUT_MS: u64 = 10;
pub const DEFAULT_SSH_ESCAPE_TIMEOUT_MS: u64 = 100;
pub const KEYBOARD_PROTOCOL_RESPONSE_FRAGMENT_TIMEOUT_MS: u64 = 150;
pub const DESIRED_KITTY_KEYBOARD_PROTOCOL_FLAGS: u8 = 7;
pub const KITTY_KEYBOARD_PROTOCOL_QUERY: &str = "\x1b[>7u\x1b[?u\x1b[c";
pub const MODIFY_OTHER_KEYS_ENABLE: &str = "\x1b[>4;2m";
pub const MODIFY_OTHER_KEYS_DISABLE: &str = "\x1b[>4;0m";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyboardProtocolNegotiationSequence {
    KittyFlags { flags: u32 },
    DeviceAttributes,
}

pub fn parse_keyboard_protocol_negotiation_sequence(
    sequence: &str,
) -> Option<KeyboardProtocolNegotiationSequence> {
    let rest = sequence.strip_prefix("\u{1b}[?")?;
    if let Some(flags) = rest.strip_suffix('u') {
        return flags
            .parse()
            .ok()
            .map(|flags| KeyboardProtocolNegotiationSequence::KittyFlags { flags });
    }
    if rest.ends_with('c')
        && rest[..rest.len() - 1]
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == ';')
    {
        return Some(KeyboardProtocolNegotiationSequence::DeviceAttributes);
    }
    None
}

pub fn is_keyboard_protocol_negotiation_sequence_prefix(sequence: &str) -> bool {
    sequence == "\u{1b}["
        || sequence
            .strip_prefix("\u{1b}[?")
            .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit() || ch == ';'))
}

pub fn is_apple_terminal_session(os: &str, term_program: Option<&str>) -> bool {
    os == "darwin" && term_program == Some("Apple_Terminal")
}

pub fn is_apple_terminal_session_from_env() -> bool {
    is_apple_terminal_session(
        if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        },
        std::env::var("TERM_PROGRAM").ok().as_deref(),
    )
}

pub fn normalize_native_shift_enter_input(
    data: &str,
    should_detect_native_shift_enter: bool,
    is_shift_pressed: bool,
) -> String {
    if should_detect_native_shift_enter && data == "\r" && is_shift_pressed {
        NATIVE_SHIFT_ENTER_SEQUENCE.into()
    } else {
        data.to_string()
    }
}

pub fn normalize_apple_terminal_input(
    data: &str,
    is_apple_terminal: bool,
    is_shift_pressed: bool,
) -> String {
    normalize_native_shift_enter_input(data, is_apple_terminal, is_shift_pressed)
}

pub fn resolve_escape_timeout_ms(lookup: impl Fn(&str) -> Option<String>) -> u64 {
    if let Some(configured) = lookup("PI_TUI_ESC_TIMEOUT") {
        if let Ok(value) = configured.parse::<f64>() {
            if value.is_finite() && value > 0.0 {
                return value as u64;
            }
        }
    }
    if lookup("SSH_CONNECTION").is_some() || lookup("SSH_TTY").is_some() {
        return DEFAULT_SSH_ESCAPE_TIMEOUT_MS;
    }
    DEFAULT_ESCAPE_TIMEOUT_MS
}

pub fn resolve_escape_timeout_ms_from_env() -> u64 {
    resolve_escape_timeout_ms(|key| std::env::var(key).ok())
}

pub fn should_detect_native_shift_enter(sequence: &str) -> bool {
    if sequence != "\r" {
        return false;
    }
    if is_apple_terminal_session_from_env() {
        return true;
    }
    if cfg!(windows) {
        return true;
    }
    matches!(
        std::env::var("PI_TUI_NATIVE_SHIFT").as_deref(),
        Ok("1") | Ok("true")
    )
}

pub fn native_shift_is_pressed() -> bool {
    matches!(
        std::env::var("PI_TUI_SHIFT").as_deref(),
        Ok("1") | Ok("true")
    ) || crate::native::is_native_modifier_pressed(crate::native::ModifierKey::Shift)
}

use std::sync::atomic::{AtomicBool, Ordering};

use crate::native::{is_native_modifier_pressed, ModifierKey};
use crate::osc::{
    TERMINAL_PROGRESS_ACTIVE_SEQUENCE, TERMINAL_PROGRESS_CLEAR_SEQUENCE,
    TERMINAL_PROGRESS_KEEPALIVE_MS,
};
use crate::render::AsAny;
use crate::stdin_buffer::{StdinBuffer, StdinBufferOptions, StdinEvents};

static KITTY_PROTOCOL_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn set_kitty_protocol_active(active: bool) {
    KITTY_PROTOCOL_ACTIVE.store(active, Ordering::Relaxed);
}

pub fn is_kitty_protocol_active() -> bool {
    KITTY_PROTOCOL_ACTIVE.load(Ordering::Relaxed)
}

/// Minimal terminal interface matching TS `Terminal`.
pub trait TerminalIo: AsAny {
    fn write(&mut self, data: &str);
    fn columns(&self) -> usize;
    fn rows(&self) -> usize;
    fn kitty_protocol_active(&self) -> bool {
        false
    }
    fn start(&mut self) {}
    fn stop(&mut self) {}
    fn hide_cursor(&mut self) {
        self.write("\x1b[?25l");
    }
    fn show_cursor(&mut self) {
        self.write("\x1b[?25h");
    }
    fn clear_line(&mut self) {
        self.write("\x1b[K");
    }
    fn clear_from_cursor(&mut self) {
        self.write("\x1b[J");
    }
    fn clear_screen(&mut self) {
        self.write("\x1b[2J\x1b[H");
    }
    fn set_title(&mut self, title: &str) {
        self.write(&format!("\x1b]0;{title}\x07"));
    }
    fn set_progress(&mut self, active: bool) {
        if active {
            self.write(TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
        } else {
            self.write(TERMINAL_PROGRESS_CLEAR_SEQUENCE);
        }
    }
    fn move_by(&mut self, lines: i32) {
        match lines.cmp(&0) {
            std::cmp::Ordering::Greater => self.write(&format!("\x1b[{lines}B")),
            std::cmp::Ordering::Less => self.write(&format!("\x1b[{}A", -lines)),
            std::cmp::Ordering::Equal => {}
        }
    }
}

/// In-memory terminal used by TUI tests (TS `BoundedWriteTerminal` / virtual writes).
pub struct MemoryTerminal {
    pub writes: Vec<String>,
    pub columns: usize,
    pub rows: usize,
    pub kitty_protocol_active: bool,
}

impl MemoryTerminal {
    pub fn new(columns: usize, rows: usize) -> Self {
        Self {
            writes: Vec::new(),
            columns,
            rows,
            kitty_protocol_active: false,
        }
    }

    pub fn output(&self) -> String {
        self.writes.concat()
    }

    pub fn clear_writes(&mut self) {
        self.writes.clear();
    }
}

impl TerminalIo for MemoryTerminal {
    fn write(&mut self, data: &str) {
        self.writes.push(data.to_string());
    }

    fn columns(&self) -> usize {
        self.columns
    }

    fn rows(&self) -> usize {
        self.rows
    }

    fn kitty_protocol_active(&self) -> bool {
        self.kitty_protocol_active
    }

    fn hide_cursor(&mut self) {}

    fn show_cursor(&mut self) {}
}

type InputHandler = Box<dyn FnMut(&str)>;

/// Real / fixture terminal matching TS `ProcessTerminal`.
pub struct ProcessTerminal {
    writes: Vec<String>,
    capture_writes: bool,
    columns_override: Option<usize>,
    rows_override: Option<usize>,
    kitty_protocol_active: bool,
    modify_other_keys_active: bool,
    keyboard_protocol_pushed: bool,
    negotiation_buffer: String,
    negotiation_flush_at: Option<u64>,
    now_ms: u64,
    stdin_buffer: Option<StdinBuffer>,
    progress_active: bool,
    progress_next_ms: Option<u64>,
    input_handler: Option<InputHandler>,
    write_log_path: Option<String>,
    started: bool,
}

impl Default for ProcessTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessTerminal {
    pub fn new() -> Self {
        let write_log_path = std::env::var("PI_TUI_WRITE_LOG")
            .ok()
            .filter(|value| !value.is_empty());
        Self {
            writes: Vec::new(),
            capture_writes: true,
            columns_override: None,
            rows_override: None,
            kitty_protocol_active: false,
            modify_other_keys_active: false,
            keyboard_protocol_pushed: false,
            negotiation_buffer: String::new(),
            negotiation_flush_at: None,
            now_ms: 0,
            stdin_buffer: None,
            progress_active: false,
            progress_next_ms: None,
            input_handler: None,
            write_log_path,
            started: false,
        }
    }

    pub fn with_dimensions(columns: usize, rows: usize) -> Self {
        let mut terminal = Self::new();
        terminal.columns_override = Some(columns);
        terminal.rows_override = Some(rows);
        terminal
    }

    pub fn captured_writes(&self) -> &[String] {
        &self.writes
    }

    pub fn clear_writes(&mut self) {
        self.writes.clear();
    }

    pub fn set_input_handler(&mut self, handler: impl FnMut(&str) + 'static) {
        self.input_handler = Some(Box::new(handler));
    }

    pub fn kitty_protocol_active(&self) -> bool {
        self.kitty_protocol_active
    }

    pub fn modify_other_keys_active(&self) -> bool {
        self.modify_other_keys_active
    }

    pub fn query_and_enable_kitty_protocol(&mut self) {
        self.setup_stdin_buffer();
        self.keyboard_protocol_pushed = true;
        self.clear_keyboard_protocol_negotiation_buffer();
        self.emit(KITTY_KEYBOARD_PROTOCOL_QUERY);
    }

    pub fn start(&mut self, on_input: impl FnMut(&str) + 'static) {
        self.input_handler = Some(Box::new(on_input));
        self.started = true;
        self.emit("\x1b[?2004h");
        crate::native::enable_virtual_terminal_input();
        self.query_and_enable_kitty_protocol();
    }

    pub fn stop(&mut self) {
        if self.clear_progress_interval() {
            self.emit(TERMINAL_PROGRESS_CLEAR_SEQUENCE);
        }
        self.emit("\x1b[?2004l");
        let should_disable = self.keyboard_protocol_pushed || self.kitty_protocol_active;
        self.clear_keyboard_protocol_negotiation_buffer();
        if should_disable {
            self.emit("\x1b[<u");
            self.keyboard_protocol_pushed = false;
            self.kitty_protocol_active = false;
            set_kitty_protocol_active(false);
        }
        self.disable_modify_other_keys();
        self.stdin_buffer = None;
        self.input_handler = None;
        self.started = false;
    }

    pub fn feed(&mut self, data: &str) {
        if let Some(mut buffer) = self.stdin_buffer.take() {
            let events = buffer.process(data);
            self.stdin_buffer = Some(buffer);
            self.dispatch_stdin_events(events);
        } else {
            self.forward_input_sequence(data);
        }
    }

    pub fn tick(&mut self, ms: u64) {
        self.now_ms = self.now_ms.saturating_add(ms);
        if let Some(mut buffer) = self.stdin_buffer.take() {
            let events = buffer.tick(ms);
            self.stdin_buffer = Some(buffer);
            self.dispatch_stdin_events(events);
        }
        if let Some(at) = self.negotiation_flush_at {
            if self.now_ms >= at {
                self.flush_keyboard_protocol_negotiation_buffer_as_input();
            }
        }
        if self.progress_active {
            if let Some(next) = self.progress_next_ms {
                if self.now_ms >= next {
                    self.emit(TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
                    self.progress_next_ms =
                        Some(self.now_ms.saturating_add(TERMINAL_PROGRESS_KEEPALIVE_MS));
                }
            }
        }
    }

    pub fn set_progress(&mut self, active: bool) {
        if active {
            self.emit(TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
            if self.progress_next_ms.is_none() {
                self.progress_next_ms =
                    Some(self.now_ms.saturating_add(TERMINAL_PROGRESS_KEEPALIVE_MS));
            }
            self.progress_active = true;
        } else {
            self.clear_progress_interval();
            self.emit(TERMINAL_PROGRESS_CLEAR_SEQUENCE);
        }
    }

    fn emit(&mut self, data: &str) {
        if self.capture_writes {
            self.writes.push(data.to_string());
        } else {
            use std::io::Write;
            let _ = std::io::stdout().write_all(data.as_bytes());
        }
        if let Some(path) = &self.write_log_path {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| {
                    use std::io::Write;
                    file.write_all(data.as_bytes())
                });
        }
    }

    fn setup_stdin_buffer(&mut self) {
        self.stdin_buffer = Some(StdinBuffer::with_options(StdinBufferOptions {
            timeout: 50,
            escape_timeout: resolve_escape_timeout_ms_from_env(),
        }));
    }

    fn dispatch_stdin_events(&mut self, events: StdinEvents) {
        for sequence in events.data {
            match self.read_keyboard_protocol_negotiation_sequence(&sequence) {
                NegotiationRead::Pending => {
                    self.schedule_keyboard_protocol_negotiation_buffer_flush();
                }
                NegotiationRead::Handled => {}
                NegotiationRead::Forward => self.forward_input_sequence(&sequence),
            }
        }
        for content in events.paste {
            self.forward_input_sequence(&format!("\x1b[200~{content}\x1b[201~"));
        }
    }

    fn handle_keyboard_protocol_negotiation_sequence(
        &mut self,
        sequence: KeyboardProtocolNegotiationSequence,
    ) -> bool {
        self.clear_keyboard_protocol_negotiation_buffer();
        match sequence {
            KeyboardProtocolNegotiationSequence::KittyFlags { flags } => {
                if flags != 0 {
                    self.disable_modify_other_keys();
                    if !self.kitty_protocol_active {
                        self.kitty_protocol_active = true;
                        set_kitty_protocol_active(true);
                    }
                } else {
                    self.enable_modify_other_keys();
                }
                true
            }
            KeyboardProtocolNegotiationSequence::DeviceAttributes => {
                if !self.kitty_protocol_active {
                    self.enable_modify_other_keys();
                }
                true
            }
        }
    }

    fn read_keyboard_protocol_negotiation_sequence(&mut self, sequence: &str) -> NegotiationRead {
        if !self.negotiation_buffer.is_empty() {
            let buffered = format!("{}{sequence}", self.negotiation_buffer);
            if let Some(parsed) = parse_keyboard_protocol_negotiation_sequence(&buffered) {
                self.clear_keyboard_protocol_negotiation_buffer();
                self.handle_keyboard_protocol_negotiation_sequence(parsed);
                return NegotiationRead::Handled;
            }
            if is_keyboard_protocol_negotiation_sequence_prefix(&buffered) {
                self.set_keyboard_protocol_negotiation_buffer(buffered);
                return NegotiationRead::Pending;
            }
            self.flush_keyboard_protocol_negotiation_buffer_as_input();
        }
        if let Some(parsed) = parse_keyboard_protocol_negotiation_sequence(sequence) {
            self.handle_keyboard_protocol_negotiation_sequence(parsed);
            return NegotiationRead::Handled;
        }
        if is_keyboard_protocol_negotiation_sequence_prefix(sequence) {
            self.set_keyboard_protocol_negotiation_buffer(sequence.to_string());
            return NegotiationRead::Pending;
        }
        NegotiationRead::Forward
    }

    fn set_keyboard_protocol_negotiation_buffer(&mut self, sequence: String) {
        self.negotiation_flush_at = None;
        self.negotiation_buffer = sequence;
    }

    fn clear_keyboard_protocol_negotiation_buffer(&mut self) {
        self.negotiation_flush_at = None;
        self.negotiation_buffer.clear();
    }

    fn flush_keyboard_protocol_negotiation_buffer_as_input(&mut self) {
        if self.negotiation_buffer.is_empty() {
            return;
        }
        let sequence = std::mem::take(&mut self.negotiation_buffer);
        self.negotiation_flush_at = None;
        self.forward_input_sequence(&sequence);
    }

    fn schedule_keyboard_protocol_negotiation_buffer_flush(&mut self) {
        if self.negotiation_buffer.is_empty() || self.negotiation_flush_at.is_some() {
            return;
        }
        self.negotiation_flush_at = Some(
            self.now_ms
                .saturating_add(KEYBOARD_PROTOCOL_RESPONSE_FRAGMENT_TIMEOUT_MS),
        );
    }

    fn forward_input_sequence(&mut self, sequence: &str) {
        let should_detect =
            sequence == "\r" && (is_apple_terminal_session_from_env() || cfg!(windows));
        let input = normalize_native_shift_enter_input(
            sequence,
            should_detect,
            should_detect && is_native_modifier_pressed(ModifierKey::Shift),
        );
        if let Some(handler) = &mut self.input_handler {
            handler(&input);
        }
    }

    fn enable_modify_other_keys(&mut self) {
        if self.kitty_protocol_active || self.modify_other_keys_active {
            return;
        }
        self.emit(MODIFY_OTHER_KEYS_ENABLE);
        self.modify_other_keys_active = true;
    }

    fn disable_modify_other_keys(&mut self) {
        if !self.modify_other_keys_active {
            return;
        }
        self.emit(MODIFY_OTHER_KEYS_DISABLE);
        self.modify_other_keys_active = false;
    }

    fn clear_progress_interval(&mut self) -> bool {
        if !self.progress_active && self.progress_next_ms.is_none() {
            return false;
        }
        self.progress_active = false;
        self.progress_next_ms = None;
        true
    }
}

enum NegotiationRead {
    Pending,
    Handled,
    Forward,
}

impl TerminalIo for ProcessTerminal {
    fn write(&mut self, data: &str) {
        self.emit(data);
    }

    fn columns(&self) -> usize {
        if let Some(columns) = self.columns_override {
            return columns;
        }
        std::env::var("COLUMNS")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|&value| value > 0)
            .unwrap_or(80)
    }

    fn rows(&self) -> usize {
        if let Some(rows) = self.rows_override {
            return rows;
        }
        std::env::var("LINES")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|&value| value > 0)
            .unwrap_or(24)
    }

    fn kitty_protocol_active(&self) -> bool {
        self.kitty_protocol_active
    }

    fn start(&mut self) {
        self.emit("\x1b[?2004h");
        crate::native::enable_virtual_terminal_input();
        self.query_and_enable_kitty_protocol();
        self.started = true;
    }

    fn stop(&mut self) {
        ProcessTerminal::stop(self);
    }

    fn set_progress(&mut self, active: bool) {
        ProcessTerminal::set_progress(self, active);
    }
}

pub fn rewrite_shift_enter_input(data: &str) -> String {
    let detect = should_detect_native_shift_enter(data);
    let shift = detect && native_shift_is_pressed();
    let after_native = normalize_native_shift_enter_input(data, detect, shift);
    normalize_apple_terminal_input(&after_native, is_apple_terminal_session_from_env(), shift)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn escape_timeout_and_shift_enter_lock_ts_terminal_tests() {
        assert_eq!(
            resolve_escape_timeout_ms(env(&[("PI_TUI_ESC_TIMEOUT", "80")])),
            80
        );
        assert_eq!(
            resolve_escape_timeout_ms(env(&[
                ("PI_TUI_ESC_TIMEOUT", "80"),
                ("SSH_TTY", "/dev/pts/1")
            ])),
            80
        );
        assert_eq!(
            resolve_escape_timeout_ms(env(&[("PI_TUI_ESC_TIMEOUT", "abc")])),
            10
        );
        assert_eq!(
            resolve_escape_timeout_ms(env(&[("PI_TUI_ESC_TIMEOUT", "0")])),
            10
        );
        assert_eq!(
            resolve_escape_timeout_ms(env(&[("PI_TUI_ESC_TIMEOUT", "-5")])),
            10
        );
        assert_eq!(
            resolve_escape_timeout_ms(env(&[("PI_TUI_ESC_TIMEOUT", "")])),
            10
        );
        assert_eq!(
            resolve_escape_timeout_ms(env(&[("SSH_CONNECTION", "10.0.0.1 22")])),
            100
        );
        assert_eq!(
            resolve_escape_timeout_ms(env(&[("SSH_TTY", "/dev/pts/1")])),
            100
        );
        assert_eq!(resolve_escape_timeout_ms(env(&[])), 10);

        assert_eq!(
            normalize_native_shift_enter_input("\r", true, true),
            NATIVE_SHIFT_ENTER_SEQUENCE
        );
        assert_eq!(normalize_native_shift_enter_input("\r", false, true), "\r");
        assert_eq!(normalize_native_shift_enter_input("\r", true, false), "\r");
        assert_eq!(
            normalize_native_shift_enter_input(NATIVE_SHIFT_ENTER_SEQUENCE, true, true),
            NATIVE_SHIFT_ENTER_SEQUENCE
        );
        assert_eq!(normalize_native_shift_enter_input("a", true, true), "a");

        assert_eq!(
            normalize_apple_terminal_input("\r", true, true),
            NATIVE_SHIFT_ENTER_SEQUENCE
        );
        assert_eq!(normalize_apple_terminal_input("\r", true, false), "\r");
        assert_eq!(normalize_apple_terminal_input("\r", false, true), "\r");
        assert_eq!(
            normalize_apple_terminal_input(NATIVE_SHIFT_ENTER_SEQUENCE, true, true),
            NATIVE_SHIFT_ENTER_SEQUENCE
        );
        assert_eq!(normalize_apple_terminal_input("a", true, true), "a");

        assert!(is_apple_terminal_session("darwin", Some("Apple_Terminal")));
        assert!(!is_apple_terminal_session("linux", Some("Apple_Terminal")));
        assert!(!is_apple_terminal_session("darwin", Some("iTerm.app")));

        assert_eq!(
            parse_keyboard_protocol_negotiation_sequence("\x1b[?7u"),
            Some(KeyboardProtocolNegotiationSequence::KittyFlags { flags: 7 })
        );
        assert_eq!(
            parse_keyboard_protocol_negotiation_sequence("\x1b[?0u"),
            Some(KeyboardProtocolNegotiationSequence::KittyFlags { flags: 0 })
        );
        assert_eq!(
            parse_keyboard_protocol_negotiation_sequence("\x1b[?62;4;52c"),
            Some(KeyboardProtocolNegotiationSequence::DeviceAttributes)
        );
        assert!(parse_keyboard_protocol_negotiation_sequence("a").is_none());
        assert!(is_keyboard_protocol_negotiation_sequence_prefix("\x1b["));
        assert!(is_keyboard_protocol_negotiation_sequence_prefix("\x1b[?7"));
        assert!(!is_keyboard_protocol_negotiation_sequence_prefix(
            "\x1b[?7u"
        ));
    }

    #[test]
    fn process_terminal_kitty_negotiation_and_progress_lock_ts() {
        let mut terminal = ProcessTerminal::new();
        let last = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
        let last_input = last.clone();
        terminal.set_input_handler(move |data| {
            *last_input.borrow_mut() = Some(data.to_string());
        });
        terminal.query_and_enable_kitty_protocol();
        assert_eq!(
            terminal.captured_writes().first().map(String::as_str),
            Some(KITTY_KEYBOARD_PROTOCOL_QUERY)
        );
        assert!(!terminal
            .captured_writes()
            .iter()
            .any(|write| write == MODIFY_OTHER_KEYS_ENABLE));
        assert!(!terminal.kitty_protocol_active());

        terminal.feed("\x1b[?7u");
        assert!(last.borrow().is_none());
        assert!(terminal.kitty_protocol_active());
        assert!(!terminal
            .captured_writes()
            .iter()
            .any(|write| write == MODIFY_OTHER_KEYS_ENABLE));
        terminal.stop();
        assert_eq!(
            terminal
                .captured_writes()
                .iter()
                .filter(|write| write.as_str() == "\x1b[<u")
                .count(),
            1
        );

        let mut zero = ProcessTerminal::new();
        zero.query_and_enable_kitty_protocol();
        zero.feed("\x1b[?0u");
        assert!(!zero.kitty_protocol_active());
        assert_eq!(
            zero.captured_writes()
                .iter()
                .filter(|write| write.as_str() == MODIFY_OTHER_KEYS_ENABLE)
                .count(),
            1
        );
        zero.stop();
        assert_eq!(
            zero.captured_writes()
                .iter()
                .filter(|write| write.as_str() == MODIFY_OTHER_KEYS_DISABLE)
                .count(),
            1
        );

        let mut da = ProcessTerminal::new();
        da.query_and_enable_kitty_protocol();
        da.feed("\x1b[?62;4;52c");
        assert!(!da.kitty_protocol_active());
        assert_eq!(
            da.captured_writes()
                .iter()
                .filter(|write| write.as_str() == MODIFY_OTHER_KEYS_ENABLE)
                .count(),
            1
        );

        let mut forward = ProcessTerminal::new();
        let last = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
        let last_input = last.clone();
        forward.set_input_handler(move |data| {
            *last_input.borrow_mut() = Some(data.to_string());
        });
        forward.query_and_enable_kitty_protocol();
        forward.feed("a");
        assert_eq!(last.borrow().as_deref(), Some("a"));

        let mut split = ProcessTerminal::new();
        split.query_and_enable_kitty_protocol();
        split.feed("\x1b[?7");
        split.tick(10);
        assert!(!split.kitty_protocol_active());
        split.feed("u");
        assert!(split.kitty_protocol_active());

        let mut replay = ProcessTerminal::new();
        let last = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
        let last_input = last.clone();
        replay.set_input_handler(move |data| {
            *last_input.borrow_mut() = Some(data.to_string());
        });
        replay.query_and_enable_kitty_protocol();
        replay.feed("\x1b[");
        replay.tick(50);
        assert!(last.borrow().is_none());
        replay.tick(150);
        assert_eq!(last.borrow().as_deref(), Some("\x1b["));

        let mut progress = ProcessTerminal::new();
        progress.set_progress(false);
        assert_eq!(progress.captured_writes(), &["\x1b]9;4;0\x07"]);

        let previous_columns = std::env::var("COLUMNS").ok();
        let previous_lines = std::env::var("LINES").ok();
        std::env::set_var("COLUMNS", "123");
        std::env::set_var("LINES", "45");
        let sized = ProcessTerminal::new();
        assert_eq!(sized.columns(), 123);
        assert_eq!(sized.rows(), 45);
        match previous_columns {
            Some(value) => std::env::set_var("COLUMNS", value),
            None => std::env::remove_var("COLUMNS"),
        }
        match previous_lines {
            Some(value) => std::env::set_var("LINES", value),
            None => std::env::remove_var("LINES"),
        }
    }
}
