use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFile {
    pub path: PathBuf,
    pub name: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPriority {
    Mandatory,
    Important,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextContribution {
    pub source: String,
    pub stable: bool,
    pub priority: ContextPriority,
    pub estimated_tokens: u64,
    pub content_hash: String,
    pub included: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBudgetReport {
    pub contributions: Vec<ContextContribution>,
    pub total_estimated_tokens: u64,
}

pub(crate) fn append_repository_context(prompt: &mut String, files: &[ContextFile]) {
    if files.is_empty() {
        return;
    }

    let mut unique_bodies: Vec<(&str, Vec<String>)> = Vec::new();
    for file in files {
        let path = file.path.to_string_lossy().into_owned();
        if let Some((_, paths)) = unique_bodies
            .iter_mut()
            .find(|(body, _)| *body == file.body)
        {
            paths.push(path);
        } else {
            unique_bodies.push((&file.body, vec![path]));
        }
    }

    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }
    prompt.push_str("<project_context>\n\nProject-specific instructions and guidelines:\n\n");
    for (body, paths) in unique_bodies {
        prompt.push_str("<project_instructions paths=");
        prompt.push_str(&serde_json::to_string(&paths).expect("context paths are JSON"));
        prompt.push_str(">\n");
        prompt.push_str(body);
        prompt.push_str("\n</project_instructions>\n\n");
    }
    prompt.push_str("</project_context>");
}

#[derive(Debug, Clone, Default)]
pub struct RootContextAccount {
    entries: Vec<RootContextEntry>,
}

#[derive(Debug, Clone)]
struct RootContextEntry {
    contribution: ContextContribution,
    body: String,
}

impl RootContextAccount {
    pub fn add(
        &mut self,
        source: impl Into<String>,
        body: impl Into<String>,
        stable: bool,
        priority: ContextPriority,
    ) {
        let body = body.into();
        let content_hash = crate::prompt::manifest::hash_text(&body);
        let duplicate = self.entries.iter().any(|entry| entry.body == body);
        let estimated_tokens = if duplicate {
            0
        } else {
            crate::prompt::manifest::estimate_tokens_from_str(&body) as u64
        };
        self.entries.push(RootContextEntry {
            contribution: ContextContribution {
                source: source.into(),
                stable,
                priority,
                estimated_tokens,
                content_hash,
                included: !duplicate,
                selected: true,
            },
            body,
        });
    }

    pub fn report(&self) -> ContextBudgetReport {
        self.report_for_budget(u64::MAX)
    }

    pub fn report_for_budget(&self, budget: u64) -> ContextBudgetReport {
        let mut selected_by_body = HashMap::new();
        let mut used = 0u64;

        for priority in [
            ContextPriority::Mandatory,
            ContextPriority::Important,
            ContextPriority::Deferred,
        ] {
            for entry in &self.entries {
                if entry.contribution.priority != priority
                    || selected_by_body.contains_key(&entry.body)
                {
                    continue;
                }
                let estimated_tokens = self
                    .entries
                    .iter()
                    .find(|candidate| candidate.body == entry.body)
                    .map(|candidate| candidate.contribution.estimated_tokens)
                    .unwrap_or(0);
                let selected = priority == ContextPriority::Mandatory
                    || used.saturating_add(estimated_tokens) <= budget;
                if selected {
                    used = used.saturating_add(estimated_tokens);
                }
                selected_by_body.insert(entry.body.clone(), selected);
            }
        }

        let contributions = self
            .entries
            .iter()
            .map(|entry| {
                let mut contribution = entry.contribution.clone();
                contribution.selected = selected_by_body.get(&entry.body).copied().unwrap_or(true);
                contribution
            })
            .collect::<Vec<_>>();
        let total_estimated_tokens = contributions
            .iter()
            .filter(|contribution| contribution.selected)
            .map(|contribution| contribution.estimated_tokens)
            .sum();

        ContextBudgetReport {
            contributions,
            total_estimated_tokens,
        }
    }

    /// Stable prompt identity remains independent from dynamic runtime state.
    pub fn stable_prefix_hash(&self) -> String {
        let mut stable_bodies = Vec::new();
        for entry in &self.entries {
            if entry.contribution.stable && !stable_bodies.contains(&entry.body.as_str()) {
                stable_bodies.push(entry.body.as_str());
            }
        }
        let stable_body = stable_bodies.join("\n\n");
        crate::prompt::manifest::hash_text(&stable_body)
    }
}

pub fn load_context_files(cwd: &Path, enabled: bool) -> Vec<ContextFile> {
    if !enabled {
        return Vec::new();
    }
    let mut files = Vec::new();
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let path = cwd.join(name);
        if let Ok(body) = fs::read_to_string(&path) {
            files.push(ContextFile {
                path,
                name: name.to_string(),
                body,
            });
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_root_context_content_is_included_once() {
        let mut account = RootContextAccount::default();
        account.add(
            "AGENTS.md",
            "same instructions",
            true,
            ContextPriority::Mandatory,
        );
        account.add(
            "CLAUDE.md",
            "same instructions",
            true,
            ContextPriority::Mandatory,
        );

        let report = account.report();
        assert_eq!(report.contributions.len(), 2);
        assert_eq!(report.contributions[0].estimated_tokens, 5);
        assert_eq!(report.contributions[1].estimated_tokens, 0);
        assert!(report.contributions.iter().all(|entry| entry.selected));
        assert_eq!(report.total_estimated_tokens, 5);
        assert_ne!(
            report.contributions[0].source,
            report.contributions[1].source
        );
        assert_eq!(
            report.contributions[0].content_hash,
            report.contributions[1].content_hash
        );
    }

    #[test]
    fn repository_prompt_retains_duplicate_provenance_without_duplicate_body() {
        let mut prompt = "base".to_string();
        append_repository_context(
            &mut prompt,
            &[
                ContextFile {
                    path: PathBuf::from("AGENTS.md"),
                    name: "AGENTS.md".into(),
                    body: "same instructions".into(),
                },
                ContextFile {
                    path: PathBuf::from("CLAUDE.md"),
                    name: "CLAUDE.md".into(),
                    body: "same instructions".into(),
                },
            ],
        );

        assert_eq!(prompt.matches("same instructions").count(), 1);
        assert!(prompt.contains("AGENTS.md"));
        assert!(prompt.contains("CLAUDE.md"));
    }

    #[test]
    fn duplicate_stable_context_does_not_change_stable_prefix_hash() {
        let mut account = RootContextAccount::default();
        account.add(
            "AGENTS.md",
            "same instructions",
            true,
            ContextPriority::Mandatory,
        );
        let before = account.stable_prefix_hash();

        account.add(
            "CLAUDE.md",
            "same instructions",
            true,
            ContextPriority::Mandatory,
        );

        assert_eq!(before, account.stable_prefix_hash());
    }

    #[test]
    fn mandatory_duplicate_is_not_dropped_when_deferred_copy_precedes_it() {
        let mut account = RootContextAccount::default();
        account.add(
            "optional_memory",
            "shared authority",
            false,
            ContextPriority::Deferred,
        );
        account.add(
            "AGENTS.md",
            "shared authority",
            true,
            ContextPriority::Mandatory,
        );

        let report = account.report_for_budget(0);
        assert!(report.contributions.iter().all(|entry| entry.selected));
        assert!(report.total_estimated_tokens > 0);
    }

    #[test]
    fn stable_duplicate_after_dynamic_copy_changes_stable_prefix_hash() {
        let mut account = RootContextAccount::default();
        account.add(
            "runtime_state",
            "shared authority",
            false,
            ContextPriority::Important,
        );
        let before = account.stable_prefix_hash();

        account.add(
            "AGENTS.md",
            "shared authority",
            true,
            ContextPriority::Mandatory,
        );

        assert_ne!(before, account.stable_prefix_hash());
    }

    #[test]
    fn mandatory_root_context_is_never_dropped() {
        let mut account = RootContextAccount::default();
        account.add(
            "system",
            "mandatory authority",
            true,
            ContextPriority::Mandatory,
        );
        account.add(
            "memory",
            "optional context that does not fit",
            false,
            ContextPriority::Important,
        );

        let report = account.report_for_budget(1);
        assert!(report
            .contributions
            .iter()
            .find(|entry| entry.source == "system")
            .is_some_and(|entry| entry.selected));
        assert!(report.total_estimated_tokens >= 4);
    }

    #[test]
    fn dynamic_state_does_not_change_stable_prefix_hash() {
        let mut account = RootContextAccount::default();
        account.add(
            "stable_prompt",
            "stable authority",
            true,
            ContextPriority::Mandatory,
        );
        let before = account.stable_prefix_hash();
        account.add(
            "runtime_state",
            "active turn 1",
            false,
            ContextPriority::Important,
        );
        assert_eq!(before, account.stable_prefix_hash());

        account.add(
            "stable_prompt",
            "changed authority",
            true,
            ContextPriority::Mandatory,
        );
        assert_ne!(before, account.stable_prefix_hash());
    }
}
