//! Package resource selector matching TS `ConfigSelectorComponent`.

use crate::keys::parse_key;
use crate::render::Component;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    User,
    Project,
}

impl ConfigScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Project => "Project",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::User => Self::Project,
            Self::Project => Self::User,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigResourceKind {
    Extensions,
    Skills,
    Prompts,
    Themes,
}

impl ConfigResourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Extensions => "Extensions",
            Self::Skills => "Skills",
            Self::Prompts => "Prompts",
            Self::Themes => "Themes",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigResource {
    pub kind: ConfigResourceKind,
    pub name: String,
    pub source: String,
    pub enabled: bool,
    pub scope: ConfigScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSelectorAction {
    None,
    Toggle,
    Close,
    Changed,
}

#[derive(Debug, Clone)]
pub struct ConfigSelector {
    pub scope: ConfigScope,
    pub items: Vec<ConfigResource>,
    pub selected: usize,
}

impl ConfigSelector {
    pub fn new(items: Vec<ConfigResource>) -> Self {
        Self {
            scope: ConfigScope::User,
            items,
            selected: 0,
        }
    }

    pub fn visible(&self) -> Vec<&ConfigResource> {
        self.items
            .iter()
            .filter(|item| item.scope == self.scope)
            .collect()
    }

    pub fn selected_item(&self) -> Option<ConfigResource> {
        self.visible().get(self.selected).cloned().cloned()
    }

    pub fn handle_key(&mut self, data: &str) -> ConfigSelectorAction {
        if data == "\t" {
            self.scope = self.scope.toggle();
            self.selected = 0;
            return ConfigSelectorAction::Changed;
        }
        if data == "\u{1b}" || data == "escape" {
            return ConfigSelectorAction::Close;
        }
        if data == "\r" || data == "\n" || data == " " {
            return self.toggle_selected();
        }
        let key = parse_key(data);
        match key.name.as_str() {
            "up" | "k" => self.move_by(-1),
            "down" | "j" => self.move_by(1),
            "enter" | "space" => return self.toggle_selected(),
            "escape" => return ConfigSelectorAction::Close,
            _ => {}
        }
        ConfigSelectorAction::None
    }

    fn toggle_selected(&mut self) -> ConfigSelectorAction {
        let scope = self.scope;
        let selected = self.selected;
        let mut index = 0;
        for item in &mut self.items {
            if item.scope != scope {
                continue;
            }
            if index == selected {
                item.enabled = !item.enabled;
                return ConfigSelectorAction::Toggle;
            }
            index += 1;
        }
        ConfigSelectorAction::None
    }

    fn move_by(&mut self, delta: isize) {
        let len = self.visible().len() as isize;
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }
}

impl Component for ConfigSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![
            "Package resources".into(),
            format!(
                "Scope: {}  (Tab switches User / Project)",
                self.scope.label()
            ),
            String::new(),
        ];
        let visible = self.visible();
        if visible.is_empty() {
            lines.push("  No resources in this scope.".into());
        } else {
            let mut last_kind = None;
            for (index, item) in visible.iter().enumerate() {
                if last_kind != Some(item.kind) {
                    lines.push(item.kind.label().to_string());
                    last_kind = Some(item.kind);
                }
                let mark = if item.enabled { "[x]" } else { "[ ]" };
                let prefix = if index == self.selected { "> " } else { "  " };
                let mut line = format!("{prefix}{mark} {}  ({})", item.name, item.source);
                if crate::render::visible_width(&line) > width {
                    line.truncate(width.max(1));
                }
                lines.push(line);
            }
        }
        lines.push(String::new());
        lines.push("Space/Enter toggle   Esc close".into());
        lines
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
    fn tab_switches_scope_and_toggle_persists() {
        let mut selector = ConfigSelector::new(vec![
            ConfigResource {
                kind: ConfigResourceKind::Extensions,
                name: "demo".into(),
                source: "./demo".into(),
                enabled: true,
                scope: ConfigScope::User,
            },
            ConfigResource {
                kind: ConfigResourceKind::Skills,
                name: "review".into(),
                source: ".pi/skills/review".into(),
                enabled: false,
                scope: ConfigScope::Project,
            },
        ]);
        let lines = selector.render(80);
        assert!(lines.iter().any(|line| line.contains("Scope: User")));
        assert!(lines.iter().any(|line| line.contains("[x] demo")));
        assert_eq!(selector.handle_key("\t"), ConfigSelectorAction::Changed);
        assert_eq!(selector.scope, ConfigScope::Project);
        assert!(selector
            .render(80)
            .iter()
            .any(|line| line.contains("[ ] review")));
        assert_eq!(selector.handle_key(" "), ConfigSelectorAction::Toggle);
        assert!(selector.items[1].enabled);
    }
}
