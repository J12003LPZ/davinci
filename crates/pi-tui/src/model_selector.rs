//! `/model` selector matching TS `ModelSelectorComponent` + `model-search.ts`.

use crate::fuzzy::fuzzy_match;
use crate::render::Component;
use crate::themes::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelectorItem {
    pub provider: String,
    pub id: String,
    pub name: String,
}

impl ModelSelectorItem {
    pub fn key(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }

    pub fn from_key(key: &str) -> Self {
        let (provider, id) = key.split_once('/').unwrap_or(("unknown", key));
        Self {
            provider: provider.to_string(),
            id: id.to_string(),
            name: id.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelScope {
    All,
    Scoped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSelectorAction {
    None,
    Select(String),
    SelectAsDefault(String),
    Cancel,
}

#[derive(Debug, Clone)]
pub struct ModelSelector {
    pub search: String,
    pub selected: usize,
    pub scope: ModelScope,
    pub current: Option<String>,
    pub default_model: Option<String>,
    pub error_message: Option<String>,
    pub refresh_status: Option<String>,
    pub refresh_status_success: bool,
    theme: Theme,
    all_models: Vec<ModelSelectorItem>,
    scoped_models: Vec<ModelSelectorItem>,
}

impl ModelSelector {
    pub fn new(
        models: Vec<ModelSelectorItem>,
        current: Option<String>,
        default_model: Option<String>,
        scoped_models: Vec<ModelSelectorItem>,
    ) -> Self {
        let scope = if scoped_models.is_empty() {
            ModelScope::All
        } else {
            ModelScope::Scoped
        };
        let mut selector = Self {
            search: String::new(),
            selected: 0,
            scope,
            current,
            default_model,
            error_message: None,
            refresh_status: Some("Refreshing model catalogs…".into()),
            refresh_status_success: false,
            theme: Theme::default(),
            all_models: models,
            scoped_models,
        };
        selector.sort_models();
        selector.selected = selector.current_index().unwrap_or(0);
        selector
    }

    pub fn reload(
        &mut self,
        models: Vec<ModelSelectorItem>,
        current: Option<String>,
        default_model: Option<String>,
        scoped_models: Vec<ModelSelectorItem>,
    ) {
        self.all_models = models;
        self.scoped_models = scoped_models;
        self.current = current;
        self.default_model = default_model;
        if self.scoped_models.is_empty() {
            self.scope = ModelScope::All;
        }
        self.sort_models();
        self.filter_models();
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn set_refresh_status(&mut self, message: Option<String>, success: bool) {
        self.refresh_status = message;
        self.refresh_status_success = success;
    }

    pub fn selected_key(&self) -> Option<String> {
        self.filtered()
            .get(self.selected)
            .map(ModelSelectorItem::key)
    }

    pub fn handle_key(&mut self, data: &str) -> ModelSelectorAction {
        match data {
            "\t" => {
                if !self.scoped_models.is_empty() {
                    self.scope = match self.scope {
                        ModelScope::All => ModelScope::Scoped,
                        ModelScope::Scoped => ModelScope::All,
                    };
                    self.selected = self.current_index().unwrap_or(0);
                    self.filter_models();
                }
                ModelSelectorAction::None
            }
            "\x1b[A" => {
                self.move_by(-1);
                ModelSelectorAction::None
            }
            "\x1b[B" => {
                self.move_by(1);
                ModelSelectorAction::None
            }
            "\r" | "\n" => self
                .selected_key()
                .map(ModelSelectorAction::Select)
                .unwrap_or(ModelSelectorAction::None),
            "\x13" => self
                .selected_key()
                .map(ModelSelectorAction::SelectAsDefault)
                .unwrap_or(ModelSelectorAction::None),
            "\x1b" | "\x03" => ModelSelectorAction::Cancel,
            "\x7f" | "\x08" => {
                self.search.pop();
                self.filter_models();
                ModelSelectorAction::None
            }
            other => {
                if !other.is_empty() && !other.chars().any(|ch| ch.is_control()) {
                    self.search.push_str(other);
                    self.filter_models();
                }
                ModelSelectorAction::None
            }
        }
    }

    pub fn get_model_selector_search_text(item: &ModelSelectorItem) -> String {
        let name = if item.name.is_empty() {
            String::new()
        } else {
            format!(" {}", item.name)
        };
        format!(
            "{} {}/{} {} {}{}",
            item.provider, item.provider, item.id, item.provider, item.id, name
        )
    }

    fn sort_models(&mut self) {
        let current = self.current.clone();
        let default = self.default_model.clone();
        self.all_models
            .sort_by(|a, b| cmp_models(a, b, current.as_deref(), default.as_deref()));
        self.scoped_models
            .sort_by(|a, b| cmp_models(a, b, current.as_deref(), default.as_deref()));
    }

    fn active_models(&self) -> &[ModelSelectorItem] {
        if self.scope == ModelScope::Scoped && !self.scoped_models.is_empty() {
            &self.scoped_models
        } else {
            &self.all_models
        }
    }

    fn filtered(&self) -> Vec<ModelSelectorItem> {
        let active = self.active_models();
        if self.search.is_empty() {
            return active.to_vec();
        }
        let mut scored: Vec<(f64, ModelSelectorItem)> = active
            .iter()
            .filter_map(|item| {
                let mut haystack = Self::get_model_selector_search_text(item);
                if self.is_default(item) {
                    haystack.push_str(" default");
                }
                let matched = fuzzy_match(&self.search, &haystack);
                matched.matches.then_some((matched.score, item.clone()))
            })
            .collect();
        if is_default_search(&self.search) {
            let defaults: Vec<ModelSelectorItem> = active
                .iter()
                .filter(|item| self.is_default(item))
                .cloned()
                .collect();
            let keys: Vec<String> = defaults.iter().map(ModelSelectorItem::key).collect();
            scored.retain(|(_, item)| !keys.contains(&item.key()));
            let mut out = defaults;
            scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            out.extend(scored.into_iter().map(|(_, item)| item));
            return out;
        }
        scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().map(|(_, item)| item).collect()
    }

    fn filter_models(&mut self) {
        let filtered = self.filtered();
        self.selected = if self.search.is_empty() {
            self.selected.min(filtered.len().saturating_sub(1))
        } else {
            0
        };
    }

    fn move_by(&mut self, delta: isize) {
        let len = self.filtered().len() as isize;
        if len == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    fn current_index(&self) -> Option<usize> {
        let current = self.current.as_deref()?;
        self.filtered()
            .iter()
            .position(|item| item.key() == current)
    }

    fn is_default(&self, item: &ModelSelectorItem) -> bool {
        self.default_model
            .as_deref()
            .is_some_and(|key| key == item.key())
    }

    fn is_current(&self, item: &ModelSelectorItem) -> bool {
        self.current.as_deref().is_some_and(|key| key == item.key())
    }
}

impl Component for ModelSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![String::new()];
        if self.scoped_models.is_empty() {
            lines.push(self.theme.fg(
                "warning",
                "Only showing models from configured providers. Use /login to add providers.",
            ));
        } else {
            let all = if self.scope == ModelScope::All {
                self.theme.fg("accent", "all")
            } else {
                self.theme.fg("muted", "all")
            };
            let scoped = if self.scope == ModelScope::Scoped {
                self.theme.fg("accent", "scoped")
            } else {
                self.theme.fg("muted", "scoped")
            };
            lines.push(format!(
                "{}{}{}{}",
                self.theme.fg("muted", "Scope: "),
                all,
                self.theme.fg("muted", " | "),
                scoped
            ));
            lines.push(format!(
                "tab scope{}",
                self.theme.fg("muted", " (all/scoped)")
            ));
        }
        lines.push(String::new());
        lines.push(format!("> {}", self.search));
        lines.push(String::new());

        let filtered = self.filtered();
        let max_visible = 10;
        let start = if filtered.is_empty() {
            0
        } else {
            self.selected
                .saturating_sub(max_visible / 2)
                .min(filtered.len().saturating_sub(max_visible))
        };
        let end = (start + max_visible).min(filtered.len());
        for (offset, item) in filtered[start..end].iter().enumerate() {
            let index = start + offset;
            let selected = index == self.selected;
            let prefix = if selected {
                self.theme.fg("accent", "→ ")
            } else {
                "  ".into()
            };
            let id = if selected {
                self.theme.fg("accent", &item.id)
            } else {
                item.id.clone()
            };
            let provider_badge = self.theme.fg("muted", &format!("[{}]", item.provider));
            let default_badge = if self.is_default(item) {
                self.theme.fg("muted", " · default")
            } else {
                String::new()
            };
            let check = if self.is_current(item) {
                self.theme.fg("success", " ✓")
            } else {
                String::new()
            };
            lines.push(truncate(
                &format!("{prefix}{id} {provider_badge}{default_badge}{check}"),
                width,
            ));
        }
        if start > 0 || end < filtered.len() {
            lines.push(self.theme.fg(
                "muted",
                &format!("  ({}/{})", self.selected + 1, filtered.len()),
            ));
        }
        if let Some(error) = &self.error_message {
            for line in error.split('\n') {
                lines.push(self.theme.fg("error", line));
            }
        } else if filtered.is_empty() {
            lines.push(self.theme.fg("muted", "  No matching models"));
        } else if let Some(selected) = filtered.get(self.selected) {
            lines.push(String::new());
            lines.push(
                self.theme
                    .fg("muted", &format!("  Model Name: {}", selected.name)),
            );
        }
        if let Some(status) = &self.refresh_status {
            lines.push(String::new());
            let role = if self.refresh_status_success {
                "success"
            } else {
                "muted"
            };
            lines.push(self.theme.fg(role, &format!("  {status}")));
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

fn cmp_models(
    a: &ModelSelectorItem,
    b: &ModelSelectorItem,
    current: Option<&str>,
    default: Option<&str>,
) -> std::cmp::Ordering {
    let a_current = current == Some(a.key().as_str());
    let b_current = current == Some(b.key().as_str());
    if a_current != b_current {
        return if a_current {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        };
    }
    let a_default = default == Some(a.key().as_str());
    let b_default = default == Some(b.key().as_str());
    if a_default != b_default {
        return if a_default {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        };
    }
    a.provider.cmp(&b.provider)
}

fn is_default_search(query: &str) -> bool {
    let normalized = query.trim().to_ascii_lowercase();
    !normalized.is_empty() && "default".starts_with(&normalized)
}

fn truncate(line: &str, width: usize) -> String {
    if crate::render::visible_width(line) <= width {
        line.to_string()
    } else {
        let mut out = String::new();
        for ch in line.chars() {
            if crate::render::visible_width(&out)
                + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1)
                > width.saturating_sub(1)
            {
                break;
            }
            out.push(ch);
        }
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<ModelSelectorItem> {
        vec![
            ModelSelectorItem {
                provider: "google".into(),
                id: "gemini".into(),
                name: "Gemini".into(),
            },
            ModelSelectorItem {
                provider: "anthropic".into(),
                id: "sonnet".into(),
                name: "Sonnet".into(),
            },
        ]
    }

    #[test]
    fn search_text_keeps_bare_id_out_of_lead() {
        let item = ModelSelectorItem {
            provider: "openrouter".into(),
            id: "openai/gpt-5".into(),
            name: "GPT-5".into(),
        };
        let text = ModelSelector::get_model_selector_search_text(&item);
        assert_eq!(
            text,
            "openrouter openrouter/openai/gpt-5 openrouter openai/gpt-5 GPT-5"
        );
        assert!(!text.starts_with("openai/gpt-5 "));
    }

    #[test]
    fn sorts_current_then_default_then_provider() {
        let mut selector = ModelSelector::new(
            items(),
            Some("google/gemini".into()),
            Some("anthropic/sonnet".into()),
            Vec::new(),
        );
        selector.search.clear();
        let filtered = selector.filtered();
        assert_eq!(filtered[0].key(), "google/gemini");
        assert_eq!(filtered[1].key(), "anthropic/sonnet");
        let lines = selector.render(80);
        assert!(lines
            .iter()
            .any(|line| line.contains("Only showing models from configured providers")));
        assert!(lines
            .iter()
            .any(|line| line.contains("Enter to select · Ctrl+S to set as default")));
        assert!(lines.iter().any(|line| line.contains("Model Name:")));
        assert!(lines.iter().any(|line| line.contains("\x1b[33m")));
        assert!(lines.iter().any(|line| line.contains("\x1b[36m")));
        assert!(lines.iter().any(|line| line.contains("\x1b[32m")));
        assert!(lines.iter().any(|line| line.contains("\x1b[2m")));
    }

    #[test]
    fn tab_toggles_scope_and_ctrl_s_sets_default() {
        let scoped = vec![ModelSelectorItem {
            provider: "anthropic".into(),
            id: "sonnet".into(),
            name: "Sonnet".into(),
        }];
        let mut selector =
            ModelSelector::new(items(), Some("anthropic/sonnet".into()), None, scoped);
        assert_eq!(selector.scope, ModelScope::Scoped);
        assert_eq!(selector.handle_key("\t"), ModelSelectorAction::None);
        assert_eq!(selector.scope, ModelScope::All);
        assert_eq!(
            selector.handle_key("\x13"),
            ModelSelectorAction::SelectAsDefault("anthropic/sonnet".into())
        );
        selector.handle_key("z");
        assert!(selector
            .render(80)
            .iter()
            .any(|line| line.contains("No matching models")));
    }
}
