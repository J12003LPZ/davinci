//! `securityScan.retentionDays`: sweep old scan directories; native-only.
//!
//! Every scan (reviews, checkpoints and graph-gate artifacts) lives in
//! `<agent_dir>/security-scans/<repo>/<scan>/`. A scan directory whose newest
//! entry is older than the retention window is removed. Repository-level
//! files such as `feedback.json` are never touched.

use std::path::Path;
use std::time::{Duration, SystemTime};

fn newest_modification(dir: &Path) -> Option<SystemTime> {
    let mut newest = std::fs::metadata(dir).and_then(|meta| meta.modified()).ok();
    for entry in std::fs::read_dir(dir).ok()?.flatten().take(32_768) {
        if let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) {
            newest = Some(newest.map_or(modified, |current: SystemTime| current.max(modified)));
        }
    }
    newest
}

/// Remove scan directories older than `retention_days`. Returns how many went.
/// A directory whose lock is held by a live scan cannot be removed on Windows
/// and is recent everywhere else, so an active scan is never swept.
pub fn sweep(agent_dir: &Path, retention_days: u32, now: SystemTime) -> usize {
    let window = Duration::from_secs(u64::from(retention_days.max(1)) * 24 * 60 * 60);
    let Some(cutoff) = now.checked_sub(window) else {
        return 0;
    };
    let Ok(repos) = std::fs::read_dir(agent_dir.join("security-scans")) else {
        return 0;
    };
    let mut removed = 0;
    for repo in repos.flatten().take(4096) {
        if !repo.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Ok(scans) = std::fs::read_dir(repo.path()) else {
            continue;
        };
        for scan in scans.flatten().take(32_768) {
            // Symlinks are never followed or removed through.
            if !scan.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            if newest_modification(&scan.path()).is_some_and(|newest| newest < cutoff)
                && std::fs::remove_dir_all(scan.path()).is_ok()
            {
                removed += 1;
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_retention_sweeps_only_expired_scan_directories() {
        let agent = tempfile::tempdir().unwrap();
        let repo = agent.path().join("security-scans").join("repo");
        std::fs::create_dir_all(repo.join("old-scan")).unwrap();
        std::fs::write(repo.join("old-scan").join("report.md"), "x").unwrap();
        std::fs::write(repo.join("feedback.json"), "[]").unwrap();
        // Nothing is older than a day yet.
        assert_eq!(sweep(agent.path(), 1, SystemTime::now()), 0);
        let later = SystemTime::now() + Duration::from_secs(3 * 24 * 60 * 60);
        assert_eq!(sweep(agent.path(), 30, later), 0);
        assert_eq!(sweep(agent.path(), 2, later), 1);
        assert!(!repo.join("old-scan").exists());
        assert!(repo.join("feedback.json").exists());
    }
}
