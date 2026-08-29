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
