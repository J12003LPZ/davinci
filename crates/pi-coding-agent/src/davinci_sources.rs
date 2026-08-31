//! Real state for the davinci TUI (`docs/ui/design.md`).
//!
//! Each function here is the real implementation of one surface's data. What
//! has no source in this workspace yet — the code graph behind Grafo, and the
//! plan behind Disegno — stays on `pi_tui::davinci::fixtures` and is listed as
//! such, rather than being faked here where it would look real.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use pi_tui::davinci::model::{ChangeRow, Model, SessionItem, Startup, TreeRow};
use pi_tui::davinci::theme::State;

/// One entry of `git status --porcelain`, already parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitChange {
    pub status: String,
    pub path: String,
}

/// `git status --porcelain` for `cwd`, or an empty list outside a repository.
pub fn git_changes(cwd: &Path) -> Vec<GitChange> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["status", "--porcelain"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_porcelain(&String::from_utf8_lossy(&output.stdout))
}

/// Pure form of [`git_changes`].
pub fn parse_porcelain(text: &str) -> Vec<GitChange> {
    text.lines()
        .filter(|line| line.len() > 3)
        .map(|line| {
            let (code, path) = line.split_at(2);
            let code = code.trim();
            // A renamed path is reported as `old -> new`; the new one is what
            // the user is looking at.
            let path = path.trim();
            let path = path.rsplit(" -> ").next().unwrap_or(path);
            GitChange {
                status: if code.is_empty() {
                    "?".to_string()
                } else {
                    code.chars().next().unwrap_or('?').to_string()
                },
                path: path.trim_matches('"').to_string(),
            }
        })
        .collect()
}

/// `Δn +a -d` for the status bar, from `git diff --numstat` plus the count of
/// changed files. Untracked files count as changed but contribute no lines.
pub fn git_delta(cwd: &Path, changes: &[GitChange]) -> (u32, u32, u32) {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["diff", "--numstat", "HEAD"])
        .output();
    let (adds, dels) = match output {
        Ok(output) if output.status.success() => {
            parse_numstat(&String::from_utf8_lossy(&output.stdout))
        }
        _ => (0, 0),
    };
    (changes.len() as u32, adds, dels)
}

/// Pure form of the `--numstat` half of [`git_delta`].
pub fn parse_numstat(text: &str) -> (u32, u32) {
    let mut adds = 0;
    let mut dels = 0;
    for line in text.lines() {
        let mut fields = line.split('\t');
        let a = fields.next().unwrap_or("-");
        let d = fields.next().unwrap_or("-");
        adds += a.parse::<u32>().unwrap_or(0);
        dels += d.parse::<u32>().unwrap_or(0);
    }
    (adds, dels)
}

/// The changes popover (`1e`), with per-file line counts where git has them.
pub fn change_rows(cwd: &Path, changes: &[GitChange]) -> Vec<ChangeRow> {
    let counts = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["diff", "--numstat", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let mut fields = line.split('\t');
                    let adds = fields.next()?.parse::<u32>().ok()?;
                    let path = fields.nth(1)?.to_string();
                    Some((path, adds))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    changes
        .iter()
        .map(|change| {
            let count = counts
                .iter()
                .find(|(path, _)| path == &change.path.replace('\\', "/"))
                .map(|(_, adds)| format!("+{adds}"))
                .unwrap_or_else(|| "·".to_string());
            ChangeRow::new(&change.status, &change.path, &count)
        })
        .collect()
}

/// The Codex workspace tree (`1e`): two levels of `cwd`, with changed paths
/// marked. Deliberately shallow — the sidebar is a map, not a file manager.
pub fn workspace_tree(cwd: &Path, changes: &[GitChange]) -> Vec<TreeRow> {
    let changed: BTreeSet<String> = changes
        .iter()
        .map(|change| change.path.replace('\\', "/"))
        .collect();
    let mut rows = Vec::new();
    for (depth, path, is_dir) in walk(cwd, 2) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let relative = path
            .strip_prefix(cwd)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let touched = changed
            .iter()
            .any(|change| change == &relative || change.starts_with(&format!("{relative}/")));
        rows.push(TreeRow::new(
            depth,
            if is_dir {
                Some(if depth == 0 { "▾" } else { "▸" })
            } else {
                None
            },
            &name,
            touched.then_some(State::Delta),
        ));
    }
    rows
}

/// Directories first, then files, alphabetically, skipping what nobody reads.
fn walk(root: &Path, depth_limit: u16) -> Vec<(u16, PathBuf, bool)> {
    fn ignored(name: &str) -> bool {
        matches!(
            name,
            ".git" | "target" | "node_modules" | ".pi" | "dist" | "build"
        )
    }

    fn level(dir: &Path, depth: u16, limit: u16, out: &mut Vec<(u16, PathBuf, bool)>) {
        if depth >= limit {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if ignored(&name) || name.starts_with('.') && depth > 0 {
                continue;
            }
            if path.is_dir() {
                dirs.push(path);
            } else {
                files.push(path);
            }
        }
        dirs.sort();
        files.sort();
        for path in dirs {
            out.push((depth, path.clone(), true));
            level(&path, depth + 1, limit, out);
        }
        for path in files {
            out.push((depth, path, false));
        }
    }

    let mut out = Vec::new();
    level(root, 0, depth_limit, &mut out);
    out
}

/// Memoria sessions (`1f`), from the real session store. Newest first, which
/// is the order the picker opens in.
pub fn sessions(session_dir: &Path, now_ms: u64) -> Vec<SessionItem> {
    let Ok(mut found) = pi_session::discover_sessions(session_dir, None) else {
        return Vec::new();
    };
    found.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    found
        .into_iter()
        .map(|summary| {
            let age = humanise(now_ms.saturating_sub(summary.modified_at) / 1_000);
            SessionItem::new(&session_name(&summary), &age).at(&summary.path.display().to_string())
        })
        .collect()
}

/// What a session is called in the picker: its own name, else the turn that
/// opened it, else its id. A wall of UUIDs is not a memory.
pub fn session_name(summary: &pi_session::SessionSummary) -> String {
    if let Some(name) = summary.name.as_ref().filter(|name| !name.trim().is_empty()) {
        return name.clone();
    }
    let opening = summary
        .all_messages_text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if opening.is_empty() {
        return summary.id.clone();
    }
    let clipped: String = opening.chars().take(48).collect();
    if clipped.len() < opening.len() {
        format!("{clipped}…")
    } else {
        clipped
    }
}

/// `3m`, `18m`, `1h`, `yesterday`, `2d`. Ages are read, not computed with.
pub fn humanise(seconds: u64) -> String {
    match seconds {
        0..=89 => "just now".to_string(),
        90..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        86_400..=172_799 => "yesterday".to_string(),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// What the empty state says it found (`1a`).
pub fn startup(cwd: &Path, branch: &str, restored: bool) -> Startup {
    let crates = std::fs::read_dir(cwd.join("crates"))
        .map(|entries| entries.flatten().filter(|e| e.path().is_dir()).count())
        .unwrap_or(0);
    Startup {
        cwd: cwd.display().to_string(),
        branch: branch.to_string(),
        language: if cwd.join("Cargo.toml").exists() {
            "rust".to_string()
        } else {
            String::new()
        },
        crates: match crates {
            0 => String::new(),
            1 => "1 crate".to_string(),
            n => format!("{n} crates"),
        },
        restored,
    }
}

/// Fill the model from everything this workspace can actually answer.
pub fn dress_from_workspace(model: &mut Model, cwd: &Path, session_dir: &Path) {
    let branch = pi_tui::resolve_git_branch(cwd).unwrap_or_default();
    let changes = git_changes(cwd);

    model.cwd = cwd.display().to_string();
    model.branch = branch.clone();
    model.changes = git_delta(cwd, &changes);
    model.changes_list = change_rows(cwd, &changes);
    model.tree = workspace_tree(cwd, &changes);
    model.sessions = sessions(session_dir, pi_session::now_ms());
    model.startup = startup(cwd, &branch, !model.sessions.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_is_parsed_into_status_and_path() {
        let parsed = parse_porcelain(
            " M crates/pi-tui/src/lib.rs\n\
             A  crates/pi-tui/src/davinci/mod.rs\n\
             ?? docs/ui/design.md\n\
             R  old/name.rs -> new/name.rs\n",
        );
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].status, "M");
        assert_eq!(parsed[0].path, "crates/pi-tui/src/lib.rs");
        assert_eq!(parsed[1].status, "A");
        assert_eq!(parsed[2].status, "?");
        assert_eq!(parsed[3].path, "new/name.rs", "a rename shows its new path");
    }

    #[test]
    fn porcelain_ignores_short_and_empty_lines() {
        assert!(parse_porcelain("").is_empty());
        assert!(parse_porcelain("\n \n").is_empty());
    }

    #[test]
    fn numstat_sums_additions_and_deletions() {
        assert_eq!(parse_numstat("3\t1\tsrc/a.rs\n10\t0\tsrc/b.rs\n"), (13, 1));
        // A binary file reports `-` for both, and contributes nothing.
        assert_eq!(parse_numstat("-\t-\tlogo.png\n"), (0, 0));
        assert_eq!(parse_numstat(""), (0, 0));
    }

    #[test]
    fn ages_read_rather_than_compute() {
        assert_eq!(humanise(5), "just now");
        assert_eq!(humanise(180), "3m");
        assert_eq!(humanise(1_080), "18m");
        assert_eq!(humanise(3_600), "1h");
        assert_eq!(humanise(90_000), "yesterday");
        assert_eq!(humanise(200_000), "2d");
    }

    #[test]
    fn the_workspace_tree_marks_changed_paths_and_skips_noise() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        std::fs::create_dir_all(root.join("crates/pi-tui/src")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]").unwrap();
        std::fs::write(root.join("crates/pi-tui/src/lib.rs"), "").unwrap();

        let changes = vec![GitChange {
            status: "M".into(),
            path: "crates/pi-tui/src/lib.rs".into(),
        }];
        let rows = workspace_tree(root, &changes);
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();

        assert!(names.contains(&"crates"));
        assert!(names.contains(&"Cargo.toml"));
        assert!(!names.contains(&"target"), "build output is not a map");
        assert!(!names.contains(&".git"), "{names:?}");

        let crates = rows.iter().find(|row| row.name == "crates").unwrap();
        assert_eq!(
            crates.status,
            Some(State::Delta),
            "a directory containing a change is marked"
        );
        assert_eq!(crates.twisty.as_deref(), Some("▾"));
    }

    #[test]
    fn startup_reports_the_workspace_it_found() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        std::fs::create_dir_all(root.join("crates/one")).unwrap();
        std::fs::create_dir_all(root.join("crates/two")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]").unwrap();

        let info = startup(root, "main", true);
        assert_eq!(info.branch, "main");
        assert_eq!(info.language, "rust");
        assert_eq!(info.crates, "2 crates");
        assert!(info.restored);

        let bare = tempfile::tempdir().expect("tempdir");
        let info = startup(bare.path(), "", false);
        assert_eq!(info.language, "");
        assert_eq!(info.crates, "");
        assert!(!info.restored);
    }

    #[test]
    fn a_session_is_named_by_its_name_then_its_opening_turn_then_its_id() {
        let mut summary = pi_session::SessionSummary {
            id: "019fe3b2-bc6e-70b5".into(),
            path: PathBuf::new(),
            cwd: String::new(),
            created_at: 0,
            modified_at: 0,
            name: Some("tui-redesign".into()),
            parent_session_id: None,
            source_format: 4,
            all_messages_text: "explain how the agent runtime works\nsure".into(),
        };
        assert_eq!(session_name(&summary), "tui-redesign");

        summary.name = None;
        assert_eq!(
            session_name(&summary),
            "explain how the agent runtime works"
        );

        summary.name = Some("   ".into());
        assert_eq!(
            session_name(&summary),
            "explain how the agent runtime works"
        );

        summary.name = None;
        summary.all_messages_text = "x".repeat(80);
        let long = session_name(&summary);
        assert_eq!(long.chars().count(), 49, "clipped and marked: {long}");
        assert!(long.ends_with('…'));

        summary.all_messages_text = String::new();
        assert_eq!(session_name(&summary), "019fe3b2-bc6e-70b5");
    }

    #[test]
    fn git_helpers_are_quiet_outside_a_repository() {
        let temp = tempfile::tempdir().expect("tempdir");
        let changes = git_changes(temp.path());
        assert!(changes.is_empty());
        assert_eq!(git_delta(temp.path(), &changes), (0, 0, 0));
    }
}
