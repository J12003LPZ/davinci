//! Handle-bound Windows metadata preserved across staged source replacement.
use super::model::WindowsMetadata;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn DeviceIoControl(
        handle: *mut std::ffi::c_void,
        code: u32,
        input: *const std::ffi::c_void,
        input_size: u32,
        output: *mut std::ffi::c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut std::ffi::c_void,
    ) -> i32;
}

#[cfg(windows)]
pub(super) fn ensure_replaceable(file: &std::fs::File) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::AsRawHandle;
    ensure_snapshot_attributes(
        file.metadata()
            .map_err(|e| e.to_string())?
            .file_attributes(),
    )?;
    let mut identifier = [0u8; 64];
    let mut returned = 0;
    // Query only: never create, transfer, or delete filesystem object identities.
    // SAFETY: live synchronous source handle and a full FILE_OBJECTID_BUFFER.
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x9009c,
            std::ptr::null(),
            0,
            identifier.as_mut_ptr().cast(),
            64,
            &mut returned,
            std::ptr::null_mut(),
        )
    } != 0
    {
        return Err("transaction cannot preserve a Windows object identifier".into());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        // No object ID (local NTFS returns FILE_NOT_FOUND for this handle-bound
        // query), or a filesystem that does not implement this control.
        Some(2 | 4312 | 1 | 50) => Ok(()),
        _ => Err(format!("query transaction object identifier: {error}")),
    }
}

#[cfg(windows)]
fn short_name_present(file: &std::fs::File) -> Result<bool, String> {
    Ok(!short_name(file)?.is_empty())
}

#[cfg(windows)]
pub(super) fn short_name(file: &std::fs::File) -> Result<Vec<u16>, String> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle};
    #[repr(C)]
    struct IoStatus {
        status_or_pointer: usize,
        information: usize,
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationFile(
            handle: *mut c_void,
            status: *mut IoStatus,
            information: *mut c_void,
            length: u32,
            class: u32,
        ) -> i32;
    }
    let mut status = IoStatus {
        status_or_pointer: 0,
        information: 0,
    };
    // FILE_NAME_INFORMATION: byte length followed by UTF-16. An 8.3 name
    // fits this aligned fixed buffer; a larger response is rejected.
    let mut information = [0u32; 16];
    // SAFETY: synchronous owned file, correctly aligned live output buffers.
    let result = unsafe {
        NtQueryInformationFile(
            file.as_raw_handle(),
            &mut status,
            information.as_mut_ptr().cast(),
            64,
            21,
        )
    };
    if result as u32 == 0xc0000034 {
        // STATUS_OBJECT_NAME_NOT_FOUND: no alias.
        return Ok(Vec::new());
    }
    if result != 0 {
        return Err(format!(
            "cannot query transaction short name: NTSTATUS {result:#x}"
        ));
    }
    if status.information < 4 || status.information > 64 {
        return Err("invalid transaction short name response".into());
    }
    let byte_len = information[0] as usize;
    if byte_len > 24 || byte_len % 2 != 0 || byte_len > status.information - 4 {
        return Err("invalid transaction short name length".into());
    }
    // Decode only the reported UTF-16 units, never padding or a C terminator.
    let name: Vec<u16> = information[1..]
        .iter()
        .flat_map(|word| [*word as u16, (*word >> 16) as u16])
        .take(byte_len / 2)
        .collect();
    validate_short_name(&name)?;
    normalize_short_name(file, name)
}

#[cfg(windows)]
fn normalize_short_name(file: &std::fs::File, alias: Vec<u16>) -> Result<Vec<u16>, String> {
    use std::os::windows::io::AsRawHandle;
    if alias.is_empty() {
        return Ok(alias);
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFinalPathNameByHandleW(
            handle: *mut std::ffi::c_void,
            path: *mut u16,
            capacity: u32,
            flags: u32,
        ) -> u32;
    }
    let mut path = vec![0u16; 32768];
    // FILE_NAME_NORMALIZED resolves the primary name even if opened by alias.
    // SAFETY: pinned live file and an initialized buffer of the supplied size.
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            path.as_mut_ptr(),
            path.len() as u32,
            0,
        )
    } as usize;
    if length == 0 {
        return Err(format!(
            "query transaction primary filename: {}",
            std::io::Error::last_os_error()
        ));
    }
    if length >= path.len() {
        return Err("transaction primary filename exceeds limit".into());
    }
    let primary = path[..length]
        .rsplit(|unit| *unit == b'\\' as u16)
        .next()
        .filter(|name| !name.is_empty())
        .ok_or("invalid transaction primary filename")?;
    // NTFS may report a DOS spelling identical to the primary name. It is not
    // an independent alias, and setting it on replacement may return no alias.
    if names_equal(primary, &alias)? {
        Ok(Vec::new())
    } else {
        Ok(alias)
    }
}

pub(super) fn validate_short_name(name: &[u16]) -> Result<(), String> {
    if name.is_empty() {
        return Ok(());
    }
    let text = String::from_utf16(name).map_err(|_| "invalid transaction short name encoding")?;
    let mut parts = text.split('.');
    let base = parts.next().unwrap_or_default();
    let extension = parts.next();
    if name.len() > 12
        || base.is_empty()
        || base.encode_utf16().count() > 8
        || extension.is_some_and(|s| s.is_empty() || s.encode_utf16().count() > 3)
        || parts.next().is_some()
        || text.chars().any(|c| c <= ' ' || "\\/:*?\"<>|".contains(c))
    {
        return Err("invalid transaction short name component".into());
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn ensure_primary_name(name: &str, alias: &[u16]) -> Result<(), String> {
    if alias.is_empty() {
        return Ok(());
    }
    if names_equal(&name.encode_utf16().collect::<Vec<_>>(), alias)? {
        return Err("transaction requires the primary filename, not its short alias".into());
    }
    Ok(())
}

#[cfg(windows)]
fn names_equal(name: &[u16], alias: &[u16]) -> Result<bool, String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn CompareStringOrdinal(
            a: *const u16,
            a_len: i32,
            b: *const u16,
            b_len: i32,
            ignore_case: i32,
        ) -> i32;
    }
    // SAFETY: bounded live UTF-16 arrays; explicit lengths, no terminator required.
    let result = unsafe {
        CompareStringOrdinal(
            name.as_ptr(),
            name.len() as i32,
            alias.as_ptr(),
            alias.len() as i32,
            1,
        )
    };
    if result == 0 {
        return Err("cannot compare transaction short name".into());
    }
    Ok(result == 2)
}

#[cfg(windows)]
pub(super) fn prepare_stage(file: &std::fs::File) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileShortNameW(handle: *mut std::ffi::c_void, name: *const u16) -> i32;
    }
    // Only an exclusively created private stage may lose its generated alias.
    // Otherwise it could publish its temporary name as an alias of the source.
    if short_name_present(file)? {
        let empty = [0u16];
        // SAFETY: live private stage with DELETE access and a terminated empty name.
        if unsafe { SetFileShortNameW(file.as_raw_handle(), empty.as_ptr()) } == 0 {
            return Err(format!(
                "clear transaction staging short name: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    ensure_replaceable(file)
}

#[cfg(windows)]
pub(super) fn set_short_name(file: &std::fs::File, name: &[u16]) -> Result<(), String> {
    validate_short_name(name)?;
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileShortNameW(handle: *mut std::ffi::c_void, name: *const u16) -> i32;
    }
    let terminated: Vec<_> = name.iter().copied().chain(Some(0)).collect();
    // SAFETY: caller owns a pinned DELETE handle and a bounded terminated name.
    if unsafe { SetFileShortNameW(file.as_raw_handle(), terminated.as_ptr()) } == 0 {
        return Err(format!(
            "restore transaction short name: {}",
            std::io::Error::last_os_error()
        ));
    }
    if short_name(file)? != name {
        return Err("transaction short name did not round-trip".into());
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn ensure_snapshot_attributes(attributes: u32) -> Result<(), String> {
    // EFS read handles expose decrypted bytes. The JSON recovery journal is not
    // encrypted, so reject before capturing either the default or named streams.
    if attributes & 0x4000 != 0 {
        return Err("transaction cannot journal an encrypted Windows file".into());
    }
    Ok(())
}

pub(super) fn capture(metadata: &std::fs::Metadata) -> Option<WindowsMetadata> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Some(WindowsMetadata {
            created: metadata.creation_time(),
            attributes: metadata.file_attributes(),
        })
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        None
    }
}

#[cfg(windows)]
pub(super) fn restore(
    file: &std::fs::File,
    expected: Option<&WindowsMetadata>,
) -> Result<(), String> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle};
    let Some(expected) = expected else {
        return Ok(());
    };
    #[repr(C)]
    struct BasicInfo {
        created: i64,
        accessed: i64,
        written: i64,
        changed: i64,
        attributes: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileInformationByHandle(
            handle: *mut c_void,
            class: i32,
            information: *const BasicInfo,
            size: u32,
        ) -> i32;
    }
    // Zero/-1 timestamps have special API meanings and cannot restore a literal
    // creation time. Reject unrepresentable snapshots before source mutation.
    if expected.created == 0 || expected.created > i64::MAX as u64 {
        return Err("unrepresentable Windows transaction creation time".into());
    }
    let current =
        capture(&file.metadata().map_err(|e| e.to_string())?).ok_or("missing Windows metadata")?;
    if (current.attributes ^ expected.attributes) & 0x800 != 0 {
        // FSCTL_SET_COMPRESSION restores NTFS LZNT1 or clears inherited
        // compression on the private stage. No source is mutated on failure.
        let format: u16 = if expected.attributes & 0x800 != 0 {
            2
        } else {
            0
        };
        let mut returned = 0;
        // SAFETY: synchronous owned handle, live two-byte input, no output.
        if unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                0x9c040,
                (&format as *const u16).cast(),
                2,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(format!(
                "restore transaction compression: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    let info = BasicInfo {
        created: expected.created as i64,
        accessed: 0,
        written: 0,
        changed: 0,
        attributes: expected.attributes,
    };
    // SAFETY: the private stage owns the live writable handle; FILE_BASIC_INFO
    // has the documented C layout and the buffer remains live for the call.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            0,
            &info,
            std::mem::size_of::<BasicInfo>() as u32,
        )
    } == 0
    {
        return Err(format!(
            "restore Windows transaction metadata: {}",
            std::io::Error::last_os_error()
        ));
    }
    if capture(&file.metadata().map_err(|e| e.to_string())?).as_ref() != Some(expected) {
        return Err("cannot preserve Windows transaction metadata".into());
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::runtime::transactions::{ProposedChange, TransactionCoordinator, TransactionOwner};
    use std::os::windows::io::AsRawHandle;

    #[test]
    fn transaction_stage_does_not_publish_temporary_short_name() {
        use crate::runtime::cache::directory::Directory;
        use std::io::Write;
        #[link(name = "kernel32")]
        extern "system" {
            fn SetFileShortNameW(handle: *mut std::ffi::c_void, name: *const u16) -> i32;
        }
        let root = tempfile::tempdir().unwrap();
        let directory = Directory::open(root.path(), false).unwrap();
        let mut stage = directory.stage_file(".davinci-txn-private-stage").unwrap();
        let alias: Vec<u16> = "STAGE.TMP".encode_utf16().chain(Some(0)).collect();
        // SAFETY: exclusively created fixture with DELETE access and terminated name.
        assert_ne!(
            unsafe { SetFileShortNameW(stage.as_raw_handle(), alias.as_ptr()) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        assert!(short_name_present(&stage).unwrap());
        assert_eq!(
            short_name(&stage).unwrap(),
            "STAGE.TMP".encode_utf16().collect::<Vec<_>>()
        );
        prepare_stage(&stage).unwrap();
        assert!(!short_name_present(&stage).unwrap());
        assert!(short_name(&stage).unwrap().is_empty());
        stage.write_all(b"after").unwrap();
        stage.sync_all().unwrap();
        drop(stage);
        directory
            .replace_source(".davinci-txn-private-stage", "published.txt", false, true)
            .unwrap();
        assert_eq!(
            std::fs::read(root.path().join("published.txt")).unwrap(),
            b"after"
        );
        assert!(!root.path().join("STAGE.TMP").exists());
    }

    #[test]
    fn transaction_preserves_existing_short_name_across_edit_delete_and_recovery() {
        use std::os::windows::fs::OpenOptionsExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn SetFileShortNameW(handle: *mut std::ffi::c_void, name: *const u16) -> i32;
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source-with-an-explicit-alias.txt");
        std::fs::write(&path, b"before").unwrap();
        let stream = format!("{}:metadata", path.display());
        std::fs::write(&stream, b"named stream").unwrap();
        let file = std::fs::OpenOptions::new()
            .access_mode(0x00010000) // DELETE, required by SetFileShortNameW.
            .custom_flags(0x02000000) // FILE_FLAG_BACKUP_SEMANTICS.
            .open(&path).unwrap();
        let alias: Vec<u16> = "ALIAS.TXT".encode_utf16().chain(Some(0)).collect();
        // SAFETY: owned fixture handle and terminated short name. No privilege elevation.
        assert_ne!(
            unsafe { SetFileShortNameW(file.as_raw_handle(), alias.as_ptr()) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        drop(file);
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        for proposal in [
            ProposedChange::write("source-with-an-explicit-alias.txt", b"after".to_vec()),
            ProposedChange::delete("source-with-an-explicit-alias.txt"),
        ] {
            let deleting = proposal.bytes.is_none();
            let preview = manager.preview(vec![proposal]).unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            if deleting {
                assert!(!path.exists());
                assert!(!root.path().join("ALIAS.TXT").exists());
            } else {
                assert_eq!(std::fs::read(&path).unwrap(), b"after");
                assert_eq!(std::fs::read(&stream).unwrap(), b"named stream");
                assert_eq!(
                    std::fs::read(root.path().join("ALIAS.TXT")).unwrap(),
                    b"after"
                );
            }
            let recovered = TransactionCoordinator::new(root.path(), preview.owner).unwrap();
            recovered.rollback(&preview.id, &|_| Ok(()), None).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"before");
            assert_eq!(std::fs::read(&stream).unwrap(), b"named stream");
            assert_eq!(
                std::fs::read(root.path().join("ALIAS.TXT")).unwrap(),
                b"before"
            );
        }
    }

    #[test]
    fn transaction_encrypted_attributes_cannot_enter_plaintext_journal() {
        for attributes in [0x4000, 0x4020, 0x6002, u32::MAX] {
            let error = ensure_snapshot_attributes(attributes).unwrap_err();
            assert!(error.contains("encrypted"), "{error}");
        }
        for attributes in [0, 0x20, 0x800, 0x2002] {
            ensure_snapshot_attributes(attributes).unwrap();
        }
    }

    #[test]
    fn transaction_short_name_interrupted_publication_recovers_without_claiming_foreign_alias() {
        use super::super::{files, model::TransactionState, store::Store};
        for restoring in [false, true] {
            for collision in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let path = "source-with-short-name.txt";
                std::fs::write(root.path().join(path), b"before").unwrap();
                let (dir, name) = files::directory(root.path(), path, false).unwrap();
                set_short_name(
                    &dir.source_alias_file(&name).unwrap(),
                    &"ALIAS.TXT".encode_utf16().collect::<Vec<_>>(),
                )
                .unwrap();
                let owner = TransactionOwner::default();
                let manager = TransactionCoordinator::new(root.path(), owner.clone()).unwrap();
                let preview = manager
                    .preview(vec![ProposedChange::write(path, b"after".to_vec())])
                    .unwrap();
                if restoring {
                    manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
                }
                let root_path = root.path().canonicalize().unwrap();
                let store = Store::open(&root_path).unwrap();
                let mut record = store.load(&preview.id, &root_path, &owner, false).unwrap();
                let change = &mut record.changes[0];
                let content = if restoring {
                    change.before_bytes.as_deref()
                } else {
                    change.proposed_bytes.as_deref()
                };
                let (image, stage) =
                    files::stage(&root_path, path, content, &change.before).unwrap();
                if restoring {
                    change.restored = Some(image);
                    change.restore_name = stage.clone();
                    change.restore_alias_pending = true;
                    record.summary.state = TransactionState::RollingBack;
                } else {
                    change.proposed = image;
                    change.staged_name = stage.clone();
                    change.alias_pending = true;
                    record.summary.state = TransactionState::Applying;
                }
                store.begin(&preview.id).unwrap();
                store.save(&record).unwrap();
                // Reproduce a host exit after the durable intent and content rename,
                // before SetFileShortNameW. No test-only recovery path is involved.
                dir.replace_source(stage.as_deref().unwrap(), &name, true, true)
                    .unwrap();
                assert!(!root.path().join("ALIAS.TXT").exists());
                if collision {
                    std::fs::write(root.path().join("foreign.txt"), b"user").unwrap();
                    set_short_name(
                        &dir.source_alias_file("foreign.txt").unwrap(),
                        &"ALIAS.TXT".encode_utf16().collect::<Vec<_>>(),
                    )
                    .unwrap();
                }
                let interrupted_identity = files::capture(&root_path, path).unwrap().0.identity;
                let recovered = TransactionCoordinator::new(&root_path, owner.clone()).unwrap();
                let result = recovered.rollback(&preview.id, &|_| Ok(()), None);
                if collision {
                    assert!(result.unwrap_err().contains("occupied"));
                    assert_eq!(
                        std::fs::read(root.path().join("ALIAS.TXT")).unwrap(),
                        b"user"
                    );
                    assert_eq!(
                        std::fs::read(root.path().join(path)).unwrap(),
                        if restoring {
                            b"before".as_slice()
                        } else {
                            b"after".as_slice()
                        }
                    );
                    // Release only the fixture alias, then retry through a new host.
                    set_short_name(&dir.source_alias_file("foreign.txt").unwrap(), &[]).unwrap();
                    let retry = TransactionCoordinator::new(&root_path, owner).unwrap();
                    assert_eq!(
                        retry
                            .rollback(&preview.id, &|_| Ok(()), None)
                            .unwrap()
                            .state,
                        TransactionState::RolledBack
                    );
                    assert_eq!(
                        std::fs::read(root.path().join("foreign.txt")).unwrap(),
                        b"user"
                    );
                } else {
                    assert_eq!(result.unwrap().state, TransactionState::RolledBack);
                    assert_eq!(std::fs::read(root.path().join(path)).unwrap(), b"before");
                    assert_eq!(
                        std::fs::read(root.path().join("ALIAS.TXT")).unwrap(),
                        b"before"
                    );
                }
                assert_eq!(
                    std::fs::read(root.path().join("ALIAS.TXT")).unwrap(),
                    b"before"
                );
                if restoring {
                    assert_eq!(
                        files::capture(&root_path, path).unwrap().0.identity,
                        interrupted_identity
                    );
                }
            }
        }
    }

    #[test]
    fn transaction_short_name_only_changes_are_conflicts() {
        use super::super::files;
        for applied in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = "source-with-short-name.txt";
            std::fs::write(root.path().join(path), b"before").unwrap();
            let (dir, name) = files::directory(root.path(), path, false).unwrap();
            set_short_name(
                &dir.source_alias_file(&name).unwrap(),
                &"ALIAS.TXT".encode_utf16().collect::<Vec<_>>(),
            )
            .unwrap();
            let manager =
                TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
            let preview = manager
                .preview(vec![ProposedChange::write(path, b"after".to_vec())])
                .unwrap();
            if applied {
                manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            }
            set_short_name(&dir.source_alias_file(&name).unwrap(), &[]).unwrap();
            let result = if applied {
                manager.rollback(&preview.id, &|_| Ok(()), None)
            } else {
                manager.apply(&preview.id, &|_| Ok(()), None)
            };
            assert!(result.unwrap_err().contains("conflict"));
            assert!(!root.path().join("ALIAS.TXT").exists());
            assert_eq!(
                std::fs::read(root.path().join(path)).unwrap(),
                if applied {
                    b"after".as_slice()
                } else {
                    b"before".as_slice()
                }
            );
        }
    }

    #[test]
    fn transaction_primary_name_matching_its_short_name_remains_editable() {
        use super::super::files;
        let root = tempfile::tempdir().unwrap();
        let path = "token.mjs";
        std::fs::write(root.path().join(path), b"before").unwrap();
        let (dir, name) = files::directory(root.path(), path, false).unwrap();
        let alias: Vec<_> = "TOKEN.MJS".encode_utf16().collect();
        // Some NTFS volumes report the DOS spelling even when it names the
        // primary directory entry. Exercise that response without changing the
        // machine-wide short-name generation policy.
        assert!(
            normalize_short_name(&dir.source_file(&name).unwrap(), alias)
                .unwrap()
                .is_empty()
        );
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        let preview = manager
            .preview(vec![ProposedChange::write(path, b"after".to_vec())])
            .unwrap();
        manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
        assert_eq!(std::fs::read(root.path().join(path)).unwrap(), b"after");
        manager.rollback(&preview.id, &|_| Ok(()), None).unwrap();
        assert_eq!(std::fs::read(root.path().join(path)).unwrap(), b"before");
        assert!(short_name(&dir.source_file(&name).unwrap())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn transaction_short_alias_cannot_replace_the_primary_name() {
        use super::super::files;
        let root = tempfile::tempdir().unwrap();
        let path = "primary-source-name.txt";
        std::fs::write(root.path().join(path), b"before").unwrap();
        let (dir, name) = files::directory(root.path(), path, false).unwrap();
        set_short_name(
            &dir.source_alias_file(&name).unwrap(),
            &"ALIAS.TXT".encode_utf16().collect::<Vec<_>>(),
        )
        .unwrap();
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        for alias in ["ALIAS.TXT", "alias.txt"] {
            let error = manager
                .preview(vec![ProposedChange::write(alias, b"after".to_vec())])
                .unwrap_err();
            assert!(error.contains("primary filename"), "{error}");
        }
        assert_eq!(std::fs::read(root.path().join(path)).unwrap(), b"before");
        assert_eq!(
            std::fs::read(root.path().join("ALIAS.TXT")).unwrap(),
            b"before"
        );
    }

    #[test]
    fn transaction_short_name_validation_rejects_unsafe_components() {
        for name in [
            ".",
            "..",
            "A/B",
            "A\\B",
            "A:B",
            "A B",
            "A*B",
            "A\0B",
            "ABCDEFGHI",
            "A.LONG",
            "A.B.C",
            "A.",
        ] {
            assert!(
                validate_short_name(&name.encode_utf16().collect::<Vec<_>>()).is_err(),
                "{name:?}"
            );
        }
        assert!(validate_short_name(&[0xd800]).is_err());
        for name in ["", "ALIAS.TXT", "ABCDEFGH.XYZ", "FILE~1"] {
            validate_short_name(&name.encode_utf16().collect::<Vec<_>>()).unwrap();
        }
    }

    fn set_compression(file: &std::fs::File, format: u16) {
        #[link(name = "kernel32")]
        extern "system" {}
        let mut returned = 0;
        // SAFETY: live synchronous file and correctly sized compression format.
        assert_ne!(
            unsafe {
                DeviceIoControl(
                    file.as_raw_handle(),
                    0x9c040,
                    (&format as *const u16).cast(),
                    2,
                    std::ptr::null_mut(),
                    0,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
    }

    #[test]
    fn transaction_preserves_object_id_by_refusing_unrepresentable_replacement() {
        #[link(name = "kernel32")]
        extern "system" {}
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tracked.txt");
        std::fs::write(&path, b"before").unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut identifier = [0u8; 64];
        let mut returned = 0;
        // SAFETY: synchronous fixture handle and a full FILE_OBJECTID_BUFFER.
        assert_ne!(
            unsafe {
                DeviceIoControl(
                    file.as_raw_handle(),
                    0x900c0,
                    std::ptr::null(),
                    0,
                    identifier.as_mut_ptr().cast(),
                    64,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        assert_eq!(returned, 64);
        drop(file);
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        for proposal in [
            ProposedChange::write("tracked.txt", b"after".to_vec()),
            ProposedChange::delete("tracked.txt"),
        ] {
            let error = manager.preview(vec![proposal]).unwrap_err();
            assert!(error.contains("object identifier"), "{error}");
            assert_eq!(std::fs::read(&path).unwrap(), b"before");
            let mut after = [0u8; 64];
            let file = std::fs::File::open(&path).unwrap();
            // SAFETY: query only, using the same live fixture handle.
            assert_ne!(
                unsafe {
                    DeviceIoControl(
                        file.as_raw_handle(),
                        0x9009c,
                        std::ptr::null(),
                        0,
                        after.as_mut_ptr().cast(),
                        64,
                        &mut returned,
                        std::ptr::null_mut(),
                    )
                },
                0
            );
            assert_eq!(after, identifier);
        }
    }

    #[test]
    fn transaction_preserves_compression_after_edit_and_delete() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("compressed.txt");
        std::fs::write(&path, b"before").unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        set_compression(&file, 2);
        let before = capture(&file.metadata().unwrap()).unwrap();
        assert_ne!(before.attributes & 0x800, 0);
        drop(file);
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        for proposal in [
            ProposedChange::write("compressed.txt", b"after".to_vec()),
            ProposedChange::delete("compressed.txt"),
        ] {
            let preview = manager.preview(vec![proposal]).unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            if path.exists() {
                assert_eq!(
                    capture(&std::fs::metadata(&path).unwrap()).as_ref(),
                    Some(&before)
                );
            }
            manager.rollback(&preview.id, &|_| Ok(()), None).unwrap();
            assert_eq!(
                capture(&std::fs::metadata(&path).unwrap()).as_ref(),
                Some(&before)
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"before");
        }
    }

    #[test]
    fn transaction_clears_inherited_stage_compression() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(0x02000000) // FILE_FLAG_BACKUP_SEMANTICS for a directory handle.
            .open(root.path())
            .unwrap();
        set_compression(&directory, 2);
        drop(directory);
        let path = root.path().join("uncompressed.txt");
        std::fs::write(&path, b"before").unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert_ne!(
            capture(&file.metadata().unwrap()).unwrap().attributes & 0x800,
            0
        );
        set_compression(&file, 0);
        let before = capture(&file.metadata().unwrap()).unwrap();
        assert_eq!(before.attributes & 0x800, 0);
        drop(file);
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        for proposal in [
            ProposedChange::write("uncompressed.txt", b"after".to_vec()),
            ProposedChange::delete("uncompressed.txt"),
        ] {
            let preview = manager.preview(vec![proposal]).unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            if path.exists() {
                assert_eq!(
                    capture(&std::fs::metadata(&path).unwrap()).as_ref(),
                    Some(&before)
                );
                assert_eq!(std::fs::read(&path).unwrap(), b"after");
            }
            manager.rollback(&preview.id, &|_| Ok(()), None).unwrap();
            assert_eq!(
                capture(&std::fs::metadata(&path).unwrap()).as_ref(),
                Some(&before)
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"before");
        }
    }

    #[test]
    fn transaction_attribute_changes_conflict_before_apply_and_rollback() {
        use std::os::windows::fs::MetadataExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn SetFileAttributesW(path: *const u16, attributes: u32) -> i32;
        }
        for rollback in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("attributes.txt");
            std::fs::write(&path, b"before").unwrap();
            let manager =
                TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
            let preview = manager
                .preview(vec![ProposedChange::write(
                    "attributes.txt",
                    b"after".to_vec(),
                )])
                .unwrap();
            if rollback {
                manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            }
            use std::os::windows::ffi::OsStrExt;
            let words: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let attributes = std::fs::metadata(&path).unwrap().file_attributes() | 2;
            // SAFETY: terminated fixture path; only its hidden attribute changes.
            assert_ne!(unsafe { SetFileAttributesW(words.as_ptr(), attributes) }, 0);
            let error = if rollback {
                manager.rollback(&preview.id, &|_| Ok(()), None)
            } else {
                manager.apply(&preview.id, &|_| Ok(()), None)
            }
            .unwrap_err();
            assert!(error.contains("conflict"), "{error}");
            assert_eq!(
                std::fs::metadata(&path).unwrap().file_attributes(),
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
}
