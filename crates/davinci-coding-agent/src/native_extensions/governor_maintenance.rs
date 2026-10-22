//! Cross-process, per-store maintenance ownership; no retained directory index.
use std::{
    io,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) fn sweep_if_due(
    root: &Path,
    sweep: impl FnOnce() -> io::Result<usize>,
) -> io::Result<usize> {
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let _lease = match davinci_sys::lock::ExclusiveFileLock::try_acquire(&root.join(".sweep.lock"))
    {
        Ok(lease) => lease,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(0),
        Err(error) => return Err(error),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let marker = root.join(".sweep.completed");
    if std::fs::read_to_string(&marker)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .and_then(|then| now.checked_sub(then))
        .is_some_and(|age| age < Duration::from_secs(3600).as_secs())
    {
        return Ok(0);
    }
    let removed = sweep()?;
    davinci_sys::fs::atomic_write(&marker, now.to_string().as_bytes())?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failures_retry_and_future_markers_do_not_disable_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sweep_if_due(dir.path(), || Err(io::Error::other("scan failed"))).is_err());
        assert_eq!(sweep_if_due(dir.path(), || Ok(2)).unwrap(), 2);
        assert_eq!(
            sweep_if_due(dir.path(), || panic!("recent sweep repeated")).unwrap(),
            0
        );
        std::fs::write(dir.path().join(".sweep.completed"), u64::MAX.to_string()).unwrap();
        assert_eq!(sweep_if_due(dir.path(), || Ok(3)).unwrap(), 3);
    }
}
