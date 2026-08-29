//! First-time setup dialog matching TS `first-time-setup.ts`.

use crate::render::Component;
use crate::themes::Theme;

pub const SETUP_LOGO_LINES: &[&str] = &["██████", "██  ██", "████  ██", "██    ██"];

pub const THEME_OPTIONS: &[(&str, &str)] = &[("dark", "Dark"), ("light", "Light")];
pub const ANALYTICS_OPTIONS: &[(bool, &str)] =
    &[(true, "Share anonymous usage data"), (false, "Don't share")];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstTimeStep {
    Theme,
    Analytics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstTimeSetupResult {
    pub theme: String,
    pub share_analytics: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstTimeAction {
    None,
    PreviewTheme(String),
    Submit(FirstTimeSetupResult),
    Cancel,
}

#[derive(Debug, Clone)]
pub struct FirstTimeSetup {
    pub step: FirstTimeStep,
    pub theme_index: usize,
    pub analytics_index: usize,
    pub detected_theme: String,
    pub app_name: String,
}

impl FirstTimeSetup {
    pub fn new(detected_theme: impl Into<String>, app_name: impl Into<String>) -> Self {
        let detected_theme = detected_theme.into();
        let theme_index = THEME_OPTIONS
            .iter()
            .position(|(value, _)| *value == detected_theme)
            .unwrap_or(0);
        Self {
            step: FirstTimeStep::Theme,
            theme_index,
            analytics_index: 0,
            detected_theme,
            app_name: app_name.into(),
        }
    }

    pub fn welcome_line(&self) -> String {
        format!("Welcome to {}, the minimal coding agent.", self.app_name)
    }

    pub fn selected_theme(&self) -> &'static str {
        THEME_OPTIONS[self.theme_index].0
    }

    pub fn selected_analytics(&self) -> bool {
        ANALYTICS_OPTIONS[self.analytics_index].0
    }

    pub fn handle_key(&mut self, data: &str) -> FirstTimeAction {
        match data {
            "k" | "\x1b[A" => {
                self.move_selection(-1);
                if self.step == FirstTimeStep::Theme {
                    FirstTimeAction::PreviewTheme(self.selected_theme().to_string())
                } else {
                    FirstTimeAction::None
                }
            }
            "j" | "\x1b[B" => {
                self.move_selection(1);
                if self.step == FirstTimeStep::Theme {
                    FirstTimeAction::PreviewTheme(self.selected_theme().to_string())
                } else {
                    FirstTimeAction::None
                }
            }
            "\r" | "\n" => {
                if self.step == FirstTimeStep::Theme {
                    self.step = FirstTimeStep::Analytics;
                    FirstTimeAction::None
                } else {
                    FirstTimeAction::Submit(FirstTimeSetupResult {
                        theme: self.selected_theme().to_string(),
                        share_analytics: self.selected_analytics(),
                    })
                }
            }
            "\x1b" => FirstTimeAction::Cancel,
            _ => FirstTimeAction::None,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.step == FirstTimeStep::Theme {
            let next = (self.theme_index as isize + delta)
                .clamp(0, THEME_OPTIONS.len() as isize - 1) as usize;
            self.theme_index = next;
        } else {
            let next = (self.analytics_index as isize + delta)
                .clamp(0, ANALYTICS_OPTIONS.len() as isize - 1) as usize;
            self.analytics_index = next;
        }
    }

    fn option_lines(&self) -> Vec<String> {
        let (labels, selected) = match self.step {
            FirstTimeStep::Theme => (
                THEME_OPTIONS
                    .iter()
                    .map(|(_, label)| (*label).to_string())
                    .collect::<Vec<_>>(),
                self.theme_index,
            ),
            FirstTimeStep::Analytics => (
                ANALYTICS_OPTIONS
                    .iter()
                    .map(|(_, label)| (*label).to_string())
                    .collect::<Vec<_>>(),
                self.analytics_index,
            ),
        };
        labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                if index == selected {
                    format!("→ {label}")
                } else {
                    format!("  {label}")
                }
            })
            .collect()
    }
}

impl Component for FirstTimeSetup {
    fn render(&self, _width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        lines.extend(SETUP_LOGO_LINES.iter().map(|line| (*line).to_string()));
        lines.push(String::new());
        lines.push(self.welcome_line());
        lines.push(String::new());
        match self.step {
            FirstTimeStep::Theme => {
                lines.push("Pick a theme.".into());
                lines.push(format!(
                    "Detected system appearance: {}",
                    self.detected_theme
                ));
            }
            FirstTimeStep::Analytics => {
                lines.push("Opt-in to anonymous usage data sharing?".into());
                lines.push(
                    "Opting in stores a tracking identifier in settings.json and enables anonymous"
                        .into(),
                );
                lines.push(
                    "usage analytics. This helps us to better debug, reproduce, and resolve issues"
                        .into(),
                );
                lines.push(
                    "and bugs within Pi. You can observe what is shared using /privacy and make"
                        .into(),
                );
                lines.push("changes anytime in settings.json.".into());
            }
        }
        lines.push(String::new());
        lines.extend(self.option_lines());
        lines.push(String::new());
        let confirm = if self.step == FirstTimeStep::Theme {
            "continue"
        } else {
            "finish"
        };
        lines.push(format!("↑↓ navigate  enter {confirm}  escape skip setup"));
        lines
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

pub fn detect_terminal_theme(theme: &Theme) -> String {
    if theme.name == "light" {
        "light".into()
    } else {
        "dark".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wizard_matches_ts_copy_and_steps() {
        let mut setup = FirstTimeSetup::new("dark", "pi");
        let rendered = setup.render(80).join("\n");
        assert!(rendered.contains("██████"));
        assert!(rendered.contains("Welcome to pi, the minimal coding agent."));
        assert!(rendered.contains("Pick a theme."));
        assert!(rendered.contains("Detected system appearance: dark"));
        assert!(rendered.contains("→ Dark"));
        assert!(rendered.contains("  Light"));
        assert!(rendered.contains("continue"));
        assert!(rendered.contains("skip setup"));
        assert_eq!(
            setup.handle_key("j"),
            FirstTimeAction::PreviewTheme("light".into())
        );
        assert_eq!(setup.handle_key("\r"), FirstTimeAction::None);
        assert_eq!(setup.step, FirstTimeStep::Analytics);
        let analytics = setup.render(80).join("\n");
        assert!(analytics.contains("Opt-in to anonymous usage data sharing?"));
        assert!(analytics.contains("Share anonymous usage data"));
        assert!(analytics.contains("Don't share"));
        assert!(analytics.contains("finish"));
        assert_eq!(
            setup.handle_key("\r"),
            FirstTimeAction::Submit(FirstTimeSetupResult {
                theme: "light".into(),
                share_analytics: true,
            })
        );
        let mut cancelled = FirstTimeSetup::new("light", "pi");
        assert_eq!(cancelled.handle_key("\x1b"), FirstTimeAction::Cancel);
    }
}
