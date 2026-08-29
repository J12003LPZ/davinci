//! TypeScript `packages/tui/src/keybindings.ts` alt-screen bindings + `matchesKey`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const SHIFT: u8 = 1;
const ALT: u8 = 2;
const CTRL: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AltScreenAction {
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    LineUp,
    LineDown,
    PreviousPrompt,
    NextPrompt,
    Search,
    SearchNext,
    SearchPrevious,
    SearchClose,
    Top,
    Bottom,
}

impl AltScreenAction {
    fn id(self) -> &'static str {
        match self {
            Self::PageUp => "tui.altScreen.pageUp",
            Self::PageDown => "tui.altScreen.pageDown",
            Self::HalfPageUp => "tui.altScreen.halfPageUp",
            Self::HalfPageDown => "tui.altScreen.halfPageDown",
            Self::LineUp => "tui.altScreen.lineUp",
            Self::LineDown => "tui.altScreen.lineDown",
            Self::PreviousPrompt => "tui.altScreen.previousPrompt",
            Self::NextPrompt => "tui.altScreen.nextPrompt",
            Self::Search => "tui.altScreen.search",
            Self::SearchNext => "tui.altScreen.searchNext",
            Self::SearchPrevious => "tui.altScreen.searchPrevious",
            Self::SearchClose => "tui.altScreen.searchClose",
            Self::Top => "tui.altScreen.top",
            Self::Bottom => "tui.altScreen.bottom",
        }
    }

    fn defaults(self) -> &'static [&'static str] {
        match self {
            Self::PageUp => &["pageUp"],
            Self::PageDown => &["pageDown"],
            Self::HalfPageUp | Self::HalfPageDown | Self::LineUp | Self::LineDown => &[],
            Self::PreviousPrompt => &["ctrl+shift+up", "ctrl+up"],
            Self::NextPrompt => &["ctrl+shift+down", "ctrl+down"],
            Self::Search => &["ctrl+shift+f"],
            Self::SearchNext => &["enter", "ctrl+g"],
            Self::SearchPrevious => &["shift+enter", "ctrl+shift+g"],
            Self::SearchClose => &["escape"],
            Self::Top => &["home"],
            Self::Bottom => &["end"],
        }
    }

    fn all() -> &'static [Self] {
        &[
            Self::PageUp,
            Self::PageDown,
            Self::HalfPageUp,
            Self::HalfPageDown,
            Self::LineUp,
            Self::LineDown,
            Self::PreviousPrompt,
            Self::NextPrompt,
            Self::Search,
            Self::SearchNext,
            Self::SearchPrevious,
            Self::SearchClose,
            Self::Top,
            Self::Bottom,
        ]
    }
}

fn bindings() -> &'static Mutex<HashMap<&'static str, Vec<String>>> {
    static BINDINGS: OnceLock<Mutex<HashMap<&'static str, Vec<String>>>> = OnceLock::new();
    BINDINGS.get_or_init(|| {
        let mut map = HashMap::new();
        for action in AltScreenAction::all() {
            map.insert(
                action.id(),
                action.defaults().iter().map(|k| (*k).to_string()).collect(),
            );
        }
        Mutex::new(map)
    })
}

pub fn set_alt_screen_bindings(overrides: &[(&str, &str)]) {
    let mut map = bindings().lock().expect("keybindings");
    for action in AltScreenAction::all() {
        map.insert(
            action.id(),
            action.defaults().iter().map(|k| (*k).to_string()).collect(),
        );
    }
    for (id, key) in overrides {
        if let Some(action) = AltScreenAction::all()
            .iter()
            .find(|action| action.id() == *id)
        {
            map.insert(action.id(), vec![(*key).to_string()]);
        }
    }
}

pub fn reset_alt_screen_bindings() {
    set_alt_screen_bindings(&[]);
}

pub fn is_key_release(data: &str) -> bool {
    data.contains(":3u") || data.ends_with(":3u")
}

pub fn matches_alt_screen(data: &str, action: AltScreenAction) -> bool {
    let map = bindings().lock().expect("keybindings");
    let keys = map.get(action.id()).cloned().unwrap_or_default();
    drop(map);
    keys.iter().any(|key| matches_key(data, key))
}

pub fn matches_key(data: &str, key_id: &str) -> bool {
    let (key, modifier) = parse_key_id(key_id);
    match key {
        "escape" | "esc" => modifier == 0 && (data == "\u{1b}" || kitty(data, 27, 0)),
        "enter" | "return" => match modifier {
            0 => data == "\r" || data == "\n" || data == "\u{1b}OM" || kitty(data, 13, 0),
            m if m == SHIFT => kitty(data, 13, SHIFT) || modify_other(data, 13, SHIFT),
            m => kitty(data, 13, m) || modify_other(data, 13, m),
        },
        "pageUp" => functional(data, modifier, &["\u{1b}[5~"], 57421, Some(('5', '~'))),
        "pageDown" => functional(data, modifier, &["\u{1b}[6~"], 57422, Some(('6', '~'))),
        "home" => functional(
            data,
            modifier,
            &["\u{1b}OH", "\u{1b}[H", "\u{1b}[1~"],
            57423,
            Some(('1', 'H')),
        ),
        "end" => functional(
            data,
            modifier,
            &["\u{1b}OF", "\u{1b}[F", "\u{1b}[4~"],
            57424,
            Some(('1', 'F')),
        ),
        "up" => arrow(data, modifier, 'A', 57419),
        "down" => arrow(data, modifier, 'B', 57420),
        other if other.len() == 1 => {
            let ch = other.chars().next().unwrap();
            if modifier == 0 {
                return data == other;
            }
            if modifier == CTRL && ch.is_ascii_lowercase() {
                let ctrl = (ch as u8 - b'a' + 1) as char;
                if data == ctrl.to_string() {
                    return true;
                }
            }
            kitty(data, ch as u32, modifier) || modify_other(data, ch as u32, modifier)
        }
        _ => false,
    }
}

fn parse_key_id(key_id: &str) -> (&str, u8) {
    let mut modifier = 0u8;
    let mut key = key_id;
    loop {
        if let Some(rest) = key.strip_prefix("ctrl+") {
            modifier |= CTRL;
            key = rest;
            continue;
        }
        if let Some(rest) = key.strip_prefix("shift+") {
            modifier |= SHIFT;
            key = rest;
            continue;
        }
        if let Some(rest) = key.strip_prefix("alt+") {
            modifier |= ALT;
            key = rest;
            continue;
        }
        break;
    }
    (key, modifier)
}

fn kitty(data: &str, codepoint: u32, modifier: u8) -> bool {
    let encoded = modifier + 1;
    let press = format!("\u{1b}[{codepoint};{encoded}u");
    let release = format!("\u{1b}[{codepoint};{encoded}:3u");
    data == press || data == release
}

fn modify_other(data: &str, codepoint: u32, modifier: u8) -> bool {
    let encoded = modifier + 1;
    data == format!("\u{1b}[27;{encoded};{codepoint}~")
}

fn functional(
    data: &str,
    modifier: u8,
    legacy: &[&str],
    kitty_code: u32,
    tilde: Option<(char, char)>,
) -> bool {
    if modifier == 0 && legacy.iter().any(|seq| data == *seq) {
        return true;
    }
    if let Some((n, end)) = tilde {
        if modifier == 0 && data == format!("\u{1b}[{n}{end}") {
            return true;
        }
        let encoded = modifier + 1;
        if data == format!("\u{1b}[{n};{encoded}{end}") {
            return true;
        }
    }
    kitty(data, kitty_code, modifier)
}

fn arrow(data: &str, modifier: u8, letter: char, kitty_code: u32) -> bool {
    if modifier == 0 && (data == format!("\u{1b}[{letter}") || data == format!("\u{1b}O{letter}")) {
        return true;
    }
    let encoded = modifier + 1;
    data == format!("\u{1b}[1;{encoded}{letter}") || kitty(data, kitty_code, modifier)
}
