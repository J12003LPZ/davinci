//! Provider auth selector matching TS `oauth-selector.ts`.

use crate::fuzzy::fuzzy_match;
use crate::input::Input;
use crate::keybindings::Keybindings;
use crate::render::Component;
use crate::themes::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSelectorMode {
    Login,
    Logout,
}

impl AuthSelectorMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Logout => "logout",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSelectorProvider {
    pub id: String,
    pub name: String,
    pub auth_type: String,
    pub method_name: Option<String>,
    pub status_type: Option<String>,
    pub status_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthSelectorAction {
    None,
    Select {
        provider_id: String,
        auth_type: String,
    },
    Cancel,
}

pub fn format_auth_selector_provider_type(auth_type: &str) -> &'static str {
    if auth_type == "oauth" {
        "subscription"
    } else {
        "API key"
    }
}

#[derive(Debug, Clone)]
pub struct OAuthSelector {
    mode: AuthSelectorMode,
    search: Input,
    all: Vec<AuthSelectorProvider>,
    filtered: Vec<AuthSelectorProvider>,
    selected: usize,
    show_auth_type_labels: bool,
    theme: Theme,
}

impl OAuthSelector {
    pub fn new(
        mode: AuthSelectorMode,
        providers: Vec<AuthSelectorProvider>,
        initial_search: Option<&str>,
    ) -> Self {
        let show_auth_type_labels = providers
            .iter()
            .map(|provider| provider.auth_type.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 1;
        let mut search = Input::new();
        if let Some(value) = initial_search {
            search.set_value(value);
        }
        let mut selector = Self {
            mode,
            search,
            all: providers.clone(),
            filtered: providers,
            selected: 0,
            show_auth_type_labels,
            theme: Theme::default(),
        };
        selector.filter_providers();
        selector
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn handle_key(&mut self, data: &str) -> OAuthSelectorAction {
        let bindings = Keybindings::defaults();
        if bindings.matches(data, "tui.select.up") {
            if !self.filtered.is_empty() {
                self.selected = self.selected.saturating_sub(1);
            }
            return OAuthSelectorAction::None;
        }
        if bindings.matches(data, "tui.select.down") {
            if !self.filtered.is_empty() {
                self.selected = (self.selected + 1).min(self.filtered.len().saturating_sub(1));
            }
            return OAuthSelectorAction::None;
        }
        if bindings.matches(data, "tui.select.confirm") || data == "\r" || data == "\n" {
            if let Some(provider) = self.filtered.get(self.selected) {
                return OAuthSelectorAction::Select {
                    provider_id: provider.id.clone(),
                    auth_type: provider.auth_type.clone(),
                };
            }
            return OAuthSelectorAction::None;
        }
        if bindings.matches(data, "tui.select.cancel") || data == "\x1b" {
            return OAuthSelectorAction::Cancel;
        }
        self.search.handle_input(data);
        self.filter_providers();
        OAuthSelectorAction::None
    }

    fn filter_providers(&mut self) {
        let query = self.search.value();
        self.filtered = if query.trim().is_empty() {
            self.all.clone()
        } else {
            self.all
                .iter()
                .filter(|provider| {
                    let haystack = format!(
                        "{} {} {} {}",
                        provider.name,
                        provider.id,
                        provider.auth_type,
                        provider.method_name.as_deref().unwrap_or("")
                    );
                    fuzzy_match(query, &haystack).matches
                })
                .cloned()
                .collect()
        };
        if self.filtered.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.filtered.len() - 1);
        }
    }

    fn format_status(provider: &AuthSelectorProvider, theme: &Theme) -> String {
        let status = match provider.status_type.as_deref() {
            None => "not configured".to_string(),
            Some(kind) if kind != provider.auth_type => {
                format!("{} configured", format_auth_selector_provider_type(kind))
            }
            Some(_) => "configured".to_string(),
        };
        theme.fg("muted", &status)
    }
}

impl Component for OAuthSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let title = if self.mode == AuthSelectorMode::Login {
            "Configure provider"
        } else {
            "Log out provider"
        };
        let mut section = crate::render::CommandSection::new(width, title, Some(&self.theme));
        section.input(self.search.render(width));
        if self.filtered.is_empty() {
            section.detail(if self.all.is_empty() {
                if self.mode == AuthSelectorMode::Login {
                    "No providers available."
                } else {
                    "No providers logged in. Use /login first."
                }
            } else {
                "No matching providers."
            });
        }
        for index in crate::render::selection_window(self.selected, self.filtered.len(), 8) {
            let provider = &self.filtered[index];
            section.item(
                index == self.selected,
                &provider.name,
                &Self::format_status(provider, &self.theme),
            );
            if index == self.selected {
                section.detail(&format!("Provider: {}", provider.id));
                if self.show_auth_type_labels || provider.method_name.is_some() {
                    section.detail(&format!(
                        "Method: {}",
                        provider
                            .method_name
                            .as_deref()
                            .unwrap_or(&provider.auth_type)
                    ));
                }
                if let Some(source) = &provider.status_source {
                    section.detail(&format!("Source: {source}"));
                }
            }
        }
        section.position(self.selected, self.filtered.len(), 8);
        section.hint("↑↓ move · enter select · esc cancel");
        section.finish()
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_selector_filters_and_selects() {
        let mut selector = OAuthSelector::new(
            AuthSelectorMode::Login,
            vec![
                AuthSelectorProvider {
                    id: "anthropic".into(),
                    name: "Anthropic".into(),
                    auth_type: "oauth".into(),
                    method_name: Some("Claude Pro/Max".into()),
                    status_type: None,
                    status_source: None,
                },
                AuthSelectorProvider {
                    id: "openai".into(),
                    name: "OpenAI".into(),
                    auth_type: "api_key".into(),
                    method_name: None,
                    status_type: Some("api_key".into()),
                    status_source: Some("OPENAI_API_KEY".into()),
                },
            ],
            None,
        );
        let rendered = selector.render(40).join("\n");
        assert!(rendered.contains("Configure provider"));
        assert!(rendered.contains("not configured"));
        selector.handle_key("open");
        let filtered = selector.render(40).join("\n");
        assert!(filtered.contains("Source: OPENAI_API_KEY"), "{filtered:?}");
        assert_eq!(
            selector.handle_key("\r"),
            OAuthSelectorAction::Select {
                provider_id: "openai".into(),
                auth_type: "api_key".into(),
            }
        );
    }

    #[test]
    fn logout_empty_copy_matches_ts() {
        let selector = OAuthSelector::new(AuthSelectorMode::Logout, Vec::new(), None);
        let rendered = selector.render(40).join("\n");
        assert!(rendered.contains("No providers logged in."));
        assert!(rendered.contains("Use /login"), "{rendered:?}");
        assert!(rendered.contains("first."), "{rendered:?}");
    }
}
