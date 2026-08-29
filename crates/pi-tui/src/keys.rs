use serde::{Deserialize, Serialize};

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
        "\u{7f}" | "backspace" => Key::Backspace,
        "\t" | "tab" => Key::Tab,
        "\u{1b}" | "escape" => Key::Escape,
        "left" | "\u{1b}[D" => Key::Left,
        "right" | "\u{1b}[C" => Key::Right,
        "up" | "\u{1b}[A" => Key::Up,
        "down" | "\u{1b}[B" => Key::Down,
        other if other.starts_with("ctrl+") && other.len() == 6 => {
            Key::Ctrl(other.chars().last().unwrap_or('c'))
        }
        other if other.chars().count() == 1 => Key::Char(other.chars().next().unwrap()),
        other => Key::Unknown(other.to_string()),
    }
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
