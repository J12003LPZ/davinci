//! Bounded observations of shell edits. This is freshness evidence, not a sandbox.
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Wall-clock bound on one snapshot. Unit tests get a generous bound so a
/// loaded CI runner cannot truncate a tiny workspace and silently drop a
/// verification the test relies on; budget exhaustion itself is covered by
/// an explicit `Duration::ZERO` capture.
const CAPTURE_BUDGET: Duration = if cfg!(test) {
    Duration::from_secs(10)
} else {
    Duration::from_millis(100)
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct FileStamp {
    // A dangling symlink and a present empty target have different identities.
    len: Option<u64>,
    digest: [u8; 32],
    readonly: bool,
    link: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    files: BTreeMap<PathBuf, Option<FileStamp>>,
    complete: bool,
    required_complete: bool,
    required_paths: Vec<PathBuf>,
}

impl Snapshot {
    #[cfg(test)]
    pub(crate) fn capture(root: &Path) -> Self {
        Self::capture_inputs(root, &[])
    }

    #[cfg(test)]
    pub(crate) fn limited_for_test(root: &Path, paths: &[PathBuf]) -> Self {
        Self::capture_with_limits(root, paths, 0, Duration::from_secs(1))
    }

    pub(crate) fn capture_inputs(root: &Path, paths: &[PathBuf]) -> Self {
        Self::capture_with_limits(root, paths, 16_384, CAPTURE_BUDGET)
    }

    pub(crate) fn capture_guarded(
        root: &Path,
        paths: &[PathBuf],
        allowed: &dyn Fn(&Path) -> bool,
    ) -> Self {
        Self::capture_with_guard(root, paths, 16_384, CAPTURE_BUDGET, allowed)
    }

    pub(crate) fn complete(&self) -> bool {
        self.complete
    }

    pub(crate) fn fingerprint(&self) -> Option<String> {
        self.complete
            .then(|| serde_json::to_vec(&self.files).ok())
            .flatten()
            .map(|bytes| format!("{:x}", davinci_sys::hex::Lower(&Sha256::digest(bytes))))
    }

    pub(crate) fn required_paths(&self) -> &[PathBuf] {
        &self.required_paths
    }

    pub(crate) fn required_inputs_observed(&self, after: &Self) -> bool {
        self.required_complete && after.required_complete
    }

    fn capture_with_limits(
        root: &Path,
        paths: &[PathBuf],
        max_entries: usize,
        budget: Duration,
    ) -> Self {
        Self::capture_with_guard(root, paths, max_entries, budget, &|_| true)
    }

    fn capture_with_guard(
        root: &Path,
        paths: &[PathBuf],
        max_entries: usize,
        budget: Duration,
        allowed: &dyn Fn(&Path) -> bool,
    ) -> Self {
        let start = Instant::now();
        let mut snapshot = Self {
            files: BTreeMap::new(),
            complete: true,
            required_complete: true,
            required_paths: paths.to_vec(),
        };
        let mut remaining_bytes = 64 * 1024 * 1024;
        // Known inputs are always observed first, even under generated roots or
        // outside the bounded inventory. Missing files retain an explicit stamp.
        for path in paths {
            let absolute = if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            };
            let relative = absolute
                .strip_prefix(root)
                .unwrap_or(&absolute)
                .to_path_buf();
            if !allowed(&absolute) {
                snapshot.complete = false;
                snapshot.required_complete = false;
                continue;
            }
            match stamp(&absolute, start, budget, &mut remaining_bytes) {
                Ok(stamp) => {
                    snapshot.files.insert(relative, stamp);
                }
                Err(()) => {
                    snapshot.required_complete = false;
                    snapshot.complete = false;
                }
            }
        }
        // Compiler/interpreter caches and runtime journals are expected shell
        // outputs, not edits to project inputs. Never exclude tests/fixtures/docs.
        let entries = walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                // Python bytecode is always generated; other exclusions are rooted
                // in the workspace so src/target and fixtures/venv remain inputs.
                entry.depth() == 0
                    || (entry.file_name() != "__pycache__"
                        && !(entry.depth() == 1
                            && matches!(
                                entry.file_name().to_str(),
                                Some(
                                    ".git"
                                        | ".davinci-transactions"
                                        | ".davinci"
                                        | ".pi"
                                        | "node_modules"
                                        | "target"
                                        | ".venv"
                                        | "venv"
                                        | ".pytest_cache"
                                        | ".mypy_cache"
                                        | ".ruff_cache"
                                )
                            )))
            });
        for (index, entry) in entries.enumerate() {
            if index >= max_entries || start.elapsed() >= budget {
                snapshot.complete = false;
                break;
            }
            let Ok(entry) = entry else {
                snapshot.complete = false;
                continue;
            };
            if entry.file_type().is_dir() {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(root) else {
                snapshot.complete = false;
                continue;
            };
            if snapshot.files.contains_key(relative) {
                continue;
            }
            if !allowed(entry.path()) {
                snapshot.complete = false;
                continue;
            }
            match stamp(entry.path(), start, budget, &mut remaining_bytes) {
                Ok(stamp) => {
                    snapshot.files.insert(relative.to_path_buf(), stamp);
                }
                Err(()) => {
                    snapshot.complete = false;
                    break;
                }
            }
        }
        snapshot
    }

    pub(crate) fn path_changed(&self, after: &Self, path: &Path) -> Option<bool> {
        let before = self.files.get(path);
        let next = after.files.get(path);
        ((self.complete || before.is_some()) && (after.complete || next.is_some()))
            .then(|| before.and_then(Option::as_ref) != next.and_then(Option::as_ref))
    }

    pub(crate) fn path_is_new(&self, path: &Path) -> bool {
        (self.complete || self.files.contains_key(path))
            && self.files.get(path).and_then(Option::as_ref).is_none()
    }

    pub(crate) fn changes(&self, after: &Self) -> Vec<PathBuf> {
        let mut paths: Vec<_> = self
            .files
            .keys()
            .chain(after.files.keys())
            .filter(|path| {
                // A truncated inventory must not mistake an unobserved entry
                // for a deletion. Explicitly missing known inputs remain Some(None).
                let before = self.files.get(*path);
                let next = after.files.get(*path);
                (self.complete || before.is_some())
                    && (after.complete || next.is_some())
                    && before.and_then(Option::as_ref) != next.and_then(Option::as_ref)
            })
            .cloned()
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }
}

fn stamp(
    path: &Path,
    start: Instant,
    budget: Duration,
    remaining_bytes: &mut u64,
) -> Result<Option<FileStamp>, ()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    let link = if metadata.file_type().is_symlink() {
        Some(std::fs::read_link(path).map_err(|_| ())?)
    } else {
        None
    };
    let target = if link.is_some() {
        match std::fs::metadata(path) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(()),
        }
    } else {
        Some(metadata)
    };
    let mut digest = Sha256::new();
    if let Some(metadata) = target.as_ref() {
        // Fingerprint the source reached through a file symlink, including an
        // explicit input whose target is outside the inventory root. Never
        // open directories/devices/FIFOs or follow directory trees here.
        if !metadata.is_file() {
            return Err(());
        }
        if metadata.len() > *remaining_bytes {
            return Err(());
        }
        let mut file = std::fs::File::open(path).map_err(|_| ())?;
        let mut buffer = [0; 16_384];
        loop {
            if start.elapsed() >= budget {
                return Err(());
            }
            let count = file.read(&mut buffer).map_err(|_| ())?;
            if count == 0 {
                break;
            }
            *remaining_bytes = remaining_bytes.checked_sub(count as u64).ok_or(())?;
            digest.update(&buffer[..count]);
        }
    }
    Ok(Some(FileStamp {
        len: target.as_ref().map(|metadata| metadata.len()),
        digest: digest.finalize().into(),
        readonly: target
            .as_ref()
            .is_some_and(|metadata| metadata.permissions().readonly()),
        link,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observes_created_deleted_and_edited_inputs_but_ignores_python_cache() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("deleted.py"), "old").unwrap();
        std::fs::write(root.path().join("edited.py"), "old").unwrap();
        let before = Snapshot::capture(root.path());
        std::fs::remove_file(root.path().join("deleted.py")).unwrap();
        std::fs::write(root.path().join("edited.py"), "new contents").unwrap();
        std::fs::create_dir(root.path().join("fixtures")).unwrap();
        std::fs::write(root.path().join("fixtures/input.py"), "new").unwrap();
        std::fs::create_dir(root.path().join("__pycache__")).unwrap();
        std::fs::write(root.path().join("__pycache__/edited.pyc"), "cache").unwrap();
        assert_eq!(
            before.changes(&Snapshot::capture(root.path())),
            ["deleted.py", "edited.py", "fixtures/input.py"].map(PathBuf::from)
        );
    }

    #[test]
    fn exhausted_snapshot_is_explicitly_unknown() {
        let root = tempfile::tempdir().unwrap();
        let incomplete = Snapshot::capture_with_limits(root.path(), &[], 0, Duration::from_secs(1));
        assert!(!incomplete.complete());
        let timed_out = Snapshot::capture_with_limits(root.path(), &[], 100, Duration::ZERO);
        assert!(!timed_out.complete());
    }

    #[test]
    fn guarded_fixture_capture_tolerates_slow_permission_checks() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("app.py"), "original").unwrap();
        let snapshot = Snapshot::capture_guarded(root.path(), &[], &|_| {
            // Exercise the same scheduling delay as a loaded hosted runner.
            std::thread::sleep(Duration::from_millis(150));
            true
        });
        assert!(snapshot.complete());
        assert_eq!(
            snapshot.path_changed(&Snapshot::capture(root.path()), Path::new("app.py")),
            Some(false)
        );
    }

    #[test]
    fn content_identity_detects_same_length_edits_with_preserved_timestamp() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("changed.py");
        std::fs::write(&file, "one").unwrap();
        let old = std::fs::metadata(&file).unwrap().modified().unwrap();
        let before = Snapshot::capture(root.path());
        std::fs::write(&file, "two").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert_eq!(
            before.changes(&Snapshot::capture(root.path())),
            [PathBuf::from("changed.py")]
        );
    }

    #[test]
    fn nested_generated_names_and_explicit_ignored_paths_remain_observed() {
        let root = tempfile::tempdir().unwrap();
        let paths = [
            "src/target/parser.rs",
            "tests/fixtures/venv/module.py",
            "target/explicit.rs",
        ]
        .map(PathBuf::from);
        for path in &paths {
            std::fs::create_dir_all(root.path().join(path).parent().unwrap()).unwrap();
            std::fs::write(root.path().join(path), "one").unwrap();
        }
        let before = Snapshot::capture_inputs(root.path(), &paths[2..]);
        for path in &paths {
            std::fs::write(root.path().join(path), "two").unwrap();
        }
        let changed = before.changes(&Snapshot::capture_inputs(root.path(), &paths[2..]));
        assert_eq!(changed.len(), paths.len());
        for path in paths {
            assert!(changed.contains(&path));
        }
    }

    #[cfg(unix)]
    #[test]
    fn known_symlink_input_fingerprints_target_bytes_and_missing_targets() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let target = root.path().join("target.py");
        std::fs::write(&target, "one").unwrap();
        let alias = workspace.join("alias.py");
        std::os::unix::fs::symlink("../target.py", &alias).unwrap();
        let paths = [PathBuf::from("alias.py")];
        let before = Snapshot::capture_inputs(&workspace, &paths);
        std::fs::write(&alias, "two").unwrap();
        let after = Snapshot::capture_inputs(&workspace, &paths);
        assert!(before.required_inputs_observed(&after));
        assert_eq!(before.changes(&after), paths);
        std::fs::remove_file(&target).unwrap();
        let deleted = Snapshot::capture_inputs(&workspace, &paths);
        assert!(deleted.complete());
        assert_eq!(after.changes(&deleted), paths);
        assert!(deleted
            .changes(&Snapshot::capture_inputs(&workspace, &paths))
            .is_empty());
        std::fs::write(&target, "").unwrap();
        let empty = Snapshot::capture_inputs(&workspace, &paths);
        assert!(empty.complete());
        assert!(deleted.required_inputs_observed(&empty));
        assert_eq!(deleted.changes(&empty), paths);
        std::fs::remove_file(&target).unwrap();
        let empty_deleted = Snapshot::capture_inputs(&workspace, &paths);
        assert!(empty_deleted.complete());
        assert!(empty.required_inputs_observed(&empty_deleted));
        assert_eq!(empty.changes(&empty_deleted), paths);
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink("alias.py", &alias).unwrap();
        let cycle = Snapshot::capture_inputs(&workspace, &paths);
        assert!(!cycle.required_inputs_observed(&cycle));
    }
}
