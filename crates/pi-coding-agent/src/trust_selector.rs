//! Interactive project-trust selector matching TypeScript `TrustSelectorComponent`.

use crate::settings::{canonicalize_path, project_trust_options, trust_entry, ProjectTrustOption};
use pi_tui::keys::Key;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustSelectorAction {
    Continue,
    Selected(ProjectTrustOption),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct TrustSelector {
    cwd: PathBuf,
    options: Vec<ProjectTrustOption>,
    selected: usize,
    saved: Option<(PathBuf, bool)>,
    project_trusted: bool,
}

fn format_decision(trust_path: Option<&Path>, decision: Option<&(PathBuf, bool)>) -> String {
    let Some((path, trusted)) = decision else {
        return "none".into();
    };
    let label = if *trusted { "trusted" } else { "untrusted" };
    if let Some(trust_path) = trust_path {
        if path != trust_path {
            return format!("{label} (inherited from {})", path.display());
        }
    }
    format!("{label} ({})", path.display())
}

impl TrustSelector {
    pub fn new(cwd: &Path, agent_dir: &Path, project_trusted: bool) -> Self {
        let options = project_trust_options(cwd, false);
        let saved = trust_entry(agent_dir, cwd);
        let selected = options
            .iter()
            .position(|option| is_saved_option(option, saved.as_ref()))
            .unwrap_or(0);
        Self {
            cwd: canonicalize_path(cwd),
            options,
            selected,
            saved,
            project_trusted,
        }
    }

    pub fn render(&self) -> String {
        let trust_path = self
            .options
            .first()
            .and_then(|option| option.saved_path.as_deref());
        let mut lines = vec![
            "─".repeat(40),
            "Project trust".into(),
            self.cwd.display().to_string(),
            String::new(),
            format!(
                "Saved decision: {}",
                format_decision(trust_path, self.saved.as_ref())
            ),
            format!(
                "Current session: {}",
                if self.project_trusted {
                    "trusted"
                } else {
                    "untrusted"
                }
            ),
            String::new(),
        ];
        for (index, option) in self.options.iter().enumerate() {
            let check = if is_saved_option(option, self.saved.as_ref()) {
                " ✓"
            } else {
                ""
            };
            let prefix = if index == self.selected { "→ " } else { "  " };
            lines.push(format!("{prefix}{}{check}", option.label));
        }
        lines.push(String::new());
        lines.push("↑↓ navigate  enter save  escape cancel".into());
        lines.push("─".repeat(40));
        lines.join("\n") + "\n"
    }

    pub fn handle_key(&mut self, key: &Key) -> TrustSelectorAction {
        match key {
            Key::Up | Key::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                TrustSelectorAction::Continue
            }
            Key::Down | Key::Char('j') => {
                if self.selected + 1 < self.options.len() {
                    self.selected += 1;
                }
                TrustSelectorAction::Continue
            }
            Key::Enter => self
                .options
                .get(self.selected)
                .cloned()
                .map(TrustSelectorAction::Selected)
                .unwrap_or(TrustSelectorAction::Continue),
            Key::Escape => TrustSelectorAction::Cancelled,
            _ => TrustSelectorAction::Continue,
        }
    }
}

fn is_saved_option(option: &ProjectTrustOption, saved: Option<&(PathBuf, bool)>) -> bool {
    let Some(saved_path) = option.saved_path.as_ref() else {
        return false;
    };
    saved.is_some_and(|(path, trusted)| path == saved_path && *trusted == option.trusted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{self, set_trust};
    use tempfile::tempdir;

    #[test]
    fn renders_typescript_trust_selector_chrome() {
        let _lock = settings::test_env_lock();
        let dir = tempdir().unwrap();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        let cwd = dir.path().join("proj");
        std::fs::create_dir_all(&cwd).unwrap();
        let selector = TrustSelector::new(&cwd, dir.path(), false);
        let rendered = selector.render();
        assert!(rendered.contains("Project trust"));
        assert!(rendered.contains("Saved decision: none"));
        assert!(rendered.contains("Current session: untrusted"));
        assert!(rendered.contains("→ Trust"));
        assert!(rendered.contains("Do not trust"));
        assert!(rendered.contains("↑↓ navigate  enter save  escape cancel"));
        std::env::remove_var("PI_CODING_AGENT_DIR");
    }

    #[test]
    fn arrow_keys_save_and_show_checkmark() {
        let _lock = settings::test_env_lock();
        let dir = tempdir().unwrap();
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        let cwd = dir.path().join("proj");
        std::fs::create_dir_all(&cwd).unwrap();
        set_trust(dir.path(), &cwd, Some(true));
        let mut selector = TrustSelector::new(&cwd, dir.path(), true);
        let rendered = selector.render();
        assert!(rendered.contains("→ Trust ✓"));
        assert!(rendered.contains("Current session: trusted"));
        assert!(selector.handle_key(&Key::Down) == TrustSelectorAction::Continue);
        match selector.handle_key(&Key::Enter) {
            TrustSelectorAction::Selected(option) => {
                assert!(option.label.starts_with("Trust parent folder"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            selector.handle_key(&Key::Escape),
            TrustSelectorAction::Cancelled
        );
        std::env::remove_var("PI_CODING_AGENT_DIR");
    }
}
