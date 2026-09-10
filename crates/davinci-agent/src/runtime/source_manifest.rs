//! Canonical source manifest and fingerprinting for evidence-backed completion.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    File,
    Directory,
    Symlink { target: String },
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub relative_path: String,
    pub kind: FileKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default)]
    pub is_executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceManifest {
    pub entries: Vec<ManifestEntry>,
    #[serde(default)]
    pub dependency_lock_hashes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_profile: Option<String>,
    pub digest: String,
    pub complete_coverage: bool,
    pub scanned_at_ms: i64,
}

impl SourceManifest {
    pub fn is_current(&self, other: &SourceManifest) -> bool {
        self.complete_coverage && other.complete_coverage && self.digest == other.digest
    }
}

pub struct SourceManifestBuilder {
    root: PathBuf,
    target_paths: Vec<String>,
    tracked_missing: Vec<String>,
    dependency_locks: BTreeMap<String, String>,
    toolchain_fingerprint: Option<String>,
    command_profile: Option<String>,
    complete_coverage: bool,
    max_scan_retries: usize,
    simulated_concurrent_edit: Option<String>,
}

impl SourceManifestBuilder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            target_paths: Vec::new(),
            tracked_missing: Vec::new(),
            dependency_locks: BTreeMap::new(),
            toolchain_fingerprint: None,
            command_profile: None,
            complete_coverage: true,
            max_scan_retries: 3,
            simulated_concurrent_edit: None,
        }
    }

    pub fn target_path(mut self, rel_path: impl Into<String>) -> Self {
        let p = normalize_rel_path(&rel_path.into());
        if !self.target_paths.contains(&p) {
            self.target_paths.push(p);
        }
        self
    }

    pub fn target_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for p in paths {
            let norm = normalize_rel_path(&p.into());
            if !self.target_paths.contains(&norm) {
                self.target_paths.push(norm);
            }
        }
        self
    }

    pub fn track_missing(mut self, rel_path: impl Into<String>) -> Self {
        let p = normalize_rel_path(&rel_path.into());
        if !self.tracked_missing.contains(&p) {
            self.tracked_missing.push(p);
        }
        self
    }

    pub fn dependency_lock(mut self, name: impl Into<String>, hash: impl Into<String>) -> Self {
        self.dependency_locks.insert(name.into(), hash.into());
        self
    }

    pub fn toolchain_fingerprint(mut self, fp: impl Into<String>) -> Self {
        self.toolchain_fingerprint = Some(fp.into());
        self
    }

    pub fn command_profile(mut self, cp: impl Into<String>) -> Self {
        self.command_profile = Some(cp.into());
        self
    }

    pub fn complete_coverage(mut self, complete: bool) -> Self {
        self.complete_coverage = complete;
        self
    }

    #[cfg(test)]
    pub fn simulate_concurrent_edit(mut self, rel_path: impl Into<String>) -> Self {
        self.simulated_concurrent_edit = Some(normalize_rel_path(&rel_path.into()));
        self
    }

    pub fn build(mut self) -> Result<SourceManifest, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let mut entries = Vec::new();
        let mut all_paths = self.target_paths.clone();
        for m in &self.tracked_missing {
            if !all_paths.contains(m) {
                all_paths.push(m.clone());
            }
        }
        all_paths.sort();
        all_paths.dedup();

        for rel in all_paths {
            let full_path = self.root.join(&rel);
            let mut retries = 0;
            let mut entry = None;

            while retries <= self.max_scan_retries {
                let current_entry = self.scan_path(&rel, &full_path)?;

                // Stability verification: read second time if file
                if matches!(current_entry.kind, FileKind::File) {
                    if let Some(ref sim) = self.simulated_concurrent_edit {
                        if sim == &rel {
                            // File was mutated concurrently
                            if retries < self.max_scan_retries {
                                retries += 1;
                                continue;
                            } else {
                                // Exceeded retries, mark incomplete coverage / unstable
                                self.complete_coverage = false;
                            }
                        }
                    }

                    // Check metadata stability
                    if let Ok(meta_after) = fs::metadata(&full_path) {
                        if Some(meta_after.len()) != current_entry.size {
                            if retries < self.max_scan_retries {
                                retries += 1;
                                continue;
                            } else {
                                self.complete_coverage = false;
                            }
                        }
                    }
                }

                entry = Some(current_entry);
                break;
            }

            if let Some(e) = entry {
                entries.push(e);
            } else {
                self.complete_coverage = false;
                entries.push(ManifestEntry {
                    relative_path: rel,
                    kind: FileKind::Missing,
                    content_hash: None,
                    size: None,
                    is_executable: false,
                });
            }
        }

        entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

        let digest = compute_manifest_digest(
            &entries,
            &self.dependency_locks,
            self.toolchain_fingerprint.as_deref(),
            self.command_profile.as_deref(),
        );

        Ok(SourceManifest {
            entries,
            dependency_lock_hashes: self.dependency_locks,
            toolchain_fingerprint: self.toolchain_fingerprint,
            command_profile: self.command_profile,
            digest,
            complete_coverage: self.complete_coverage,
            scanned_at_ms: now,
        })
    }

    fn scan_path(&self, rel: &str, full: &Path) -> Result<ManifestEntry, String> {
        // Use symlink_metadata to avoid automatically following symlinks
        let sym_meta = match fs::symlink_metadata(full) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ManifestEntry {
                    relative_path: rel.to_string(),
                    kind: FileKind::Missing,
                    content_hash: None,
                    size: None,
                    is_executable: false,
                });
            }
            Err(e) => return Err(format!("failed to read metadata for {rel}: {e}")),
        };

        let file_type = sym_meta.file_type();
        if file_type.is_symlink() {
            let target = match fs::read_link(full) {
                Ok(t) => t.to_string_lossy().replace('\\', "/"),
                Err(e) => return Err(format!("failed to read symlink target for {rel}: {e}")),
            };
            return Ok(ManifestEntry {
                relative_path: rel.to_string(),
                kind: FileKind::Symlink { target },
                content_hash: None,
                size: Some(sym_meta.len()),
                is_executable: false,
            });
        }

        if file_type.is_dir() {
            return Ok(ManifestEntry {
                relative_path: rel.to_string(),
                kind: FileKind::Directory,
                content_hash: None,
                size: None,
                is_executable: false,
            });
        }

        // Normal file
        let bytes = match fs::read(full) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ManifestEntry {
                    relative_path: rel.to_string(),
                    kind: FileKind::Missing,
                    content_hash: None,
                    size: None,
                    is_executable: false,
                });
            }
            Err(e) => return Err(format!("failed to read file content for {rel}: {e}")),
        };

        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let content_hash = format!("{:x}", hasher.finalize());

        let is_executable = is_path_executable(full, &sym_meta);

        Ok(ManifestEntry {
            relative_path: rel.to_string(),
            kind: FileKind::File,
            content_hash: Some(content_hash),
            size: Some(bytes.len() as u64),
            is_executable,
        })
    }
}

pub fn compute_manifest_digest(
    entries: &[ManifestEntry],
    dependency_locks: &BTreeMap<String, String>,
    toolchain_fingerprint: Option<&str>,
    command_profile: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();

    // Entries length
    hasher.update((entries.len() as u64).to_le_bytes());
    for entry in entries {
        hasher.update((entry.relative_path.len() as u64).to_le_bytes());
        hasher.update(entry.relative_path.as_bytes());

        match &entry.kind {
            FileKind::Missing => {
                hasher.update([0u8]);
            }
            FileKind::File => {
                hasher.update([1u8]);
                if let Some(hash) = &entry.content_hash {
                    hasher.update((hash.len() as u64).to_le_bytes());
                    hasher.update(hash.as_bytes());
                } else {
                    hasher.update(0u64.to_le_bytes());
                }
                hasher.update(entry.size.unwrap_or(0).to_le_bytes());
                hasher.update([if entry.is_executable { 1u8 } else { 0u8 }]);
            }
            FileKind::Directory => {
                hasher.update([2u8]);
            }
            FileKind::Symlink { target } => {
                hasher.update([3u8]);
                hasher.update((target.len() as u64).to_le_bytes());
                hasher.update(target.as_bytes());
            }
        }
    }

    // Dependency locks
    hasher.update((dependency_locks.len() as u64).to_le_bytes());
    for (k, v) in dependency_locks {
        hasher.update((k.len() as u64).to_le_bytes());
        hasher.update(k.as_bytes());
        hasher.update((v.len() as u64).to_le_bytes());
        hasher.update(v.as_bytes());
    }

    // Toolchain fingerprint
    if let Some(tf) = toolchain_fingerprint {
        hasher.update([1u8]);
        hasher.update((tf.len() as u64).to_le_bytes());
        hasher.update(tf.as_bytes());
    } else {
        hasher.update([0u8]);
    }

    // Command profile
    if let Some(cp) = command_profile {
        hasher.update([1u8]);
        hasher.update((cp.len() as u64).to_le_bytes());
        hasher.update(cp.as_bytes());
    } else {
        hasher.update([0u8]);
    }

    format!("{:x}", hasher.finalize())
}

fn normalize_rel_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

#[cfg(unix)]
fn is_path_executable(_path: &Path, meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    (meta.permissions().mode() & 0o111) != 0
}

#[cfg(windows)]
fn is_path_executable(path: &Path, _meta: &fs::Metadata) -> bool {
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        matches!(ext.to_ascii_lowercase().as_str(), "exe" | "bat" | "cmd")
    } else {
        false
    }
}

#[cfg(not(any(unix, windows)))]
fn is_path_executable(_path: &Path, _meta: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_manifest_head_unchanged_but_bytes_edited() {
        let dir = tempdir().unwrap();
        let file_a = dir.path().join("src.rs");
        fs::write(&file_a, "fn main() {}\n").unwrap();

        let m1 = SourceManifestBuilder::new(dir.path())
            .target_path("src.rs")
            .build()
            .unwrap();

        // Edit bytes without changing any git commit / HEAD
        fs::write(&file_a, "fn main() { println!(); }\n").unwrap();

        let m2 = SourceManifestBuilder::new(dir.path())
            .target_path("src.rs")
            .build()
            .unwrap();

        assert_ne!(m1.digest, m2.digest);
        assert!(!m1.is_current(&m2));
    }

    #[test]
    fn test_manifest_untracked_dependency_created() {
        let dir = tempdir().unwrap();
        let m1 = SourceManifestBuilder::new(dir.path())
            .target_path("src/lib.rs")
            .track_missing("dep.rs")
            .build()
            .unwrap();

        assert_eq!(
            m1.entries
                .iter()
                .find(|e| e.relative_path == "dep.rs")
                .unwrap()
                .kind,
            FileKind::Missing
        );

        // Now create dep.rs
        fs::write(dir.path().join("dep.rs"), "pub fn helper() {}").unwrap();

        let m2 = SourceManifestBuilder::new(dir.path())
            .target_path("src/lib.rs")
            .track_missing("dep.rs")
            .build()
            .unwrap();

        assert_eq!(
            m2.entries
                .iter()
                .find(|e| e.relative_path == "dep.rs")
                .unwrap()
                .kind,
            FileKind::File
        );
        assert_ne!(m1.digest, m2.digest);
    }

    #[test]
    fn test_manifest_file_removed() {
        let dir = tempdir().unwrap();
        let file_p = dir.path().join("foo.txt");
        fs::write(&file_p, "content").unwrap();

        let m1 = SourceManifestBuilder::new(dir.path())
            .target_path("foo.txt")
            .build()
            .unwrap();

        fs::remove_file(&file_p).unwrap();

        let m2 = SourceManifestBuilder::new(dir.path())
            .target_path("foo.txt")
            .build()
            .unwrap();

        assert_ne!(m1.digest, m2.digest);
        assert_eq!(m2.entries[0].kind, FileKind::Missing);
    }

    #[test]
    fn test_manifest_lockfile_changed() {
        let dir = tempdir().unwrap();
        let m1 = SourceManifestBuilder::new(dir.path())
            .dependency_lock("Cargo.lock", "hash111")
            .build()
            .unwrap();

        let m2 = SourceManifestBuilder::new(dir.path())
            .dependency_lock("Cargo.lock", "hash222")
            .build()
            .unwrap();

        assert_ne!(m1.digest, m2.digest);
    }

    #[test]
    fn test_manifest_crlf_only_change() {
        let dir = tempdir().unwrap();
        let file_p = dir.path().join("file.txt");
        fs::write(&file_p, "line1\nline2\n").unwrap();

        let m1 = SourceManifestBuilder::new(dir.path())
            .target_path("file.txt")
            .build()
            .unwrap();

        // CRLF-only change
        fs::write(&file_p, "line1\r\nline2\r\n").unwrap();

        let m2 = SourceManifestBuilder::new(dir.path())
            .target_path("file.txt")
            .build()
            .unwrap();

        assert_ne!(m1.digest, m2.digest);
    }

    #[test]
    fn test_manifest_symlink_target_changed() {
        let dir = tempdir().unwrap();
        let target_a = dir.path().join("a.txt");
        let target_b = dir.path().join("b.txt");
        fs::write(&target_a, "A").unwrap();
        fs::write(&target_b, "B").unwrap();

        #[cfg(unix)]
        {
            let link_p = dir.path().join("link.txt");
            std::os::unix::fs::symlink(&target_a, &link_p).unwrap();
            let m1 = SourceManifestBuilder::new(dir.path())
                .target_path("link.txt")
                .build()
                .unwrap();

            fs::remove_file(&link_p).unwrap();
            std::os::unix::fs::symlink(&target_b, &link_p).unwrap();

            let m2 = SourceManifestBuilder::new(dir.path())
                .target_path("link.txt")
                .build()
                .unwrap();

            assert_ne!(m1.digest, m2.digest);
        }

        // On Windows if symlink creation is not permitted without admin, test FileKind::Symlink directly
        let entry_a = ManifestEntry {
            relative_path: "link.txt".into(),
            kind: FileKind::Symlink {
                target: "a.txt".into(),
            },
            content_hash: None,
            size: Some(5),
            is_executable: false,
        };
        let entry_b = ManifestEntry {
            relative_path: "link.txt".into(),
            kind: FileKind::Symlink {
                target: "b.txt".into(),
            },
            content_hash: None,
            size: Some(5),
            is_executable: false,
        };
        let locks = BTreeMap::new();
        let d1 = compute_manifest_digest(&[entry_a], &locks, None, None);
        let d2 = compute_manifest_digest(&[entry_b], &locks, None, None);
        assert_ne!(d1, d2);
    }

    #[test]
    fn test_manifest_incomplete_scope() {
        let dir = tempdir().unwrap();
        let file_p = dir.path().join("main.rs");
        fs::write(&file_p, "fn main() {}").unwrap();

        let m1 = SourceManifestBuilder::new(dir.path())
            .target_path("main.rs")
            .complete_coverage(false)
            .build()
            .unwrap();

        assert!(!m1.complete_coverage);
        assert!(!m1.is_current(&m1));
    }

    #[test]
    fn test_manifest_edit_during_scan() {
        let dir = tempdir().unwrap();
        let file_p = dir.path().join("code.rs");
        fs::write(&file_p, "initial").unwrap();

        let m = SourceManifestBuilder::new(dir.path())
            .target_path("code.rs")
            .simulate_concurrent_edit("code.rs")
            .build()
            .unwrap();

        // Exceeded retries on concurrent modification marked coverage incomplete
        assert!(!m.complete_coverage);
    }
}
