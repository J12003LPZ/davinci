use crate::autocomplete::AutocompleteSuggestions;
use crate::editor::Editor;
use crate::render::Component;
use crate::settings::SettingsList;
use crate::themes::Theme;
use crate::tool_card::ToolCard;
use crate::transcript::Transcript;
use crate::SelectList;

/// Fullscreen / regular chat chrome used by interactive mode.
#[derive(Debug, Clone)]
pub struct ChatChrome {
    pub transcript: Transcript,
    pub editor: Editor,
    pub selector: Option<SelectList>,
    pub settings_list: Option<SettingsList>,
    pub tool_cards: Vec<ToolCard>,
    pub autocomplete: Option<AutocompleteSuggestions>,
    pub autocomplete_selected: usize,
    pub theme: Theme,
    pub status: String,
    pub title: String,
}

impl ChatChrome {
    pub fn new(theme: Theme, title: impl Into<String>) -> Self {
        Self {
            transcript: Transcript::default(),
            editor: Editor::new(),
            selector: None,
            settings_list: None,
            tool_cards: Vec::new(),
            autocomplete: None,
            autocomplete_selected: 0,
            theme,
            status: String::new(),
            title: title.into(),
        }
    }

    pub fn apply_mouse(&mut self, y: u16, viewport: usize) {
        if let Some(selector) = &mut self.selector {
            if (y as usize) < selector.items.len() {
                selector.selected = y as usize;
            }
        } else {
            self.transcript.scroll_by(0, viewport);
        }
    }
}

impl Component for ChatChrome {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![format!("{}  theme={}", self.title, self.theme.name)];
        if !self.status.is_empty() {
            lines.push(self.status.clone());
        }
        lines.extend(self.transcript.render(width));
        for card in &self.tool_cards {
            lines.extend(card.render(width));
        }
        if let Some(settings) = &self.settings_list {
            lines.push(String::new());
            lines.extend(settings.render(width));
        } else if let Some(selector) = &self.selector {
            lines.push(String::new());
            lines.extend(selector.render(width));
        } else {
            lines.extend(self.editor.render(width));
            if let Some(suggestions) = &self.autocomplete {
                for (index, item) in suggestions.items.iter().enumerate() {
                    let prefix = if index == self.autocomplete_selected {
                        "> "
                    } else {
                        "  "
                    };
                    let desc = item
                        .description
                        .as_deref()
                        .map(|value| format!("  {value}"))
                        .unwrap_or_default();
                    lines.push(format!("{prefix}{}{desc}", item.label));
                }
            }
        }
        lines
    }

    fn handle_input(&mut self, data: &str) {
        if let Some(settings) = &mut self.settings_list {
            settings.handle_input(data);
        } else if let Some(selector) = &mut self.selector {
            selector.query.push_str(data);
        } else {
            self.editor.handle_input(data);
        }
    }

    fn invalidate(&mut self) {
        self.editor.invalidate();
        self.transcript.invalidate();
    }
}
