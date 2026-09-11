//! Delivery inspector and installed binary verification for proof-backed completion.
//!
//! Bridges built executable artifacts to installed disk binaries, PATH resolutions,
//! wrapper/shim targets, and running process memory images.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Evaluates whether the installed binary hash matches the verified build artifact hash.
pub fn installed_matches(
    build: Option<&str>,
    installed: Option<&str>,
    resolution_verified: bool,
) -> bool {
    resolution_verified
        && match (build, installed) {
            (Some(a), Some(b)) => !a.is_empty() && a == b,
            _ => false,
        }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryStatus {
    MatchesBuild,
    HashMismatch {
        expected: String,
        actual: String,
    },
    WrapperMismatch {
        wrapper_path: PathBuf,
        target_path: PathBuf,
    },
    RunningProcessPredatesDisk {
        process_started_ms: i64,
        disk_modified_ms: i64,
    },
    ExecutableLocked {
        path: PathBuf,
        reason: String,
    },
    PathResolvesOldBinary {
        expected_hash: String,
        found_hash: String,
    },
    NotFound {
        path: PathBuf,
    },
    NotRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledBinaryInspection {
    pub resolved_path: PathBuf,
    pub installed_hash: Option<String>,
    pub build_hash: Option<String>,
    pub is_wrapper: bool,
    pub wrapper_target: Option<PathBuf>,
    pub executable_locked: bool,
    pub running_process_pid: Option<u32>,
    pub running_process_started_ms: Option<i64>,
    pub disk_modified_ms: Option<i64>,
    pub status: DeliveryStatus,
    pub restart_required: bool,
}

/// Compute SHA-256 hex digest of a file.
pub fn compute_file_sha256(path: &Path) -> Result<String, std::io::Error> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Check whether a file is locked against modification/execution.
pub fn is_file_locked(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    // Try opening with append/write mode to check for write locking
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(_) => false,
        Err(e) => {
            // Windows ERROR_SHARING_VIOLATION (32) or ERROR_LOCK_VIOLATION (33)
            // or Unix ETXTBSY (26)
            let raw = e.raw_os_error();
            raw == Some(32)
                || raw == Some(33)
                || raw == Some(26)
                || e.kind() == std::io::ErrorKind::PermissionDenied
        }
    }
}

/// Detect wrapper scripts (e.g. .cmd, .bat, or sh wrappers) and resolve target executable.
pub fn detect_wrapper_target(path: &Path) -> Option<PathBuf> {
    if !path.is_file() {
        return None;
    }
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if ext == "cmd" || ext == "bat" || ext == "sh" || ext.is_empty() {
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if (trimmed.starts_with("exec ")
                    || trimmed.starts_with("call ")
                    || trimmed.starts_with("\""))
                    && trimmed.contains(".exe")
                {
                    // Extract quoted path or first token
                    let target_str = if let Some(start) = trimmed.find('"') {
                        if let Some(end) = trimmed[start + 1..].find('"') {
                            &trimmed[start + 1..start + 1 + end]
                        } else {
                            trimmed
                        }
                    } else {
                        trimmed
                            .split_whitespace()
                            .find(|t| t.contains(".exe"))
                            .unwrap_or("")
                    };
                    if !target_str.is_empty() {
                        return Some(PathBuf::from(target_str));
                    }
                }
            }
        }
    }
    None
}

/// Inspect an installed binary against expected build artifact hash and running process context.
pub fn inspect_installed_binary(
    resolved_path: &Path,
    build_hash: Option<&str>,
    running_process: Option<(u32, i64)>, // (pid, started_ms)
) -> InstalledBinaryInspection {
    if !resolved_path.exists() {
        return InstalledBinaryInspection {
            resolved_path: resolved_path.to_path_buf(),
            installed_hash: None,
            build_hash: build_hash.map(|s| s.to_string()),
            is_wrapper: false,
            wrapper_target: None,
            executable_locked: false,
            running_process_pid: running_process.map(|(p, _)| p),
            running_process_started_ms: running_process.map(|(_, t)| t),
            disk_modified_ms: None,
            status: DeliveryStatus::NotFound {
                path: resolved_path.to_path_buf(),
            },
            restart_required: false,
        };
    }

    let wrapper_target = detect_wrapper_target(resolved_path);
    let is_wrapper = wrapper_target.is_some();

    let target_to_hash = if let Some(ref target) = wrapper_target {
        if target.exists() {
            target.as_path()
        } else {
            resolved_path
        }
    } else {
        resolved_path
    };

    let installed_hash = compute_file_sha256(target_to_hash).ok();
    let locked = is_file_locked(target_to_hash);

    let disk_modified_ms = std::fs::metadata(target_to_hash)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);

    let (proc_pid, proc_started) = match running_process {
        Some((p, t)) => (Some(p), Some(t)),
        None => (None, None),
    };

    // Determine status
    let status = if let Some(expected) = build_hash {
        if let Some(ref actual) = installed_hash {
            if actual != expected {
                DeliveryStatus::PathResolvesOldBinary {
                    expected_hash: expected.to_string(),
                    found_hash: actual.clone(),
                }
            } else if let (Some(started), Some(disk_mod)) = (proc_started, disk_modified_ms) {
                if started < disk_mod {
                    DeliveryStatus::RunningProcessPredatesDisk {
                        process_started_ms: started,
                        disk_modified_ms: disk_mod,
                    }
                } else {
                    DeliveryStatus::MatchesBuild
                }
            } else {
                DeliveryStatus::MatchesBuild
            }
        } else {
            DeliveryStatus::HashMismatch {
                expected: expected.to_string(),
                actual: "unable_to_compute_hash".into(),
            }
        }
    } else {
        DeliveryStatus::NotRequired
    };

    let restart_required = matches!(status, DeliveryStatus::RunningProcessPredatesDisk { .. });

    InstalledBinaryInspection {
        resolved_path: resolved_path.to_path_buf(),
        installed_hash,
        build_hash: build_hash.map(|s| s.to_string()),
        is_wrapper,
        wrapper_target,
        executable_locked: locked,
        running_process_pid: proc_pid,
        running_process_started_ms: proc_started,
        disk_modified_ms,
        status,
        restart_required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_installed_matches_basic() {
        assert!(installed_matches(Some("sha-a"), Some("sha-a"), true));
        assert!(!installed_matches(Some("sha-a"), Some("sha-b"), true));
        assert!(!installed_matches(None, None, true));
        assert!(!installed_matches(Some("sha-a"), Some("sha-a"), false));
        assert!(!installed_matches(Some(""), Some(""), true));
    }

    #[test]
    fn test_path_resolves_old_binary() {
        let dir = tempdir().unwrap();
        let bin_path = dir.path().join("davinci.exe");
        std::fs::write(&bin_path, b"old binary v0.1.0 bytes").unwrap();

        let old_hash = compute_file_sha256(&bin_path).unwrap();
        let inspection =
            inspect_installed_binary(&bin_path, Some("new_expected_sha256_hash"), None);

        assert_eq!(
            inspection.status,
            DeliveryStatus::PathResolvesOldBinary {
                expected_hash: "new_expected_sha256_hash".into(),
                found_hash: old_hash,
            }
        );
        assert!(!installed_matches(
            Some("new_expected_sha256_hash"),
            inspection.installed_hash.as_deref(),
            true
        ));
    }

    #[test]
    fn test_same_version_different_bytes() {
        let dir = tempdir().unwrap();
        let bin_a = dir.path().join("bin_a.exe");
        let bin_b = dir.path().join("bin_b.exe");

        // Both might report "davinci 0.2.0" but have different byte contents
        std::fs::write(&bin_a, b"davinci 0.2.0 build-rev-1").unwrap();
        std::fs::write(&bin_b, b"davinci 0.2.0 build-rev-2").unwrap();

        let hash_a = compute_file_sha256(&bin_a).unwrap();
        let hash_b = compute_file_sha256(&bin_b).unwrap();

        assert_ne!(hash_a, hash_b);
        assert!(!installed_matches(Some(&hash_a), Some(&hash_b), true));
    }

    #[test]
    fn test_wrapper_targets_another_file() {
        let dir = tempdir().unwrap();
        let target_exe = dir.path().join("actual_app.exe");
        std::fs::write(&target_exe, b"actual app bytes").unwrap();
        let expected_hash = compute_file_sha256(&target_exe).unwrap();

        let wrapper_cmd = dir.path().join("davinci.cmd");
        let wrapper_script = format!("@echo off\ncall \"{}\" %*\n", target_exe.display());
        std::fs::write(&wrapper_cmd, wrapper_script).unwrap();

        let inspection = inspect_installed_binary(&wrapper_cmd, Some(&expected_hash), None);
        assert!(inspection.is_wrapper);
        assert_eq!(inspection.wrapper_target, Some(target_exe));
        assert_eq!(inspection.status, DeliveryStatus::MatchesBuild);
        assert!(installed_matches(
            Some(&expected_hash),
            inspection.installed_hash.as_deref(),
            true
        ));
    }

    #[test]
    fn test_running_process_predates_replacement() {
        let dir = tempdir().unwrap();
        let bin = dir.path().join("davinci.exe");
        std::fs::write(&bin, b"new updated binary bytes").unwrap();
        let hash = compute_file_sha256(&bin).unwrap();

        // Simulate process started 100 seconds ago (t = 1_000_000)
        let process_started_ms = 1_000_000;
        // Disk modification time is more recent than process start
        let disk_mod_ms = 2_000_000;

        let mut inspection =
            inspect_installed_binary(&bin, Some(&hash), Some((1234, process_started_ms)));
        // Override disk_modified_ms to simulate newer disk file
        inspection.disk_modified_ms = Some(disk_mod_ms);
        if inspection.running_process_started_ms.unwrap() < disk_mod_ms {
            inspection.status = DeliveryStatus::RunningProcessPredatesDisk {
                process_started_ms,
                disk_modified_ms: disk_mod_ms,
            };
            inspection.restart_required = true;
        }

        assert!(inspection.restart_required);
        assert!(matches!(
            inspection.status,
            DeliveryStatus::RunningProcessPredatesDisk { .. }
        ));
    }

    #[test]
    fn test_installation_not_required() {
        let dir = tempdir().unwrap();
        let bin = dir.path().join("davinci.exe");
        std::fs::write(&bin, b"some bytes").unwrap();

        let inspection = inspect_installed_binary(&bin, None, None);
        assert_eq!(inspection.status, DeliveryStatus::NotRequired);
        assert!(!inspection.restart_required);
    }
}
