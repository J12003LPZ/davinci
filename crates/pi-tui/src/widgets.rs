//! Remaining TypeScript TUI primitives: box, stacks, scroll, input, settings.

use crate::component::{wrap, Component};

#[derive(Debug, Clone)]
pub struct BoxWidget {
    pub title: Option<String>,
    pub body: Vec<String>,
}

impl Component for BoxWidget {
    fn render(&self, width: usize) -> Vec<String> {
        let inner = width.saturating_sub(2).max(1);
        let mut lines = vec![format!("┌{}┐", "─".repeat(inner))];
        if let Some(title) = &self.title {
            for line in wrap(title, inner) {
                lines.push(format!("│{line:<inner$}│"));
            }
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

#[derive(Debug, Clone, Default)]
pub struct VStack {
    pub children: Vec<Vec<String>>,
}

impl Component for VStack {
    fn render(&self, _width: usize) -> Vec<String> {
        self.children.iter().flatten().cloned().collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct HStack {
    pub children: Vec<String>,
    pub gap: usize,
}

impl Component for HStack {
    fn render(&self, width: usize) -> Vec<String> {
        let gap = " ".repeat(self.gap);
        let line = self.children.join(&gap);
        wrap(&line, width)
    }
}

#[derive(Debug, Clone)]
pub struct ScrollView {
    pub lines: Vec<String>,
    pub offset: usize,
    pub height: usize,
}

impl Component for ScrollView {
    fn render(&self, width: usize) -> Vec<String> {
        self.lines
            .iter()
            .skip(self.offset)
            .take(self.height)
            .flat_map(|line| wrap(line, width))
            .take(self.height)
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Input {
    pub value: String,
    pub placeholder: String,
}

impl Component for Input {
    fn render(&self, width: usize) -> Vec<String> {
        let text = if self.value.is_empty() {
            self.placeholder.as_str()
        } else {
            self.value.as_str()
        };
        wrap(text, width)
    }
}

#[derive(Debug, Clone)]
pub struct SettingsList {
    pub items: Vec<(String, bool)>,
    pub selected: usize,
}

impl Component for SettingsList {
    fn render(&self, width: usize) -> Vec<String> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, (name, on))| {
                let mark = if *on { "[x]" } else { "[ ]" };
                let prefix = if i == self.selected { ">" } else { " " };
                let mut line = format!("{prefix} {mark} {name}");
                if line.len() > width {
                    line.truncate(width);
                }
                line
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widgets_render() {
        let boxw = BoxWidget {
            title: Some("settings".into()),
            body: vec!["theme".into()],
        };
        assert!(boxw.render(20).iter().any(|l| l.contains("settings")));
        let stack = VStack {
            children: vec![vec!["a".into()], vec!["b".into()]],
        };
        assert_eq!(stack.render(10), vec!["a", "b"]);
        let scroll = ScrollView {
            lines: vec!["1".into(), "2".into(), "3".into()],
            offset: 1,
            height: 1,
        };
        assert_eq!(scroll.render(10), vec!["2"]);
    }
}
