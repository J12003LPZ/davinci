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
///
/// Age alone does not prove a scan is finished: a long or resumed scan can
/// sit on old files, and on Unix holding `active.lock` does not stop another
/// process from deleting the directory around it. So a scan is removed only
/// while the sweeper itself holds that lease (the same lock `Store::open`
/// takes), and any scan whose lease is held, or cannot be checked, is kept.
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
            if newest_modification(&scan.path()).is_none_or(|newest| newest >= cutoff) {
                continue;
            }
            let Ok(lease) =
                davinci_sys::lock::ExclusiveFileLock::try_acquire(&scan.path().join("active.lock"))
            else {
                continue;
            };
            // Windows cannot delete a file through someone else's handle, our
            // own exclusive one included, so the lease is released first there;
            // a scan that grabs it in that instant makes the removal fail.
            #[cfg(windows)]
            drop(lease);
            if std::fs::remove_dir_all(scan.path()).is_ok() {
                removed += 1;
            }
            #[cfg(not(windows))]
            drop(lease);
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

    #[test]
    fn security_retention_keeps_an_old_scan_whose_lease_is_held() {
        let agent = tempfile::tempdir().unwrap();
        let scan = agent
            .path()
            .join("security-scans")
            .join("repo")
            .join("running");
        std::fs::create_dir_all(&scan).unwrap();
        std::fs::write(scan.join("checkpoint.json"), "{}").unwrap();
        let lease =
            davinci_sys::lock::ExclusiveFileLock::try_acquire(&scan.join("active.lock")).unwrap();
        let later = SystemTime::now() + Duration::from_secs(10 * 24 * 60 * 60);
        assert_eq!(sweep(agent.path(), 1, later), 0);
        assert!(
            scan.join("checkpoint.json").exists(),
            "a running scan was swept"
        );
        drop(lease);
        assert_eq!(sweep(agent.path(), 1, later), 1);
        assert!(!scan.exists());
    }
}
