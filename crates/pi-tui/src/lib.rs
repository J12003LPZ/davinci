//! TUI component contract.
//!
//! Mirrors `vendor/pi/packages/tui/src/tui.ts`: `render(width) -> lines`.

pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

pub trait Component {
    fn render(&self, width: usize) -> Vec<String>;
    fn handle_input(&mut self, data: &str);
}

#[derive(Default)]
pub struct Container {
    pub children: Vec<Box<dyn Component + Send>>,
}

impl Component for Container {
    fn render(&self, width: usize) -> Vec<String> {
        self.children
            .iter()
            .flat_map(|child| child.render(width))
            .collect()
    }

    fn handle_input(&mut self, data: &str) {
        if let Some(child) = self.children.last_mut() {
            child.handle_input(data);
        }
    }
}

#[derive(Debug, Clone)]
pub struct Editor {
    pub lines: Vec<String>,
    pub cursor: usize,
    pub focused: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: 0,
            focused: true,
        }
    }
}

impl Editor {
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

impl Component for Editor {
    fn render(&self, width: usize) -> Vec<String> {
        self.lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let mut clipped = if line.chars().count() > width {
                    line.chars().take(width).collect()
                } else {
                    line.clone()
                };
                if self.focused && index == self.lines.len().saturating_sub(1) {
                    clipped.push_str(CURSOR_MARKER);
                }
                clipped
            })
            .collect()
    }

    fn handle_input(&mut self, data: &str) {
        match data {
            "\n" | "\r" => self.lines.push(String::new()),
            "\u{7f}" | "\u{08}" => {
                if let Some(last) = self.lines.last_mut() {
                    last.pop();
                }
            }
            other if !other.is_empty() && !other.starts_with('\u{1b}') => {
                if let Some(last) = self.lines.last_mut() {
                    last.push_str(other);
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Exit,
    Help,
    Clear,
    Sessions,
    Unknown(String),
}

pub fn parse_slash(input: &str) -> Option<SlashCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let name = trimmed
        .trim_start_matches('/')
        .split_whitespace()
        .next()
        .unwrap_or("");
    Some(match name {
        "exit" | "quit" | "q" => SlashCommand::Exit,
        "help" | "h" => SlashCommand::Help,
        "clear" => SlashCommand::Clear,
        "sessions" => SlashCommand::Sessions,
        other => SlashCommand::Unknown(other.to_string()),
    })
}

#[derive(Debug, Clone)]
pub struct ChatView {
    pub title: String,
    pub history: Vec<String>,
    pub editor: Editor,
    pub status: String,
}

impl Default for ChatView {
    fn default() -> Self {
        Self {
            title: "pi".into(),
            history: Vec::new(),
            editor: Editor::default(),
            status: String::new(),
        }
    }
}

impl ChatView {
    pub fn push_user(&mut self, text: &str) {
        self.history.push(format!("you: {text}"));
    }

    pub fn push_assistant(&mut self, text: &str) {
        self.history.push(format!("pi: {text}"));
    }
}

impl Component for ChatView {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![self.title.clone(), String::new()];
        lines.extend(self.history.iter().cloned());
        if !self.history.is_empty() {
            lines.push(String::new());
        }
        if !self.status.is_empty() {
            lines.push(self.status.clone());
        }
        lines.push(String::from("> ") + &self.editor.render(width.saturating_sub(2)).join("\n"));
        lines
            .into_iter()
            .map(|line| {
                if line.chars().count() > width {
                    line.chars().take(width).collect()
                } else {
                    line
                }
            })
            .collect()
    }

    fn handle_input(&mut self, data: &str) {
        self.editor.handle_input(data);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Enter,
    CtrlC,
    Char(char),
    Other(String),
}

pub fn parse_key(data: &str) -> Key {
    match data {
        "\n" | "\r" | "\r\n" => Key::Enter,
        "\u{3}" => Key::CtrlC,
        other if other.chars().count() == 1 => Key::Char(other.chars().next().unwrap()),
        other => Key::Other(other.to_string()),
    }
}

#[derive(Debug, Clone)]
pub struct SelectList {
    pub items: Vec<String>,
    pub selected: usize,
}

impl Component for SelectList {
    fn render(&self, width: usize) -> Vec<String> {
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let prefix = if index == self.selected { "> " } else { "  " };
                let mut line = format!("{prefix}{item}");
                if line.chars().count() > width {
                    line = line.chars().take(width).collect();
                }
                line
            })
            .collect()
    }

    fn handle_input(&mut self, data: &str) {
        match parse_key(data) {
            Key::Char('j') | Key::Other(_) if data == "j" || data == "down" => {
                if self.selected + 1 < self.items.len() {
                    self.selected += 1;
                }
            }
            Key::Char('k') | Key::Other(_) if data == "k" || data == "up" => {
                self.selected = self.selected.saturating_sub(1);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_snapshot_includes_cursor_marker() {
        let mut editor = Editor::default();
        editor.handle_input("hello");
        let lines = editor.render(40);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("hello"));
        assert!(lines[0].contains(CURSOR_MARKER));
    }

    #[test]
    fn container_stacks_children() {
        let mut container = Container::default();
        container.children.push(Box::new(SelectList {
            items: vec!["one".into(), "two".into()],
            selected: 1,
        }));
        container.children.push(Box::new(Editor {
            lines: vec!["prompt".into()],
            cursor: 6,
            focused: true,
        }));
        let lines = container.render(20);
        assert_eq!(lines[0], "  one");
        assert_eq!(lines[1], "> two");
        assert!(lines[2].starts_with("prompt"));
    }

    #[test]
    fn parse_enter_and_ctrl_c() {
        assert_eq!(parse_key("\n"), Key::Enter);
        assert_eq!(parse_key("\u{3}"), Key::CtrlC);
    }

    #[test]
    fn slash_commands() {
        assert_eq!(parse_slash("/exit"), Some(SlashCommand::Exit));
        assert_eq!(parse_slash("/help"), Some(SlashCommand::Help));
        assert_eq!(parse_slash("hello"), None);
    }

    #[test]
    fn chat_view_renders_history_and_prompt() {
        let mut view = ChatView {
            title: "pi 0.84.4".into(),
            ..ChatView::default()
        };
        view.push_user("hi");
        view.push_assistant("echo:hi");
        let lines = view.render(40);
        assert!(lines.iter().any(|line| line.contains("you: hi")));
        assert!(lines.iter().any(|line| line.contains("pi: echo:hi")));
        assert!(lines.iter().any(|line| line.starts_with('>')));
    }
}
