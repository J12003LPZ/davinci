//! First-time setup dialog matching TS `first-time-setup.ts`.

use crate::render::Component;
use crate::themes::Theme;

pub const SETUP_LOGO_LINES: &[&str] = &["██████", "██  ██", "████  ██", "██    ██"];

pub const THEME_OPTIONS: &[(&str, &str)] = &[
    ("dark", "Dark"),
    ("light", "Light"),
    ("vox", "Vox — Editorial collage"),
];
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
}

impl Component for FirstTimeSetup {
    fn render(&self, width: usize) -> Vec<String> {
        let mut section = crate::render::CommandSection::new(
            width,
            &format!("Welcome to {}", self.app_name),
            None,
        );
        section.detail(&self.welcome_line());
        match self.step {
            FirstTimeStep::Theme => {
                section.detail(&format!(
                    "Choose a theme · detected: {}",
                    self.detected_theme
                ));
                for (index, (_, label)) in THEME_OPTIONS.iter().enumerate() {
                    section.item(
                        index == self.theme_index,
                        label,
                        if index == self.theme_index {
                            "selected"
                        } else {
                            ""
                        },
                    );
                }
                section.hint("↑↓ move · enter continue · esc skip setup");
            }
            FirstTimeStep::Analytics => {
                section.detail("Anonymous usage data helps diagnose and reproduce product issues. You can change this later in settings.");
                for (index, (_, label)) in ANALYTICS_OPTIONS.iter().enumerate() {
                    section.item(
                        index == self.analytics_index,
                        label,
                        if index == self.analytics_index {
                            "selected"
                        } else {
                            ""
                        },
                    );
                }
                section.hint("↑↓ move · enter finish · esc skip setup");
            }
        }
        section.finish()
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

pub fn detect_terminal_theme(theme: &Theme) -> String {
    detect_terminal_theme_auto(Some(&theme.name))
}

pub fn detect_terminal_theme_auto(fallback_name: Option<&str>) -> String {
    let colorfgbg = std::env::var("COLORFGBG").ok();
    let osc11 = std::env::var("PI_OSC11_REPLY").ok();
    let scheme = std::env::var("PI_COLOR_SCHEME_REPLY").ok();
    let detected = crate::osc::detect_terminal_theme_for_auto(
        scheme.as_deref(),
        osc11.as_deref(),
        colorfgbg.as_deref(),
    );
    if detected.source != "fallback" {
        return detected.theme;
    }
    match fallback_name {
        Some("light") => "light".into(),
        _ => "dark".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_uses_terminal_native_sections_without_the_block_logo() {
        let setup = FirstTimeSetup::new("dark", "pi");
        let rendered = setup.render(32).join("\n");
        assert!(!rendered.contains("██████"), "{rendered}");
        assert!(
            rendered.contains(crate::davinci::ui::SELECTION_BAR.trim()),
            "{rendered}"
        );
        for width in [0, 1, 20, 32, 40] {
            for row in setup.render(width) {
                assert!(
                    crate::render::visible_width_stripped(&row) <= width,
                    "{width}: {row:?}"
                );
            }
        }
    }

    #[test]
    fn wizard_matches_ts_copy_and_steps() {
        let mut setup = FirstTimeSetup::new("dark", "pi");
        let rendered = setup.render(80).join("\n");
        assert!(rendered.contains("Welcome to pi, the minimal coding agent."));
        assert!(rendered.contains("Choose a theme"));
        assert!(rendered.contains("detected: dark"));
        assert!(rendered.contains("Dark"));
        assert!(rendered.contains("Light"));
        assert!(rendered.contains("continue"));
        assert!(rendered.contains("skip setup"));
        assert_eq!(
            setup.handle_key("j"),
            FirstTimeAction::PreviewTheme("light".into())
        );
        assert_eq!(setup.handle_key("\r"), FirstTimeAction::None);
        assert_eq!(setup.step, FirstTimeStep::Analytics);
        let analytics = setup.render(80).join("\n");
        assert!(analytics.contains("Anonymous usage data"));
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

    #[test]
    fn auto_theme_prefers_osc_and_colorfgbg() {
        std::env::remove_var("PI_COLOR_SCHEME_REPLY");
        std::env::remove_var("PI_OSC11_REPLY");
        std::env::set_var("COLORFGBG", "0;15");
        assert_eq!(detect_terminal_theme_auto(Some("dark")), "light");
        std::env::set_var("PI_OSC11_REPLY", "\x1b]11;#000000\x07");
        assert_eq!(detect_terminal_theme_auto(Some("light")), "dark");
        std::env::set_var("PI_COLOR_SCHEME_REPLY", "\x1b[?997;2n");
        assert_eq!(detect_terminal_theme_auto(Some("dark")), "light");
        std::env::remove_var("COLORFGBG");
        std::env::remove_var("PI_OSC11_REPLY");
        std::env::remove_var("PI_COLOR_SCHEME_REPLY");
    }
}
