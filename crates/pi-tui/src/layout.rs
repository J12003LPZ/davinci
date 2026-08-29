use crate::component::{wrap, Component};

#[derive(Debug, Clone)]
pub struct Container {
    pub children: Vec<String>,
    pub padding: usize,
}

impl Container {
    pub fn new(children: Vec<String>) -> Self {
        Self {
            children,
            padding: 0,
        }
    }
}

impl Component for Container {
    fn render(&self, width: usize) -> Vec<String> {
        let inner = width.saturating_sub(self.padding * 2).max(1);
        let pad = " ".repeat(self.padding);
        self.children
            .iter()
            .flat_map(|child| wrap(child, inner))
            .map(|line| format!("{pad}{line}"))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct Overlay {
    pub title: String,
    pub body: Vec<String>,
}

impl Overlay {
    pub fn new(title: impl Into<String>, body: Vec<String>) -> Self {
        Self {
            title: title.into(),
            body,
        }
    }
}

impl Component for Overlay {
    fn render(&self, width: usize) -> Vec<String> {
        let inner = width.saturating_sub(2).max(1);
        let mut lines = vec![format!("┌{}┐", "─".repeat(inner))];
        let title = wrap(&self.title, inner.saturating_sub(2));
        for line in title {
            lines.push(format!(
                "│ {line:<width$}│",
                width = inner.saturating_sub(1)
            ));
        }
        for line in &self.body {
            for wrapped in wrap(line, inner) {
                lines.push(format!("│{wrapped:<inner$}│"));
            }
        }
        lines.push(format!("└{}┘", "─".repeat(inner)));
        lines
    }
}

const OSC133_ZONE_START: &str = "\u{1b}]133;A\u{07}";
const OSC133_ZONE_END: &str = "\u{1b}]133;B\u{07}";
const OSC133_ZONE_FINAL: &str = "\u{1b}]133;C\u{07}";

#[derive(Debug, Clone, Default)]
pub struct ChatView {
    pub lines: Vec<(String, String)>,
    pub semantic_zones: bool,
}

impl ChatView {
    pub fn push(&mut self, role: impl Into<String>, text: impl Into<String>) {
        self.lines.push((role.into(), text.into()));
    }
}

impl Component for ChatView {
    fn render(&self, width: usize) -> Vec<String> {
        let mut out = Vec::new();
        for (role, text) in &self.lines {
            let mut block = vec![format!("{role}:")];
            block.extend(wrap(text, width));
            block.push(String::new());
            if self.semantic_zones && !block.is_empty() {
                let last = block.len() - 1;
                block[0] = format!("{OSC133_ZONE_START}{}", block[0]);
                block[last] = format!("{OSC133_ZONE_END}{OSC133_ZONE_FINAL}{}", block[last]);
            }
            out.extend(block);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_and_chat_render() {
        let overlay = Overlay::new("models", vec!["anthropic/claude".into()]);
        let lines = overlay.render(24);
        assert!(lines[0].starts_with('┌'));
        assert!(lines.iter().any(|l| l.contains("models")));
        let mut chat = ChatView::default();
        chat.push("user", "hello");
        chat.push("assistant", "hi");
        let rendered = chat.render(20);
        assert!(rendered.iter().any(|l| l.contains("hello")));
    }
}
