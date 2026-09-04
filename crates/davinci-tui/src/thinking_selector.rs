//! `/thinking` selector matching TS `ThinkingSelectorComponent`.

use crate::fuzzy::fuzzy_match;
use crate::render::Component;
use crate::themes::Theme;

pub const LEVEL_DESCRIPTIONS: &[(&str, &str)] = &[
    ("off", "No reasoning"),
    ("minimal", "Very brief reasoning (~1k tokens)"),
    ("low", "Light reasoning (~2k tokens)"),
    ("medium", "Moderate reasoning (~8k tokens)"),
    ("high", "Deep reasoning (~16k tokens)"),
    ("xhigh", "Extra-high reasoning (~32k tokens)"),
    ("max", "Maximum reasoning"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThinkingSelectorAction {
    None,
    Select(String),
    SelectAsDefault(String),
    Cancel,
}

#[derive(Debug, Clone)]
pub struct ThinkingSelector {
    pub search: String,
    pub selected: usize,
    pub current: String,
    pub default_level: String,
    levels: Vec<String>,
    theme: Theme,
}

impl ThinkingSelector {
    pub fn new(
        current: impl Into<String>,
        levels: Vec<String>,
        default_level: impl Into<String>,
    ) -> Self {
        let current = current.into();
        let selected = levels
            .iter()
            .position(|level| level == &current)
            .unwrap_or(0);
        Self {
            search: String::new(),
            selected,
            current,
            default_level: default_level.into(),
            levels,
            theme: Theme::default(),
        }
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn handle_key(&mut self, data: &str) -> ThinkingSelectorAction {
        match data {
            "\x1b" | "\x1b[27~" => ThinkingSelectorAction::Cancel,
            "\r" | "\n" => self
                .selected_level()
                .map(ThinkingSelectorAction::Select)
                .unwrap_or(ThinkingSelectorAction::None),
            "\x13" | "\x1b[115;5u" => self
                .selected_level()
                .map(ThinkingSelectorAction::SelectAsDefault)
                .unwrap_or(ThinkingSelectorAction::None),
            "\x1b[A" | "\x10" => {
                self.move_by(-1);
                ThinkingSelectorAction::None
            }
            "\x1b[B" | "\x0e" => {
                self.move_by(1);
                ThinkingSelectorAction::None
            }
            "\x7f" | "\x08" => {
                self.search.pop();
                self.clamp_selected();
                ThinkingSelectorAction::None
            }
            other if !other.is_empty() && other.chars().all(|ch| !ch.is_control()) => {
                self.search.push_str(other);
                self.selected = 0;
                ThinkingSelectorAction::None
            }
            _ => ThinkingSelectorAction::None,
        }
    }

    pub fn selected_level(&self) -> Option<String> {
        self.filtered().get(self.selected).cloned()
    }

    fn filtered(&self) -> Vec<String> {
        if self.search.is_empty() {
            return self.levels.clone();
        }
        let query = self.search.to_lowercase();
        self.levels
            .iter()
            .filter(|level| {
                let desc = description_for(level);
                fuzzy_match(&query, &format!("{level} {desc}")).matches
                    || level.to_lowercase().contains(&query)
            })
            .cloned()
            .collect()
    }

    fn move_by(&mut self, delta: isize) {
        let len = self.filtered().len() as isize;
        if len == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    fn clamp_selected(&mut self) {
        let len = self.filtered().len();
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len - 1);
        }
    }
}

fn description_for(level: &str) -> &'static str {
    LEVEL_DESCRIPTIONS
        .iter()
        .find(|(name, _)| *name == level)
        .map(|(_, desc)| *desc)
        .unwrap_or("")
}

impl Component for ThinkingSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![
            String::new(),
            "Thinking Level".into(),
            String::new(),
            self.theme
                .fg("muted", "Ctrl+T cycles thinking levels in-session"),
            String::new(),
            format!("> {}", self.search),
            String::new(),
        ];
        let filtered = self.filtered();
        if filtered.is_empty() {
            lines.push(self.theme.fg("muted", "  No matching levels"));
        }
        for (index, level) in filtered.iter().enumerate() {
            let selected = index == self.selected;
            let prefix = if selected {
                self.theme.fg("accent", "→ ")
            } else {
                "  ".into()
            };
            let name = if selected {
                self.theme.fg("accent", level)
            } else {
                level.clone()
            };
            let mut desc = description_for(level).to_string();
            if level == &self.default_level {
                desc.push_str(" · default");
            }
            let check = if level == &self.current {
                self.theme.fg("success", " ✓")
            } else {
                String::new()
            };
            let line = format!("{prefix}{name}  {}{check}", self.theme.fg("muted", &desc));
            lines.push(truncate(&line, width));
        }
        lines.push(String::new());
        lines.push(self.theme.fg(
            "dim",
            "  Enter to select · Ctrl+S to set as default · Esc to cancel",
        ));
        lines
    }

    fn invalidate(&mut self) {}
}

fn truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = if ch.is_ascii() { 1 } else { 2 };
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels() -> Vec<String> {
        LEVEL_DESCRIPTIONS
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect()
    }

    #[test]
    fn enter_selects_current_and_ctrl_s_persists_default() {
        let mut selector = ThinkingSelector::new("high", levels(), "off");
        assert_eq!(
            selector.handle_key("\r"),
            ThinkingSelectorAction::Select("high".into())
        );
        assert_eq!(
            selector.handle_key("\x13"),
            ThinkingSelectorAction::SelectAsDefault("high".into())
        );
    }

    #[test]
    fn search_filters_and_escape_cancels() {
        let mut selector = ThinkingSelector::new("off", levels(), "off");
        let _ = selector.handle_key("x");
        let _ = selector.handle_key("h");
        assert_eq!(selector.selected_level().as_deref(), Some("xhigh"));
        assert_eq!(selector.handle_key("\x1b"), ThinkingSelectorAction::Cancel);
        let rendered = selector.render(80).join("\n");
        assert!(rendered.contains("Thinking Level"));
        assert!(rendered.contains("Enter to select · Ctrl+S to set as default · Esc to cancel"));
    }
}
