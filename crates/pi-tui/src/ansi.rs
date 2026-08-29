//! ANSI-aware wrap/truncate matching `vendor/pi/packages/tui/src/utils.ts`.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Osc8Terminator {
    Bel,
    St,
}

impl Osc8Terminator {
    fn as_str(self) -> &'static str {
        match self {
            Self::Bel => "\x07",
            Self::St => "\x1b\\",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveHyperlink {
    params: String,
    url: String,
    terminator: Osc8Terminator,
}

fn is_printable_ascii(text: &str) -> bool {
    text.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

fn could_be_emoji(segment: &str) -> bool {
    let Some(cp) = segment.chars().next().map(|ch| ch as u32) else {
        return false;
    };
    (0x1f000..=0x1fbff).contains(&cp)
        || (0x2300..=0x23ff).contains(&cp)
        || (0x2600..=0x27bf).contains(&cp)
        || (0x2b50..=0x2b55).contains(&cp)
        || segment.contains('\u{fe0f}')
        || segment.chars().count() > 2
}

fn is_zero_width_cluster(segment: &str) -> bool {
    !segment.is_empty()
        && segment.chars().all(|ch| {
            matches!(ch,
                '\u{0000}'..='\u{0008}'
                    | '\u{000e}'..='\u{001f}'
                    | '\u{007f}'..='\u{009f}'
                    | '\u{00ad}'
                    | '\u{0300}'..='\u{036f}'
                    | '\u{0483}'..='\u{0489}'
                    | '\u{0591}'..='\u{05bd}'
                    | '\u{05bf}'
                    | '\u{05c1}'..='\u{05c2}'
                    | '\u{05c4}'..='\u{05c5}'
                    | '\u{05c7}'
                    | '\u{0610}'..='\u{061a}'
                    | '\u{064b}'..='\u{065f}'
                    | '\u{0670}'
                    | '\u{06d6}'..='\u{06dc}'
                    | '\u{06df}'..='\u{06e4}'
                    | '\u{06e7}'..='\u{06e8}'
                    | '\u{06ea}'..='\u{06ed}'
                    | '\u{0711}'
                    | '\u{0730}'..='\u{074a}'
                    | '\u{07a6}'..='\u{07b0}'
                    | '\u{07eb}'..='\u{07f3}'
                    | '\u{0816}'..='\u{0819}'
                    | '\u{081b}'..='\u{0823}'
                    | '\u{0825}'..='\u{0827}'
                    | '\u{0829}'..='\u{082d}'
                    | '\u{0859}'..='\u{085b}'
                    | '\u{08d3}'..='\u{08e1}'
                    | '\u{08e3}'..='\u{0902}'
                    | '\u{093a}'
                    | '\u{093c}'
                    | '\u{0941}'..='\u{0948}'
                    | '\u{094d}'
                    | '\u{0951}'..='\u{0957}'
                    | '\u{0962}'..='\u{0963}'
                    | '\u{09bc}'
                    | '\u{09c1}'..='\u{09c4}'
                    | '\u{09cd}'
                    | '\u{09e2}'..='\u{09e3}'
                    | '\u{0a01}'..='\u{0a02}'
                    | '\u{0a3c}'
                    | '\u{0a41}'..='\u{0a42}'
                    | '\u{0a47}'..='\u{0a48}'
                    | '\u{0a4b}'..='\u{0a4d}'
                    | '\u{0a51}'
                    | '\u{0a70}'..='\u{0a71}'
                    | '\u{0a75}'
                    | '\u{0a81}'..='\u{0a82}'
                    | '\u{0abc}'
                    | '\u{0ac1}'..='\u{0ac5}'
                    | '\u{0ac7}'..='\u{0ac8}'
                    | '\u{0acd}'
                    | '\u{0ae2}'..='\u{0ae3}'
                    | '\u{0afa}'..='\u{0aff}'
                    | '\u{0b01}'
                    | '\u{0b3c}'
                    | '\u{0b3f}'
                    | '\u{0b41}'..='\u{0b44}'
                    | '\u{0b4d}'
                    | '\u{0b56}'
                    | '\u{0b62}'..='\u{0b63}'
                    | '\u{0b82}'
                    | '\u{0bc0}'
                    | '\u{0bcd}'
                    | '\u{0c00}'
                    | '\u{0c04}'
                    | '\u{0c3c}'
                    | '\u{0c3e}'..='\u{0c40}'
                    | '\u{0c46}'..='\u{0c48}'
                    | '\u{0c4a}'..='\u{0c4d}'
                    | '\u{0c55}'..='\u{0c56}'
                    | '\u{0c62}'..='\u{0c63}'
                    | '\u{0c81}'
                    | '\u{0cbc}'
                    | '\u{0cbf}'
                    | '\u{0cc6}'
                    | '\u{0ccc}'..='\u{0ccd}'
                    | '\u{0ce2}'..='\u{0ce3}'
                    | '\u{0d00}'..='\u{0d01}'
                    | '\u{0d3b}'..='\u{0d3c}'
                    | '\u{0d41}'..='\u{0d44}'
                    | '\u{0d4d}'
                    | '\u{0d62}'..='\u{0d63}'
                    | '\u{0dca}'
                    | '\u{0dd2}'..='\u{0dd4}'
                    | '\u{0dd6}'
                    | '\u{0e31}'
                    | '\u{0e34}'..='\u{0e3a}'
                    | '\u{0e47}'..='\u{0e4e}'
                    | '\u{0eb1}'
                    | '\u{0eb4}'..='\u{0ebc}'
                    | '\u{0ec8}'..='\u{0ecd}'
                    | '\u{0f18}'..='\u{0f19}'
                    | '\u{0f35}'
                    | '\u{0f37}'
                    | '\u{0f39}'
                    | '\u{0f71}'..='\u{0f7e}'
                    | '\u{0f80}'..='\u{0f84}'
                    | '\u{0f86}'..='\u{0f87}'
                    | '\u{0f8d}'..='\u{0f97}'
                    | '\u{0f99}'..='\u{0fbc}'
                    | '\u{0fc6}'
                    | '\u{102d}'..='\u{1030}'
                    | '\u{1032}'
                    | '\u{1036}'..='\u{1037}'
                    | '\u{1039}'
                    | '\u{103d}'
                    | '\u{1058}'..='\u{1059}'
                    | '\u{105e}'..='\u{1060}'
                    | '\u{1071}'..='\u{1074}'
                    | '\u{1082}'
                    | '\u{1085}'..='\u{1086}'
                    | '\u{108d}'
                    | '\u{109d}'
                    | '\u{135d}'..='\u{135f}'
                    | '\u{1712}'..='\u{1714}'
                    | '\u{1732}'..='\u{1733}'
                    | '\u{1752}'..='\u{1753}'
                    | '\u{1772}'..='\u{1773}'
                    | '\u{17b4}'..='\u{17b5}'
                    | '\u{17b7}'..='\u{17bd}'
                    | '\u{17c6}'
                    | '\u{17c9}'..='\u{17d3}'
                    | '\u{17dd}'
                    | '\u{180b}'..='\u{180d}'
                    | '\u{1885}'..='\u{1886}'
                    | '\u{18a9}'
                    | '\u{1920}'..='\u{1922}'
                    | '\u{1927}'..='\u{1928}'
                    | '\u{1932}'
                    | '\u{1939}'..='\u{193b}'
                    | '\u{1a17}'..='\u{1a18}'
                    | '\u{1a1b}'
                    | '\u{1a56}'
                    | '\u{1a58}'..='\u{1a5e}'
                    | '\u{1a60}'
                    | '\u{1a62}'
                    | '\u{1a65}'..='\u{1a6c}'
                    | '\u{1a73}'..='\u{1a7c}'
                    | '\u{1a7f}'
                    | '\u{1ab0}'..='\u{1abe}'
                    | '\u{1b00}'..='\u{1b03}'
                    | '\u{1b34}'
                    | '\u{1b36}'..='\u{1b3a}'
                    | '\u{1b3c}'
                    | '\u{1b42}'
                    | '\u{1b6b}'..='\u{1b73}'
                    | '\u{1b80}'..='\u{1b81}'
                    | '\u{1ba2}'..='\u{1ba5}'
                    | '\u{1ba8}'..='\u{1ba9}'
                    | '\u{1bab}'..='\u{1bad}'
                    | '\u{1be6}'
                    | '\u{1be8}'..='\u{1be9}'
                    | '\u{1bed}'
                    | '\u{1bef}'..='\u{1bf1}'
                    | '\u{1c2c}'..='\u{1c33}'
                    | '\u{1c36}'..='\u{1c37}'
                    | '\u{1cd0}'..='\u{1cd2}'
                    | '\u{1cd4}'..='\u{1ce0}'
                    | '\u{1ce2}'..='\u{1ce8}'
                    | '\u{1ced}'
                    | '\u{1cf4}'
                    | '\u{1cf8}'..='\u{1cf9}'
                    | '\u{1dc0}'..='\u{1df9}'
                    | '\u{1dfb}'..='\u{1dff}'
                    | '\u{20d0}'..='\u{20f0}'
                    | '\u{2cef}'..='\u{2cf1}'
                    | '\u{2d7f}'
                    | '\u{2de0}'..='\u{2dff}'
                    | '\u{302a}'..='\u{302d}'
                    | '\u{3099}'
                    | '\u{309a}'
                    | '\u{a66f}'..='\u{a672}'
                    | '\u{a674}'..='\u{a67d}'
                    | '\u{a69e}'..='\u{a69f}'
                    | '\u{a6f0}'..='\u{a6f1}'
                    | '\u{a802}'
                    | '\u{a806}'
                    | '\u{a80b}'
                    | '\u{a825}'..='\u{a826}'
                    | '\u{a8c4}'..='\u{a8c5}'
                    | '\u{a8e0}'..='\u{a8f1}'
                    | '\u{a8ff}'
                    | '\u{a926}'..='\u{a92d}'
                    | '\u{a947}'..='\u{a951}'
                    | '\u{a980}'..='\u{a982}'
                    | '\u{a9b3}'
                    | '\u{a9b6}'..='\u{a9b9}'
                    | '\u{a9bc}'..='\u{a9bd}'
                    | '\u{a9e5}'
                    | '\u{aa29}'..='\u{aa2e}'
                    | '\u{aa31}'..='\u{aa32}'
                    | '\u{aa35}'..='\u{aa36}'
                    | '\u{aa43}'
                    | '\u{aa4c}'
                    | '\u{aa7c}'
                    | '\u{aab0}'
                    | '\u{aab2}'..='\u{aab4}'
                    | '\u{aab7}'..='\u{aab8}'
                    | '\u{aabe}'..='\u{aabf}'
                    | '\u{aac1}'
                    | '\u{aaec}'..='\u{aaed}'
                    | '\u{aaf6}'
                    | '\u{abe5}'
                    | '\u{abe8}'
                    | '\u{abed}'
                    | '\u{fb1e}'
                    | '\u{fe00}'..='\u{fe0f}'
                    | '\u{fe20}'..='\u{fe2f}'
                    | '\u{200b}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2060}'..='\u{2064}'
                    | '\u{2066}'..='\u{206f}'
                    | '\u{feff}'
                    | '\u{fff9}'..='\u{fffb}'
            )
        })
}

fn is_mark_char(ch: char) -> bool {
    matches!(ch, '\u{0300}'..='\u{036f}' | '\u{064b}'..='\u{065f}' | '\u{0900}'..='\u{0902}'
        | '\u{093a}' | '\u{093c}' | '\u{0941}'..='\u{094d}' | '\u{0951}'..='\u{0957}'
        | '\u{0962}'..='\u{0963}' | '\u{09bc}' | '\u{09c1}'..='\u{09c4}' | '\u{09cd}'
        | '\u{0a3c}' | '\u{0abc}' | '\u{0acd}' | '\u{0b3c}' | '\u{0b4d}' | '\u{0bcd}'
        | '\u{0c4d}' | '\u{0ccd}' | '\u{0d4d}' | '\u{0dca}' | '\u{0e31}' | '\u{0e34}'..='\u{0e3a}'
        | '\u{0e47}'..='\u{0e4e}' | '\u{0eb1}' | '\u{0eb4}'..='\u{0ebc}' | '\u{0ec8}'..='\u{0ecd}'
        | '\u{102d}'..='\u{1030}' | '\u{1032}' | '\u{1036}'..='\u{1037}' | '\u{1039}'
        | '\u{3099}' | '\u{309a}' | '\u{302a}'..='\u{302f}')
        || UnicodeWidthChar::width(ch) == Some(0)
}

fn is_non_printing_char(ch: char) -> bool {
    is_mark_char(ch)
        || matches!(ch, '\u{0000}'..='\u{001f}' | '\u{007f}'..='\u{009f}' | '\u{00ad}'
            | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
            | '\u{feff}')
}

fn is_terminal_spacing_mark(ch: char) -> bool {
    matches!(
        ch,
        '\u{065f}'
            | '\u{0f7f}'
            | '\u{102b}'
            | '\u{102c}'
            | '\u{1031}'
            | '\u{1033}'..='\u{1035}'
            | '\u{1038}'
            | '\u{103a}'..='\u{103e}'
            | '\u{093e}'..='\u{0940}'
            | '\u{09be}'..='\u{09c0}'
            | '\u{0abe}'..='\u{0ac0}'
            | '\u{0b3e}'
            | '\u{0bbe}'..='\u{0bbf}'
            | '\u{0c3e}'..='\u{0c44}'
            | '\u{0cbe}'
            | '\u{0d3e}'..='\u{0d40}'
            | '\u{0dcf}'
            | '\u{0e30}'
            | '\u{0e32}'
            | '\u{0eb0}'
            | '\u{0eb2}'
    ) && ch != '\u{1734}'
        && ch != '\u{302e}'
        && ch != '\u{302f}'
}

fn is_terminal_spacing_mark_cluster(segment: &str) -> bool {
    !segment.is_empty() && segment.chars().all(is_terminal_spacing_mark)
}

fn east_asian_width(cp: u32) -> usize {
    char::from_u32(cp)
        .and_then(UnicodeWidthChar::width)
        .unwrap_or(0)
}

fn looks_like_rgi_emoji(segment: &str) -> bool {
    could_be_emoji(segment)
        && (segment.contains('\u{fe0f}')
            || segment.contains('\u{200d}')
            || segment.chars().any(|ch| {
                let cp = ch as u32;
                (0x1f300..=0x1faff).contains(&cp) || (0x2600..=0x27bf).contains(&cp)
            }))
}

pub fn grapheme_width(segment: &str) -> usize {
    if segment == "\t" {
        return 3;
    }
    if is_terminal_spacing_mark_cluster(segment) {
        return segment.chars().count();
    }
    if is_zero_width_cluster(segment) {
        return 0;
    }
    if looks_like_rgi_emoji(segment) {
        return 2;
    }
    let base: String = segment
        .chars()
        .skip_while(|ch| is_non_printing_char(*ch))
        .collect();
    let Some(cp) = base.chars().next().map(|ch| ch as u32) else {
        return 0;
    };
    if (0x1f1e6..=0x1f1ff).contains(&cp) {
        return 2;
    }
    let mut width = east_asian_width(cp);
    let mut follows_mark = false;
    for ch in base.chars().skip(1) {
        if is_terminal_spacing_mark(ch) {
            width += 1;
            follows_mark = false;
        } else if is_mark_char(ch) {
            follows_mark = true;
        } else if !is_non_printing_char(ch) {
            let c = ch as u32;
            if follows_mark || (0xff00..=0xffef).contains(&c) {
                width += east_asian_width(c);
            } else if c == 0x0e33 || c == 0x0eb3 {
                width += 1;
            }
            follows_mark = false;
        }
    }
    width
}

pub fn extract_ansi_code(text: &str, pos: usize) -> Option<(String, usize)> {
    if pos >= text.len() || !text.is_char_boundary(pos) || !text[pos..].starts_with('\x1b') {
        return None;
    }
    let rest = &text[pos..];
    let mut chars = rest.chars();
    chars.next()?;
    match chars.next() {
        Some('[') => {
            let mut j = 2;
            let bytes = rest.as_bytes();
            while j < rest.len() {
                let b = bytes[j];
                if matches!(b, b'm' | b'G' | b'K' | b'H' | b'J') {
                    return Some((rest[..j + 1].to_string(), j + 1));
                }
                j += 1;
            }
            None
        }
        Some(']') | Some('_') => {
            let mut j = 2;
            let bytes = rest.as_bytes();
            while j < rest.len() {
                if bytes[j] == 0x07 {
                    return Some((rest[..j + 1].to_string(), j + 1));
                }
                if bytes[j] == 0x1b && j + 1 < rest.len() && bytes[j + 1] == b'\\' {
                    return Some((rest[..j + 2].to_string(), j + 2));
                }
                j += 1;
            }
            None
        }
        _ => None,
    }
}

fn parse_osc8_hyperlink(ansi_code: &str) -> Option<Option<ActiveHyperlink>> {
    if !ansi_code.starts_with("\x1b]8;") {
        return None;
    }
    let terminator = if ansi_code.ends_with('\u{07}') {
        Osc8Terminator::Bel
    } else {
        Osc8Terminator::St
    };
    let trim = if terminator == Osc8Terminator::Bel {
        1
    } else {
        2
    };
    let body = &ansi_code[4..ansi_code.len() - trim];
    let separator = body.find(';')?;
    let params = body[..separator].to_string();
    let url = body[separator + 1..].to_string();
    if url.is_empty() {
        Some(None)
    } else {
        Some(Some(ActiveHyperlink {
            params,
            url,
            terminator,
        }))
    }
}

fn format_osc8_hyperlink(hyperlink: &ActiveHyperlink) -> String {
    format!(
        "\x1b]8;{};{}{}",
        hyperlink.params,
        hyperlink.url,
        hyperlink.terminator.as_str()
    )
}

fn format_osc8_close(terminator: Osc8Terminator) -> String {
    format!("\x1b]8;;{}", terminator.as_str())
}

fn get_active_osc8_close(prefix: &str) -> String {
    if !prefix.contains("\x1b]8;") {
        return String::new();
    }
    let mut active = None;
    let mut i = 0;
    while i < prefix.len() {
        if let Some((code, len)) = extract_ansi_code(prefix, i) {
            if let Some(hyperlink) = parse_osc8_hyperlink(&code) {
                active = hyperlink;
            }
            i += len;
        } else {
            i += prefix[i..].chars().next().map_or(1, |ch| ch.len_utf8());
        }
    }
    active
        .map(|link| format_osc8_close(link.terminator))
        .unwrap_or_default()
}

/// TS `visibleWidth`.
pub fn visible_width(str: &str) -> usize {
    if str.is_empty() {
        return 0;
    }
    if is_printable_ascii(str) {
        return str.len();
    }
    let mut clean = str.to_string();
    if clean.contains('\t') {
        clean = clean.replace('\t', "   ");
    }
    if clean.contains('\x1b') {
        let mut stripped = String::new();
        let mut i = 0;
        while i < clean.len() {
            if let Some((_, len)) = extract_ansi_code(&clean, i) {
                i += len;
                continue;
            }
            let ch = clean[i..].chars().next().unwrap();
            stripped.push(ch);
            i += ch.len_utf8();
        }
        clean = stripped;
    }
    clean.graphemes(true).map(grapheme_width).sum()
}

pub fn strip_terminal_sequences(str: &str) -> String {
    if !str.contains('\x1b') {
        return str.to_string();
    }
    let mut result = String::new();
    let mut i = 0;
    while i < str.len() {
        if let Some((_, len)) = extract_ansi_code(str, i) {
            i += len;
            continue;
        }
        let ch = str[i..].chars().next().unwrap();
        result.push(ch);
        i += ch.len_utf8();
    }
    result
}

pub fn normalize_terminal_output(str: &str) -> String {
    let mut normalized = str.replace('\u{0e33}', "\u{0e4d}\u{0e32}");
    normalized = normalized.replace('\u{0eb3}', "\u{0ecd}\u{0eb2}");
    if !normalized.contains('\t') {
        return normalized;
    }
    let mut result = String::new();
    let mut i = 0;
    while i < normalized.len() {
        if let Some((code, len)) = extract_ansi_code(&normalized, i) {
            result.push_str(&code);
            i += len;
            continue;
        }
        let ch = normalized[i..].chars().next().unwrap();
        if ch == '\t' {
            result.push_str("   ");
        } else {
            result.push(ch);
        }
        i += ch.len_utf8();
    }
    result
}

#[derive(Default)]
struct AnsiCodeTracker {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    inverse: bool,
    hidden: bool,
    strikethrough: bool,
    fg_color: Option<String>,
    bg_color: Option<String>,
    active_hyperlink: Option<ActiveHyperlink>,
}

impl AnsiCodeTracker {
    fn process(&mut self, ansi_code: &str) {
        if let Some(hyperlink) = parse_osc8_hyperlink(ansi_code) {
            self.active_hyperlink = hyperlink;
            return;
        }
        if !ansi_code.ends_with('m') {
            return;
        }
        let Some(params) = ansi_code
            .strip_prefix("\x1b[")
            .and_then(|rest| rest.strip_suffix('m'))
        else {
            return;
        };
        if params.is_empty() || params == "0" {
            self.reset();
            return;
        }
        let parts: Vec<&str> = params.split(';').collect();
        let mut i = 0;
        while i < parts.len() {
            let code = parts[i].parse::<i32>().unwrap_or(-1);
            if code == 38 || code == 48 {
                if parts.get(i + 1) == Some(&"5") && parts.get(i + 2).is_some() {
                    let color = format!("{};{};{}", parts[i], parts[i + 1], parts[i + 2]);
                    if code == 38 {
                        self.fg_color = Some(color);
                    } else {
                        self.bg_color = Some(color);
                    }
                    i += 3;
                    continue;
                }
                if parts.get(i + 1) == Some(&"2") && parts.get(i + 4).is_some() {
                    let color = format!(
                        "{};{};{};{};{}",
                        parts[i],
                        parts[i + 1],
                        parts[i + 2],
                        parts[i + 3],
                        parts[i + 4]
                    );
                    if code == 38 {
                        self.fg_color = Some(color);
                    } else {
                        self.bg_color = Some(color);
                    }
                    i += 5;
                    continue;
                }
            }
            match code {
                0 => self.reset(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 => self.underline = true,
                5 => self.blink = true,
                7 => self.inverse = true,
                8 => self.hidden = true,
                9 => self.strikethrough = true,
                21 => self.bold = false,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                23 => self.italic = false,
                24 => self.underline = false,
                25 => self.blink = false,
                27 => self.inverse = false,
                28 => self.hidden = false,
                29 => self.strikethrough = false,
                39 => self.fg_color = None,
                49 => self.bg_color = None,
                30..=37 | 90..=97 => self.fg_color = Some(code.to_string()),
                40..=47 | 100..=107 => self.bg_color = Some(code.to_string()),
                _ => {}
            }
            i += 1;
        }
    }

    fn reset(&mut self) {
        self.bold = false;
        self.dim = false;
        self.italic = false;
        self.underline = false;
        self.blink = false;
        self.inverse = false;
        self.hidden = false;
        self.strikethrough = false;
        self.fg_color = None;
        self.bg_color = None;
    }

    fn get_active_codes(&self) -> String {
        let mut codes = Vec::new();
        if self.bold {
            codes.push("1".into());
        }
        if self.dim {
            codes.push("2".into());
        }
        if self.italic {
            codes.push("3".into());
        }
        if self.underline {
            codes.push("4".into());
        }
        if self.blink {
            codes.push("5".into());
        }
        if self.inverse {
            codes.push("7".into());
        }
        if self.hidden {
            codes.push("8".into());
        }
        if self.strikethrough {
            codes.push("9".into());
        }
        if let Some(fg) = &self.fg_color {
            codes.push(fg.clone());
        }
        if let Some(bg) = &self.bg_color {
            codes.push(bg.clone());
        }
        let mut result = if codes.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", codes.join(";"))
        };
        if let Some(link) = &self.active_hyperlink {
            result.push_str(&format_osc8_hyperlink(link));
        }
        result
    }

    fn get_line_end_reset(&self) -> String {
        let mut result = String::new();
        if self.underline {
            result.push_str("\x1b[24m");
        }
        if let Some(link) = &self.active_hyperlink {
            result.push_str(&format_osc8_close(link.terminator));
        }
        result
    }
}

fn update_tracker_from_text(text: &str, tracker: &mut AnsiCodeTracker) {
    let mut i = 0;
    while i < text.len() {
        if let Some((code, len)) = extract_ansi_code(text, i) {
            tracker.process(&code);
            i += len;
        } else {
            i += text[i..].chars().next().map_or(1, |ch| ch.len_utf8());
        }
    }
}

fn is_cjk_break(segment: &str) -> bool {
    segment.chars().any(|ch| {
        matches!(
            ch,
            '\u{3040}'..='\u{30ff}'
                | '\u{3100}'..='\u{312f}'
                | '\u{3400}'..='\u{4dbf}'
                | '\u{4e00}'..='\u{9fff}'
                | '\u{ac00}'..='\u{d7af}'
                | '\u{f900}'..='\u{faff}'
                | '\u{20000}'..='\u{2a6df}'
        )
    })
}

fn split_into_tokens_with_ansi(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut pending_ansi = String::new();
    let mut current_kind: Option<bool> = None;
    let mut i = 0;
    let flush_current =
        |current: &mut String, current_kind: &mut Option<bool>, tokens: &mut Vec<String>| {
            if !current.is_empty() {
                tokens.push(std::mem::take(current));
                *current_kind = None;
            }
        };
    while i < text.len() {
        if let Some((code, len)) = extract_ansi_code(text, i) {
            pending_ansi.push_str(&code);
            i += len;
            continue;
        }
        let mut end = i;
        while end < text.len() && extract_ansi_code(text, end).is_none() {
            end += text[end..].chars().next().map_or(1, |ch| ch.len_utf8());
        }
        for segment in text[i..end].graphemes(true) {
            if segment != " " && is_cjk_break(segment) {
                flush_current(&mut current, &mut current_kind, &mut tokens);
                let mut token = std::mem::take(&mut pending_ansi);
                token.push_str(segment);
                tokens.push(token);
                continue;
            }
            let segment_is_space = segment == " ";
            if !current.is_empty() && current_kind != Some(segment_is_space) {
                flush_current(&mut current, &mut current_kind, &mut tokens);
            }
            if !pending_ansi.is_empty() {
                current.push_str(&pending_ansi);
                pending_ansi.clear();
            }
            current_kind = Some(segment_is_space);
            current.push_str(segment);
        }
        i = end;
    }
    if !pending_ansi.is_empty() {
        if !current.is_empty() {
            current.push_str(&pending_ansi);
        } else if let Some(last) = tokens.last_mut() {
            last.push_str(&pending_ansi);
        } else {
            current = pending_ansi;
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn break_long_word(word: &str, width: usize, tracker: &mut AnsiCodeTracker) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_line = tracker.get_active_codes();
    let mut current_width = 0usize;
    let mut segments = Vec::new();
    let mut i = 0;
    while i < word.len() {
        if let Some((code, len)) = extract_ansi_code(word, i) {
            segments.push((true, code));
            i += len;
        } else {
            let mut end = i;
            while end < word.len() && extract_ansi_code(word, end).is_none() {
                end += word[end..].chars().next().map_or(1, |ch| ch.len_utf8());
            }
            for grapheme in word[i..end].graphemes(true) {
                segments.push((false, grapheme.to_string()));
            }
            i = end;
        }
    }
    for (is_ansi, value) in segments {
        if is_ansi {
            current_line.push_str(&value);
            tracker.process(&value);
            continue;
        }
        if value.is_empty() {
            continue;
        }
        let gw = visible_width(&value);
        if current_width + gw > width {
            current_line.push_str(&tracker.get_line_end_reset());
            lines.push(std::mem::take(&mut current_line));
            current_line = tracker.get_active_codes();
            current_width = 0;
        }
        current_line.push_str(&value);
        current_width += gw;
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn wrap_single_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    if visible_width(line) <= width {
        return vec![line.to_string()];
    }
    let mut wrapped = Vec::new();
    let mut tracker = AnsiCodeTracker::default();
    let tokens = split_into_tokens_with_ansi(line);
    let mut current_line = String::new();
    let mut current_visible = 0usize;
    for token in tokens {
        let token_visible = visible_width(&token);
        let is_whitespace = token.trim().is_empty();
        if token_visible > width && !is_whitespace {
            if !current_line.is_empty() {
                current_line.push_str(&tracker.get_line_end_reset());
                wrapped.push(std::mem::take(&mut current_line));
            }
            let broken = break_long_word(&token, width, &mut tracker);
            for line in broken.iter().take(broken.len().saturating_sub(1)) {
                wrapped.push(line.clone());
            }
            current_line = broken.last().cloned().unwrap_or_default();
            current_visible = visible_width(&current_line);
            continue;
        }
        if current_visible + token_visible > width && current_visible > 0 {
            let mut line_to_wrap = current_line.trim_end().to_string();
            line_to_wrap.push_str(&tracker.get_line_end_reset());
            wrapped.push(line_to_wrap);
            if is_whitespace {
                current_line = tracker.get_active_codes();
                current_visible = 0;
            } else {
                current_line = format!("{}{token}", tracker.get_active_codes());
                current_visible = token_visible;
            }
        } else {
            current_line.push_str(&token);
            current_visible += token_visible;
        }
        update_tracker_from_text(&token, &mut tracker);
    }
    if !current_line.is_empty() {
        wrapped.push(current_line);
    }
    if wrapped.is_empty() {
        vec![String::new()]
    } else {
        wrapped
            .into_iter()
            .map(|line| line.trim_end().to_string())
            .collect()
    }
}

/// TS `wrapTextWithAnsi`.
pub fn wrap_text_with_ansi(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let input_lines: Vec<&str> = {
        let mut lines = Vec::new();
        let mut rest = text;
        loop {
            let crlf = rest.find("\r\n");
            let cr = rest.find('\r');
            let lf = rest.find('\n');
            let next = [crlf.map(|i| (i, 2)), cr.map(|i| (i, 1)), lf.map(|i| (i, 1))]
                .into_iter()
                .flatten()
                .min_by_key(|(idx, _)| *idx);
            match next {
                Some((idx, skip)) => {
                    lines.push(&rest[..idx]);
                    rest = &rest[idx + skip..];
                }
                None => {
                    lines.push(rest);
                    break;
                }
            }
        }
        lines
    };
    let mut result = Vec::new();
    let mut tracker = AnsiCodeTracker::default();
    for input_line in input_lines {
        let prefix = if result.is_empty() {
            String::new()
        } else {
            tracker.get_active_codes()
        };
        let prefixed = format!("{prefix}{input_line}");
        result.extend(wrap_single_line(&prefixed, width));
        update_tracker_from_text(input_line, &mut tracker);
    }
    if result.is_empty() {
        vec![String::new()]
    } else {
        result
    }
}

fn truncate_fragment_to_width(text: &str, max_width: usize) -> (String, usize) {
    if max_width == 0 || text.is_empty() {
        return (String::new(), 0);
    }
    if is_printable_ascii(text) {
        let clipped: String = text.chars().take(max_width).collect();
        let width = clipped.len();
        return (clipped, width);
    }
    let has_ansi = text.contains('\x1b');
    let has_tabs = text.contains('\t');
    if !has_ansi && !has_tabs {
        let mut result = String::new();
        let mut width = 0;
        for segment in text.graphemes(true) {
            let w = grapheme_width(segment);
            if width + w > max_width {
                break;
            }
            result.push_str(segment);
            width += w;
        }
        return (result, width);
    }
    let mut result = String::new();
    let mut width = 0;
    let mut i = 0;
    let mut pending_ansi = String::new();
    while i < text.len() {
        if let Some((code, len)) = extract_ansi_code(text, i) {
            pending_ansi.push_str(&code);
            i += len;
            continue;
        }
        if text[i..].starts_with('\t') {
            if width + 3 > max_width {
                break;
            }
            result.push_str(&pending_ansi);
            pending_ansi.clear();
            result.push('\t');
            width += 3;
            i += 1;
            continue;
        }
        let mut end = i;
        while end < text.len()
            && !text[end..].starts_with('\t')
            && extract_ansi_code(text, end).is_none()
        {
            end += text[end..].chars().next().map_or(1, |ch| ch.len_utf8());
        }
        for segment in text[i..end].graphemes(true) {
            let w = grapheme_width(segment);
            if width + w > max_width {
                return (result, width);
            }
            result.push_str(&pending_ansi);
            pending_ansi.clear();
            result.push_str(segment);
            width += w;
        }
        i = end;
    }
    (result, width)
}

fn finalize_truncated_result(
    prefix: &str,
    prefix_width: usize,
    ellipsis: &str,
    ellipsis_width: usize,
    max_width: usize,
    pad: bool,
) -> String {
    let reset = "\x1b[0m";
    let hyperlink_close = get_active_osc8_close(prefix);
    let visible = prefix_width + ellipsis_width;
    let result = if ellipsis.is_empty() {
        format!("{prefix}{hyperlink_close}{reset}")
    } else {
        format!("{prefix}{hyperlink_close}{reset}{ellipsis}{reset}")
    };
    if pad {
        format!("{result}{}", " ".repeat(max_width.saturating_sub(visible)))
    } else {
        result
    }
}

/// TS `truncateToWidth`.
pub fn truncate_to_width(text: &str, max_width: usize, ellipsis: &str, pad: bool) -> String {
    if max_width == 0 {
        return String::new();
    }
    if text.is_empty() {
        return if pad {
            " ".repeat(max_width)
        } else {
            String::new()
        };
    }
    let ellipsis_width = visible_width(ellipsis);
    if ellipsis_width >= max_width {
        let text_width = visible_width(text);
        if text_width <= max_width {
            return if pad {
                format!("{text}{}", " ".repeat(max_width - text_width))
            } else {
                text.to_string()
            };
        }
        let (clipped, clipped_width) = truncate_fragment_to_width(ellipsis, max_width);
        if clipped_width == 0 {
            return if pad {
                " ".repeat(max_width)
            } else {
                String::new()
            };
        }
        return finalize_truncated_result("", 0, &clipped, clipped_width, max_width, pad);
    }
    if is_printable_ascii(text) {
        if text.len() <= max_width {
            return if pad {
                format!("{text}{}", " ".repeat(max_width - text.len()))
            } else {
                text.to_string()
            };
        }
        let target = max_width - ellipsis_width;
        return finalize_truncated_result(
            &text[..target],
            target,
            ellipsis,
            ellipsis_width,
            max_width,
            pad,
        );
    }
    let target_width = max_width - ellipsis_width;
    let mut result = String::new();
    let mut pending_ansi = String::new();
    let mut visible_so_far = 0usize;
    let mut kept_width = 0usize;
    let mut keep_contiguous = true;
    let mut overflowed = false;
    let has_ansi = text.contains('\x1b');
    let has_tabs = text.contains('\t');
    if !has_ansi && !has_tabs {
        for segment in text.graphemes(true) {
            let width = grapheme_width(segment);
            if keep_contiguous && kept_width + width <= target_width {
                result.push_str(segment);
                kept_width += width;
            } else {
                keep_contiguous = false;
            }
            visible_so_far += width;
            if visible_so_far > max_width {
                overflowed = true;
                break;
            }
        }
        if !overflowed {
            return if pad {
                format!(
                    "{text}{}",
                    " ".repeat(max_width.saturating_sub(visible_so_far))
                )
            } else {
                text.to_string()
            };
        }
        return finalize_truncated_result(
            &result,
            kept_width,
            ellipsis,
            ellipsis_width,
            max_width,
            pad,
        );
    }
    let mut i = 0;
    while i < text.len() {
        if let Some((code, len)) = extract_ansi_code(text, i) {
            pending_ansi.push_str(&code);
            i += len;
            continue;
        }
        if text[i..].starts_with('\t') {
            if keep_contiguous && kept_width + 3 <= target_width {
                result.push_str(&pending_ansi);
                pending_ansi.clear();
                result.push('\t');
                kept_width += 3;
            } else {
                keep_contiguous = false;
                pending_ansi.clear();
            }
            visible_so_far += 3;
            if visible_so_far > max_width {
                overflowed = true;
                break;
            }
            i += 1;
            continue;
        }
        let mut end = i;
        while end < text.len()
            && !text[end..].starts_with('\t')
            && extract_ansi_code(text, end).is_none()
        {
            end += text[end..].chars().next().map_or(1, |ch| ch.len_utf8());
        }
        for segment in text[i..end].graphemes(true) {
            let width = grapheme_width(segment);
            if keep_contiguous && kept_width + width <= target_width {
                result.push_str(&pending_ansi);
                pending_ansi.clear();
                result.push_str(segment);
                kept_width += width;
            } else {
                keep_contiguous = false;
                pending_ansi.clear();
            }
            visible_so_far += width;
            if visible_so_far > max_width {
                overflowed = true;
                break;
            }
        }
        if overflowed {
            break;
        }
        i = end;
    }
    if !overflowed && i >= text.len() {
        return if pad {
            format!(
                "{text}{}",
                " ".repeat(max_width.saturating_sub(visible_so_far))
            )
        } else {
            text.to_string()
        };
    }
    finalize_truncated_result(
        &result,
        kept_width,
        ellipsis,
        ellipsis_width,
        max_width,
        pad,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_to_width_matches_ts() {
        let text = "🙂界".repeat(100_000);
        let truncated = truncate_to_width(&text, 40, "…", false);
        assert!(visible_width(&truncated) <= 40);
        assert!(truncated.ends_with("…\x1b[0m"));

        let text = format!("\x1b[31m{}\x1b[0m", "hello ".repeat(1000));
        let truncated = truncate_to_width(&text, 20, "…", false);
        assert!(visible_width(&truncated) <= 20);
        assert!(truncated.contains("\x1b[31m"));
        assert!(truncated.ends_with("\x1b[0m…\x1b[0m"));

        let open = "\x1b]8;;https://example.com\x07";
        let close = "\x1b]8;;\x07";
        let text = format!("{open}some-longer-label-here{close}");
        assert_eq!(
            truncate_to_width(&text, 15, "...", false),
            format!("{open}some-longer-{close}\x1b[0m...\x1b[0m")
        );

        let text = format!("abc\x1bnot-ansi {}", "🙂".repeat(1000));
        let truncated = truncate_to_width(&text, 20, "…", false);
        assert!(visible_width(&truncated) <= 20);

        assert_eq!(truncate_to_width("abcdef", 1, "🙂", false), "");
        assert_eq!(
            truncate_to_width("abcdef", 2, "🙂", false),
            "\x1b[0m🙂\x1b[0m"
        );
        assert!(visible_width(&truncate_to_width("abcdef", 2, "🙂", false)) <= 2);
        assert_eq!(truncate_to_width("a", 2, "🙂", false), "a");
        assert_eq!(truncate_to_width("界", 2, "🙂", false), "界");

        let truncated = truncate_to_width("🙂界🙂界🙂界", 8, "…", true);
        assert_eq!(visible_width(&truncated), 8);

        let truncated =
            truncate_to_width(&format!("\x1b[31m{}", "hello".repeat(100)), 10, "", false);
        assert!(visible_width(&truncated) <= 10);
        assert!(truncated.ends_with("\x1b[0m"));

        assert_eq!(
            truncate_to_width("🙂\t界 \x1b_abc\x07", 7, "…", true),
            "🙂\t\x1b[0m…\x1b[0m "
        );
    }

    #[test]
    fn visible_width_matches_ts() {
        assert_eq!(visible_width("\t\x1b[31m界\x1b[0m"), 5);
        assert_eq!(visible_width("र्क"), 2);
        assert_eq!(visible_width("नेटवर्क"), 5);
        assert_eq!(visible_width("सर्वाधिकार सुरक्षित। ऑर्डर पर क्लिक करें"), 33);
        assert_eq!(visible_width("র্ক"), 2);
        assert_eq!(visible_width("ર્ક"), 2);
        assert_eq!(visible_width("ର୍କ"), 2);
        assert_eq!(visible_width("ర్క"), 2);
        assert_eq!(visible_width("ര്‍ക"), 2);
        assert_eq!(visible_width("e\u{0301}"), 1);
        assert_eq!(visible_width("čřžůú"), 5);
        assert_eq!(visible_width("שָׁ"), 1);
        assert_eq!(visible_width("بّ"), 1);
        assert_eq!(visible_width("རྐ"), 1);
        assert_eq!(visible_width("ᜠ᜴"), 1);
        assert_eq!(visible_width("가〮"), 2);
        assert_eq!(visible_width("가〯"), 2);
        assert_eq!(visible_width("网络"), 4);
        assert_eq!(visible_width("ネットワーク"), 12);
        assert_eq!(visible_width("が"), 2);
        assert_eq!(visible_width("か\u{3099}"), 2);
        assert_eq!(visible_width("ကာ"), 2);
        assert_eq!(visible_width("ကေ"), 2);
        assert_eq!(visible_width("က်"), 2);
        assert_eq!(visible_width("ကျ"), 2);
        assert_eq!(visible_width("ကြ"), 2);
        assert_eq!(visible_width("ကဳ"), 2);
        assert_eq!(visible_width("ကဴ"), 2);
        assert_eq!(visible_width("ကဵ"), 2);
        assert_eq!(visible_width("ကး"), 2);
        assert_eq!(visible_width("ကို"), 1);
        assert_eq!(visible_width("က္"), 1);
        assert_eq!(visible_width("ำ"), 1);
        assert_eq!(visible_width("ຳ"), 1);
        assert_eq!(visible_width("กำ"), 2);
        assert_eq!(visible_width("ກຳ"), 2);
        assert_eq!(normalize_terminal_output("ำ"), "ํา");
        assert_eq!(normalize_terminal_output("ຳ"), "ໍາ");
        assert_eq!(
            visible_width(&normalize_terminal_output("ำabc")),
            visible_width("ำabc")
        );
        assert_eq!(visible_width("🇨"), 2);
        assert_eq!(visible_width("🇨🇳"), 2);
        assert_eq!(visible_width("\x1b]133;A\x07hello\x1b]133;B\x07"), 5);
        assert_eq!(visible_width("\x1b]133;A\x1b\\hello\x1b]133;B\x1b\\"), 5);
    }

    #[test]
    fn wrap_text_with_ansi_matches_ts() {
        let underline_on = "\x1b[4m";
        let underline_off = "\x1b[24m";
        let url = "https://example.com/very/long/path/that/will/wrap";
        let text = format!("read this thread {underline_on}{url}{underline_off}");
        let wrapped = wrap_text_with_ansi(&text, 40);
        assert_eq!(wrapped[0], "read this thread");
        assert!(wrapped[1].starts_with(underline_on));
        assert!(wrapped[1].contains("https://"));

        let text = format!("{underline_on}underlined text here {underline_off}more");
        let wrapped = wrap_text_with_ansi(&text, 18);
        assert!(!wrapped[0].contains(&format!(" {underline_off}")));

        let url = "https://example.com/very/long/path/that/will/definitely/wrap";
        let text = format!("prefix {underline_on}{url}{underline_off} suffix");
        let wrapped = wrap_text_with_ansi(&text, 30);
        for line in wrapped.iter().take(wrapped.len().saturating_sub(1)).skip(1) {
            if line.contains(underline_on) {
                assert!(line.ends_with(underline_off));
                assert!(!line.ends_with("\x1b[0m"));
            }
        }

        let bg_blue = "\x1b[44m";
        let reset = "\x1b[0m";
        let text = format!("{bg_blue}hello world this is blue background text{reset}");
        let wrapped = wrap_text_with_ansi(&text, 15);
        for line in &wrapped {
            assert!(line.contains(bg_blue));
        }
        for line in wrapped.iter().take(wrapped.len().saturating_sub(1)) {
            assert!(!line.ends_with("\x1b[0m"));
        }

        assert_eq!(
            wrap_text_with_ansi("first\nsecond\r\nthird\rfourth", 80),
            ["first", "second", "third", "fourth"]
        );
        let red = "\x1b[31m";
        assert_eq!(
            wrap_text_with_ansi(&format!("{red}first\r\nsecond\rthird{reset}"), 80),
            [
                format!("{red}first"),
                format!("{red}second"),
                format!("{red}third{reset}")
            ]
        );

        let text = "This is an example 中文汉字测试段落内容中文汉字测试段落内容.";
        assert_eq!(
            wrap_text_with_ansi(text, 40),
            [
                "This is an example 中文汉字测试段落内容",
                "中文汉字测试段落内容."
            ]
        );
        let text =
            format!("{red}This is an example 中文汉字测试段落内容中文汉字测试段落内容.{reset}");
        let wrapped = wrap_text_with_ansi(&text, 40);
        assert_eq!(wrapped.len(), 2);
        assert_eq!(
            wrapped[0],
            format!("{red}This is an example 中文汉字测试段落内容")
        );
        assert_eq!(wrapped[1], format!("{red}中文汉字测试段落内容.{reset}"));

        let two = wrap_text_with_ansi("  ", 1);
        assert!(visible_width(&two[0]) <= 1);

        let text = format!("{red}hello world this is red{reset}");
        let wrapped = wrap_text_with_ansi(&text, 10);
        for line in wrapped.iter().skip(1) {
            assert!(line.starts_with(red));
        }
        for line in wrapped.iter().take(wrapped.len().saturating_sub(1)) {
            assert!(!line.ends_with("\x1b[0m"));
        }

        let url = "https://example.com";
        let input = format!("\x1b]8;;{url}\x1b\\0123456789\x1b]8;;\x1b\\");
        let lines = wrap_text_with_ansi(&input, 6);
        for line in &lines {
            let stripped = line
                .replace("\x1b]8;;https://example.com\x1b\\", "")
                .replace("\x1b]8;;\x1b\\", "");
            let stripped = stripped
                .split("\x1b[")
                .enumerate()
                .map(|(i, part)| {
                    if i == 0 {
                        part.to_string()
                    } else {
                        part.find('m')
                            .map(|idx| part[idx + 1..].to_string())
                            .unwrap_or_default()
                    }
                })
                .collect::<String>();
            if !stripped.trim().is_empty() {
                assert!(
                    line.starts_with(&format!("\x1b]8;;{url}\x1b\\"))
                        || line.contains(&format!("\x1b]8;;{url}\x1b\\"))
                );
            }
        }
        for line in lines.iter().take(lines.len().saturating_sub(1)) {
            if line.contains(&format!("\x1b]8;;{url}\x1b\\")) {
                assert!(line.ends_with("\x1b]8;;\x1b\\"));
            }
        }

        let url = format!("https://example.com/oauth/{}", "a".repeat(32));
        let input = format!("\x1b]8;;{url}\x07{url}\x1b]8;;\x07");
        let lines = wrap_text_with_ansi(&input, 20);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(line.contains(&format!("\x1b]8;;{url}\x07")));
            assert!(!line.contains(&format!("\x1b]8;;{url}\x1b\\")));
        }
        for line in lines.iter().take(lines.len() - 1) {
            assert!(line.ends_with("\x1b]8;;\x07"));
        }

        let url = "https://example.com";
        let input = format!("before \x1b]8;;{url}\x1b\\link\x1b]8;;\x1b\\ after");
        let lines = wrap_text_with_ansi(&input, 80);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].matches("\x1b]8;;https:").count(), 1);
        assert_eq!(lines[0].matches("\x1b]8;;\x1b\\").count(), 1);
    }
}
