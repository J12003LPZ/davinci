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
            "\x1b" | "\x03" => ExtensionDialogAction::Cancel,
            _ => ExtensionDialogAction::None,
        }
    }
}

impl Component for ExtensionSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let mut section = crate::render::CommandSection::new(width, &self.title, None);
        if self.options.is_empty() {
            section.detail("No options available.");
        } else {
            for index in crate::render::selection_window(self.selected, self.options.len(), 8) {
                section.item(index == self.selected, &self.options[index], "");
            }
            section.position(self.selected, self.options.len(), 8);
        }
        section.hint("↑↓ move · enter select · esc cancel");
        section.finish()
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

#[derive(Debug, Clone)]
pub struct ExtensionInput {
    pub title: String,
    pub placeholder: String,
    input: crate::input::Input,
}

impl ExtensionInput {
    pub fn new(title: impl Into<String>, placeholder: impl Into<String>) -> Self {
        let mut input = crate::input::Input::new();
        input.focused = true;
        Self {
            title: title.into(),
            placeholder: placeholder.into(),
            input,
        }
    }

    pub fn value(&self) -> &str {
        self.input.get_value()
    }

    pub fn handle_key(&mut self, data: &str) -> ExtensionDialogAction {
        match self.input.handle_key(data) {
            crate::input::InputAction::Submit(value) => ExtensionDialogAction::Submit(value),
            crate::input::InputAction::Cancel => ExtensionDialogAction::Cancel,
            crate::input::InputAction::None => ExtensionDialogAction::None,
        }
    }
}

impl Component for ExtensionInput {
    fn render(&self, width: usize) -> Vec<String> {
        let mut section = crate::render::CommandSection::new(width, &self.title, None);
        if self.input.get_value().is_empty() && !self.placeholder.is_empty() {
            section.detail(&self.placeholder);
        } else {
            section.input(self.input.render(width.saturating_sub(3)));
        }
        section.hint("enter submit · esc cancel");
        section.finish()
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
        let mut section = crate::render::CommandSection::new(width, &self.title, None);
        section.input(self.editor.render(width.saturating_sub(3)));
        section.hint("ctrl+s save · esc cancel");
        section.finish()
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionProgress {
    pub title: String,
    pub model: String,
    pub message: String,
    pub ratio: Option<f64>,
    pub detail: Option<String>,
}

impl ExtensionProgress {
    pub fn new(
        title: impl Into<String>,
        model: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            model: model.into(),
            message: message.into(),
            ratio: None,
            detail: None,
        }
    }

    pub fn progress_bar(ratio: f64) -> String {
        let clamped = ratio.clamp(0.0, 1.0);
        let filled = (clamped * 40.0).round() as usize;
        let filled = filled.min(40);
        format!(
            "{}{} {}%",
            "█".repeat(filled),
            "─".repeat(40 - filled),
            (clamped * 100.0).round() as i32
        )
    }

    pub fn handle_key(&mut self, data: &str) -> ExtensionDialogAction {
        match data {
            "\x1b" | "\x03" => ExtensionDialogAction::Cancel,
            _ => ExtensionDialogAction::None,
        }
    }
}

impl Component for ExtensionProgress {
    fn render(&self, width: usize) -> Vec<String> {
        let mut section = crate::render::CommandSection::new(width, &self.title, None);
        if !self.model.is_empty() {
            section.detail(&format!("Model: {}", self.model));
        }
        if !self.message.is_empty() {
            section.detail(&self.message);
        }
        if let Some(ratio) = self.ratio {
            section.detail(&Self::progress_bar(ratio));
        }
        if let Some(detail) = &self.detail {
            section.detail(detail);
        }
        section.hint("esc/ctrl+c stop");
        section.finish()
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
        let mut section = crate::render::CommandSection::new(width, &self.title, None);
        section.detail(&self.message);
        section.hint("enter/y confirm · esc/n cancel");
        section.finish()
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
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
        let mut cjk = ExtensionInput::new("Name", "");
        for ch in "你好世界。你好，世界".chars() {
            cjk.handle_key(&ch.to_string());
        }
        cjk.handle_key("\x05");
        cjk.handle_key("\x17");
        assert_eq!(cjk.value(), "你好世界。你好，");

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

        let mut progress = ExtensionProgress::new("Loading model", "local", "Starting…");
        progress.ratio = Some(0.5);
        progress.detail = Some("512 B / 1.00 KiB".into());
        let rendered = progress.render(80).join("\n");
        assert!(rendered.contains("Loading model"));
        assert!(rendered.contains("Starting…"));
        assert!(rendered.contains("50%"));
        assert!(rendered.contains("512 B / 1.00 KiB"));
        assert!(rendered.contains("esc/ctrl+c stop"));
        assert_eq!(progress.handle_key("\x1b"), ExtensionDialogAction::Cancel);
        assert_eq!(progress.handle_key("\x03"), ExtensionDialogAction::Cancel);
    }
}

#[cfg(test)]
mod section_style_regressions {
    use super::*;

    #[test]
    fn extension_dialogs_use_shared_sections_and_keep_empty_cancel_hints() {
        let selector = ExtensionSelector::new("Choose 模型 🦀", Vec::new());
        let empty = crate::render::strip_terminal_sequences(&selector.render(24).join("\n"));
        assert!(empty.contains("No options"), "{empty}");
        assert!(empty.to_lowercase().contains("esc"), "{empty}");

        let selector = ExtensionSelector::new(
            "Choose option",
            vec!["one".into(), "模型 café 🦀 with a very long option".into()],
        );
        let rendered = crate::render::strip_terminal_sequences(&selector.render(28).join("\n"));
        assert!(
            rendered.contains(crate::davinci::ui::SELECTION_BAR.trim()),
            "{rendered}"
        );
        for width in [0, 1, 12, 24, 28, 40] {
            for component in [
                selector.render(width),
                ExtensionInput::new("Input title", "placeholder text").render(width),
                ExtensionEditor::new("Editor title", "hello").render(width),
                ExtensionConfirm::new("Confirm title", "very long confirmation message 模型 🦀")
                    .render(width),
                ExtensionProgress::new(
                    "Loading 模型 🦀",
                    "provider/model",
                    "long progress message",
                )
                .render(width),
            ] {
                for row in component {
                    assert!(
                        crate::render::visible_width_stripped(&row) <= width,
                        "{width}: {row:?}"
                    );
                }
            }
        }
    }
}
