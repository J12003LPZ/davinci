//! Settings submenus matching TS `settings-selector.ts` Theme/Warnings/model-thinking.

use crate::render::Component;

pub const AUTOMATIC_THEME_VALUE: &str = "/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSubmenuKind {
    Theme,
    Warnings,
    ModelThinking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsSubmenuAction {
    None,
    Cancel,
    Preview(String),
    Apply { id: String, value: String },
}

#[derive(Debug, Clone)]
pub struct ModelThinkingItem {
    pub key: String,
    pub label: String,
    pub level: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SettingsSubmenu {
    pub kind: SettingsSubmenuKind,
    selected: usize,
    theme_mode: ThemeMode,
    single_theme: String,
    light_theme: String,
    dark_theme: String,
    themes: Vec<String>,
    anthropic_extra_usage: bool,
    models: Vec<ModelThinkingItem>,
    thinking_levels: Vec<String>,
    thinking_step: ThinkingStep,
    thinking_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeMode {
    Single,
    Automatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThinkingStep {
    Model,
    Level,
}

impl SettingsSubmenu {
    pub fn theme(current: &str, themes: Vec<String>) -> Self {
        let auto = parse_auto_theme(current);
        let (light, dark) = auto.clone().unwrap_or_else(|| {
            let name = if themes.iter().any(|theme| theme == current) {
                current.to_string()
            } else {
                themes.first().cloned().unwrap_or_else(|| "dark".into())
            };
            (name.clone(), name)
        });
        let single = if auto.is_some() || current.contains('/') {
            dark.clone()
        } else {
            current.to_string()
        };
        Self {
            kind: SettingsSubmenuKind::Theme,
            selected: 0,
            theme_mode: if auto.is_some() {
                ThemeMode::Automatic
            } else {
                ThemeMode::Single
            },
            single_theme: single,
            light_theme: light,
            dark_theme: dark,
            themes,
            anthropic_extra_usage: true,
            models: Vec::new(),
            thinking_levels: vec![
                "off".into(),
                "minimal".into(),
                "low".into(),
                "medium".into(),
                "high".into(),
                "xhigh".into(),
                "max".into(),
            ],
            thinking_step: ThinkingStep::Model,
            thinking_model: None,
        }
    }

    pub fn warnings(anthropic_extra_usage: bool) -> Self {
        let mut submenu = Self::theme("dark", vec!["dark".into()]);
        submenu.kind = SettingsSubmenuKind::Warnings;
        submenu.anthropic_extra_usage = anthropic_extra_usage;
        submenu
    }

    pub fn model_thinking(models: Vec<ModelThinkingItem>) -> Self {
        let mut submenu = Self::theme("dark", vec!["dark".into()]);
        submenu.kind = SettingsSubmenuKind::ModelThinking;
        submenu.models = models;
        submenu
    }

    pub fn is_submenu_setting(id: &str) -> bool {
        matches!(id, "theme" | "warnings" | "model-thinking")
    }

    pub fn handle_key(&mut self, data: &str) -> SettingsSubmenuAction {
        match data {
            "\x1b[A" | "k" => {
                self.move_sel(-1);
                self.preview()
            }
            "\x1b[B" | "j" => {
                self.move_sel(1);
                self.preview()
            }
            " " => self.activate(true),
            "\r" | "\n" => self.activate(false),
            "\x1b" => self.cancel(),
            _ => SettingsSubmenuAction::None,
        }
    }

    fn cancel(&mut self) -> SettingsSubmenuAction {
        match self.kind {
            SettingsSubmenuKind::Theme if self.theme_mode == ThemeMode::Automatic => {
                self.theme_mode = ThemeMode::Single;
                self.selected = 0;
                SettingsSubmenuAction::None
            }
            SettingsSubmenuKind::ModelThinking if self.thinking_step == ThinkingStep::Level => {
                self.thinking_step = ThinkingStep::Model;
                self.thinking_model = None;
                self.selected = 0;
                SettingsSubmenuAction::None
            }
            _ => SettingsSubmenuAction::Cancel,
        }
    }

    fn activate(&mut self, cycle_only: bool) -> SettingsSubmenuAction {
        match self.kind {
            SettingsSubmenuKind::Theme => self.activate_theme(cycle_only),
            SettingsSubmenuKind::Warnings => {
                self.anthropic_extra_usage = !self.anthropic_extra_usage;
                SettingsSubmenuAction::Apply {
                    id: "warnings.anthropic-extra-usage".into(),
                    value: if self.anthropic_extra_usage {
                        "true".into()
                    } else {
                        "false".into()
                    },
                }
            }
            SettingsSubmenuKind::ModelThinking => self.activate_thinking(),
        }
    }

    fn activate_theme(&mut self, cycle_only: bool) -> SettingsSubmenuAction {
        match self.theme_mode {
            ThemeMode::Single => {
                let items = self.single_items();
                let Some(value) = items.get(self.selected).cloned() else {
                    return SettingsSubmenuAction::None;
                };
                if value == AUTOMATIC_THEME_VALUE {
                    self.theme_mode = ThemeMode::Automatic;
                    self.selected = 0;
                    return SettingsSubmenuAction::Preview(self.automatic_setting());
                }
                if cycle_only {
                    return SettingsSubmenuAction::Preview(value);
                }
                self.single_theme = value.clone();
                SettingsSubmenuAction::Apply {
                    id: "theme".into(),
                    value,
                }
            }
            ThemeMode::Automatic => match self.selected {
                0 => {
                    self.light_theme = self.next_theme(&self.light_theme);
                    SettingsSubmenuAction::Preview(self.automatic_setting())
                }
                1 => {
                    self.dark_theme = self.next_theme(&self.dark_theme);
                    SettingsSubmenuAction::Preview(self.automatic_setting())
                }
                2 if !cycle_only => SettingsSubmenuAction::Apply {
                    id: "theme".into(),
                    value: self.automatic_setting(),
                },
                3 => {
                    self.theme_mode = ThemeMode::Single;
                    self.single_theme = self.dark_theme.clone();
                    self.selected = 0;
                    SettingsSubmenuAction::Preview(self.single_theme.clone())
                }
                _ => SettingsSubmenuAction::None,
            },
        }
    }

    fn activate_thinking(&mut self) -> SettingsSubmenuAction {
        if self.thinking_step == ThinkingStep::Model {
            let Some(model) = self.models.get(self.selected).cloned() else {
                return SettingsSubmenuAction::None;
            };
            self.thinking_model = Some(model.key);
            self.thinking_step = ThinkingStep::Level;
            self.selected = 0;
            return SettingsSubmenuAction::None;
        }
        let Some(model) = self.thinking_model.clone() else {
            return SettingsSubmenuAction::None;
        };
        let levels = self.level_values();
        let Some(level) = levels.get(self.selected).cloned() else {
            return SettingsSubmenuAction::None;
        };
        if let Some(item) = self.models.iter_mut().find(|item| item.key == model) {
            item.level = if level == "__clear__" {
                None
            } else {
                Some(level.clone())
            };
        }
        self.thinking_step = ThinkingStep::Model;
        self.thinking_model = None;
        self.selected = 0;
        SettingsSubmenuAction::Apply {
            id: "model-thinking".into(),
            value: format!("{model}={level}"),
        }
    }

    fn preview(&self) -> SettingsSubmenuAction {
        if self.kind != SettingsSubmenuKind::Theme || self.theme_mode != ThemeMode::Single {
            return SettingsSubmenuAction::None;
        }
        let items = self.single_items();
        match items.get(self.selected) {
            Some(value) if value == AUTOMATIC_THEME_VALUE => {
                SettingsSubmenuAction::Preview(self.automatic_setting())
            }
            Some(value) => SettingsSubmenuAction::Preview(value.clone()),
            None => SettingsSubmenuAction::None,
        }
    }

    fn move_sel(&mut self, delta: isize) {
        let len = self.item_count() as isize;
        if len == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    fn item_count(&self) -> usize {
        match self.kind {
            SettingsSubmenuKind::Theme => match self.theme_mode {
                ThemeMode::Single => self.single_items().len(),
                ThemeMode::Automatic => 4,
            },
            SettingsSubmenuKind::Warnings => 1,
            SettingsSubmenuKind::ModelThinking => match self.thinking_step {
                ThinkingStep::Model => self.models.len(),
                ThinkingStep::Level => self.level_values().len(),
            },
        }
    }

    fn single_items(&self) -> Vec<String> {
        let mut items = vec![AUTOMATIC_THEME_VALUE.to_string()];
        items.extend(self.themes.iter().cloned());
        items
    }

    fn next_theme(&self, current: &str) -> String {
        if self.themes.is_empty() {
            return current.to_string();
        }
        let index = self
            .themes
            .iter()
            .position(|theme| theme == current)
            .unwrap_or(0);
        self.themes[(index + 1) % self.themes.len()].clone()
    }

    fn automatic_setting(&self) -> String {
        format!("{}/{}", self.light_theme, self.dark_theme)
    }

    fn level_values(&self) -> Vec<String> {
        let mut levels = self.thinking_levels.clone();
        if self
            .thinking_model
            .as_ref()
            .and_then(|key| self.models.iter().find(|item| &item.key == key))
            .and_then(|item| item.level.as_ref())
            .is_some()
        {
            levels.push("__clear__".into());
        }
        levels
    }
}

pub fn parse_auto_theme(setting: &str) -> Option<(String, String)> {
    let (light, dark) = setting.split_once('/')?;
    if light.is_empty() || dark.is_empty() || light == "/" {
        return None;
    }
    Some((light.to_string(), dark.to_string()))
}

impl Component for SettingsSubmenu {
    fn render(&self, width: usize) -> Vec<String> {
        match self.kind {
            SettingsSubmenuKind::Theme => self.render_theme(width),
            SettingsSubmenuKind::Warnings => {
                let mut section = crate::render::CommandSection::new(width, "Warnings", None);
                section.item(
                    true,
                    "Anthropic extra usage",
                    if self.anthropic_extra_usage {
                        "on"
                    } else {
                        "off"
                    },
                );
                section.detail("Controls the extra-usage warning shown for Anthropic accounts.");
                section.hint("enter toggle · esc back");
                section.finish()
            }
            SettingsSubmenuKind::ModelThinking => self.render_thinking(width),
        }
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

impl SettingsSubmenu {
    fn render_theme(&self, width: usize) -> Vec<String> {
        match self.theme_mode {
            ThemeMode::Single => {
                let items = self.single_items();
                let mut section = crate::render::CommandSection::new(width, "Theme", None);
                section.detail("Select one theme, or Automatic to follow terminal appearance.");
                for index in crate::render::selection_window(self.selected, items.len(), 8) {
                    let value = &items[index];
                    let label = if value == AUTOMATIC_THEME_VALUE {
                        "Automatic"
                    } else {
                        value
                    };
                    section.item(index == self.selected, label, "");
                }
                section.position(self.selected, items.len(), 8);
                section.hint("↑↓ move · enter select · esc back");
                section.finish()
            }
            ThemeMode::Automatic => {
                let rows = [
                    ("Light theme", self.light_theme.as_str()),
                    ("Dark theme", self.dark_theme.as_str()),
                    ("Apply", "save and go back"),
                    ("Change mode", "switch to single theme"),
                ];
                let mut section =
                    crate::render::CommandSection::new(width, "Automatic theme", None);
                section.detail("Choose themes for terminal light and dark appearance.");
                section.detail("Light/dark detection requires terminal support.");
                for (index, (label, value)) in rows.iter().enumerate() {
                    section.item(index == self.selected, label, value);
                }
                section.hint("↑↓ move · enter select · esc back");
                section.finish()
            }
        }
    }

    fn render_thinking(&self, width: usize) -> Vec<String> {
        match self.thinking_step {
            ThinkingStep::Model => {
                let mut section =
                    crate::render::CommandSection::new(width, "Per-model thinking level", None);
                section.detail("Select a model to configure.");
                if self.models.is_empty() {
                    section.detail("No models available.");
                } else {
                    for index in
                        crate::render::selection_window(self.selected, self.models.len(), 8)
                    {
                        let model = &self.models[index];
                        section.item(
                            index == self.selected,
                            &model.label,
                            model.level.as_deref().unwrap_or("default"),
                        );
                    }
                    section.position(self.selected, self.models.len(), 8);
                }
                section.hint("↑↓ move · enter select · esc back");
                section.finish()
            }
            ThinkingStep::Level => {
                let title = self
                    .thinking_model
                    .as_deref()
                    .and_then(|key| self.models.iter().find(|item| item.key == key))
                    .map(|item| format!("Thinking level for {}", item.label))
                    .unwrap_or_else(|| "Thinking level".into());
                let levels = self.level_values();
                let mut section = crate::render::CommandSection::new(width, &title, None);
                section.detail("Select the default thinking level for this model.");
                for index in crate::render::selection_window(self.selected, levels.len(), 8) {
                    let level = &levels[index];
                    let label = if level == "__clear__" {
                        "Clear override"
                    } else {
                        level
                    };
                    section.item(index == self.selected, label, "");
                }
                section.position(self.selected, levels.len(), 8);
                section.hint("↑↓ move · enter select · esc back");
                section.finish()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_submenu_matches_ts_single_and_automatic() {
        let mut menu =
            SettingsSubmenu::theme("dark", vec!["dark".into(), "light".into(), "pi".into()]);
        let rendered = menu.render(80).join("\n");
        assert!(rendered.contains("Select one theme, or Automatic"));
        assert!(rendered.contains("Automatic"));
        assert_eq!(
            menu.handle_key("\r"),
            SettingsSubmenuAction::Preview("dark/dark".into())
        );
        let automatic = menu.render(80).join("\n");
        assert!(automatic.contains("Automatic theme"));
        assert!(automatic.contains("Light theme"));
        assert!(automatic.contains("Dark theme"));
        assert!(automatic.contains("save and go back"));
        assert!(automatic.contains("switch to single theme"));
        menu.selected = 2;
        assert_eq!(
            menu.handle_key("\r"),
            SettingsSubmenuAction::Apply {
                id: "theme".into(),
                value: "dark/dark".into(),
            }
        );
        assert_eq!(
            parse_auto_theme("light/dark"),
            Some(("light".into(), "dark".into()))
        );
        assert!(parse_auto_theme("dark").is_none());
    }

    #[test]
    fn warnings_and_model_thinking_submenus_match_ts() {
        let mut warnings = SettingsSubmenu::warnings(true);
        assert!(warnings
            .render(80)
            .join("\n")
            .contains("Anthropic extra usage"));
        assert_eq!(
            warnings.handle_key("\r"),
            SettingsSubmenuAction::Apply {
                id: "warnings.anthropic-extra-usage".into(),
                value: "false".into(),
            }
        );
        let mut thinking = SettingsSubmenu::model_thinking(vec![ModelThinkingItem {
            key: "openai/gpt".into(),
            label: "gpt [openai]".into(),
            level: None,
        }]);
        assert!(thinking
            .render(80)
            .join("\n")
            .contains("Per-model thinking level"));
        assert_eq!(thinking.handle_key("\r"), SettingsSubmenuAction::None);
        assert!(thinking
            .render(80)
            .join("\n")
            .contains("Thinking level for gpt [openai]"));
        thinking.selected = thinking
            .level_values()
            .iter()
            .position(|level| level == "high")
            .unwrap_or(0);
        assert_eq!(
            thinking.handle_key("\r"),
            SettingsSubmenuAction::Apply {
                id: "model-thinking".into(),
                value: "openai/gpt=high".into(),
            }
        );
    }
}

#[cfg(test)]
mod section_regressions {
    use super::*;

    #[test]
    fn submenus_use_shared_sections_and_keep_nested_back_behavior() {
        let mut menu = SettingsSubmenu::theme(
            "dark",
            vec![
                "dark".into(),
                "light".into(),
                "模型-🦀-very-long-theme".into(),
            ],
        );
        menu.selected = 2;
        let rendered = crate::render::strip_terminal_sequences(&menu.render(28).join("\n"));
        assert!(
            rendered.contains(crate::davinci::ui::SELECTION_BAR.trim()),
            "{rendered}"
        );
        assert!(rendered.to_lowercase().contains("esc"), "{rendered}");
        for width in [0, 1, 20, 28, 40] {
            for row in menu.render(width) {
                assert!(
                    crate::render::visible_width_stripped(&row) <= width,
                    "{width}: {row:?}"
                );
            }
        }
        menu.selected = 0;
        assert!(matches!(
            menu.handle_key("\r"),
            SettingsSubmenuAction::Preview(_)
        ));
        assert_eq!(menu.handle_key("\x1b"), SettingsSubmenuAction::None);
        assert_eq!(menu.handle_key("\x1b"), SettingsSubmenuAction::Cancel);
    }
}
