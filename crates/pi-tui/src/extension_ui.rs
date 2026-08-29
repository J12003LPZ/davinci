//! Extension UI surfaces matching TS `ExtensionUIContext`.

use crate::editor::Editor;
use crate::render::Component;

pub const MAX_WIDGET_LINES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetPlacement {
    AboveEditor,
    BelowEditor,
}

impl WidgetPlacement {
    pub fn parse(value: &str) -> Self {
        if value == "belowEditor" {
            Self::BelowEditor
        } else {
            Self::AboveEditor
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AboveEditor => "aboveEditor",
            Self::BelowEditor => "belowEditor",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionWidget {
    pub key: String,
    pub lines: Vec<String>,
    pub placement: WidgetPlacement,
}

impl ExtensionWidget {
    pub fn new(key: impl Into<String>, lines: Vec<String>, placement: WidgetPlacement) -> Self {
        let mut lines = lines;
        if lines.len() > MAX_WIDGET_LINES {
            lines.truncate(MAX_WIDGET_LINES);
            lines.push("... (widget truncated)".into());
        }
        Self {
            key: key.into(),
            lines,
            placement,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionDialogAction {
    None,
    Cancel,
    Select(String),
    Submit(String),
    Confirm(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionSelector {
    pub title: String,
    pub options: Vec<String>,
    pub selected: usize,
}

impl ExtensionSelector {
    pub fn new(title: impl Into<String>, options: Vec<String>) -> Self {
        Self {
            title: title.into(),
            options,
            selected: 0,
        }
    }

    pub fn handle_key(&mut self, data: &str) -> ExtensionDialogAction {
        match data {
            "\x1b[A" | "k" => {
                if !self.options.is_empty() {
                    self.selected = (self.selected + self.options.len() - 1) % self.options.len();
                }
                ExtensionDialogAction::None
            }
            "\x1b[B" | "j" => {
                if !self.options.is_empty() {
                    self.selected = (self.selected + 1) % self.options.len();
                }
                ExtensionDialogAction::None
            }
            "\r" | "\n" => self
                .options
                .get(self.selected)
                .cloned()
                .map(ExtensionDialogAction::Select)
                .unwrap_or(ExtensionDialogAction::Cancel),
            "\x1b" => ExtensionDialogAction::Cancel,
            _ => ExtensionDialogAction::None,
        }
    }
}

impl Component for ExtensionSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![truncate(&self.title, width)];
        if self.options.is_empty() {
            lines.push("  No options".into());
            return lines;
        }
        for (index, option) in self.options.iter().enumerate() {
            let prefix = if index == self.selected { "> " } else { "  " };
            lines.push(truncate(&format!("{prefix}{option}"), width));
        }
        lines.push("  enter select  escape cancel".into());
        lines
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionInput {
    pub title: String,
    pub placeholder: String,
    pub value: String,
}

impl ExtensionInput {
    pub fn new(title: impl Into<String>, placeholder: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            placeholder: placeholder.into(),
            value: String::new(),
        }
    }

    pub fn handle_key(&mut self, data: &str) -> ExtensionDialogAction {
        match data {
            "\r" | "\n" => ExtensionDialogAction::Submit(self.value.clone()),
            "\x1b" => ExtensionDialogAction::Cancel,
            "\x7f" | "\x08" => {
                self.value.pop();
                ExtensionDialogAction::None
            }
            other if !other.chars().any(char::is_control) => {
                self.value.push_str(other);
                ExtensionDialogAction::None
            }
            _ => ExtensionDialogAction::None,
        }
    }
}

impl Component for ExtensionInput {
    fn render(&self, width: usize) -> Vec<String> {
        let shown = if self.value.is_empty() {
            format!("  {}", self.placeholder)
        } else {
            format!("  {}", self.value)
        };
        vec![
            truncate(&self.title, width),
            truncate(&shown, width),
            "  enter submit  escape cancel".into(),
        ]
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

#[derive(Debug, Clone)]
pub struct ExtensionEditor {
    pub title: String,
    pub editor: Editor,
}

impl ExtensionEditor {
    pub fn new(title: impl Into<String>, prefill: impl Into<String>) -> Self {
        let mut editor = Editor::new();
        let prefill = prefill.into();
        if !prefill.is_empty() {
            editor.handle_input(&prefill);
        }
        Self {
            title: title.into(),
            editor,
        }
    }

    pub fn handle_key(&mut self, data: &str) -> ExtensionDialogAction {
        match data {
            "\x1b" => ExtensionDialogAction::Cancel,
            "\x13" => ExtensionDialogAction::Submit(self.editor.submit()),
            other => {
                self.editor.handle_input(other);
                ExtensionDialogAction::None
            }
        }
    }
}

impl Component for ExtensionEditor {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![truncate(&self.title, width)];
        lines.extend(self.editor.render(width));
        lines.push("  ctrl+s save  escape cancel".into());
        lines
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionConfirm {
    pub title: String,
    pub message: String,
}

impl ExtensionConfirm {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
        }
    }

    pub fn handle_key(&mut self, data: &str) -> ExtensionDialogAction {
        match data {
            "\r" | "\n" | "y" | "Y" => ExtensionDialogAction::Confirm(true),
            "\x1b" | "n" | "N" => ExtensionDialogAction::Confirm(false),
            _ => ExtensionDialogAction::None,
        }
    }
}

impl Component for ExtensionConfirm {
    fn render(&self, width: usize) -> Vec<String> {
        vec![
            truncate(&self.title, width),
            truncate(&format!("  {}", self.message), width),
            "  enter/y confirm  escape/n cancel".into(),
        ]
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

fn truncate(text: &str, width: usize) -> String {
    if crate::render::visible_width(text) <= width {
        text.to_string()
    } else {
        text.chars().take(width).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widgets_truncate_and_dialogs_match_ts_ui_context() {
        let widget = ExtensionWidget::new(
            "status",
            (0..12).map(|i| format!("line {i}")).collect(),
            WidgetPlacement::AboveEditor,
        );
        assert_eq!(widget.lines.len(), 11);
        assert_eq!(
            widget.lines.last().map(String::as_str),
            Some("... (widget truncated)")
        );

        let mut selector = ExtensionSelector::new("Pick", vec!["one".into(), "two".into()]);
        assert_eq!(selector.handle_key("\x1b[B"), ExtensionDialogAction::None);
        assert_eq!(
            selector.handle_key("\r"),
            ExtensionDialogAction::Select("two".into())
        );
        assert!(selector.render(40).join("\n").contains("Pick"));

        let mut input = ExtensionInput::new("Name", "type here");
        input.handle_key("ab");
        assert_eq!(
            input.handle_key("\r"),
            ExtensionDialogAction::Submit("ab".into())
        );

        let mut editor = ExtensionEditor::new("Edit", "hello");
        assert_eq!(
            editor.handle_key("\x13"),
            ExtensionDialogAction::Submit("hello".into())
        );

        let mut confirm = ExtensionConfirm::new("Unload?", "model-a");
        assert_eq!(
            confirm.handle_key("y"),
            ExtensionDialogAction::Confirm(true)
        );
        assert_eq!(
            confirm.handle_key("\x1b"),
            ExtensionDialogAction::Confirm(false)
        );
    }
}
