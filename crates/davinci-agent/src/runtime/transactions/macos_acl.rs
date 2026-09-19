//! Descriptor-bound macOS ACLs, serialized in Apple's portable external format.
//! Layout: Libc/include/sys/acl.h and xnu/bsd/sys/kauth.h.
const HEADER_BYTES: usize = 44;
const ENTRY_BYTES: usize = 24;
const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = HEADER_BYTES + ENTRY_BYTES * MAX_ENTRIES;

pub(super) fn validate(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < HEADER_BYTES || bytes.len() > MAX_BYTES {
        return Err("invalid macOS ACL size".into());
    }
    let count = u32::from_be_bytes(bytes[36..40].try_into().unwrap()) as usize;
    if bytes[..4] != 0x012c_c16du32.to_be_bytes()
        || bytes[4..36].iter().any(|&b| b != 0)
        || count > MAX_ENTRIES
        || bytes.len() != HEADER_BYTES + ENTRY_BYTES * count
    {
        return Err("invalid macOS ACL header or entry count".into());
    }
    Ok(())
}

pub(super) fn capture(file: &std::fs::File) -> Result<Option<Vec<u8>>, String> {
    #[cfg(target_os = "macos")]
    {
        platform::capture(file)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = file;
        Ok(None)
    }
}

#[cfg(target_os = "macos")]
pub(super) use platform::restore;

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::{ffi::c_void, fs::File, io, os::fd::AsRawFd};
    const ACL_TYPE_EXTENDED: i32 = 0x100;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: i32, kind: i32) -> *mut c_void;
        fn acl_set_fd_np(fd: i32, acl: *mut c_void, kind: i32) -> i32;
        fn acl_copy_ext(buffer: *mut c_void, acl: *mut c_void, size: isize) -> isize;
        fn acl_copy_int(buffer: *const c_void) -> *mut c_void;
        fn acl_free(object: *mut c_void) -> i32;
    }
    struct Acl(*mut c_void);
    impl Drop for Acl {
        fn drop(&mut self) {
            // SAFETY: owned allocation from acl_get_fd_np or acl_copy_int.
            unsafe {
                acl_free(self.0);
            }
        }
    }
    fn error(action: &str) -> String {
        format!(
            "{action} macOS transaction ACL: {}",
            io::Error::last_os_error()
        )
    }
    pub(super) fn capture(file: &File) -> Result<Option<Vec<u8>>, String> {
        // SAFETY: live descriptor; supported ACL type.
        let raw = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
        if raw.is_null() {
            if io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                return Ok(None);
            }
            return Err(error("read"));
        }
        // The documented remove-ACL sentinel is not an allocated ACL.
        if raw as usize == 1 {
            return Ok(None);
        }
        let acl = Acl(raw);
        // u32 storage supplies the alignment required by kauth_filesec.
        let mut storage = [0u32; MAX_BYTES / 4];
        // SAFETY: valid ACL and aligned writable buffer with its exact byte size.
        let size = unsafe { acl_copy_ext(storage.as_mut_ptr().cast(), acl.0, MAX_BYTES as isize) };
        if size < 0 || size as usize > MAX_BYTES {
            return Err(error("serialize"));
        }
        // SAFETY: bounded initialized storage, copied before its lifetime ends.
        let bytes =
            unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), size as usize) }
                .to_vec();
        validate(&bytes)?;
        Ok(Some(bytes))
    }
    pub(crate) fn restore(file: &File, expected: Option<&[u8]>) -> Result<(), String> {
        if capture(file)?.as_deref() == expected {
            return Ok(());
        }
        let acl = if let Some(bytes) = expected {
            // acl_copy_int has no length argument: validate BEFORE calling it.
            validate(bytes)?;
            let mut storage = [0u32; MAX_BYTES / 4];
            for (word, chunk) in storage.iter_mut().zip(bytes.chunks_exact(4)) {
                *word = u32::from_ne_bytes(chunk.try_into().unwrap());
            }
            // SAFETY: aligned complete header plus exactly the declared entries.
            let raw = unsafe { acl_copy_int(storage.as_ptr().cast()) };
            if raw.is_null() {
                return Err(error("decode"));
            }
            Some(Acl(raw))
        } else {
            None
        };
        // _FILESEC_REMOVE_ACL is the documented pointer-valued sentinel 1.
        let raw = acl.as_ref().map_or(1usize as *mut c_void, |acl| acl.0);
        // SAFETY: private stage descriptor and owned ACL or documented sentinel.
        if unsafe { acl_set_fd_np(file.as_raw_fd(), raw, ACL_TYPE_EXTENDED) } != 0 {
            return Err(error("restore"));
        }
        if capture(file)?.as_deref() != expected {
            return Err("cannot preserve macOS transaction ACL".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    fn add_acl(path: &std::path::Path, entry: &str) {
        let output = std::process::Command::new("/bin/chmod")
            .args(["+a", entry])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_acl_survives_edit_delete_and_fresh_recovery() {
        use super::super::{ProposedChange, TransactionCoordinator, TransactionOwner};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source");
        std::fs::write(&path, b"before").unwrap();
        add_acl(&path, "everyone deny execute");
        let expected = capture(&std::fs::File::open(&path).unwrap()).unwrap();
        assert!(expected.is_some());
        let owner = TransactionOwner::default();
        for proposal in [
            ProposedChange::write("source", b"after".to_vec()),
            ProposedChange::delete("source"),
        ] {
            let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
            let preview = manager.preview(vec![proposal]).unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            if path.exists() {
                assert_eq!(
                    capture(&std::fs::File::open(&path).unwrap()).unwrap(),
                    expected
                );
            }
            TransactionCoordinator::new(root.path(), owner.clone())
                .unwrap()
                .rollback(&preview.id, &|_| Ok(()), None)
                .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"before");
            assert_eq!(
                capture(&std::fs::File::open(&path).unwrap()).unwrap(),
                expected
            );
        }
        // Removing an inherited stage ACL is distinct from installing an empty ACL.
        restore(&std::fs::File::open(&path).unwrap(), None).unwrap();
        assert!(capture(&std::fs::File::open(&path).unwrap())
            .unwrap()
            .is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_acl_only_changes_refuse_apply_and_rollback() {
        use super::super::{ProposedChange, TransactionCoordinator, TransactionOwner};
        for rollback in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("source");
            std::fs::write(&path, b"before").unwrap();
            let manager =
                TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
            let preview = manager
                .preview(vec![ProposedChange::write("source", b"after".to_vec())])
                .unwrap();
            if rollback {
                manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            }
            add_acl(&path, "everyone deny execute");
            let expected = capture(&std::fs::File::open(&path).unwrap()).unwrap();
            let result = if rollback {
                manager.rollback(&preview.id, &|_| Ok(()), None)
            } else {
                manager.apply(&preview.id, &|_| Ok(()), None)
            };
            assert!(result.unwrap_err().contains("conflict"));
            assert_eq!(
                capture(&std::fs::File::open(&path).unwrap()).unwrap(),
                expected
            );
            assert_eq!(
                std::fs::read(&path).unwrap(),
                if rollback {
                    b"after".as_slice()
                } else {
                    b"before".as_slice()
                }
            );
        }
    }

    #[test]
    fn macos_acl_external_format_is_bounded_before_ffi() {
        let mut bytes = vec![0; HEADER_BYTES];
        bytes[..4].copy_from_slice(&0x012c_c16du32.to_be_bytes());
        assert!(validate(&bytes).is_ok());
        for length in 0..HEADER_BYTES {
            assert!(validate(&bytes[..length]).is_err());
        }
        bytes[36..40].copy_from_slice(&1u32.to_be_bytes());
        assert!(validate(&bytes).is_err());
        bytes.resize(HEADER_BYTES + ENTRY_BYTES, 0);
        assert!(validate(&bytes).is_ok());
        bytes[36..40].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(validate(&bytes).is_err());
        bytes[36..40].copy_from_slice(&128u32.to_be_bytes());
        bytes.resize(MAX_BYTES, 0);
        assert!(validate(&bytes).is_ok());
        bytes.push(0);
        assert!(validate(&bytes).is_err());
    }
}
