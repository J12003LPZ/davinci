use unicode_width::UnicodeWidthStr;

pub trait Component {
    fn render(&self, width: usize) -> Vec<String>;
}

#[derive(Debug, Clone)]
pub struct Text {
    pub value: String,
}

impl Text {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

impl Component for Text {
    fn render(&self, width: usize) -> Vec<String> {
        wrap(&self.value, width.max(1))
    }
}

pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split(' ') {
            if current.is_empty() {
                current = word.to_string();
                while UnicodeWidthStr::width(current.as_str()) > width {
                    let (head, tail) = split_width(&current, width);
                    lines.push(head);
                    current = tail;
                }
            } else {
                let candidate = format!("{current} {word}");
                if UnicodeWidthStr::width(candidate.as_str()) <= width {
                    current = candidate;
                } else {
                    lines.push(std::mem::take(&mut current));
                    current = word.to_string();
                    while UnicodeWidthStr::width(current.as_str()) > width {
                        let (head, tail) = split_width(&current, width);
                        lines.push(head);
                        current = tail;
                    }
                }
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        width += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    width
}

/// Wrap text while preserving ANSI SGR sequences (TS `wrapTextWithAnsi`).
pub fn wrap_text_with_ansi(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut current = String::new();
        for word in split_ansi_words(paragraph) {
            if current.is_empty() {
                current = word;
                while visible_width(&current) > width {
                    let (head, tail) = split_ansi_width(&current, width);
                    lines.push(head);
                    current = tail;
                }
            } else {
                let candidate = format!("{current} {word}");
                if visible_width(&candidate) <= width {
                    current = candidate;
                } else {
                    lines.push(std::mem::take(&mut current));
                    current = word;
                    while visible_width(&current) > width {
                        let (head, tail) = split_ansi_width(&current, width);
                        lines.push(head);
                        current = tail;
                    }
                }
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn split_ansi_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            current.push(ch);
            current.push('[');
            chars.next();
            for next in chars.by_ref() {
                current.push(next);
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        if ch == ' ' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn split_ansi_width(text: &str, width: usize) -> (String, String) {
    let mut acc = 0;
    let mut idx = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch == '\u{1b}' {
            let start = i;
            let mut end = text.len();
            for (j, next) in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    end = j + next.len_utf8();
                    break;
                }
            }
            idx = end;
            let _ = start;
            continue;
        }
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if acc + w > width {
            return (text[..i].to_string(), text[i..].to_string());
        }
        acc += w;
        idx = i + ch.len_utf8();
    }
    (text[..idx].to_string(), text[idx..].to_string())
}

fn split_width(text: &str, width: usize) -> (String, String) {
    let mut acc = 0;
    let mut idx = 0;
    for (i, ch) in text.char_indices() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if acc + w > width {
            idx = i;
            break;
        }
        acc += w;
        idx = i + ch.len_utf8();
    }
    (text[..idx].to_string(), text[idx..].to_string())
}
