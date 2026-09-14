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
        let duplicate = self
            .entries
            .iter()
            .any(|entry| entry.contribution.content_hash == content_hash);
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
        let mut selected_by_hash = HashMap::new();
        let mut used = 0u64;

        for priority in [ContextPriority::Mandatory, ContextPriority::Important] {
            for entry in &self.entries {
                if !entry.contribution.included || entry.contribution.priority != priority {
                    continue;
                }
                let selected = priority == ContextPriority::Mandatory
                    || used.saturating_add(entry.contribution.estimated_tokens) <= budget;
                if selected {
                    used = used.saturating_add(entry.contribution.estimated_tokens);
                }
                selected_by_hash.insert(entry.contribution.content_hash.clone(), selected);
            }
        }

        for entry in &self.entries {
            if !entry.contribution.included
                || entry.contribution.priority != ContextPriority::Deferred
            {
                continue;
            }
            let selected = used.saturating_add(entry.contribution.estimated_tokens) <= budget;
            if selected {
                used = used.saturating_add(entry.contribution.estimated_tokens);
            }
            selected_by_hash.insert(entry.contribution.content_hash.clone(), selected);
        }

        let contributions = self
            .entries
            .iter()
            .map(|entry| {
                let mut contribution = entry.contribution.clone();
                contribution.selected = selected_by_hash
                    .get(&contribution.content_hash)
                    .copied()
                    .unwrap_or(true);
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
        let stable_body = self
            .entries
            .iter()
            .filter(|entry| entry.contribution.stable)
            .map(|entry| entry.body.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
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
