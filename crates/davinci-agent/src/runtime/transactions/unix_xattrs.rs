//! Bounded extended attributes on pinned file descriptors.
use std::collections::BTreeMap;

pub(super) const MAX_BYTES: usize = 1024 * 1024;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
const MAX_NAMES: usize = 64 * 1024;
const MAX_ATTRIBUTES: usize = 64;

pub(super) fn validate(attributes: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    if attributes.len() > MAX_ATTRIBUTES
        || attributes
            .iter()
            .any(|(name, _)| name.is_empty() || name.len() > 255 || name.contains('\0'))
        || attributes
            .iter()
            .map(|(name, value)| name.len().saturating_add(value.len()))
            .sum::<usize>()
            > MAX_BYTES
    {
        return Err("transaction extended attributes exceed bounds or have invalid names".into());
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn parse_names(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if bytes.len() > MAX_NAMES || bytes.last() != Some(&0) {
        return Err("invalid extended attribute name list".into());
    }
    let mut names = Vec::new();
    for name in bytes[..bytes.len() - 1].split(|&b| b == 0) {
        if name.is_empty() || name.len() > 255 || names.len() == MAX_ATTRIBUTES {
            return Err("invalid extended attribute name list".into());
        }
        names.push(
            std::str::from_utf8(name)
                .map_err(|_| "non-UTF8 extended attribute name unsupported")?
                .to_owned(),
        );
    }
    names.sort();
    if names.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("duplicate extended attribute name".into());
    }
    Ok(names)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use super::*;
    use std::{ffi::CString, fs::File, io, os::fd::AsRawFd};

    fn names(file: &File) -> Result<Vec<String>, String> {
        let mut bytes = vec![0u8; MAX_NAMES];
        // SAFETY: live descriptor and a writable buffer of the supplied size.
        let count = unsafe {
            #[cfg(target_os = "linux")]
            {
                libc::flistxattr(file.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len())
            }
            #[cfg(target_os = "macos")]
            {
                libc::flistxattr(file.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len(), 0)
            }
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            // A filesystem without xattr support has no attributes to copy.
            if error.raw_os_error() == Some(libc::ENOTSUP) {
                return Ok(Vec::new());
            }
            return Err(format!("list transaction extended attributes: {error}"));
        }
        let bytes = bytes
            .get(..count as usize)
            .ok_or("invalid extended attribute list length")?;
        parse_names(bytes)
    }

    pub(super) fn capture(file: &File) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let before = names(file)?;
        let mut attributes = BTreeMap::new();
        let mut remaining = MAX_BYTES;
        for name in &before {
            remaining = remaining
                .checked_sub(name.len())
                .ok_or("extended attribute byte limit")?;
            let key = CString::new(name.as_bytes()).map_err(|e| e.to_string())?;
            // One extra byte ensures a zero remaining budget still asks for data,
            // not just an unbounded size query. Oversized values fail closed.
            let mut value = vec![0u8; remaining + 1];
            // SAFETY: live descriptor, NUL-terminated name and bounded writable buffer.
            let count = unsafe {
                #[cfg(target_os = "linux")]
                {
                    libc::fgetxattr(
                        file.as_raw_fd(),
                        key.as_ptr(),
                        value.as_mut_ptr().cast(),
                        value.len(),
                    )
                }
                #[cfg(target_os = "macos")]
                {
                    libc::fgetxattr(
                        file.as_raw_fd(),
                        key.as_ptr(),
                        value.as_mut_ptr().cast(),
                        value.len(),
                        0,
                        0,
                    )
                }
            };
            if count < 0 {
                return Err(format!(
                    "read transaction extended attribute: {}",
                    io::Error::last_os_error()
                ));
            }
            remaining = remaining
                .checked_sub(count as usize)
                .ok_or("extended attribute byte limit")?;
            value.truncate(count as usize);
            attributes.insert(name.clone(), value);
        }
        if names(file)? != before {
            return Err("conflict: extended attribute names changed during capture".into());
        }
        validate(&attributes)?;
        Ok(attributes)
    }

    pub(super) fn restore(file: &File, expected: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
        validate(expected)?;
        for name in names(file)? {
            if expected.contains_key(&name) {
                continue;
            }
            let key = CString::new(name).map_err(|e| e.to_string())?;
            // SAFETY: live private stage descriptor and NUL-terminated name.
            let result = unsafe {
                #[cfg(target_os = "linux")]
                {
                    libc::fremovexattr(file.as_raw_fd(), key.as_ptr())
                }
                #[cfg(target_os = "macos")]
                {
                    libc::fremovexattr(file.as_raw_fd(), key.as_ptr(), 0)
                }
            };
            if result != 0 {
                return Err(format!(
                    "remove inherited stage attribute: {}",
                    io::Error::last_os_error()
                ));
            }
        }
        for (name, value) in expected {
            let key = CString::new(name.as_bytes()).map_err(|e| e.to_string())?;
            // SAFETY: live private stage descriptor and valid buffers for their sizes.
            let result = unsafe {
                #[cfg(target_os = "linux")]
                {
                    libc::fsetxattr(
                        file.as_raw_fd(),
                        key.as_ptr(),
                        value.as_ptr().cast(),
                        value.len(),
                        0,
                    )
                }
                #[cfg(target_os = "macos")]
                {
                    libc::fsetxattr(
                        file.as_raw_fd(),
                        key.as_ptr(),
                        value.as_ptr().cast(),
                        value.len(),
                        0,
                        0,
                    )
                }
            };
            if result != 0 {
                return Err(format!(
                    "restore transaction extended attribute: {}",
                    io::Error::last_os_error()
                ));
            }
        }
        if capture(file)? != *expected {
            return Err("cannot preserve transaction extended attributes".into());
        }
        Ok(())
    }
}

pub(super) fn capture(file: &std::fs::File) -> Result<BTreeMap<String, Vec<u8>>, String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        platform::capture(file)
    }
    #[cfg(windows)]
    {
        let _ = file;
        Ok(BTreeMap::new())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = file;
        Err("transaction extended attributes unsupported on this platform".into())
    }
}

#[cfg(unix)]
pub(super) fn restore(
    file: &std::fs::File,
    expected: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        platform::restore(file, expected)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (file, expected);
        Err("transaction extended attributes unsupported on this platform".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn transaction_xattrs_survive_edit_delete_and_fresh_recovery() {
        use super::super::{ProposedChange, TransactionCoordinator, TransactionOwner};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source");
        std::fs::write(&path, b"before").unwrap();
        let attributes = BTreeMap::from([
            ("user.davinci.binary".into(), vec![0, 255, 1]),
            ("user.davinci.empty".into(), Vec::new()),
        ]);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        restore(&file, &attributes).unwrap();
        drop(file);
        let owner = TransactionOwner::default();
        let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
        for proposal in [
            ProposedChange::write("source", b"after".to_vec()),
            ProposedChange::delete("source"),
        ] {
            let preview = manager.preview(vec![proposal]).unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            if path.exists() {
                assert_eq!(
                    capture(&std::fs::File::open(&path).unwrap()).unwrap(),
                    attributes
                );
            }
            TransactionCoordinator::new(root.path(), owner.clone())
                .unwrap()
                .rollback(&preview.id, &|_| Ok(()), None)
                .unwrap();
            assert_eq!(
                capture(&std::fs::File::open(&path).unwrap()).unwrap(),
                attributes
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"before");
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn transaction_xattr_only_changes_conflict_without_overwriting() {
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
            let attributes = BTreeMap::from([("user.davinci.new".into(), b"external".to_vec())]);
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            restore(&file, &attributes).unwrap();
            drop(file);
            let error = if rollback {
                manager.rollback(&preview.id, &|_| Ok(()), None)
            } else {
                manager.apply(&preview.id, &|_| Ok(()), None)
            }
            .unwrap_err();
            assert!(error.contains("conflict"), "{error}");
            assert_eq!(
                capture(&std::fs::File::open(&path).unwrap()).unwrap(),
                attributes
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
    fn transaction_xattr_names_are_bounded_and_unambiguous() {
        assert_eq!(
            parse_names(b"user.b\0user.a\0").unwrap(),
            vec!["user.a", "user.b"]
        );
        assert!(parse_names(b"").unwrap().is_empty());
        for input in [
            b"missing terminator".as_slice(),
            b"\0",
            b"a\0a\0",
            b"\xff\0",
        ] {
            assert!(parse_names(input).is_err());
        }
        assert!(parse_names(&[vec![b'a'; 256], vec![0]].concat()).is_err());
        let names: Vec<u8> = (0..65)
            .flat_map(|i| format!("user.{i}\0").into_bytes())
            .collect();
        assert!(parse_names(&names).is_err());
    }

    #[test]
    fn transaction_xattr_values_share_one_bounded_budget() {
        let mut attributes = BTreeMap::from([("user.a".into(), vec![0; MAX_BYTES - 6])]);
        assert!(validate(&attributes).is_ok());
        attributes.insert("user.b".into(), Vec::new());
        assert!(validate(&attributes).is_err());
        assert!(validate(&BTreeMap::from([("bad\0name".into(), Vec::new())])).is_err());
    }
}
