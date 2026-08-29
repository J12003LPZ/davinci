use serde::{Deserialize, Serialize};
use std::io::{self, Read};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Ctrl(char),
    Unknown(String),
}

pub fn parse_key(raw: &str) -> Key {
    match raw {
        "\r" | "\n" | "enter" => Key::Enter,
        "\u{7f}" | "\u{8}" | "backspace" => Key::Backspace,
        "\t" | "tab" => Key::Tab,
        "\u{1b}" | "escape" => Key::Escape,
        "left" | "\u{1b}[D" => Key::Left,
        "right" | "\u{1b}[C" => Key::Right,
        "up" | "\u{1b}[A" => Key::Up,
        "down" | "\u{1b}[B" => Key::Down,
        other if other.starts_with("ctrl+") && other.len() == 6 => {
            Key::Ctrl(other.chars().last().unwrap_or('c'))
        }
        other if other.len() == 1 => parse_bytes(other.as_bytes()),
        other => Key::Unknown(other.to_string()),
    }
}

pub fn parse_bytes(bytes: &[u8]) -> Key {
    if bytes.is_empty() {
        return Key::Unknown(String::new());
    }
    if bytes.len() == 1 {
        return match bytes[0] {
            b'\r' | b'\n' => Key::Enter,
            0x7f | 0x08 => Key::Backspace,
            b'\t' => Key::Tab,
            0x1b => Key::Escape,
            b @ 1..=26 => Key::Ctrl((b'a' + b - 1) as char),
            b => Key::Char(b as char),
        };
    }
    parse_key(&String::from_utf8_lossy(bytes))
}

pub fn read_key(stdin: &mut impl Read) -> io::Result<Option<Key>> {
    let mut buf = [0u8; 64];
    let n = stdin.read(&mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(parse_bytes(&buf[..n])))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keybinding {
    pub key: String,
    pub action: String,
}

pub fn default_keybindings() -> Vec<Keybinding> {
    vec![
        Keybinding {
            key: "ctrl+c".into(),
            action: "interrupt".into(),
        },
        Keybinding {
            key: "ctrl+p".into(),
            action: "cycle_model".into(),
        },
        Keybinding {
            key: "escape".into(),
            action: "cancel".into(),
        },
    ]
}
