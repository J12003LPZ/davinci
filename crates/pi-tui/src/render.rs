use unicode_width::UnicodeWidthStr;

pub trait Component {
    fn render(&self, width: usize) -> Vec<String>;
    fn handle_input(&mut self, _data: &str) {}
    fn invalidate(&mut self);
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
