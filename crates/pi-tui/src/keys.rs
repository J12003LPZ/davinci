#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub name: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

pub fn parse_key(input: &str) -> Key {
    let lower = input.to_ascii_lowercase();
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut name = lower.as_str();
    if let Some(rest) = name.strip_prefix("ctrl+") {
        ctrl = true;
        name = rest;
    }
    if let Some(rest) = name.strip_prefix("alt+") {
        alt = true;
        name = rest;
    }
    if let Some(rest) = name.strip_prefix("shift+") {
        shift = true;
        name = rest;
    }
    Key {
        name: name.to_string(),
        ctrl,
        alt,
        shift,
    }
}

const KITTY_FUNCTIONAL: &[(u32, u32)] = &[
    (57399, 48),
    (57400, 49),
    (57401, 50),
    (57402, 51),
    (57403, 52),
    (57404, 53),
    (57405, 54),
    (57406, 55),
    (57407, 56),
    (57408, 57),
    (57409, 46),
    (57410, 47),
    (57411, 42),
    (57412, 45),
    (57413, 43),
    (57415, 61),
    (57416, 44),
];

fn normalize_kitty_functional(codepoint: u32) -> u32 {
    KITTY_FUNCTIONAL
        .iter()
        .find(|(from, _)| *from == codepoint)
        .map(|(_, to)| *to)
        .unwrap_or(codepoint)
}

/// Decode Kitty CSI-u printable input matching `decodeKittyPrintable` in
/// `vendor/pi/packages/tui/src/keys.ts`.
pub fn decode_kitty_printable(data: &str) -> Option<String> {
    let rest = data.strip_prefix("\u{1b}[")?;
    let rest = rest.strip_suffix('u')?;
    let (codepoint_part, mods) = rest.split_once(';').unwrap_or((rest, "1"));
    let mut parts = codepoint_part.split(':');
    let codepoint = parts.next()?.parse::<u32>().ok()?;
    let shifted = parts.next().and_then(|value| {
        if value.is_empty() {
            None
        } else {
            value.parse::<u32>().ok()
        }
    });
    let mod_value = mods
        .split(':')
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1);
    let modifier = mod_value.saturating_sub(1);
    // TS: reject alt/ctrl and unsupported Super/Meta bits. Shift (1) and lock (8) are allowed.
    const SHIFT: u32 = 1;
    const ALT: u32 = 2;
    const CTRL: u32 = 4;
    const LOCK: u32 = 8;
    if (modifier & !(SHIFT | LOCK)) != 0 {
        return None;
    }
    if modifier & (ALT | CTRL) != 0 {
        return None;
    }
    let mut effective = codepoint;
    if modifier & SHIFT != 0 {
        if let Some(shifted) = shifted {
            effective = shifted;
        }
    }
    effective = normalize_kitty_functional(effective);
    if !(32..57344).contains(&effective) {
        return None;
    }
    char::from_u32(effective).map(|ch| ch.to_string())
}

/// TS `isKeyRelease`.
pub fn is_key_release(data: &str) -> bool {
    if data.contains("\x1b[200~") {
        return false;
    }
    data.contains(":3u")
        || data.contains(":3~")
        || data.contains(":3A")
        || data.contains(":3B")
        || data.contains(":3C")
        || data.contains(":3D")
        || data.contains(":3H")
        || data.contains(":3F")
}
