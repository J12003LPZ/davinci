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
