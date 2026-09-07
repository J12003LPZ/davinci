//! Immutable source access for native security reviews; no TypeScript equivalent.

use super::command::{ScanCommand, Selection};
use super::config::ScanConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceFile {
    pub hash: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConflictStage {
    pub path: String,
    pub stage: u8,
    pub object_id: String,
    pub mode: String,
}

impl ConflictStage {
    pub fn side(&self) -> &'static str {
        match self.stage {
            1 => "index-base",
            2 => "index-ours",
            3 => "index-theirs",
            _ => "invalid-index-stage",
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.path.is_empty()
            || self.path.starts_with('/')
            || self.path.contains(['\\', ':'])
            || self
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || !(1..=3).contains(&self.stage)
            || !["100644", "100755", "120000", "160000"].contains(&self.mode.as_str())
            || ![40, 64].contains(&self.object_id.len())
            || !self
                .object_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || self.object_id.bytes().all(|byte| byte == b'0')
        {
            return Err("invalid conflict stage identity".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConflictSource {
    pub identity: ConflictStage,
    pub source: SourceFile,
    pub supporting: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    pub id: String,
    pub files: BTreeMap<String, SourceFile>,
    pub skipped: Vec<SkippedFile>,
    pub bytes: u64,
    #[serde(default)]
    pub base_files: BTreeMap<String, SourceFile>,
    #[serde(default)]
    pub revisions: Option<(String, String)>,
    #[serde(default)]
    pub conflicts: Vec<ConflictStage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflict_sources: Vec<ConflictSource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub supporting_files: BTreeMap<String, SourceFile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub supporting_base_files: BTreeMap<String, SourceFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supporting_skipped: Vec<SkippedFile>,
}

const MAX_PATH_COMPONENT: usize = 255;

pub(super) fn portable_relative(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
        || value.chars().any(char::is_control)
    {
        return Err("scope must be a portable repository-relative path".into());
    }
    for component in value.split('/') {
        let lower = component.to_ascii_lowercase();
        let stem = lower.split('.').next().unwrap_or("");
        if component.is_empty()
            || component.len() > MAX_PATH_COMPONENT
            || [".", "..", ".git"].contains(&lower.as_str())
            || component.ends_with(['.', ' '])
            || ["con", "prn", "aux", "nul"].contains(&stem)
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && matches!(stem.as_bytes().get(3).copied(), Some(b'1'..=b'9')))
        {
            return Err("scope is not a portable repository-relative path".into());
        }
    }
    Ok(())
}

pub fn relative_scope(value: &str) -> Result<PathBuf, String> {
    if value == "." {
        return Ok(PathBuf::new());
    }
    let normalized = value.replace('\\', "/");
    portable_relative(&normalized)?;
    let mut path = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => (),
            _ => return Err("scope escapes repository".into()),
        }
    }
    Ok(path)
}

pub(super) fn scope(root: &Path, value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.is_absolute() {
        if value.starts_with("\\\\")
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return Err("UNC and parent traversal scopes are denied".into());
        }
        let resolved = path
            .canonicalize()
            .map_err(|_| "cannot resolve absolute scope")?;
        let relative = resolved
            .strip_prefix(root)
            .map_err(|_| "absolute scope is outside repository")?;
        return Ok(relative.to_path_buf());
    }
    relative_scope(value)
}

fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(super) fn denied(path: &Path) -> bool {
    path.components().any(|component| {
        let part = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        part == ".env"
            || part.starts_with(".env.")
            || [
                "credentials",
                "credentials.json",
                "auth.json",
                "id_rsa",
                "id_ed25519",
            ]
            .contains(&part.as_str())
            || part.ends_with(".pem")
            || part.ends_with(".p12")
            || part.ends_with(".pfx")
            || part.ends_with(".key")
    })
}

pub(super) fn excluded_dir(name: &str) -> bool {
    [
        ".git",
        "target",
        "node_modules",
        ".davinci",
        ".pi",
        ".venv",
        "__pycache__",
        "dist",
        "build",
        "security-scans",
    ]
    .contains(&name)
}

impl Snapshot {
    #[cfg(test)]
    pub fn capture(
        root: &Path,
        command: &ScanCommand,
        config: &ScanConfig,
    ) -> Result<Self, String> {
        Self::capture_interruptible(root, command, config, &|| false)
    }

    pub(super) fn capture_interruptible(
        root: &Path,
        command: &ScanCommand,
        config: &ScanConfig,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        let target = Self::capture_targets_interruptible(root, command, config, cancelled)?;
        super::supporting::capture(root, command, config, target, cancelled)
    }

    pub(super) fn capture_targets_interruptible(
        root: &Path,
        command: &ScanCommand,
        config: &ScanConfig,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        if cancelled() {
            return Err("security snapshot cancelled".into());
        }
        config.validate()?;
        if command.selection != Selection::Worktree {
            return super::git::capture_revision_interruptible(root, command, config, cancelled);
        }
        let root = root
            .canonicalize()
            .map_err(|_| "cannot resolve repository root")?;
        let inventory = super::git::inventory(&root, cancelled)?;
        let scopes = command
            .scopes
            .iter()
            .map(|s| scope(&root, s))
            .collect::<Result<Vec<_>, _>>()?;
        if scopes
            .iter()
            .any(|scope| fs::symlink_metadata(root.join(scope)).is_err())
        {
            return Err("requested worktree scope does not exist".into());
        }
        let mut snapshot = Self {
            id: String::new(),
            files: BTreeMap::new(),
            skipped: Vec::new(),
            bytes: 0,
            ..Default::default()
        };
        let mut enumerated = 0;
        for entry in walkdir::WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                if entry.depth() == 0 {
                    return true;
                }
                if entry.file_type().is_dir() && excluded_dir(&entry.file_name().to_string_lossy())
                {
                    return false;
                }
                if entry.depth() > 1 {
                    if let Some(parent) = entry.path().parent() {
                        if let Ok(metadata) = fs::symlink_metadata(parent) {
                            if linked(&metadata) {
                                return false;
                            }
                        }
                    }
                }
                true
            })
        {
            if cancelled() {
                return Err("security snapshot cancelled".into());
            }
            let entry = entry.map_err(|_| "repository enumeration failed")?;
            if entry.depth() == 0 {
                continue;
            }
            enumerated += 1;
            if enumerated > config.max_inventory_entries {
                snapshot.skip(".", "inventory limit reached; remaining scope deferred");
                break;
            }
            let relative = entry
                .path()
                .strip_prefix(&root)
                .map_err(|_| "entry outside root")?;
            if !scopes.is_empty()
                && !scopes
                    .iter()
                    .any(|s| relative.starts_with(s) || s.as_os_str().is_empty())
            {
                continue;
            }
            let Some(path) = relative.to_str().map(|s| s.replace('\\', "/")) else {
                return Err("repository path has unsupported encoding".into());
            };
            if portable_relative(&path).is_err() {
                snapshot.skip(&path, "non-portable path");
                continue;
            }
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(|_| "cannot inspect source metadata")?;
            if linked(&metadata) {
                snapshot.skip(&path, "symlink or reparse point");
                continue;
            }
            if metadata.is_dir() {
                continue;
            }
            if inventory
                .as_ref()
                .is_some_and(|paths| !paths.contains(&path))
            {
                continue;
            }
            if denied(relative) {
                snapshot.skip(&path, "credential path denied");
                continue;
            }
            if !metadata.is_file() {
                snapshot.skip(&path, "not a regular file");
                continue;
            }
            let limit = if entry.file_name() == "SECURITY.md" {
                config.max_policy_bytes
            } else {
                config.max_file_bytes
            };
            if metadata.len() > limit {
                snapshot.skip(&path, "file size limit");
                continue;
            }
            let canonical = entry
                .path()
                .canonicalize()
                .map_err(|_| "cannot resolve source")?;
            if !canonical.starts_with(&root) {
                return Err("source escaped repository".into());
            }
            let file = match open_confined(&root, relative) {
                Ok(file) => file,
                Err(_) => {
                    snapshot.skip(&path, "unreadable source");
                    continue;
                }
            };
            let before = file
                .metadata()
                .map_err(|_| "cannot inspect opened source")?;
            let mut bytes = Vec::new();
            (&file)
                .take(limit.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|_| "source read failed")?;
            let after = file
                .metadata()
                .map_err(|_| "cannot recheck opened source")?;
            if before.len() != after.len()
                || before.modified().ok() != after.modified().ok()
                || bytes.len() as u64 != after.len()
                || bytes.len() as u64 > limit
            {
                snapshot.skip(&path, "source changed during capture or exceeded limit");
                continue;
            }
            if snapshot.bytes.saturating_add(bytes.len() as u64) > config.max_snapshot_bytes {
                snapshot.skip(&path, "snapshot byte limit");
                continue;
            }
            if bytes.contains(&0) {
                snapshot.skip(&path, "binary source");
                continue;
            }
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => {
                    snapshot.skip(&path, "unsupported encoding");
                    continue;
                }
            };
            if text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----") {
                snapshot.skip(
                    &path,
                    "private-key-shaped material withheld; local lead only",
                );
                continue;
            }
            snapshot.bytes += text.len() as u64;
            snapshot.files.insert(
                path,
                SourceFile {
                    hash: super::sha256_hex(text.as_bytes()),
                    text,
                },
            );
        }
        if snapshot.files.is_empty() && !scopes.is_empty() {
            snapshot.skip(".", "requested scope contains no eligible files");
        }
        let conflicts = super::git::conflicts(&root, cancelled)?
            .into_iter()
            .filter(|stage| {
                scopes.is_empty()
                    || scopes
                        .iter()
                        .any(|scope| Path::new(&stage.path).starts_with(scope))
            })
            .collect();
        super::conflict_capture::capture(&mut snapshot, &root, conflicts, config, cancelled)?;
        let mut digest = Sha256::new();
        digest.update(
            serde_json::to_vec(&snapshot.conflicts)
                .map_err(|_| "cannot encode conflict identities")?,
        );
        digest.update(
            serde_json::to_vec(&snapshot.conflict_sources)
                .map_err(|_| "cannot encode conflict sources")?,
        );
        digest.update(
            serde_json::to_vec(&(command, config))
                .map_err(|_| "cannot encode snapshot policy identity")?,
        );
        for (path, file) in &snapshot.files {
            digest.update((path.len() as u64).to_le_bytes());
            digest.update(path.as_bytes());
            digest.update(file.hash.as_bytes());
        }
        for skipped in &snapshot.skipped {
            digest.update((skipped.path.len() as u64).to_le_bytes());
            digest.update(skipped.path.as_bytes());
            digest.update((skipped.reason.len() as u64).to_le_bytes());
            digest.update(skipped.reason.as_bytes());
        }
        snapshot.id = format!("{:x}", digest.finalize());
        Ok(snapshot)
    }

    pub(super) fn skip(&mut self, path: &str, reason: &str) {
        self.skipped.push(SkippedFile {
            path: path.into(),
            reason: reason.into(),
        });
    }

    pub fn read(&self, path: &str, start: usize, end: usize) -> Result<String, String> {
        self.read_side(path, start, end, "current")
    }

    pub fn file(&self, path: &str, side: &str) -> Result<&SourceFile, String> {
        if ["index-base", "index-ours", "index-theirs"].contains(&side) {
            return self
                .conflict_source(path, side)
                .map(|source| &source.source);
        }
        if side != "current" && side != "base" && side != self.current_side() {
            return Err("evidence side does not match captured source".into());
        }
        let files = match side {
            "base" => &self.base_files,
            "current" | "worktree" | "head" => &self.files,
            _ => return Err("unknown snapshot side".into()),
        };
        files
            .get(path)
            .or_else(|| {
                if side == "base" {
                    self.supporting_base_files.get(path)
                } else {
                    self.supporting_files.get(path)
                }
            })
            .ok_or_else(|| "source is not in the authorized snapshot".into())
    }

    pub fn sources(&self) -> impl Iterator<Item = (&str, &str, &SourceFile)> {
        self.files
            .iter()
            .map(|(path, file)| (self.current_side(), path.as_str(), file))
            .chain(
                self.base_files
                    .iter()
                    .map(|(path, file)| ("base", path.as_str(), file)),
            )
            .chain(
                self.supporting_files
                    .iter()
                    .map(|(path, file)| (self.current_side(), path.as_str(), file)),
            )
            .chain(
                self.supporting_base_files
                    .iter()
                    .map(|(path, file)| ("base", path.as_str(), file)),
            )
            .chain(self.conflict_sources.iter().map(|source| {
                (
                    source.identity.side(),
                    source.identity.path.as_str(),
                    &source.source,
                )
            }))
    }

    pub fn source_count(&self) -> usize {
        self.files.len()
            + self.base_files.len()
            + self.supporting_files.len()
            + self.supporting_base_files.len()
            + self.conflict_sources.len()
    }

    pub fn is_target(&self, path: &str, side: &str) -> bool {
        if side == "base" {
            self.base_files.contains_key(path)
        } else if side == "current" || side == self.current_side() {
            self.files.contains_key(path)
        } else {
            self.conflict_source(path, side)
                .is_ok_and(|source| !source.supporting)
        }
    }

    pub(super) fn conflict_source(
        &self,
        path: &str,
        side: &str,
    ) -> Result<&ConflictSource, String> {
        let stage = match side {
            "index-base" => 1,
            "index-ours" => 2,
            "index-theirs" => 3,
            _ => return Err("unknown index side".into()),
        };
        // Capture sorts this vector; checkpoint recovery enforces that invariant.
        self.conflict_sources
            .binary_search_by(|source| {
                source
                    .identity
                    .path
                    .as_str()
                    .cmp(path)
                    .then(source.identity.stage.cmp(&stage))
            })
            .map(|index| &self.conflict_sources[index])
            .map_err(|_| "conflict source is not in the authorized snapshot".into())
    }

    pub fn current_side(&self) -> &'static str {
        if self
            .revisions
            .as_ref()
            .is_some_and(|(_, head)| head != "worktree")
        {
            "head"
        } else {
            "worktree"
        }
    }

    pub fn read_side(
        &self,
        path: &str,
        start: usize,
        end: usize,
        side: &str,
    ) -> Result<String, String> {
        relative_scope(path)?;
        if start == 0 || end < start || end - start >= 200 {
            return Err("source read requires 1..200 lines".into());
        }
        let file = self.file(path, side)?;
        let lines: Vec<_> = file.text.lines().collect();
        if end > lines.len() {
            return Err("source range is outside the file".into());
        }
        let result = lines[start - 1..end].join("\n");
        if result.len() > 64 * 1024 {
            return Err("source range exceeds byte limit; request fewer lines".into());
        }
        Ok(result)
    }
}

// Validate the opened object before reading, rather than trusting a path check
// that an attacker can invalidate between canonicalization and open.
#[cfg(windows)]
pub(super) fn open_confined(root: &Path, relative: &Path) -> std::io::Result<File> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFinalPathNameByHandleW(
            handle: *mut std::ffi::c_void,
            path: *mut u16,
            size: u32,
            flags: u32,
        ) -> u32;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(root.join(relative))?;
    if linked(&file.metadata()?) || !file.metadata()?.is_file() {
        return Err(std::io::Error::other(
            "source is not a regular non-reparse file",
        ));
    }
    let mut buffer = vec![0u16; 32768];
    // SAFETY: the handle belongs to the live File and the buffer is writable
    // for the advertised length; no ownership is transferred.
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    };
    if length == 0 || length as usize >= buffer.len() {
        return Err(std::io::Error::last_os_error());
    }
    let opened = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize]));
    if !opened.starts_with(root) || opened != root.join(relative) {
        return Err(std::io::Error::other(
            "opened source identity differs from authorized path",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
pub(super) fn open_confined(root: &Path, relative: &Path) -> std::io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    let mut directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)?;
    let components: Vec<_> = relative.components().collect();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(std::io::Error::other("invalid source component"));
        };
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| std::io::Error::other("invalid source name"))?;
        let last = index + 1 == components.len();
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if last { 0 } else { libc::O_DIRECTORY };
        // SAFETY: valid directory fd and NUL-terminated name; openat returns
        // a new owned descriptor which is immediately wrapped in File.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let opened = unsafe { File::from_raw_fd(fd) };
        if last {
            if !opened.metadata()?.is_file() {
                return Err(std::io::Error::other("not a regular source file"));
            }
            return Ok(opened);
        }
        directory = opened;
    }
    Err(std::io::Error::other("source path is empty"))
}

#[cfg(not(any(windows, unix)))]
pub(super) fn open_confined(_: &Path, _: &Path) -> std::io::Result<File> {
    Err(std::io::Error::other(
        "confined source access unavailable on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_scoped_snapshot_captures_support_without_expanding_targets() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("entry.rs"), "helper::check();\n").unwrap();
        fs::write(dir.path().join("helper.rs"), "fn check() {}\n").unwrap();
        fs::write(dir.path().join(".env"), "fixture withheld\n").unwrap();
        let command = ScanCommand::parse("--scope entry.rs").unwrap();
        let snapshot = Snapshot::capture(dir.path(), &command, &ScanConfig::default()).unwrap();
        assert_eq!(snapshot.files.keys().collect::<Vec<_>>(), ["entry.rs"]);
        assert_eq!(
            snapshot.supporting_files["helper.rs"].text,
            "fn check() {}\n"
        );
        assert!(!snapshot.supporting_files.contains_key(".env"));
        fs::write(dir.path().join("helper.rs"), "changed after snapshot\n").unwrap();
        assert_eq!(snapshot.read("helper.rs", 1, 1).unwrap(), "fn check() {}");
        let restricted = ScanConfig {
            supporting_reads: false,
            ..ScanConfig::default()
        };
        let restricted = Snapshot::capture(dir.path(), &command, &restricted).unwrap();
        assert!(restricted.supporting_files.is_empty());
        assert!(restricted.read("helper.rs", 1, 1).is_err());
    }

    #[test]
    fn security_snapshot_changes_when_bytes_change_at_same_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("code.rs"), "first\nsecond\n").unwrap();
        let config = ScanConfig::default();
        let command = ScanCommand::parse(".").unwrap();
        let first = Snapshot::capture(dir.path(), &command, &config).unwrap();
        std::fs::write(dir.path().join("code.rs"), "changed\nsecond\n").unwrap();
        let second = Snapshot::capture(dir.path(), &command, &config).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(first.read("code.rs", 1, 1).unwrap(), "first");
        assert!(first.read("../code.rs", 1, 1).is_err());
        assert!(first.read("code.rs", 0, 1).is_err());
        assert!(first.read("code.rs", 1, 201).is_err());
    }

    #[test]
    fn security_scope_rejects_parent_drive_unc_and_reparse_escape() {
        for value in [
            "../outside",
            "C:relative",
            r"\\server\share",
            r"\\?\C:\data",
            "src/file:stream",
            "src/../outside",
            "foo<.rs",
            "foo>.rs",
            "foo|.rs",
            "foo?.rs",
            "foo*.rs",
            "con.rs",
            "PRN.txt",
            "aux.rs",
            "nul.rs",
            "com1.rs",
            "lpt9.rs",
            "foo./x.rs",
            "foo /x.rs",
            ".git/config",
            &format!("{}.rs", "a".repeat(256)),
        ] {
            assert!(relative_scope(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn security_inventory_records_skips_and_does_not_copy_credentials() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "FIXTURE=not-a-real-secret").unwrap();
        std::fs::write(dir.path().join("binary.bin"), [0, 255]).unwrap();
        std::fs::write(dir.path().join("source.rs"), "fn main() {}\n").unwrap();
        let snapshot = Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert!(snapshot.read(".env", 1, 1).is_err());
        assert!(snapshot.skipped.iter().any(|row| row.path == ".env"));
        assert!(snapshot.skipped.iter().any(|row| row.path == "binary.bin"));
        assert!(snapshot.read("source.rs", 1, 1).is_ok());
    }

    #[test]
    fn security_scope_opened_regular_file_matches_authorized_identity() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("source.rs"), "fixture\n").unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(open_confined(&root, Path::new("source.rs")).is_ok());
        assert!(open_confined(&root, Path::new("missing.rs")).is_err());
    }

    #[test]
    fn security_scope_checks_final_file_identity() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("source.rs"), "fixture\n").unwrap();
        let snapshot = Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        let hash = super::super::sha256_hex(b"fixture\n");
        assert_eq!(snapshot.files["source.rs"].hash, hash);
        assert_eq!(snapshot.read("source.rs", 1, 1).unwrap(), "fixture");
        std::fs::write(dir.path().join("source.rs"), "changed after capture\n").unwrap();
        assert_eq!(snapshot.read("source.rs", 1, 1).unwrap(), "fixture");
        assert_eq!(snapshot.files["source.rs"].hash, hash);
        let root = dir.path().canonicalize().unwrap();
        assert!(open_confined(&root, Path::new("source.rs")).is_ok());
        assert!(open_confined(&root, Path::new("missing.rs")).is_err());
        assert!(snapshot.read("../source.rs", 1, 1).is_err());
    }

    #[test]
    fn security_inventory_records_every_skip_and_truncation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("keep.rs"), "fn keep() {}\n").unwrap();
        std::fs::write(dir.path().join("extra.rs"), "fn extra() {}\n").unwrap();
        let snapshot = Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("").unwrap(),
            &ScanConfig {
                max_inventory_entries: 1,
                ..ScanConfig::default()
            },
        )
        .unwrap();
        assert!(snapshot
            .skipped
            .iter()
            .any(|row| row.reason.contains("inventory limit")));
        assert!(snapshot.files.len() <= 1);
        std::fs::write(dir.path().join(".env"), "FIXTURE=not-a-real-secret").unwrap();
        std::fs::write(dir.path().join("binary.bin"), [0, 255]).unwrap();
        std::fs::write(dir.path().join("source.rs"), "fn main() {}\n").unwrap();
        let skipped = Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert!(skipped.read(".env", 1, 1).is_err());
        assert!(skipped.skipped.iter().any(|row| row.path == ".env"));
        assert!(skipped.skipped.iter().any(|row| row.path == "binary.bin"));
        assert!(skipped.read("source.rs", 1, 1).is_ok());
    }

    #[test]
    fn security_inventory_skips_windows_junctions_and_nonportable_names() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.rs"), "fn secret() {}\n").unwrap();
        std::fs::write(dir.path().join("keep.rs"), "fn keep() {}\n").unwrap();
        let link = dir.path().join("escape");
        #[cfg(windows)]
        let planted = std::os::windows::fs::symlink_dir(outside.path(), &link).is_ok()
            || std::process::Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    &link.to_string_lossy(),
                    &outside.path().to_string_lossy(),
                ])
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
        #[cfg(unix)]
        let planted = std::os::unix::fs::symlink(outside.path(), &link).is_ok();
        #[cfg(not(any(windows, unix)))]
        let planted = false;
        assert!(planted, "could not plant a directory link for confinement");
        let snapshot = Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert!(snapshot.read("keep.rs", 1, 1).is_ok());
        assert!(!snapshot.files.contains_key("secret.rs"));
        assert!(!snapshot.files.contains_key("escape/secret.rs"));
        assert!(snapshot.skipped.iter().any(|row| row.path == "escape"
            || row.reason.contains("reparse")
            || row.reason.contains("symlink")));
        assert!(relative_scope("src/file:stream").is_err());
        assert!(relative_scope(r"\\?\C:\data").is_err());
        assert!(relative_scope("con.rs").is_err());
    }
}
