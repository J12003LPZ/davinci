//! Real state for the davinci TUI (`docs/ui/design.md`).
//!
//! Each function here is the real implementation of one surface's data. What
//! has no source in this workspace yet — the code graph behind Grafo, and the
//! plan behind Disegno — stays on `davinci_tui::davinci::fixtures` and is listed as
//! such, rather than being faked here where it would look real.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use davinci_tui::davinci::model::{ChangeRow, Model, SessionItem, Startup, TreeRow};
use davinci_tui::davinci::theme::State;

/// One entry of `git status --porcelain`, already parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitChange {
    pub status: String,
    pub path: String,
}

/// `git -C cwd` for the inspection the TUI runs on its own in the user's
/// workspace. `safe.bareRepository=explicit` stops git from treating the
/// workspace root as a bare repository it discovered by itself: sandboxed code
/// can write a root `HEAD`, `objects/`, `refs/` and a `config` with
/// `core.fsmonitor`, which the next `git status` would then run on the host.
/// `isolated_git_command_in` also forces `core.fsmonitor` and hooks off for
/// repositories found normally, neutralizes the repository's filter drivers,
/// and drops user config and inherited `GIT_*` overrides. Callers that diff
/// must pass `--no-ext-diff --no-textconv` (diff drivers have no safe override).
pub(crate) fn host_git(cwd: &Path) -> Command {
    let mut command = davinci_sys::process::isolated_git_command_in(cwd);
    command.arg("-C").arg(cwd);
    command
}

/// A git failure as text: stderr when git said something, the exit status
/// otherwise.
fn git_failure(what: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    // First line only: usage dumps run to hundreds of lines.
    let stderr = stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if stderr.is_empty() {
        format!("{what} failed ({})", output.status)
    } else {
        format!("{what} failed: {stderr}")
    }
}

/// `git status --porcelain` for `cwd`. An empty list means a clean tree or a
/// directory git establishes is not a repository; a spawn failure or any other
/// git failure (a missing object with lazy fetching off, a corrupt index) is an
/// error, so the caller can say the state is unavailable instead of showing a
/// clean tree.
pub fn git_changes(cwd: &Path) -> Result<Vec<GitChange>, String> {
    let output = host_git(cwd)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|error| format!("git status could not start: {error}"))?;
    if !output.status.success() {
        if String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
            return Ok(Vec::new());
        }
        return Err(git_failure("git status", &output));
    }
    Ok(parse_porcelain(&String::from_utf8_lossy(&output.stdout)))
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
pub fn git_delta(cwd: &Path, changes: &[GitChange]) -> Result<(u32, u32, u32), String> {
    let output = host_git(cwd)
        .args([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--numstat",
            "HEAD",
        ])
        .output()
        .map_err(|error| format!("git diff could not start: {error}"))?;
    if !output.status.success() {
        // A repository with no commit yet has no HEAD to compare with: there
        // are no tracked lines to count, which is an answer, not a failure.
        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
        let no_head =
            stderr.contains("bad revision 'head'") || stderr.contains("ambiguous argument 'head'");
        if no_head || stderr.contains("not a git repository") {
            return Ok((changes.len() as u32, 0, 0));
        }
        return Err(git_failure("git diff", &output));
    }
    let (adds, dels) = parse_numstat(&String::from_utf8_lossy(&output.stdout));
    Ok((changes.len() as u32, adds, dels))
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
    let counts = host_git(cwd)
        .args([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--numstat",
            "HEAD",
        ])
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

/// Workspace-relative paths for composer completion. Four levels deep and
/// forward-slashed on every platform, because that is how a path gets typed
/// into a prompt.
pub fn completion_paths(cwd: &Path) -> Vec<String> {
    let mut paths: Vec<String> = walk(cwd, 4)
        .into_iter()
        .map(|(_, path, is_dir)| {
            let relative = path
                .strip_prefix(cwd)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if is_dir {
                format!("{relative}/")
            } else {
                relative
            }
        })
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// Memoria sessions (`1f`), from the real session store. Newest first, which
/// is the order the picker opens in.
pub fn sessions(session_dir: &Path, now_ms: u64) -> Vec<SessionItem> {
    let Ok(mut found) = davinci_session::discover_sessions(session_dir, None) else {
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
pub fn session_name(summary: &davinci_session::SessionSummary) -> String {
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
        found: Vec::new(),
    }
}

/// Everything [`dress_from_workspace`] reads, as data, so the reading — four
/// git subprocesses and a tree walk — can happen on a worker thread instead
/// of the drawing one.
pub struct WorkspaceDress {
    /// What `gh` said about this branch's pull request, when the worker
    /// asked it; `None` leaves the model's badge as it was.
    pub pr: Option<Option<davinci_tui::davinci::model::PrBadge>>,
    /// Said once: why the pull request badge cannot show.
    pub hint: Option<String>,
    pub cwd: String,
    pub branch: String,
    pub changes: (u32, u32, u32),
    pub changes_list: Vec<ChangeRow>,
    pub tree: Vec<TreeRow>,
    pub paths: Vec<String>,
    pub sessions: Vec<SessionItem>,
    pub startup: Startup,
}

/// Read everything the workspace can answer. Safe on any thread.
pub fn compute_workspace_dress(cwd: &Path, session_dir: &Path) -> WorkspaceDress {
    let branch = davinci_tui::resolve_git_branch(cwd).unwrap_or_default();
    // Unavailable git state is said, never shown as a clean tree.
    let mut unavailable = None;
    let changes = git_changes(cwd).unwrap_or_else(|error| {
        unavailable = Some(error);
        Vec::new()
    });
    let delta = if unavailable.is_some() {
        (0, 0, 0)
    } else {
        git_delta(cwd, &changes).unwrap_or_else(|error| {
            unavailable = Some(error);
            (changes.len() as u32, 0, 0)
        })
    };
    let sessions = sessions(session_dir, davinci_session::now_ms());
    let startup = startup(cwd, &branch, !sessions.is_empty());
    WorkspaceDress {
        pr: None,
        hint: unavailable.map(|error| format!("git state unavailable: {error}")),
        cwd: cwd.display().to_string(),
        branch,
        changes: delta,
        changes_list: change_rows(cwd, &changes),
        tree: workspace_tree(cwd, &changes),
        paths: completion_paths(cwd),
        sessions,
        startup,
    }
}

/// Put a computed dress on the model.
pub fn apply_workspace_dress(model: &mut Model, dress: WorkspaceDress) {
    if let Some(hint) = dress.hint {
        model.push_toast(hint);
    }
    if let Some(pr) = dress.pr {
        model.pr_badge = pr;
    }
    model.cwd = dress.cwd;
    model.branch = dress.branch;
    model.changes = dress.changes;
    model.changes_list = dress.changes_list;
    model.tree = dress.tree;
    model.paths = dress.paths;
    model.sessions = dress.sessions;
    // What the session found is known only to the shell; a later dress from
    // the workspace thread must not blank it.
    let found = std::mem::take(&mut model.startup.found);
    model.startup = dress.startup;
    model.startup.found = found;
}

/// Fill the model from everything this workspace can actually answer,
/// synchronously — the opening frame, and tests.
pub fn dress_from_workspace(model: &mut Model, cwd: &Path, session_dir: &Path) {
    apply_workspace_dress(model, compute_workspace_dress(cwd, session_dir));
}

/// When `gh` was last asked about which branch, and what it answered.
#[derive(Default)]
pub struct PrLookup {
    asked: Option<(std::time::Instant, String)>,
    answer: Option<davinci_tui::davinci::model::PrBadge>,
    hinted: bool,
    /// Set by the lookup that found `gh` missing; the worker takes it once.
    hint: Option<String>,
}

/// What to tell the user when `gh` could not even start: only for a
/// repository whose origin is on GitHub, where the badge would have shown.
fn missing_gh_hint(error: &std::io::Error, origin: &str) -> Option<String> {
    (error.kind() == std::io::ErrorKind::NotFound && origin.contains("github.com")).then(|| {
        "install the GitHub CLI (gh) to show this branch's pull request in the footer".to_owned()
    })
}

/// How long a `gh` answer is trusted before the next re-read asks again.
const PR_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// The pull request for `branch`, asking `gh` at most once a minute per
/// branch (`None` between asks: the footer keeps what it has). A missing
/// `gh`, no pull request or no network all read as no badge.
fn pull_request_for(
    cwd: &Path,
    branch: &str,
    last: &mut PrLookup,
) -> Option<Option<davinci_tui::davinci::model::PrBadge>> {
    if branch.is_empty() || std::env::var_os("DAVINCI_DISABLE_PR_BADGE").is_some() {
        return None;
    }
    if let Some((at, asked)) = &last.asked {
        if asked == branch && at.elapsed() < PR_TTL {
            return None;
        }
    }
    let output = std::process::Command::new("gh")
        .args([
            "pr",
            "view",
            "--json",
            "number,state,reviewDecision,isDraft,url",
        ])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    last.asked = Some((std::time::Instant::now(), branch.to_owned()));
    if let (Err(error), false) = (&output, last.hinted) {
        let origin = host_git(cwd)
            .args(["remote", "get-url", "origin"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        last.hint = missing_gh_hint(error, &origin);
        last.hinted = last.hint.is_some();
    }
    last.answer = output
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| {
            davinci_tui::davinci::model::PrBadge::parse(&String::from_utf8_lossy(&out.stdout))
        });
    Some(last.answer.clone())
}

/// A worker that re-reads the workspace off the drawing thread.
///
/// `redress()` used to spawn four git subprocesses and walk the tree from the
/// key-handling path, which is a visible stall on a large repository. Requests
/// are coalesced: however many pile up while one read runs, the worker reads
/// once more.
pub struct WorkspaceDresser {
    requests: std::sync::mpsc::Sender<()>,
    results: std::sync::mpsc::Receiver<WorkspaceDress>,
}

impl WorkspaceDresser {
    pub fn start(cwd: PathBuf, session_dir: PathBuf) -> Self {
        let (requests, incoming) = std::sync::mpsc::channel::<()>();
        let (outgoing, results) = std::sync::mpsc::channel::<WorkspaceDress>();
        std::thread::spawn(move || {
            let mut last_pr = PrLookup::default();
            while incoming.recv().is_ok() {
                // Coalesce the backlog: one read answers them all.
                while incoming.try_recv().is_ok() {}
                let mut dress = compute_workspace_dress(&cwd, &session_dir);
                dress.pr = pull_request_for(&cwd, &dress.branch, &mut last_pr);
                dress.hint = last_pr.hint.take();
                if outgoing.send(dress).is_err() {
                    break;
                }
            }
        });
        Self { requests, results }
    }

    /// Ask for a re-read; returns at once.
    pub fn request(&self) {
        let _ = self.requests.send(());
    }

    /// Apply any finished read; `true` when the model changed.
    pub fn apply_ready(&self, model: &mut Model) -> bool {
        let mut dirty = false;
        while let Ok(dress) = self.results.try_recv() {
            apply_workspace_dress(model, dress);
            dirty = true;
        }
        dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_gh_is_explained_only_for_a_github_origin() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(missing_gh_hint(&missing, "git@github.com:me/app.git").is_some());
        assert!(missing_gh_hint(&missing, "https://gitlab.com/me/app").is_none());
        assert!(missing_gh_hint(&missing, "").is_none());
        assert!(missing_gh_hint(&denied, "git@github.com:me/app.git").is_none());
    }

    #[test]
    fn porcelain_is_parsed_into_status_and_path() {
        let parsed = parse_porcelain(
            " M crates/davinci-tui/src/lib.rs\n\
             A  crates/davinci-tui/src/davinci/mod.rs\n\
             ?? docs/ui/design.md\n\
             R  old/name.rs -> new/name.rs\n",
        );
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].status, "M");
        assert_eq!(parsed[0].path, "crates/davinci-tui/src/lib.rs");
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
        std::fs::create_dir_all(root.join("crates/davinci-tui/src")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]").unwrap();
        std::fs::write(root.join("crates/davinci-tui/src/lib.rs"), "").unwrap();

        let changes = vec![GitChange {
            status: "M".into(),
            path: "crates/davinci-tui/src/lib.rs".into(),
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
        let mut summary = davinci_session::SessionSummary {
            id: "019fe3b2-bc6e-70b5".into(),
            path: PathBuf::new(),
            cwd: String::new(),
            created_at: 0,
            modified_at: 0,
            name: Some("tui-redesign".into()),
            parent_session_id: None,
            source_format: 4,
            all_messages_text: "explain how the agent runtime works\nsure".into(),
            messages_text_bytes: 40,
            message_count: 2,
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
        let changes = git_changes(temp.path()).unwrap();
        assert!(changes.is_empty());
        assert_eq!(git_delta(temp.path(), &changes), Ok((0, 0, 0)));
    }

    /// Review re-check item 1: a git failure inside a repository is an error
    /// the workspace dress reports, not an empty change list. Missing objects
    /// are the realistic case now that lazy fetching is off.
    #[test]
    fn audit_wp01_git_failure_is_reported_not_shown_as_a_clean_tree() {
        let git = |root: &Path, args: &[&str]| {
            let out = Command::new("git")
                .current_dir(root)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        git(root, &["init", "-q"]);
        std::fs::write(root.join("a.txt"), "one").unwrap();
        git(root, &["add", "."]);
        git(
            root,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "c",
            ],
        );
        std::fs::write(root.join("a.txt"), "two").unwrap();
        let sessions = tempfile::tempdir().unwrap();

        // Healthy: the change is listed and nothing is hinted.
        let dress = compute_workspace_dress(root, sessions.path());
        assert_eq!(dress.changes.0, 1, "{:?}", dress.hint);
        assert!(dress.hint.is_none(), "{:?}", dress.hint);

        // Corrupt the index: status now fails. It must be said, not hidden.
        std::fs::write(root.join(".git").join("index"), b"not an index").unwrap();
        let error = git_changes(root).unwrap_err();
        assert!(error.contains("git status failed"), "{error}");
        let dress = compute_workspace_dress(root, sessions.path());
        let hint = dress.hint.expect("unavailable state must be reported");
        assert!(hint.contains("git state unavailable"), "{hint}");

        // A directory git says is not a repository is an explicit empty answer.
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(git_changes(plain.path()), Ok(Vec::new()));
    }

    #[test]
    fn host_git_never_discovers_a_bare_repository_implicitly() {
        let command = host_git(Path::new("work"));
        let args: Vec<_> = command.get_args().collect();
        let has = |setting: &str| {
            args.windows(2)
                .any(|pair| pair[0] == "-c" && pair[1] == setting)
        };
        assert!(has("safe.bareRepository=explicit"));
        assert!(has("core.fsmonitor=false"));
        assert!(args.ends_with(&["-C", "work"].map(std::ffi::OsStr::new)));
    }

    #[test]
    fn a_workspace_root_planted_as_a_bare_repository_is_not_run() {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("git is unavailable: skipping");
            return;
        }
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let slashed = |path: &Path| path.display().to_string().replace('\\', "/");
        let marker = root.join("fsmonitor-ran");
        std::fs::write(root.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(root.join("objects")).unwrap();
        std::fs::create_dir_all(root.join("refs/heads")).unwrap();
        std::fs::write(
            root.join("config"),
            format!(
                "[core]\n\trepositoryformatversion = 0\n\tbare = false\n\tworktree = {}\n\tfsmonitor = echo ran > {} #\n",
                slashed(root),
                slashed(&marker)
            ),
        )
        .unwrap();
        let changes = git_changes(root).unwrap_or_default();
        let _ = git_delta(root, &changes);
        change_rows(root, &changes);
        assert!(changes.is_empty());
        assert!(!marker.exists(), "host git ran the planted core.fsmonitor");
    }

    #[test]
    fn audit_wp01_b003_host_git_never_runs_a_repository_fsmonitor() {
        let git = |root: &Path, args: &[&str]| {
            let out = Command::new("git").current_dir(root).args(args).output();
            out.map(|out| out.status.success()).unwrap_or(false)
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        if !git(root, &["init", "-q"]) {
            eprintln!("git is unavailable: skipping");
            return;
        }
        let marker = root.join("fsmonitor-ran");
        let hook = format!(
            "echo ran > {} #",
            marker.display().to_string().replace('\\', "/")
        );
        assert!(git(root, &["config", "core.fsmonitor", &hook]));
        std::fs::write(root.join("new.txt"), "x").unwrap();

        // Positive control: plain git runs the planted hook.
        git(root, &["status", "--porcelain"]);
        if !marker.exists() {
            eprintln!("git here does not run core.fsmonitor shell hooks: skipping");
            return;
        }
        std::fs::remove_file(&marker).unwrap();

        let changes = git_changes(root).unwrap();
        git_delta(root, &changes).unwrap();
        assert!(
            changes.iter().any(|change| change.path == "new.txt"),
            "real status must still be reported: {changes:?}"
        );
        assert!(
            !marker.exists(),
            "host git ran the repository core.fsmonitor"
        );
    }
}
