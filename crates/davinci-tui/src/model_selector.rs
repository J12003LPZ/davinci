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
    /// Catalog models whose provider has no credentials yet: listed dimmed
    /// with a `/login` hint so every model stays discoverable.
    locked_models: Vec<ModelSelectorItem>,
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
            locked_models: Vec::new(),
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

    pub fn with_locked_models(mut self, mut locked: Vec<ModelSelectorItem>) -> Self {
        let known: std::collections::BTreeSet<String> =
            self.all_models.iter().map(ModelSelectorItem::key).collect();
        locked.retain(|item| !known.contains(&item.key()));
        locked.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.id.cmp(&b.id)));
        self.locked_models = locked;
        self
    }

    pub fn is_locked(&self, key: &str) -> bool {
        self.locked_models.iter().any(|item| item.key() == key)
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
        let mut out = self.filtered_available();
        if self.scope == ModelScope::All || self.scoped_models.is_empty() {
            out.extend(self.filtered_locked());
        }
        out
    }

    fn filtered_locked(&self) -> Vec<ModelSelectorItem> {
        if self.search.is_empty() {
            return self.locked_models.clone();
        }
        self.locked_models
            .iter()
            .filter(|item| {
                fuzzy_match(&self.search, &Self::get_model_selector_search_text(item)).matches
            })
            .cloned()
            .collect()
    }

    fn filtered_available(&self) -> Vec<ModelSelectorItem> {
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
        let mut section =
            crate::render::CommandSection::new(width, "Choose model", Some(&self.theme));
        if self.scoped_models.is_empty() {
            section.message(
                "warning",
                "Configured providers only. Use /login to add a provider.",
            );
        } else {
            let scope = match self.scope {
                ModelScope::All => "all",
                ModelScope::Scoped => "scoped",
            };
            section.detail(&format!("Scope: {scope} · tab changes scope"));
        }
        if !self.locked_models.is_empty() {
            section.detail(&format!(
                "{} models need provider credentials.",
                self.locked_models.len()
            ));
        }
        section.search(&self.search);

        let filtered = self.filtered();
        if filtered.is_empty() {
            section.detail("No matching models.");
        }
        for index in crate::render::selection_window(self.selected, filtered.len(), 10) {
            let item = &filtered[index];
            let selected = index == self.selected;
            let locked = self.is_locked(&item.key());
            let mut state = item.provider.clone();
            if locked {
                state.push_str(" · unavailable");
            }
            if self.is_current(item) {
                state.push_str(" · current");
            }
            if self.is_default(item) {
                state.push_str(" · default");
            }
            let label = if item.name.is_empty() || item.name == item.id {
                item.id.clone()
            } else {
                format!("{} · {}", item.name, item.id)
            };
            section.item(selected, &label, &state);
            if selected {
                section.detail(&format!("ID: {}", item.key()));
                if locked {
                    section.message(
                        "warning",
                        &format!("Unavailable until /login {} succeeds.", item.provider),
                    );
                }
            }
        }
        section.position(self.selected, filtered.len(), 10);
        if let Some(error) = &self.error_message {
            section.message("error", error);
        }
        if let Some(status) = &self.refresh_status {
            section.message(
                if self.refresh_status_success {
                    "success"
                } else {
                    "muted"
                },
                status,
            );
        }
        section.hint("↑↓ move · type search · enter select · ctrl+s default · esc cancel");
        section.finish()
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
        // This asserts on colour, and sibling tests in this binary set
        // NO_COLOR process-wide. Hold the lock across the render, not just
        // the assertions, or the rows are painted with colour already off.
        let _guard = crate::themes::NO_COLOR_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var("NO_COLOR").ok();
        std::env::remove_var("NO_COLOR");

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
        let rendered = crate::render::strip_terminal_sequences(&lines.join("\n"));
        assert!(rendered.contains("Configured providers only."));
        assert!(rendered.contains("enter select"));
        assert!(rendered.contains("ID: google/gemini"));
        // Role colors present: warning, accent (copper), and muted.
        let joined = lines.join("\n");
        for role in ["warning", "accent", "muted"] {
            let colored = selector.theme.fg(role, "probe");
            let code = colored
                .strip_suffix("probe\x1b[39m")
                .or_else(|| colored.strip_suffix("probe\x1b[22m"))
                .unwrap_or("");
            assert!(
                !code.is_empty() && joined.contains(code),
                "missing {role} color {code:?}"
            );
        }

        if let Some(value) = previous {
            std::env::set_var("NO_COLOR", value);
        }
    }

    #[test]
    fn command_section_render_uses_shared_focus_and_keeps_locked_guidance() {
        let locked = ModelSelectorItem {
            provider: "provider-长".into(),
            id: "locked-model-with-a-long-name".into(),
            name: "Locked Model 🦀".into(),
        };
        let mut selector =
            ModelSelector::new(items(), Some("google/gemini".into()), None, Vec::new())
                .with_locked_models(vec![locked]);
        selector.selected = selector.filtered().len().saturating_sub(1);
        let rendered = crate::render::strip_terminal_sequences(&selector.render(32).join("\n"));
        assert!(rendered.contains("Choose model"), "{rendered}");
        assert!(
            rendered.contains(crate::davinci::ui::SELECTION_BAR.trim()),
            "{rendered}"
        );
        assert!(rendered.contains("/login"), "{rendered}");
        assert!(rendered.contains("provider-长 succeeds."), "{rendered}");
        assert!(!rendered.contains("COGITATOR"), "{rendered}");
        for width in [0, 1, 20, 32, 40] {
            for row in selector.render(width) {
                assert!(
                    crate::render::visible_width_stripped(&row) <= width,
                    "{width}: {row:?}"
                );
            }
        }
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
