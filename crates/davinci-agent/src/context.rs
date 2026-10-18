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
    // A file above `cwd` (the repository root's, in a subdirectory session)
    // is named from the root, so it never reads as `cwd`'s own.
    let repository = canonical_cwd.as_ref().and_then(|_| repository_root(cwd));
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
            .or_else(|| {
                let relative = file.path.strip_prefix(repository.as_ref()?).ok()?;
                Some(format!(
                    "<repository root>/{}",
                    relative.to_string_lossy().replace('\\', "/")
                ))
            })
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

/// Tokens each file adds to the request, in file order. The prompt sends one
/// copy of identical bodies (see `append_repository_context`), so a repeat
/// costs nothing; `/context` and the `/reload` sheet must not charge it.
pub fn context_file_charges(files: &[ContextFile]) -> Vec<u64> {
    let mut seen: HashSet<&str> = HashSet::new();
    files
        .iter()
        .map(|file| {
            if seen.insert(file.body.as_str()) {
                (file.body.len() as u64).div_ceil(4)
            } else {
                0
            }
        })
        .collect()
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

/// The checkout `cwd` is in: the nearest directory, `cwd` included, holding a
/// `.git` entry (a directory, or a file in a linked worktree or submodule).
/// `None` outside any repository.
pub fn repository_root(cwd: &Path) -> Option<PathBuf> {
    let cwd = fs::canonicalize(cwd).ok()?;
    cwd.ancestors()
        .find(|directory| directory.join(".git").exists())
        .map(Path::to_path_buf)
}

/// `AGENTS.md` from the repository root down to `cwd`, outermost first, so a
/// nested file reads after the rules it refines. Outside a repository there
/// is no codebase to describe and nothing is loaded: a session started in a
/// home or Desktop folder must not pick up whatever instructions sit there.
pub fn load_context_files(cwd: &Path, enabled: bool) -> Vec<ContextFile> {
    if !enabled {
        return Vec::new();
    }
    let (Some(root), Ok(canonical)) = (repository_root(cwd), fs::canonicalize(cwd)) else {
        return Vec::new();
    };
    let mut directories = canonical
        .ancestors()
        .take_while(|directory| directory.starts_with(&root))
        .collect::<Vec<_>>();
    directories.reverse();
    directories
        .into_iter()
        .filter_map(|directory| {
            // `cwd`'s own file keeps the path the caller gave, so it is
            // labelled relative to `cwd` exactly as before.
            let path = if directory == canonical {
                cwd.join(INSTRUCTION_FILE)
            } else {
                directory.join(INSTRUCTION_FILE)
            };
            let body = fs::read_to_string(&path).ok()?;
            Some(ContextFile {
                path,
                name: INSTRUCTION_FILE.to_string(),
                body,
            })
        })
        .collect()
}

pub fn load_context_files_for_targets(
    cwd: &Path,
    enabled: bool,
    targets: &[PathBuf],
) -> Vec<ContextFile> {
    if !enabled || repository_root(cwd).is_none() {
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

/// `size + mtime` of an on-disk instruction file: what a request-boundary
/// check can read without opening the file.
pub(crate) type FileStamp = (Option<std::time::SystemTime>, u64);

pub(crate) fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok(), meta.len()))
}

/// `plugin:<name>` entries are SessionStart hook output, not files.
pub(crate) fn is_plugin_context(file: &ContextFile) -> bool {
    file.path.to_string_lossy().starts_with("plugin:")
}

/// Bring loaded on-disk instruction files up to date: re-read a file whose
/// stamp moved, drop one that is gone, leave every other file (and the
/// `plugin:` pseudo-files) untouched. Returns whether any body changed, so a
/// caller that sees `false` knows the system prompt is byte-identical.
pub(crate) fn reload_changed_context_files(
    files: &mut Vec<ContextFile>,
    stamps: &mut HashMap<PathBuf, FileStamp>,
) -> bool {
    let mut changed = false;
    files.retain_mut(|file| {
        if is_plugin_context(file) {
            return true;
        }
        let meta = match fs::metadata(&file.path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                stamps.remove(&file.path);
                changed = true;
                return false;
            }
            // Unreadable right now (a lock, a permission flip): keep what loaded.
            Err(_) => return true,
        };
        let stamp = (meta.modified().ok(), meta.len());
        if stamps.get(&file.path) == Some(&stamp) {
            return true;
        }
        if let Ok(body) = fs::read_to_string(&file.path) {
            if body != file.body {
                file.body = body;
                changed = true;
            }
            stamps.insert(file.path.clone(), stamp);
        }
        true
    });
    changed
}

/// Custom-message type of the appended nested-instruction delivery.
pub const NESTED_INSTRUCTIONS_CUSTOM_TYPE: &str = "davinci.nested_instructions";

/// Every `AGENTS.md` that governs `target`: the repository root's down to the
/// nearest existing directory of the target, outermost first. Empty when the
/// target is outside the checkout of `cwd`. Paths are canonical.
pub(crate) fn governing_instruction_files(cwd: &Path, target: &Path) -> Vec<ContextFile> {
    let Some(root) = repository_root(cwd) else {
        return Vec::new();
    };
    let absolute = if target.is_absolute() {
        target.to_path_buf()
    } else {
        cwd.join(target)
    };
    // A file about to be created, or a directory not made yet, still sits
    // under the nearest directory that exists.
    let mut start = Some(absolute.as_path());
    let mut resolved = None;
    while let Some(candidate) = start {
        if let Ok(canonical) = fs::canonicalize(candidate) {
            resolved = Some(canonical);
            break;
        }
        start = candidate.parent();
    }
    let Some(resolved) = resolved else {
        return Vec::new();
    };
    let directory = if resolved.is_dir() {
        resolved
    } else {
        match resolved.parent() {
            Some(parent) => parent.to_path_buf(),
            None => return Vec::new(),
        }
    };
    if !directory.starts_with(&root) {
        return Vec::new();
    }
    let mut chain = directory
        .ancestors()
        .take_while(|path| path.starts_with(&root))
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    chain.reverse();
    chain
        .into_iter()
        .filter_map(|dir| {
            let path = dir.join(INSTRUCTION_FILE);
            let body = fs::read_to_string(&path).ok()?;
            Some(ContextFile {
                path,
                name: INSTRUCTION_FILE.to_string(),
                body,
            })
        })
        .collect()
}

/// Where a nested instruction file applies, named the way
/// `append_repository_context` names files: from `cwd`, else from the
/// repository root, never with the checkout's absolute path.
fn scope_label(file: &ContextFile, cwd: &Path) -> String {
    let directory = file.path.parent().unwrap_or(&file.path);
    let canonical_cwd = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let relative = |base: &Path| {
        directory
            .strip_prefix(base)
            .ok()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
    };
    let label = match relative(&canonical_cwd) {
        Some(path) => path,
        None => match repository_root(cwd).and_then(|root| relative(&root)) {
            Some(path) => format!("<repository root>/{path}"),
            None => file.name.clone(),
        },
    };
    let label = label.trim_end_matches('/');
    if label.is_empty() || label == "<repository root>" {
        "./".to_string()
    } else {
        format!("{label}/")
    }
}

/// The appended message that delivers nested instruction files: each body is
/// fenced and scoped to its directory so it never reads as global.
pub(crate) fn render_nested_instructions(files: &[ContextFile], cwd: &Path) -> String {
    let mut text = String::from(
        "<nested_instructions>
These project instructions apply only to files under the directory named in each block, and add to the instructions already given. They do not apply to files elsewhere.

",
    );
    for file in files {
        let scope = scope_label(file, cwd);
        text.push_str("<project_instructions scope=");
        text.push_str(&serde_json::to_string(&scope).expect("scope is JSON"));
        text.push_str(
            ">
",
        );
        text.push_str(&file.body);
        text.push_str(
            "
</project_instructions>

",
        );
    }
    text.push_str("</nested_instructions>");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(path: &Path) -> (Vec<ContextFile>, HashMap<PathBuf, FileStamp>) {
        let files = vec![ContextFile {
            path: path.to_path_buf(),
            name: INSTRUCTION_FILE.into(),
            body: fs::read_to_string(path).unwrap(),
        }];
        let stamps = HashMap::from([(path.to_path_buf(), file_stamp(path).unwrap())]);
        (files, stamps)
    }

    #[test]
    fn reload_rereads_only_changed_files_and_drops_deleted_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        fs::write(&path, "first").unwrap();
        let (mut files, mut stamps) = loaded(&path);
        files.push(ContextFile {
            path: PathBuf::from("plugin:demo"),
            name: "plugin:demo".into(),
            body: "hook output".into(),
        });

        assert!(!reload_changed_context_files(&mut files, &mut stamps));
        assert_eq!(files[0].body, "first");

        fs::write(&path, "second, longer").unwrap();
        assert!(reload_changed_context_files(&mut files, &mut stamps));
        assert_eq!(files[0].body, "second, longer");
        // Now current: the next check is quiet again.
        assert!(!reload_changed_context_files(&mut files, &mut stamps));

        fs::remove_file(&path).unwrap();
        assert!(reload_changed_context_files(&mut files, &mut stamps));
        let names: Vec<&str> = files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, ["plugin:demo"], "plugin entries are never statted");
    }

    #[test]
    fn a_touched_file_with_the_same_body_is_not_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        fs::write(&path, "same").unwrap();
        let (mut files, mut stamps) = loaded(&path);
        // A stale stamp forces a re-read; identical content must report no change.
        stamps.insert(path.clone(), (None, 0));
        assert!(!reload_changed_context_files(&mut files, &mut stamps));
        assert_eq!(stamps[&path], file_stamp(&path).unwrap());
    }

    #[test]
    fn governing_files_run_from_the_root_to_the_target_and_stop_at_the_checkout() {
        let outer = tempfile::tempdir().unwrap();
        fs::write(outer.path().join("AGENTS.md"), "above the checkout").unwrap();
        let repo = outer.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("a/b")).unwrap();
        fs::create_dir_all(repo.join("sibling")).unwrap();
        fs::write(repo.join("AGENTS.md"), "root").unwrap();
        fs::write(repo.join("a/b/AGENTS.md"), "ab").unwrap();
        fs::write(repo.join("sibling/AGENTS.md"), "sibling").unwrap();
        let bodies = |cwd: &Path, target: &str| -> Vec<String> {
            governing_instruction_files(cwd, Path::new(target))
                .into_iter()
                .map(|file| file.body)
                .collect()
        };
        assert_eq!(bodies(&repo, "a/b/f.rs"), ["root", "ab"]);
        // A path not created yet resolves through its nearest existing directory.
        assert_eq!(bodies(&repo, "a/b/new/dir/f.rs"), ["root", "ab"]);
        // From a subdirectory session a sibling directory still governs its files.
        assert_eq!(
            bodies(&repo.join("a"), "../sibling/f.rs"),
            ["root", "sibling"]
        );
        assert!(bodies(&repo, "../AGENTS.md").is_empty());
        assert!(bodies(outer.path(), "AGENTS.md").is_empty());
    }

    #[test]
    fn nested_instructions_are_labelled_by_scope_without_the_checkout_path() {
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir_all(repo.path().join(".git")).unwrap();
        fs::create_dir_all(repo.path().join("app/inner")).unwrap();
        fs::create_dir_all(repo.path().join("lib")).unwrap();
        fs::write(repo.path().join("app/inner/AGENTS.md"), "inner rules").unwrap();
        fs::write(repo.path().join("lib/AGENTS.md"), "lib rules").unwrap();
        let files = ["app/inner", "lib"]
            .iter()
            .flat_map(|dir| {
                governing_instruction_files(&repo.path().join("app"), &repo.path().join(dir))
            })
            .collect::<Vec<_>>();
        let text = render_nested_instructions(&files, &repo.path().join("app"));
        assert!(text.contains(r#"scope="inner/""#), "{text}");
        assert!(text.contains(r#"scope="<repository root>/lib/""#), "{text}");
        assert!(text.contains("inner rules") && text.contains("lib rules"));
        assert!(!text.contains(repo.path().to_str().unwrap()), "{text}");
    }

    #[test]
    fn identical_instruction_bodies_are_charged_once() {
        let file = |name: &str, body: &str| ContextFile {
            path: PathBuf::from(name),
            name: name.into(),
            body: body.into(),
        };
        let files = [
            file("AGENTS.md", "twelve chars"),
            file("CLAUDE.md", "twelve chars"),
            file("src/AGENTS.md", "other"),
        ];
        assert_eq!(context_file_charges(&files), [3, 0, 2]);
    }

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
        fs::create_dir(root.path().join(".git")).unwrap();
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

    /// A session started in a folder that is no checkout (a home or Desktop
    /// folder) loads no instructions, even when an AGENTS.md sits there.
    #[test]
    fn nothing_is_loaded_outside_a_repository() {
        let outside = tempfile::tempdir().unwrap();
        assert!(repository_root(outside.path()).is_none());
        fs::create_dir(outside.path().join("src")).unwrap();
        fs::write(outside.path().join("AGENTS.md"), "@CLAUDE.md").unwrap();
        fs::write(outside.path().join("src/file.rs"), "").unwrap();
        assert!(load_context_files(outside.path(), true).is_empty());
        assert!(load_context_files_for_targets(
            outside.path(),
            true,
            &[PathBuf::from("src/file.rs")]
        )
        .is_empty());
    }

    /// A folder holding several checkouts is not one: the AGENTS.md beside
    /// them is never read from inside any of them.
    #[test]
    fn a_checkout_reads_from_its_root_down_and_never_above_it() {
        let desktop = tempfile::tempdir().unwrap();
        fs::write(desktop.path().join("AGENTS.md"), "desktop rules").unwrap();
        let repo = desktop.path().join("project");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("crates/app")).unwrap();
        fs::write(repo.join("AGENTS.md"), "project rules").unwrap();
        fs::write(repo.join("crates/app/AGENTS.md"), "app rules").unwrap();

        let bodies = |cwd: &Path| -> Vec<String> {
            load_context_files(cwd, true)
                .into_iter()
                .map(|file| file.body)
                .collect()
        };
        assert_eq!(bodies(&repo), ["project rules"]);
        // In a subdirectory the root's rules come first, then the nearer ones.
        let app = repo.join("crates/app");
        assert_eq!(bodies(&app), ["project rules", "app rules"]);
        assert_eq!(bodies(&repo.join("crates")), ["project rules"]);

        // A linked worktree marks its root with a `.git` file.
        let worktree = desktop.path().join("worktree");
        fs::create_dir(&worktree).unwrap();
        fs::write(worktree.join(".git"), "gitdir: elsewhere").unwrap();
        fs::write(worktree.join("AGENTS.md"), "worktree rules").unwrap();
        assert_eq!(bodies(&worktree), ["worktree rules"]);

        // The root's file is labelled from the root, the nearer one from cwd.
        let mut prompt = String::new();
        append_repository_context(&mut prompt, &load_context_files(&app, true), &app);
        assert!(
            prompt.contains(r#"paths=["<repository root>/AGENTS.md"]"#),
            "{prompt}"
        );
        assert!(prompt.contains(r#"paths=["AGENTS.md"]"#), "{prompt}");
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
        fs::create_dir_all(checkout.join(".git")).unwrap();
        fs::create_dir_all(checkout.join("src")).unwrap();
        fs::write(checkout.join("src/AGENTS.md"), "nested instructions").unwrap();
        fs::write(checkout.join("src/file.rs"), "").unwrap();
        let link = root.path().join("linked-checkout");
        std::os::unix::fs::symlink(&checkout, &link).unwrap();
        let files = load_context_files_for_targets(&link, true, &[PathBuf::from("src/file.rs")]);
        let mut prompt = String::new();
        append_repository_context(&mut prompt, &files, &link);
        assert!(prompt.contains("src/AGENTS.md"), "{prompt}");
        assert!(prompt.contains("nested instructions"), "{prompt}");
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
