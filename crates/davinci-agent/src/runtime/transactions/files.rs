//! Reuses cache runtime's handle-bound directory primitives, never its evictable storage.
use super::model::{Image, MAX_FILE_BYTES, STORE_NAME};
use crate::runtime::cache::directory::Directory;
use crate::runtime::checkpoints::compute_sha256;
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub(super) fn normalize(root: &Path, path: &str) -> Result<String, String> {
    if path.is_empty() || path.contains(['\0', ':']) || path.starts_with(['/', '\\']) {
        return Err("transaction requires a relative workspace path".into());
    }
    let portable = path.replace('\\', "/");
    let mut parts = Vec::new();
    for part in portable.split('/') {
        if part == "." {
            continue;
        }
        if part.is_empty() || part == ".." || part.ends_with(['.', ' ']) {
            return Err("invalid transaction path component".into());
        }
        let lower = part.to_ascii_lowercase();
        if lower == ".git"
            || lower == STORE_NAME
            || lower.starts_with(".davinci_patch_journal")
            || lower.starts_with(".pi_patch_journal")
            || lower.starts_with(".davinci_rewind_journal")
            || lower.starts_with(".pi_rewind_journal")
            || lower.starts_with(".davinci-txn-")
        {
            return Err("transaction target is a reserved state path".into());
        }
        parts.push(part);
    }
    let relative = parts.join("/");
    if relative.is_empty() || relative.len() > 4096 {
        return Err("invalid transaction path length".into());
    }
    crate::apply_patch::sanitize_relative_path(root, &relative)?;
    Ok(relative)
}

pub(super) fn directory(
    root: &Path,
    path: &str,
    create: bool,
) -> Result<(Directory, String), String> {
    let target = root.join(path);
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("invalid source name")?;
    let parent = target.parent().ok_or("source has no parent")?;
    let dir =
        Directory::open(parent, create).map_err(|e| format!("confined source directory: {e}"))?;
    Ok((dir, name.into()))
}

pub(super) fn capture(root: &Path, path: &str) -> Result<(Image, Option<Vec<u8>>), String> {
    let target = root.join(path);
    let parent = target.parent().ok_or("source has no parent")?;
    let dir = match Directory::open(parent, false) {
        Ok(dir) => dir,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Image::missing(), None)),
        Err(e) => return Err(format!("confined source directory: {e}")),
    };
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("invalid source name")?;
    capture_in(&dir, name)
}

fn capture_in(dir: &Directory, name: &str) -> Result<(Image, Option<Vec<u8>>), String> {
    let mut file = match dir.source_file(name) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Image::missing(), None)),
        Err(e) => return Err(format!("confined source read: {e}")),
    };
    let before = file.metadata().map_err(|e| e.to_string())?;
    let identity = identity(&file, &before)?;
    let xattrs = super::unix_xattrs::capture(&file)?;
    let macos_acl = super::macos_acl::capture(&file)?;
    #[cfg(windows)]
    let streams = super::windows_streams::capture(dir, name, &file)?;
    #[cfg(not(windows))]
    let streams = std::collections::BTreeMap::new();
    if before.len() > MAX_FILE_BYTES as u64 {
        return Err("transaction file exceeds byte limit".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let after = file.metadata().map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE_BYTES
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || mode(&before) != mode(&after)
        || unix_owner(&before) != unix_owner(&after)
        || xattrs != super::unix_xattrs::capture(&file)?
        || macos_acl != super::macos_acl::capture(&file)?
    {
        return Err("conflict: source changed during capture".into());
    }
    dir.check_current()
        .map_err(|e| format!("source directory changed: {e}"))?;
    Ok((
        Image {
            hash: Some(compute_sha256(&bytes)),
            identity: Some(identity),
            mode: Some(mode(&after)),
            access: access(&file)?,
            unix_owner: unix_owner(&after),
            macos_acl,
            xattrs,
            streams,
        },
        Some(bytes),
    ))
}

pub(super) fn stage(
    root: &Path,
    path: &str,
    bytes: Option<&[u8]>,
    original: &Image,
) -> Result<(Image, Option<String>), String> {
    let Some(bytes) = bytes else {
        return Ok((Image::missing(), None));
    };
    let (dir, _) = directory(root, path, true)?;
    let name = format!(".davinci-txn-{}.tmp", uuid::Uuid::new_v4());
    let result = (|| {
        let mut file = dir.stage_file(&name).map_err(|e| e.to_string())?;
        #[cfg(test)]
        failure_tests::before_write(&mut file, bytes)?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        let wanted = original.mode.unwrap_or(0o644);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            restore_unix_owner(&file, original.unix_owner)?;
            file.set_permissions(std::fs::Permissions::from_mode(wanted & 0o7777))
                .map_err(|e| e.to_string())?;
            if original.unix_owner.is_some() {
                super::unix_xattrs::restore(&file, &original.xattrs)?;
                #[cfg(target_os = "macos")]
                super::macos_acl::restore(&file, original.macos_acl.as_deref())?;
            }
            let restored = file.metadata().map_err(|e| e.to_string())?;
            if mode(&restored) != wanted {
                return Err("cannot preserve transaction file permissions".into());
            }
        }
        #[cfg(windows)]
        {
            super::windows_streams::restore(&dir, &name, &file, &original.streams)?;
            if let Some(access) = &original.access {
                super::windows_acl::apply(&file, access)?;
            }
            let mut permissions = file.metadata().map_err(|e| e.to_string())?.permissions();
            permissions.set_readonly(wanted & 0o222 == 0);
            file.set_permissions(permissions)
                .map_err(|e| e.to_string())?;
        }
        #[cfg(test)]
        failure_tests::before_sync()?;
        file.sync_all().map_err(|e| e.to_string())?;
        dir.sync().map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        Ok((
            Image {
                hash: Some(compute_sha256(bytes)),
                identity: Some(identity(&file, &metadata)?),
                mode: Some(mode(&metadata)),
                access: access(&file)?,
                unix_owner: unix_owner(&metadata),
                macos_acl: super::macos_acl::capture(&file)?,
                xattrs: super::unix_xattrs::capture(&file)?,
                streams: original.streams.clone(),
            },
            Some(name.clone()),
        ))
    })();
    if result.is_err() {
        let _ = dir.remove(&name);
    }
    result
}

pub(super) fn replace(
    root: &Path,
    path: &str,
    expected: &Image,
    staged: Option<&str>,
    proposed: &Image,
) -> Result<(), String> {
    let (dir, name) = directory(root, path, staged.is_some())?;
    if capture_in(&dir, &name)?.0 != *expected {
        return Err(format!("conflict: {path} changed before mutation"));
    }
    dir.check_current()
        .map_err(|e| format!("source directory changed: {e}"))?;
    if let Some(staged) = staged {
        if capture_in(&dir, staged)?.0 != *proposed {
            return Err(format!(
                "conflict: staged bytes or identity changed for {path}"
            ));
        }
        dir.replace_source(staged, &name, expected.hash.is_some())
            .map_err(|e| format!("replace {path}: {e}"))
    } else if expected.hash.is_some() {
        dir.remove_source(&name)
            .map_err(|e| format!("remove {path}: {e}"))
    } else {
        Ok(())
    }
}

pub(super) fn cleanup(root: &Path, path: &str, name: Option<&str>) {
    if let Some(name) = name {
        if let Ok((dir, _)) = directory(root, path, false) {
            let _ = dir.remove(name);
        }
    }
}

pub(super) fn root(path: &Path) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("transaction workspace: {e}"))?;
    Directory::open(&path, false).map_err(|e| format!("transaction workspace: {e}"))?;
    Ok(path)
}

fn access(file: &File) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        super::windows_acl::read(file).map(Some)
    }
    #[cfg(unix)]
    {
        let _ = file;
        Ok(None)
    }
}

fn unix_owner(metadata: &Metadata) -> Option<[u32; 2]> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some([metadata.uid(), metadata.gid()])
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

#[cfg(unix)]
fn restore_unix_owner(file: &File, expected: Option<[u32; 2]>) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let Some([uid, gid]) = expected else {
        return Ok(());
    };
    if unix_owner(&file.metadata().map_err(|e| e.to_string())?) == expected {
        return Ok(());
    }
    // SAFETY: the descriptor remains owned by file. Only the private stage is
    // changed; normal OS authorization applies. Do this before restoring mode.
    if unsafe { libc::fchown(file.as_raw_fd(), uid, gid) } != 0 {
        return Err(format!(
            "restore transaction ownership: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unix_owner(&file.metadata().map_err(|e| e.to_string())?) != expected {
        return Err("cannot preserve transaction file ownership".into());
    }
    Ok(())
}

fn mode(metadata: &Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7777
    }
    #[cfg(windows)]
    {
        if metadata.permissions().readonly() {
            0o444
        } else {
            0o644
        }
    }
}

pub(super) fn identity(file: &File, metadata: &Metadata) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = file;
        if metadata.nlink() != 1 {
            return Err("linked transaction source denied".into());
        }
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        #[derive(Default)]
        struct Information {
            attributes: u32,
            created: [u32; 2],
            accessed: [u32; 2],
            written: [u32; 2],
            volume: u32,
            size_high: u32,
            size_low: u32,
            links: u32,
            index_high: u32,
            index_low: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetFileInformationByHandle(
                handle: *mut std::ffi::c_void,
                result: *mut Information,
            ) -> i32;
        }
        let _ = metadata;
        let mut information = Information::default();
        // SAFETY: the file owns a live handle; the C-layout buffer matches BY_HANDLE_FILE_INFORMATION.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if information.links != 1 || information.attributes & 0x400 != 0 {
            return Err("linked transaction source denied".into());
        }
        Ok(format!(
            "{}:{}:{}",
            information.volume, information.index_high, information.index_low
        ))
    }
}

#[cfg(test)]
mod failure_tests {
    use super::super::{
        ProposedChange, TransactionCoordinator, TransactionOwner, TransactionState,
    };
    use super::*;
    use std::cell::Cell;

    thread_local! {
        static FAILURE: Cell<(usize, bool)> = const { Cell::new((usize::MAX, false)) };
    }

    pub(super) fn before_write(file: &mut File, bytes: &[u8]) -> Result<(), String> {
        let fail = FAILURE.with(|state| {
            let (remaining, sync) = state.get();
            state.set((remaining.saturating_sub(1), sync));
            remaining == 1 && !sync
        });
        if fail {
            file.write_all(&bytes[..bytes.len().min(3)])
                .map_err(|e| e.to_string())?;
            FAILURE.with(|state| state.set((usize::MAX, false)));
            return Err("injected partial source-stage write failure".into());
        }
        Ok(())
    }

    pub(super) fn before_sync() -> Result<(), String> {
        let fail = FAILURE.with(|state| state.get() == (0, true));
        if fail {
            FAILURE.with(|state| state.set((usize::MAX, false)));
            return Err("injected source-stage sync failure".into());
        }
        Ok(())
    }

    fn fail_stage<T>(index: usize, sync: bool, operation: impl FnOnce() -> T) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                FAILURE.with(|state| state.set((usize::MAX, false)));
            }
        }
        let _reset = Reset;
        FAILURE.with(|state| state.set((index, sync)));
        operation()
    }

    #[test]
    fn transaction_source_stage_failures_preserve_sources_and_allow_recovery() {
        for rollback in [false, true] {
            for sync in [false, true] {
                for index in 1..=2 {
                    let root = tempfile::tempdir().unwrap();
                    for path in ["a.txt", "b.txt"] {
                        std::fs::write(root.path().join(path), b"before").unwrap();
                    }
                    let owner = TransactionOwner::default();
                    let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
                    let preview = manager
                        .preview(vec![
                            ProposedChange::write("a.txt", b"after".to_vec()),
                            ProposedChange::write("b.txt", b"after".to_vec()),
                        ])
                        .unwrap();
                    if rollback {
                        manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
                    }
                    let error = fail_stage(index, sync, || {
                        if rollback {
                            manager.rollback(&preview.id, &|_| Ok(()), None)
                        } else {
                            manager.apply(&preview.id, &|_| Ok(()), None)
                        }
                    })
                    .unwrap_err();
                    assert!(error.contains("injected"), "{error}");
                    for path in ["a.txt", "b.txt"] {
                        assert_eq!(
                            std::fs::read(root.path().join(path)).unwrap(),
                            if rollback {
                                b"after".as_slice()
                            } else {
                                b"before".as_slice()
                            }
                        );
                    }
                    assert!(std::fs::read_dir(root.path())
                        .unwrap()
                        .map(Result::unwrap)
                        .all(|entry| !entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".davinci-txn-")));
                    let recovered = TransactionCoordinator::new(root.path(), owner).unwrap();
                    if !rollback {
                        recovered.apply(&preview.id, &|_| Ok(()), None).unwrap();
                    }
                    recovered.rollback(&preview.id, &|_| Ok(()), None).unwrap();
                    assert_eq!(
                        recovered.status(&preview.id).unwrap().state,
                        TransactionState::RolledBack
                    );
                    for path in ["a.txt", "b.txt"] {
                        assert_eq!(std::fs::read(root.path().join(path)).unwrap(), b"before");
                    }
                }
            }
        }
    }
}
