use std::any::Any;

use unicode_width::UnicodeWidthStr;

pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub trait Component: AsAny {
    fn render(&self, width: usize) -> Vec<String>;
    fn handle_input(&mut self, _data: &str) {}
    fn invalidate(&mut self);
    fn wants_key_release(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct Text {
    pub value: String,
}

impl Component for Text {
    fn render(&self, width: usize) -> Vec<String> {
        wrap_text(&self.value, width)
    }

    fn invalidate(&mut self) {}
}

pub fn visible_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// TS `visibleWidth`: strip ANSI/OSC/APC then measure columns.
pub fn visible_width_stripped(text: &str) -> usize {
    visible_width(&strip_terminal_sequences(text))
}

pub fn strip_terminal_sequences(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if (0x40..=0x7e).contains(&b) {
                        break;
                    }
                }
                continue;
            }
            if i + 1 < bytes.len()
                && (bytes[i + 1] == b']' || bytes[i + 1] == b'_' || bytes[i + 1] == b'^')
            {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in raw.split(' ') {
            if current.is_empty() {
                current = word.to_string();
                continue;
            }
            if visible_width(&current) + 1 + visible_width(word) <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}
