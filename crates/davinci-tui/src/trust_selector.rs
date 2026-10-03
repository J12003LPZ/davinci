//! `/trust` selector matching TS `TrustSelectorComponent`.

use crate::render::Component;
use crate::themes::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustUpdate {
    pub path: String,
    pub decision: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustOption {
    pub label: String,
    pub trusted: bool,
    pub updates: Vec<TrustUpdate>,
    pub saved_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustSavedDecision {
    pub path: String,
    pub decision: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustSelectorAction {
    None,
    Select {
        trusted: bool,
        updates: Vec<TrustUpdate>,
    },
    Cancel,
}

#[derive(Debug, Clone)]
pub struct TrustSelector {
    cwd: String,
    options: Vec<TrustOption>,
    saved: Option<TrustSavedDecision>,
    project_trusted: bool,
    selected: usize,
    theme: Theme,
}

impl TrustSelector {
    pub fn new(
        cwd: impl Into<String>,
        options: Vec<TrustOption>,
        saved: Option<TrustSavedDecision>,
        project_trusted: bool,
    ) -> Self {
        let selected = options
            .iter()
            .position(|option| is_saved_option(option, saved.as_ref()))
            .unwrap_or(0);
        Self {
            cwd: cwd.into(),
            options,
            saved,
            project_trusted,
            selected,
            theme: Theme::default(),
        }
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn handle_key(&mut self, data: &str) -> TrustSelectorAction {
        match data {
            "\x1b" | "\x1b[27~" => TrustSelectorAction::Cancel,
            "\r" | "\n" => self
                .options
                .get(self.selected)
                .map(|option| TrustSelectorAction::Select {
                    trusted: option.trusted,
                    updates: option.updates.clone(),
                })
                .unwrap_or(TrustSelectorAction::None),
            "\x1b[A" | "\x10" | "k" => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                TrustSelectorAction::None
            }
            "\x1b[B" | "\x0e" | "j" => {
                if self.selected + 1 < self.options.len() {
                    self.selected += 1;
                }
                TrustSelectorAction::None
            }
            _ => TrustSelectorAction::None,
        }
    }
}

fn is_saved_option(option: &TrustOption, saved: Option<&TrustSavedDecision>) -> bool {
    let Some(saved) = saved else {
        return false;
    };
    option.saved_path.as_deref() == Some(saved.path.as_str()) && option.trusted == saved.decision
}

fn format_decision(trust_path: Option<&str>, decision: Option<&TrustSavedDecision>) -> String {
    let Some(decision) = decision else {
        return "none".into();
    };
    let label = if decision.decision {
        "trusted"
    } else {
        "untrusted"
    };
    if trust_path.is_some_and(|path| path != decision.path) {
        format!("{label} (inherited from {})", decision.path)
    } else {
        format!("{label} ({})", decision.path)
    }
}

impl Component for TrustSelector {
    fn invalidate(&mut self) {}

    fn render(&self, width: usize) -> Vec<String> {
        let saved_path = self
            .options
            .first()
            .and_then(|option| option.saved_path.as_deref());
        let mut section =
            crate::render::CommandSection::new(width, "Project trust", Some(&self.theme));
        section.detail(&self.cwd);
        section.detail(&format!(
            "Saved: {}",
            format_decision(saved_path, self.saved.as_ref())
        ));
        section.detail(&format!(
            "Current session: {}",
            if self.project_trusted {
                "trusted"
            } else {
                "untrusted"
            }
        ));
        if self.options.is_empty() {
            section.detail("No trust choices are available.");
        }
        for index in crate::render::selection_window(self.selected, self.options.len(), 8) {
            let option = &self.options[index];
            let saved = is_saved_option(option, self.saved.as_ref());
            section.item(
                index == self.selected,
                &option.label,
                if saved { "saved" } else { "" },
            );
            if index == self.selected {
                if let Some(path) = option.saved_path.as_deref() {
                    section.detail(&format!("Applies to: {path}"));
                }
            }
        }
        section.position(self.selected, self.options.len(), 8);
        section.hint("↑↓ move · enter save · esc cancel");
        section.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::strip_terminal_sequences;

    fn options_for(cwd: &str) -> Vec<TrustOption> {
        let parent = std::path::Path::new(cwd)
            .parent()
            .map(|path| path.to_string_lossy().into_owned());
        let mut options = vec![TrustOption {
            label: "Trust".into(),
            trusted: true,
            updates: vec![TrustUpdate {
                path: cwd.into(),
                decision: Some(true),
            }],
            saved_path: Some(cwd.into()),
        }];
        if let Some(parent) = parent {
            if parent != cwd {
                options.push(TrustOption {
                    label: format!("Trust parent folder ({parent})"),
                    trusted: true,
                    updates: vec![
                        TrustUpdate {
                            path: parent.clone(),
                            decision: Some(true),
                        },
                        TrustUpdate {
                            path: cwd.into(),
                            decision: None,
                        },
                    ],
                    saved_path: Some(parent),
                });
            }
        }
        options.push(TrustOption {
            label: "Do not trust".into(),
            trusted: false,
            updates: vec![TrustUpdate {
                path: cwd.into(),
                decision: Some(false),
            }],
            saved_path: Some(cwd.into()),
        });
        options
    }

    fn rendered(selector: &TrustSelector) -> String {
        strip_terminal_sequences(&selector.render(120).join("\n"))
    }

    #[test]
    fn trust_selector_uses_shared_focus_and_wraps_paths() {
        let selector = TrustSelector::new(
            "/very/long/项目/path/that/needs/wrapping",
            options_for("/very/long/项目/path/that/needs/wrapping"),
            None,
            false,
        );
        let rendered = strip_terminal_sequences(&selector.render(32).join("\n"));
        assert!(
            rendered.contains(crate::davinci::ui::SELECTION_BAR.trim()),
            "{rendered}"
        );
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
    fn marks_the_saved_trusted_decision() {
        let selector = TrustSelector::new(
            "/project",
            options_for("/project"),
            Some(TrustSavedDecision {
                path: "/project".into(),
                decision: true,
            }),
            true,
        );
        let output = rendered(&selector);
        assert!(output.contains("Saved: trusted (/project)"));
        assert!(output.contains("Current session: trusted"));
        assert!(output.contains("Trust") && output.contains("saved"));
        assert!(!output.contains("Do not trust     saved"));
    }

    #[test]
    fn selects_a_trust_decision() {
        let mut selector = TrustSelector::new("/project", options_for("/project"), None, false);
        let action = selector.handle_key("\n");
        assert_eq!(
            action,
            TrustSelectorAction::Select {
                trusted: true,
                updates: vec![TrustUpdate {
                    path: "/project".into(),
                    decision: Some(true),
                }],
            }
        );
    }

    #[test]
    fn labels_saved_ancestor_decisions_as_inherited() {
        let selector = TrustSelector::new(
            "/parent/project/nested",
            options_for("/parent/project/nested"),
            Some(TrustSavedDecision {
                path: "/parent".into(),
                decision: true,
            }),
            true,
        );
        assert!(rendered(&selector).contains("Saved: trusted (inherited from /parent)"));
    }

    #[test]
    fn adds_a_trust_parent_option() {
        let mut selector = TrustSelector::new(
            "/parent/project",
            options_for("/parent/project"),
            Some(TrustSavedDecision {
                path: "/parent".into(),
                decision: true,
            }),
            true,
        );
        let output = rendered(&selector);
        assert!(output.contains("Saved: trusted (inherited from /parent)"));
        assert!(output.contains("Trust parent folder (/parent)") && output.contains("saved"));
        assert_eq!(
            selector.handle_key("\n"),
            TrustSelectorAction::Select {
                trusted: true,
                updates: vec![
                    TrustUpdate {
                        path: "/parent".into(),
                        decision: Some(true),
                    },
                    TrustUpdate {
                        path: "/parent/project".into(),
                        decision: None,
                    },
                ],
            }
        );
    }
}
