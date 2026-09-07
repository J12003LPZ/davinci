//! Fixed-argument, bounded local Git inventory. No repository commands are executed.
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn executable() -> Result<PathBuf, String> {
    let path = std::env::var_os("PATH").ok_or("Git executable unavailable")?;
    for dir in std::env::split_paths(&path).filter(|dir| dir.is_absolute()) {
        let candidate = dir.join(if cfg!(windows) { "git.exe" } else { "git" });
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .map_err(|_| "cannot resolve Git executable".into());
        }
    }
    Err("Git executable unavailable".into())
}

pub fn root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|path| path.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

#[cfg(test)]
fn run(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    run_interruptible(root, args, &|| false)
}

pub(super) fn run_interruptible(
    root: &Path,
    args: &[&str],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, String> {
    let (bytes, status) = run_status_interruptible(root, args, cancelled)?;
    if status.success() {
        Ok(bytes)
    } else {
        Err("Git inventory failed".into())
    }
}

fn run_status_interruptible(
    root: &Path,
    args: &[&str],
    cancelled: &dyn Fn() -> bool,
) -> Result<(Vec<u8>, std::process::ExitStatus), String> {
    if cancelled() {
        return Err("security Git capture cancelled".into());
    }
    let executable = executable()?;
    if executable.starts_with(root) {
        return Err("repository-owned Git executable denied".into());
    }
    let mut command = Command::new(executable);
    command
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.pager=cat",
        ])
        .args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "cannot start local Git inventory")?;
    let stdout = child.stdout.take().ok_or("Git output unavailable")?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = tx.send(bounded_read(stdout, 16 * 1024 * 1024));
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if cancelled() {
            terminate_child(&mut child);
            return Err("security Git capture cancelled".into());
        }
        if Instant::now() >= deadline {
            terminate_child(&mut child);
            return Err("Git inventory timed out".into());
        }
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(bytes)) => loop {
                if cancelled() {
                    terminate_child(&mut child);
                    return Err("security Git capture cancelled".into());
                }
                if let Some(status) = child.try_wait().map_err(|_| "cannot inspect Git process")? {
                    return Ok((bytes, status));
                }
                if Instant::now() >= deadline {
                    terminate_child(&mut child);
                    return Err("Git inventory timed out".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            },
            Ok(Err(error)) => {
                terminate_child(&mut child);
                return Err(error);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
            Err(_) => {
                terminate_child(&mut child);
                return Err("Git inventory reader failed".into());
            }
        }
    }
}

fn terminate_child(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{}", child.id())])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn bounded_read(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read bounded process output".to_string())?;
    if bytes.len() as u64 > limit {
        Err("process output exceeded byte limit".into())
    } else {
        Ok(bytes)
    }
}

pub fn inventory(
    root: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<BTreeSet<String>>, String> {
    if !root.join(".git").exists() {
        return Ok(None);
    }
    let bytes = run_interruptible(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
        cancelled,
    )?;
    bytes
        .split(|b| *b == 0)
        .filter(|v| !v.is_empty())
        .map(|v| {
            let path = std::str::from_utf8(v).map_err(|_| "unsupported Git filename encoding")?;
            super::snapshot::relative_scope(path)?;
            Ok(path.to_string())
        })
        .collect::<Result<BTreeSet<_>, String>>()
        .map(Some)
}

pub fn conflicts(
    root: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<super::snapshot::ConflictStage>, String> {
    if !root.join(".git").exists() {
        return Ok(Vec::new());
    }
    let bytes = run_interruptible(root, &["ls-files", "--unmerged", "-z"], cancelled)?;
    bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(|record| {
            let record =
                std::str::from_utf8(record).map_err(|_| "unsupported conflict filename")?;
            let (identity, path) = record.split_once('\t').ok_or("invalid conflict record")?;
            super::snapshot::relative_scope(path)?;
            let fields = identity.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3
                || ![40, 64].contains(&fields[1].len())
                || !fields[1].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("invalid conflict identity".into());
            }
            let stage = fields[2]
                .parse::<u8>()
                .map_err(|_| "invalid conflict stage")?;
            if !(1..=3).contains(&stage) {
                return Err("invalid conflict stage".into());
            }
            let conflict = super::snapshot::ConflictStage {
                path: path.into(),
                stage,
                object_id: fields[1].into(),
                mode: fields[0].into(),
            };
            conflict.validate()?;
            Ok(conflict)
        })
        .collect()
}

fn revision(root: &Path, name: &str, cancelled: &dyn Fn() -> bool) -> Result<String, String> {
    if name.starts_with('-') || name.len() > 256 || name.chars().any(char::is_control) {
        return Err("invalid local revision".into());
    }
    let expression = format!("{name}^{{commit}}");
    let bytes = run_interruptible(
        root,
        &["rev-parse", "--verify", "--end-of-options", &expression],
        cancelled,
    )?;
    let id = std::str::from_utf8(&bytes)
        .map_err(|_| "invalid revision identity")?
        .trim();
    if ![40, 64].contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid revision identity".into());
    }
    Ok(id.into())
}

// Only a missing symbolic branch is unborn. Broken objects, detached HEAD errors,
// inaccessible metadata, and command failures must retain their failure meaning.
fn head_revision(root: &Path, cancelled: &dyn Fn() -> bool) -> Result<Option<String>, String> {
    let (symbolic, status) =
        run_status_interruptible(root, &["symbolic-ref", "--quiet", "HEAD"], cancelled)?;
    if status.success() {
        let name = std::str::from_utf8(&symbolic)
            .map_err(|_| "invalid HEAD reference")?
            .trim();
        if !name.starts_with("refs/heads/") || name.chars().any(char::is_control) {
            return Err("invalid HEAD branch reference".into());
        }
        let (_, exists) =
            run_status_interruptible(root, &["show-ref", "--verify", "--quiet", name], cancelled)?;
        if exists.code() == Some(1) {
            return Ok(None);
        }
        if !exists.success() {
            return Err("cannot validate HEAD branch".into());
        }
    } else if status.code() != Some(1) {
        return Err("cannot resolve HEAD reference".into());
    }
    revision(root, "HEAD", cancelled).map(Some)
}

fn first_parent(
    root: &Path,
    head: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<String>, String> {
    let commit = run_interruptible(root, &["cat-file", "commit", head], cancelled)?;
    // Inspect only structural ASCII headers: commit messages need not be UTF-8.
    for line in commit
        .split(|byte| *byte == b'\n')
        .take_while(|line| !line.is_empty())
    {
        if let Some(parent) = line.strip_prefix(b"parent ") {
            let parent = std::str::from_utf8(parent).map_err(|_| "invalid parent identity")?;
            if ![40, 64].contains(&parent.len()) || !parent.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("invalid parent identity".into());
            }
            return revision(root, parent, cancelled).map(Some);
        }
    }
    Ok(None)
}

#[cfg(test)]
fn capture_revision(
    root: &Path,
    request: &super::command::ScanCommand,
    config: &super::ScanConfig,
) -> Result<super::snapshot::Snapshot, String> {
    capture_revision_interruptible(root, request, config, &|| false)
}

pub fn capture_revision_interruptible(
    root: &Path,
    request: &super::command::ScanCommand,
    config: &super::ScanConfig,
    cancelled: &dyn Fn() -> bool,
) -> Result<super::snapshot::Snapshot, String> {
    capture_revision_sources(root, request, config, cancelled, false)
}

pub(super) fn capture_revision_sources(
    root: &Path,
    request: &super::command::ScanCommand,
    config: &super::ScanConfig,
    cancelled: &dyn Fn() -> bool,
    include_unchanged: bool,
) -> Result<super::snapshot::Snapshot, String> {
    use super::{
        command::Selection,
        snapshot::{relative_scope, Snapshot, SourceFile},
    };
    let root = root.canonicalize().map_err(|_| "cannot resolve Git root")?;
    if !root.join(".git").exists() {
        return Err("revision selection requires a Git worktree".into());
    }
    let (base, head) = match &request.selection {
        Selection::Changed => (head_revision(&root, cancelled)?, None),
        Selection::Diff(Some((base, head))) => (
            Some(revision(&root, base, cancelled)?),
            Some(revision(&root, head, cancelled)?),
        ),
        Selection::Diff(None) => {
            let head = revision(&root, "HEAD", cancelled)?;
            (first_parent(&root, &head, cancelled)?, Some(head))
        }
        Selection::Worktree => return Err("expected revision selection".into()),
    };
    let mut changed = BTreeSet::new();
    let run = |root: &Path, args: &[&str]| run_interruptible(root, args, cancelled);
    if let Some(base) = &base {
        let mut args = vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--name-only",
            "-z",
            base.as_str(),
        ];
        if let Some(head) = &head {
            args.push(head);
        }
        args.push("--");
        for bytes in run(&root, &args)?
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
        {
            changed.insert(
                std::str::from_utf8(bytes)
                    .map_err(|_| "unsupported changed filename")?
                    .to_string(),
            );
        }
    }
    if base.is_none() {
        if let Some(head) = &head {
            for path in run(
                &root,
                &["ls-tree", "-r", "-z", "--name-only", "--full-tree", head],
            )?
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            {
                changed.insert(
                    std::str::from_utf8(path)
                        .map_err(|_| "unsupported initial filename")?
                        .to_string(),
                );
            }
        }
    }
    if head.is_none() {
        for bytes in run(&root, &["ls-files", "--others", "--exclude-standard", "-z"])?
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
        {
            changed.insert(
                std::str::from_utf8(bytes)
                    .map_err(|_| "unsupported untracked filename")?
                    .to_string(),
            );
        }
        if base.is_none() {
            changed.extend(inventory(&root, cancelled)?.unwrap_or_default());
        }
    }
    let scopes = request
        .scopes
        .iter()
        .map(|s| super::snapshot::scope(&root, s))
        .collect::<Result<Vec<_>, _>>()?;
    changed.retain(|path| {
        scopes.is_empty()
            || scopes
                .iter()
                .any(|scope| Path::new(path).starts_with(scope))
    });
    for path in &changed {
        relative_scope(path)?;
    }
    if changed.len() > config.max_inventory_entries {
        return Err("changed inventory exceeds configured limit".into());
    }
    let existing: Vec<_> = changed
        .iter()
        .filter(|path| std::fs::symlink_metadata(root.join(path)).is_ok())
        .cloned()
        .collect();
    let mut snapshot = if head.is_none() && (!existing.is_empty() || include_unchanged) {
        let mut worktree = request.clone();
        worktree.selection = Selection::Worktree;
        worktree.scopes = if include_unchanged {
            Vec::new()
        } else {
            existing
        };
        Snapshot::capture_targets_interruptible(&root, &worktree, config, cancelled)?
    } else {
        Snapshot::default()
    };
    if head.is_none() {
        let conflicts = conflicts(&root, cancelled)?
            .into_iter()
            .filter(|stage| include_unchanged || changed.contains(&stage.path))
            .collect();
        super::conflict_capture::capture(&mut snapshot, &root, conflicts, config, cancelled)?;
    }
    let mut supporting_entries = 0usize;
    for (side, revision) in [("base", base.as_ref()), ("head", head.as_ref())] {
        let Some(revision) = revision else {
            continue;
        };
        let tree = run(&root, &["ls-tree", "-r", "-z", "--full-tree", revision])?;
        for record in tree.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            if cancelled() {
                return Err("security Git capture cancelled".into());
            }
            let record = std::str::from_utf8(record).map_err(|_| "unsupported tree encoding")?;
            let (identity, path) = record.split_once('\t').ok_or("invalid tree entry")?;
            if !include_unchanged && !changed.contains(path) {
                continue;
            }
            if include_unchanged {
                supporting_entries += 1;
                if supporting_entries > config.max_inventory_entries
                    || snapshot.source_count() >= config.max_inventory_entries
                {
                    snapshot.skip(".", "supporting revision inventory limit reached");
                    break;
                }
            }
            let relative = relative_scope(path)?;
            let fields = identity.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 {
                return Err("invalid tree identity".into());
            }
            if !["100644", "100755"].contains(&fields[0]) || fields[1] != "blob" {
                snapshot.skip(path, "non-regular revision entry");
                continue;
            }
            if super::snapshot::denied(&relative)
                || relative
                    .components()
                    .any(|part| super::snapshot::excluded_dir(&part.as_os_str().to_string_lossy()))
            {
                snapshot.skip(path, "excluded revision source");
                continue;
            }
            if snapshot.source_count() >= config.max_inventory_entries {
                snapshot.skip(path, "revision source inventory limit");
                continue;
            }
            let size = run(&root, &["cat-file", "-s", fields[2]])?;
            let size: u64 = std::str::from_utf8(&size)
                .map_err(|_| "invalid object size")?
                .trim()
                .parse()
                .map_err(|_| "invalid object size")?;
            let limit = if relative
                .file_name()
                .is_some_and(|name| name == "SECURITY.md")
            {
                config.max_policy_bytes.min(config.max_file_bytes)
            } else {
                config.max_file_bytes
            };
            if size > limit || snapshot.bytes.saturating_add(size) > config.max_snapshot_bytes {
                snapshot.skip(path, "revision source exceeds byte limit");
                continue;
            }
            let bytes = run(&root, &["cat-file", "blob", fields[2]])?;
            if bytes.len() as u64 != size {
                return Err("revision object size changed".into());
            }
            let text = match String::from_utf8(bytes) {
                Ok(text)
                    if !(text.contains('\0')
                        || text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----")) =>
                {
                    text
                }
                _ => {
                    snapshot.skip(
                        path,
                        "binary, unsupported encoding, or private key material",
                    );
                    continue;
                }
            };
            snapshot.bytes += text.len() as u64;
            let file = SourceFile {
                hash: super::sha256_hex(text.as_bytes()),
                text,
            };
            if side == "base" {
                snapshot.base_files.insert(path.into(), file);
            } else {
                snapshot.files.insert(path.into(), file);
            }
        }
    }
    snapshot.revisions = Some((
        base.unwrap_or_else(|| if head.is_some() { "empty" } else { "unborn" }.into()),
        head.unwrap_or_else(|| "worktree".into()),
    ));
    snapshot.id.clear();
    snapshot.id = super::sha256_hex(
        &serde_json::to_vec(&(&snapshot, request, config))
            .map_err(|_| "cannot encode snapshot identity")?,
    );
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::super::{command::ScanCommand, ScanConfig};
    use super::*;

    #[test]
    fn security_supporting_revision_bytes_match_the_selected_sides() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]).unwrap();
        std::fs::write(dir.path().join("entry.rs"), "old entry\n").unwrap();
        std::fs::write(dir.path().join("helper.rs"), "committed helper\n").unwrap();
        commit(dir.path());
        std::fs::write(dir.path().join("entry.rs"), "new entry\n").unwrap();
        commit(dir.path());
        std::fs::write(dir.path().join("helper.rs"), "dirty helper\n").unwrap();
        let diff = super::super::snapshot::Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("--diff --scope entry.rs").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.base_files.len(), 1);
        assert_eq!(
            diff.read_side("helper.rs", 1, 1, "head").unwrap(),
            "committed helper"
        );
        assert_eq!(
            diff.read_side("helper.rs", 1, 1, "base").unwrap(),
            "committed helper"
        );
        assert!(!diff.is_target("helper.rs", "head"));
        std::fs::write(dir.path().join("entry.rs"), "worktree entry\n").unwrap();
        let changed = super::super::snapshot::Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("--changed --scope entry.rs").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert_eq!(changed.files.len(), 1);
        assert_eq!(
            changed.read_side("helper.rs", 1, 1, "worktree").unwrap(),
            "dirty helper"
        );
        assert_eq!(
            changed.read_side("helper.rs", 1, 1, "base").unwrap(),
            "committed helper"
        );
    }

    fn commit(root: &Path) {
        run(root, &["add", "--all"]).unwrap();
        run(
            root,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--no-verify",
                "-qm",
                "fixture",
            ],
        )
        .unwrap();
    }

    #[test]
    fn security_changed_includes_staged_unstaged_untracked_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]).unwrap();
        for name in ["staged.rs", "unstaged.rs", "deleted.rs"] {
            std::fs::write(dir.path().join(name), "before\n").unwrap();
        }
        std::fs::write(dir.path().join(".gitignore"), "ignored.rs\n").unwrap();
        commit(dir.path());
        std::fs::write(dir.path().join("staged.rs"), "staged\n").unwrap();
        run(dir.path(), &["add", "staged.rs"]).unwrap();
        std::fs::write(dir.path().join("unstaged.rs"), "unstaged\n").unwrap();
        std::fs::write(dir.path().join("new.rs"), "untracked\n").unwrap();
        std::fs::write(dir.path().join("ignored.rs"), "ignored private material\n").unwrap();
        std::fs::remove_file(dir.path().join("deleted.rs")).unwrap();
        let snapshot = capture_revision(
            dir.path(),
            &ScanCommand::parse("--changed").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert_eq!(snapshot.files["staged.rs"].text, "staged\n");
        assert_eq!(snapshot.files["unstaged.rs"].text, "unstaged\n");
        assert_eq!(snapshot.files["new.rs"].text, "untracked\n");
        assert!(!snapshot.files.contains_key("deleted.rs"));
        assert!(!snapshot.files.contains_key("ignored.rs"));
        assert_eq!(snapshot.base_files["deleted.rs"].text, "before\n");
    }

    #[test]
    fn security_diff_preserves_both_sides_and_ignores_dirty_bytes() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]).unwrap();
        std::fs::write(dir.path().join("old.rs"), "committed source\n").unwrap();
        commit(dir.path());
        std::fs::rename(dir.path().join("old.rs"), dir.path().join("new.rs")).unwrap();
        commit(dir.path());
        std::fs::write(dir.path().join("new.rs"), "dirty bytes\n").unwrap();
        let snapshot = capture_revision(
            dir.path(),
            &ScanCommand::parse("--diff").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert_eq!(snapshot.base_files["old.rs"].text, "committed source\n");
        assert_eq!(snapshot.files["new.rs"].text, "committed source\n");
        assert_eq!(snapshot.current_side(), "head");
        assert!(snapshot.read_side("new.rs", 1, 1, "worktree").is_err());
    }

    #[test]
    fn security_diff_preserves_base_head_and_rename_locations() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]).unwrap();
        std::fs::write(dir.path().join("old.rs"), "committed source\n").unwrap();
        commit(dir.path());
        std::fs::rename(dir.path().join("old.rs"), dir.path().join("new.rs")).unwrap();
        commit(dir.path());
        std::fs::write(dir.path().join("new.rs"), "dirty bytes\n").unwrap();
        let snapshot = capture_revision(
            dir.path(),
            &ScanCommand::parse("--diff").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert_eq!(snapshot.base_files["old.rs"].text, "committed source\n");
        assert_eq!(snapshot.files["new.rs"].text, "committed source\n");
        assert_eq!(snapshot.current_side(), "head");
        assert!(snapshot.read_side("old.rs", 1, 1, "base").is_ok());
        assert!(snapshot.read_side("new.rs", 1, 1, "head").is_ok());
        assert!(snapshot.read_side("new.rs", 1, 1, "worktree").is_err());
    }

    #[test]
    fn security_changed_unborn_repository_has_empty_baseline() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]).unwrap();
        std::fs::write(dir.path().join("new.rs"), "new\n").unwrap();
        let snapshot = capture_revision(
            dir.path(),
            &ScanCommand::parse("--changed").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert!(snapshot.base_files.is_empty());
        assert_eq!(snapshot.files.len(), 1);
    }

    #[test]
    fn security_changed_corrupt_head_is_not_an_unborn_repository() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]).unwrap();
        std::fs::write(dir.path().join("new.rs"), "source\n").unwrap();
        commit(dir.path());
        let branch = run(dir.path(), &["symbolic-ref", "HEAD"]).unwrap();
        let branch = std::str::from_utf8(&branch).unwrap().trim();
        std::fs::write(
            dir.path().join(".git").join(branch),
            format!("{}\n", "1".repeat(40)),
        )
        .unwrap();
        assert!(capture_revision(
            dir.path(),
            &ScanCommand::parse("--changed").unwrap(),
            &ScanConfig::default()
        )
        .is_err());
    }

    #[test]
    fn security_diff_initial_commit_uses_empty_baseline() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]).unwrap();
        std::fs::write(dir.path().join("new.rs"), "committed\n").unwrap();
        commit(dir.path());
        let snapshot = capture_revision(
            dir.path(),
            &ScanCommand::parse("--diff").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert!(snapshot.base_files.is_empty());
        assert_eq!(snapshot.files["new.rs"].text, "committed\n");
        assert_eq!(snapshot.revisions.as_ref().unwrap().0, "empty");
    }

    #[test]
    fn security_git_cancellation_prevents_process_start() {
        let dir = tempfile::tempdir().unwrap();
        let result = run_status_interruptible(dir.path(), &["init", "-q"], &|| true);
        assert_eq!(result.unwrap_err(), "security Git capture cancelled");
        assert!(!dir.path().join(".git").exists());
    }

    #[test]
    fn security_process_output_is_bounded_without_newlines() {
        let over = std::io::Cursor::new(vec![b'x'; 16 * 1024 * 1024 + 1]);
        assert_eq!(
            bounded_read(over, 16 * 1024 * 1024).unwrap_err(),
            "process output exceeded byte limit"
        );
        let exact = std::io::Cursor::new(vec![b'y'; 64]);
        assert_eq!(bounded_read(exact, 64).unwrap().len(), 64);
        assert!(!bounded_read(std::io::Cursor::new(vec![b'z'; 8]), 8)
            .unwrap()
            .contains(&b'\n'));
    }

    #[test]
    fn security_cancel_stops_descendants_on_supported_platform() {
        let mut child = if cfg!(windows) {
            Command::new("cmd")
                .args(["/C", "ping", "-n", "20", "127.0.0.1"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 20"]);
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        };
        terminate_child(&mut child);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn security_unresolved_merge_preserves_stage_identities_and_defers_coverage() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q"]).unwrap();
        std::fs::write(dir.path().join("source.rs"), "base\n").unwrap();
        std::fs::write(dir.path().join("second.rs"), "base\n").unwrap();
        commit(dir.path());
        run(dir.path(), &["checkout", "-qb", "other"]).unwrap();
        std::fs::write(dir.path().join("source.rs"), "theirs\n").unwrap();
        std::fs::write(dir.path().join("second.rs"), "theirs\n").unwrap();
        commit(dir.path());
        run(dir.path(), &["checkout", "-qb", "ours", "HEAD^1"]).unwrap();
        std::fs::write(dir.path().join("source.rs"), "ours\n").unwrap();
        std::fs::write(dir.path().join("second.rs"), "ours\n").unwrap();
        commit(dir.path());
        assert!(run(
            dir.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "merge",
                "--no-commit",
                "other"
            ]
        )
        .is_err());
        for selection in ["", "--changed"] {
            let snapshot = super::super::snapshot::Snapshot::capture(
                dir.path(),
                &ScanCommand::parse(selection).unwrap(),
                &ScanConfig::default(),
            )
            .unwrap();
            assert_eq!(snapshot.conflicts.len(), 6);
            assert_eq!(snapshot.conflict_sources.len(), 6);
            for (side, expected) in [
                ("index-base", "base"),
                ("index-ours", "ours"),
                ("index-theirs", "theirs"),
            ] {
                assert_eq!(
                    snapshot.read_side("source.rs", 1, 1, side).unwrap(),
                    expected
                );
                assert!(snapshot.is_target("source.rs", side));
                assert_eq!(
                    super::super::tools::execute(
                        &snapshot,
                        "sec_source_read",
                        serde_json::json!({
                            "path":"source.rs","startLine":1,"endLine":1,"snapshotSide":side
                        })
                    )
                    .unwrap()["text"],
                    expected
                );
            }
            assert_eq!(
                snapshot
                    .conflicts
                    .iter()
                    .map(|stage| stage.stage)
                    .collect::<Vec<_>>(),
                [1, 2, 3, 1, 2, 3]
            );
            assert!(snapshot
                .skipped
                .iter()
                .any(|skip| skip.path == "source.rs" && skip.reason.contains("unresolved merge")));
            let agent = tempfile::tempdir().unwrap();
            let store = super::super::store::Store::open(
                agent.path(),
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                true,
            )
            .unwrap();
            store
                .checkpoint(&ScanCommand::parse(selection).unwrap(), &snapshot)
                .unwrap();
            let saved = store.load(1024 * 1024).unwrap().snapshot;
            assert_eq!(
                serde_json::to_value(&saved).unwrap(),
                serde_json::to_value(&snapshot).unwrap()
            );
            let inventory = super::super::tools::execute(
                &saved,
                "sec_source_list",
                serde_json::json!({"offset":0}),
            )
            .unwrap();
            assert_eq!(inventory["total"], saved.source_count());
            let stage = inventory["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["path"] == "source.rs" && row["snapshotSide"] == "index-theirs")
                .unwrap();
            assert_eq!(stage["indexIdentity"]["stage"], 3);
            assert_eq!(stage["contentHash"], super::super::sha256_hex(b"theirs\n"));
        }
        for supporting_reads in [false, true] {
            let config = ScanConfig {
                supporting_reads,
                ..ScanConfig::default()
            };
            let scoped = super::super::snapshot::Snapshot::capture(
                dir.path(),
                &ScanCommand::parse("--scope source.rs").unwrap(),
                &config,
            )
            .unwrap();
            assert!(scoped.is_target("source.rs", "index-ours"));
            assert!(!scoped.is_target("second.rs", "index-ours"));
            assert_eq!(
                scoped.file("second.rs", "index-ours").is_ok(),
                supporting_reads
            );
            config.validate_snapshot(&scoped).unwrap();
            if supporting_reads {
                assert!(ScanConfig {
                    supporting_reads: false,
                    ..config.clone()
                }
                .validate_snapshot(&scoped)
                .is_err());
            }
            let agent = tempfile::tempdir().unwrap();
            let store = super::super::store::Store::open(
                agent.path(),
                dir.path(),
                &uuid::Uuid::new_v4().to_string(),
                true,
            )
            .unwrap();
            store
                .checkpoint(&ScanCommand::parse("--scope source.rs").unwrap(), &scoped)
                .unwrap();
            assert_eq!(
                store.load(1024 * 1024).unwrap().snapshot.bytes,
                scoped.bytes
            );
        }
        let limited_config = ScanConfig {
            max_inventory_entries: 2,
            supporting_reads: false,
            ..ScanConfig::default()
        };
        let limited = super::super::snapshot::Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("--changed --scope source.rs").unwrap(),
            &limited_config,
        )
        .unwrap();
        limited_config.validate_snapshot(&limited).unwrap();
        assert_eq!(limited.source_count(), 2);
        assert!(limited
            .skipped
            .iter()
            .any(|row| row.reason == "revision source inventory limit"));
        std::fs::remove_file(dir.path().join("source.rs")).unwrap();
        let deleted = capture_revision(
            dir.path(),
            &ScanCommand::parse("--changed").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert_eq!(deleted.conflicts.len(), 6);
        assert_eq!(deleted.conflict_sources.len(), 6);
        assert_eq!(
            deleted
                .read_side("source.rs", 1, 1, "index-theirs")
                .unwrap(),
            "theirs"
        );
        assert!(deleted
            .skipped
            .iter()
            .any(|skip| skip.reason.contains("unresolved merge")));
    }
}
