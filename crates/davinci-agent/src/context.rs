use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use davinci_ai::ChatMessage;

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

#[derive(Debug, Clone)]
pub struct SelectedRootContext {
    pub report: ContextBudgetReport,
    pub repository_files: Vec<ContextFile>,
    pub ephemeral_messages: Vec<ChatMessage>,
}

pub(crate) fn append_repository_context(prompt: &mut String, files: &[ContextFile], cwd: &Path) {
    if files.is_empty() {
        return;
    }

    // Target-specific loaders canonicalize nested paths. Resolve the root
    // once, only when lexical stripping cannot preserve their relative scope.
    let canonical_cwd = files
        .iter()
        .any(|file| file.path.strip_prefix(cwd).is_err())
        .then(|| fs::canonicalize(cwd).ok())
        .flatten();
    let mut unique_bodies: Vec<(&str, Vec<String>)> = Vec::new();
    for file in files {
        // Preserve nested instruction scope without making the checkout root
        // part of the reusable provider prefix.
        let path = file
            .path
            .strip_prefix(cwd)
            .ok()
            .or_else(|| {
                canonical_cwd
                    .as_ref()
                    .and_then(|root| file.path.strip_prefix(root).ok())
            })
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| file.name.clone());
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

/// The repository instruction file davinci reads. `CLAUDE.md` is Claude
/// Code's: its rules name Claude Code's own tools and rituals, and a large one
/// cost every request thousands of tokens davinci could not act on.
pub const INSTRUCTION_FILE: &str = "AGENTS.md";

pub fn load_context_files(cwd: &Path, enabled: bool) -> Vec<ContextFile> {
    if !enabled {
        return Vec::new();
    }
    let path = cwd.join(INSTRUCTION_FILE);
    fs::read_to_string(&path)
        .map(|body| ContextFile {
            path,
            name: INSTRUCTION_FILE.to_string(),
            body,
        })
        .into_iter()
        .collect()
}

pub fn load_context_files_for_targets(
    cwd: &Path,
    enabled: bool,
    targets: &[PathBuf],
) -> Vec<ContextFile> {
    if !enabled {
        return Vec::new();
    }

    let root = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut files = load_context_files(cwd, true);
    let mut seen = HashSet::new();
    for file in &files {
        seen.insert(fs::canonicalize(&file.path).unwrap_or_else(|_| file.path.clone()));
    }

    for target in targets {
        let absolute = if target.is_absolute() {
            target.clone()
        } else {
            cwd.join(target)
        };
        let start = if absolute.is_dir() {
            absolute
        } else {
            absolute.parent().unwrap_or(cwd).to_path_buf()
        };
        let start = fs::canonicalize(&start).unwrap_or(start);
        if !start.starts_with(&root) {
            continue;
        }
        let mut ancestors = start
            .ancestors()
            .take_while(|path| path.starts_with(&root))
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        ancestors.reverse();
        for directory in ancestors {
            let path = directory.join(INSTRUCTION_FILE);
            if !path.is_file() {
                continue;
            }
            let identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !seen.insert(identity) {
                continue;
            }
            if let Ok(body) = fs::read_to_string(&path) {
                files.push(ContextFile {
                    path,
                    name: INSTRUCTION_FILE.to_string(),
                    body,
                });
            }
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
    fn checkout_roots_do_not_change_instruction_prefix_or_cache_identity() {
        let prompt_at = |root: &str, body: &str| {
            let mut prompt = "stable system".to_owned();
            append_repository_context(
                &mut prompt,
                &[ContextFile {
                    path: PathBuf::from(root).join("AGENTS.md"),
                    name: "AGENTS.md".into(),
                    body: body.into(),
                }],
                Path::new(root),
            );
            prompt
        };
        let first = prompt_at("/tmp/checkout-a", "same instructions");
        let second = prompt_at("/work/checkout-b", "same instructions");
        assert_eq!(first, second);
        assert!(!first.contains("/tmp/checkout-a"));
        assert_eq!(
            crate::runtime::hash_system_prompt_with_manifest(&first, None),
            crate::runtime::hash_system_prompt_with_manifest(&second, None)
        );
        assert_ne!(
            first,
            prompt_at("/work/checkout-b", "different instructions")
        );
    }

    #[test]
    fn stable_instruction_prefix_retains_nested_scope() {
        let mut prompt = String::new();
        append_repository_context(
            &mut prompt,
            &[ContextFile {
                path: PathBuf::from("/checkout/src/AGENTS.md"),
                name: "AGENTS.md".into(),
                body: "instructions for src".into(),
            }],
            Path::new("/checkout"),
        );
        assert!(prompt.contains("src/AGENTS.md"));
        assert!(!prompt.contains("/checkout"));
    }

    #[test]
    fn only_agents_md_is_read_at_the_root_and_beside_targets() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(root.path().join("AGENTS.md"), "davinci rules").unwrap();
        fs::write(root.path().join("CLAUDE.md"), "claude code rules").unwrap();
        fs::write(root.path().join("src/CLAUDE.md"), "nested claude rules").unwrap();
        fs::write(root.path().join("src/file.rs"), "").unwrap();

        let files =
            load_context_files_for_targets(root.path(), true, &[PathBuf::from("src/file.rs")]);
        let bodies: Vec<&str> = files.iter().map(|file| file.body.as_str()).collect();
        assert_eq!(bodies, ["davinci rules"]);

        // A directory with CLAUDE.md alone gives davinci no instructions.
        fs::remove_file(root.path().join("AGENTS.md")).unwrap();
        assert!(load_context_files(root.path(), true).is_empty());
    }

    #[test]
    fn relative_cwd_loader_preserves_nested_instruction_scope() {
        let current = std::env::current_dir().unwrap();
        let root = tempfile::tempdir_in(&current).unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(root.path().join("AGENTS.md"), "root instructions").unwrap();
        fs::write(root.path().join("src/AGENTS.md"), "nested instructions").unwrap();
        fs::write(root.path().join("src/file.rs"), "").unwrap();
        let cwd = root.path().strip_prefix(&current).unwrap();
        let files = load_context_files_for_targets(cwd, true, &[PathBuf::from("src/file.rs")]);
        let mut prompt = String::new();
        append_repository_context(&mut prompt, &files, cwd);
        assert!(prompt.contains("src/AGENTS.md"), "{prompt}");
        assert!(prompt.contains("root instructions") && prompt.contains("nested instructions"));
        assert!(!prompt.contains(root.path().to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cwd_loader_preserves_nested_instruction_scope() {
        let root = tempfile::tempdir().unwrap();
        let checkout = root.path().join("checkout");
        fs::create_dir_all(checkout.join("src")).unwrap();
        fs::write(checkout.join("src/AGENTS.md"), "nested instructions").unwrap();
        fs::write(checkout.join("src/file.rs"), "").unwrap();
        let link = root.path().join("linked-checkout");
        std::os::unix::fs::symlink(&checkout, &link).unwrap();
        let files = load_context_files_for_targets(&link, true, &[PathBuf::from("src/file.rs")]);
        let mut prompt = String::new();
        append_repository_context(&mut prompt, &files, &link);
        assert!(prompt.contains("src/AGENTS.md"), "{prompt}");
        assert!(!prompt.contains(root.path().to_str().unwrap()));
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
            Path::new("."),
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
