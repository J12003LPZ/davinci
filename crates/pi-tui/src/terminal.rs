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
}
