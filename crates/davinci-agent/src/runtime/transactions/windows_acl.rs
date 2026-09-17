//! Bounded owner, group, DACL, integrity label and resource properties. No privilege changes.
use std::{ffi::c_void, fs::File, io, os::windows::io::AsRawHandle, ptr};

const DACL_INFORMATION: u32 = 4;
// Resource properties require READ_CONTROL to query and WRITE_DAC to restore,
// unlike audit SACLs. Do not request ACCESS_SYSTEM_SECURITY or enable privileges.
const ATTRIBUTE_INFORMATION: u32 = 0x20;
const LABEL_INFORMATION: u32 = 0x10;
const FILTERED_SACL_INFORMATION: u32 = ATTRIBUTE_INFORMATION | LABEL_INFORMATION;
const ACCESS_INFORMATION: u32 = 1 | 2 | DACL_INFORMATION | FILTERED_SACL_INFORMATION;
const MAX_DESCRIPTOR: usize = 64 * 1024;
const DACL_AUTO_INHERIT_REQ: u16 = 0x100;
const DACL_AUTO_INHERITED: u16 = 0x400;
const DACL_PROTECTED: u16 = 0x1000;
#[link(name = "advapi32")]
extern "system" {
    fn GetKernelObjectSecurity(
        handle: *mut c_void,
        information: u32,
        descriptor: *mut c_void,
        size: u32,
        needed: *mut u32,
    ) -> i32;
    fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
        descriptor: *const c_void,
        revision: u32,
        information: u32,
        text: *mut *mut u16,
        size: *mut u32,
    ) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
    fn GetSecurityDescriptorDacl(
        descriptor: *const c_void,
        present: *mut i32,
        acl: *mut *mut c_void,
        defaulted: *mut i32,
    ) -> i32;
    fn GetSecurityDescriptorSacl(
        descriptor: *const c_void,
        present: *mut i32,
        acl: *mut *mut c_void,
        defaulted: *mut i32,
    ) -> i32;
    fn GetAclInformation(acl: *const c_void, information: *mut u32, size: u32, class: i32) -> i32;
    fn GetSecurityDescriptorControl(
        descriptor: *const c_void,
        control: *mut u16,
        revision: *mut u32,
    ) -> i32;
    fn SetSecurityDescriptorControl(descriptor: *mut c_void, bits: u16, value: u16) -> i32;
    fn GetSecurityDescriptorOwner(
        descriptor: *const c_void,
        owner: *mut *mut c_void,
        defaulted: *mut i32,
    ) -> i32;
    fn GetSecurityDescriptorGroup(
        descriptor: *const c_void,
        group: *mut *mut c_void,
        defaulted: *mut i32,
    ) -> i32;
}
#[link(name = "ntdll")]
extern "system" {
    fn NtSetSecurityObject(handle: *mut c_void, information: u32, descriptor: *const c_void)
        -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}
#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

pub(super) fn read(file: &File) -> Result<String, String> {
    read_information(file, ACCESS_INFORMATION)
}

fn read_information(file: &File, information: u32) -> Result<String, String> {
    // Four-byte alignment is required for a self-relative security descriptor.
    let mut descriptor = vec![0u32; MAX_DESCRIPTOR / 4];
    let mut needed = 0;
    let mut text = ptr::null_mut();
    let mut length = 0;
    // SAFETY: live handle, aligned bounded buffer, and initialized output pointers.
    unsafe {
        if GetKernelObjectSecurity(
            file.as_raw_handle(),
            information,
            descriptor.as_mut_ptr().cast(),
            MAX_DESCRIPTOR as u32,
            &mut needed,
        ) == 0
        {
            return Err(format!("read source DACL: {}", io::Error::last_os_error()));
        }
        let (mut present, mut defaulted) = (0, 0);
        let mut sacl = ptr::null_mut();
        if GetSecurityDescriptorSacl(
            descriptor.as_ptr().cast(),
            &mut present,
            &mut sacl,
            &mut defaulted,
        ) == 0
        {
            return Err("invalid source resource attributes".into());
        }
        let mut acl_size = [0u32; 3];
        if present != 0
            && !sacl.is_null()
            && GetAclInformation(sacl, acl_size.as_mut_ptr(), 12, 2) == 0
        {
            return Err("invalid source resource attribute ACL".into());
        }
        // This is a filtered SACL containing only requested label/resource ACEs.
        // Absent, null and empty filtered SACLs mean no explicit label/properties.
        // Never normalize the DACL, where null and empty differ in authority.
        let encoded_information = (information & !FILTERED_SACL_INFORMATION)
            | if acl_size[0] != 0 {
                8 | (information & FILTERED_SACL_INFORMATION)
            } else {
                0
            };
        if ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.as_ptr().cast(),
            1,
            encoded_information,
            &mut text,
            &mut length,
        ) == 0
        {
            return Err(format!(
                "encode source DACL: {}",
                io::Error::last_os_error()
            ));
        }
        let _owned = Local(text.cast());
        if text.is_null() || length == 0 || length as usize > MAX_DESCRIPTOR {
            return Err("source DACL exceeds bound".into());
        }
        let words = std::slice::from_raw_parts(text, length as usize);
        let end = words
            .iter()
            .position(|&c| c == 0)
            .ok_or("source DACL is not terminated")?;
        String::from_utf16(&words[..end]).map_err(|e| e.to_string())
    }
}

pub(super) fn apply(file: &File, text: &str) -> Result<(), String> {
    if text.len() > MAX_DESCRIPTOR
        || !(text.starts_with("D:") || text.starts_with("O:"))
        || text.contains('\0')
    {
        return Err("invalid source DACL".into());
    }
    let words: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: bounded NUL-terminated text is parsed by Windows; allocated descriptor
    // and its ACL remain owned until NtSetSecurityObject has completed.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            words.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            return Err(format!("parse source DACL: {}", io::Error::last_os_error()));
        }
        let _owned = Local(descriptor);
        let (mut present, mut defaulted) = (0, 0);
        let mut acl = ptr::null_mut();
        let mut owner = ptr::null_mut();
        let mut group = ptr::null_mut();
        if GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) == 0
            || present == 0
            || acl.is_null()
        {
            return Err("source DACL is absent or invalid".into());
        }
        if GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) == 0
            || GetSecurityDescriptorGroup(descriptor, &mut group, &mut defaulted) == 0
            || (text.starts_with("O:") && (owner.is_null() || group.is_null()))
        {
            return Err("source owner or group is invalid".into());
        }
        let information = DACL_INFORMATION
            | FILTERED_SACL_INFORMATION
            | if owner.is_null() { 0 } else { 1 }
            | if group.is_null() { 0 } else { 2 };
        let (mut control, mut revision) = (0, 0);
        if GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0
            || SetSecurityDescriptorControl(
                descriptor,
                DACL_AUTO_INHERIT_REQ,
                if control & DACL_AUTO_INHERITED != 0 {
                    DACL_AUTO_INHERIT_REQ
                } else {
                    0
                },
            ) == 0
        {
            return Err("invalid source inheritance flags".into());
        }
        // Restore the captured descriptor on the already-open staging handle.
        // SetSecurityInfo applies the current automatic inheritance model and
        // changes legacy DACL control flags, breaking exact rollback identity.
        // This native call retains the descriptor's inheritance/protection bits
        // and uses the same WRITE_OWNER/WRITE_DAC access checks, without elevation.
        let status = NtSetSecurityObject(
            file.as_raw_handle(),
            information
                | if control & DACL_PROTECTED != 0 {
                    0x80000000
                } else {
                    0x20000000
                },
            descriptor,
        );
        if status < 0 {
            return Err(format!(
                "restore source DACL: {}",
                io::Error::from_raw_os_error(RtlNtStatusToDosError(status) as i32)
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::transactions::{ProposedChange, TransactionCoordinator, TransactionOwner};

    #[test]
    fn transaction_preserves_mandatory_integrity_label() {
        let root = tempfile::tempdir().unwrap();
        let directory =
            crate::runtime::cache::directory::Directory::open(root.path(), false).unwrap();
        let file = directory.stage_file("label.txt").unwrap();
        let words: Vec<u16> = "S:(ML;;NW;;;LW)".encode_utf16().chain(Some(0)).collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: private fixture, bounded terminated SDDL, no privilege changes.
        unsafe {
            assert_ne!(
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    words.as_ptr(),
                    1,
                    &mut descriptor,
                    ptr::null_mut()
                ),
                0
            );
            let _owned = Local(descriptor);
            let status = NtSetSecurityObject(file.as_raw_handle(), 0x10, descriptor);
            assert!(
                status >= 0,
                "label fixture: {}",
                RtlNtStatusToDosError(status)
            );
        }
        let before = read_information(&file, 7 | 0x10).unwrap();
        assert!(
            before.contains("(ML;;NW;;;LW)"),
            "fixture must have low integrity label"
        );
        drop(file);
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        for proposal in [
            ProposedChange::write("label.txt", b"after".to_vec()),
            ProposedChange::delete("label.txt"),
        ] {
            let preview = manager.preview(vec![proposal]).unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            let path = root.path().join("label.txt");
            if path.exists() {
                assert_eq!(
                    read_information(&File::open(&path).unwrap(), 7 | 0x10).unwrap(),
                    before,
                    "edit must preserve integrity label"
                );
            }
            manager.rollback(&preview.id, &|_| Ok(()), None).unwrap();
            assert_eq!(
                read_information(&File::open(&path).unwrap(), 7 | 0x10).unwrap(),
                before,
                "rollback must preserve integrity label"
            );
        }
    }

    fn set_resource_attribute(file: &File, value: &str) {
        let text = format!(r#"S:(RA;;;;;WD;("Department",TS,0,"{value}"))"#);
        let words: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: fixture text is terminated and its parsed descriptor remains
        // owned while Windows sets only resource properties on this test file.
        unsafe {
            assert_ne!(
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    words.as_ptr(),
                    1,
                    &mut descriptor,
                    ptr::null_mut()
                ),
                0,
                "{}",
                io::Error::last_os_error()
            );
            let _owned = Local(descriptor);
            let status = NtSetSecurityObject(file.as_raw_handle(), 0x20, descriptor);
            assert!(
                status >= 0,
                "resource attribute fixture failed: {}",
                RtlNtStatusToDosError(status)
            );
        }
    }

    #[test]
    fn transaction_preserves_security_resource_attributes() {
        let root = tempfile::tempdir().unwrap();
        let directory =
            crate::runtime::cache::directory::Directory::open(root.path(), false).unwrap();
        let file = directory.stage_file("resource.txt").unwrap();
        set_resource_attribute(&file, "Engineering");
        let before = read_information(&file, 7 | 0x20).unwrap();
        assert!(
            before.contains("Department"),
            "fixture must contain resource properties"
        );
        drop(file);
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        for proposal in [
            ProposedChange::write("resource.txt", b"after".to_vec()),
            ProposedChange::delete("resource.txt"),
        ] {
            let preview = manager.preview(vec![proposal]).unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            let path = root.path().join("resource.txt");
            if path.exists() {
                assert!(
                    read_information(&File::open(&path).unwrap(), 7 | 0x20).unwrap() == before,
                    "edit must preserve resource attributes"
                );
            }
            manager.rollback(&preview.id, &|_| Ok(()), None).unwrap();
            assert!(
                read_information(&File::open(&path).unwrap(), 7 | 0x20).unwrap() == before,
                "rollback must preserve resource attributes"
            );
        }
    }

    #[test]
    fn transaction_resource_attribute_changes_conflict() {
        use std::os::windows::fs::OpenOptionsExt;
        for rollback in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("resource.txt");
            std::fs::write(&path, b"before").unwrap();
            let original = std::fs::OpenOptions::new()
                .access_mode(0x60000)
                .open(&path)
                .unwrap();
            set_resource_attribute(&original, "Engineering");
            assert!(read_information(&original, 7 | 0x20)
                .unwrap()
                .contains("Engineering"));
            drop(original);
            let manager =
                TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
            let preview = manager
                .preview(vec![ProposedChange::write(
                    "resource.txt",
                    b"after".to_vec(),
                )])
                .unwrap();
            if rollback {
                manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            }
            let file = std::fs::OpenOptions::new()
                .access_mode(0x60000)
                .open(&path)
                .unwrap();
            set_resource_attribute(&file, "Changed");
            let expected = read_information(&file, 7 | 0x20).unwrap();
            assert!(expected.contains("Changed"));
            drop(file);
            let result = if rollback {
                manager.rollback(&preview.id, &|_| Ok(()), None)
            } else {
                manager.apply(&preview.id, &|_| Ok(()), None)
            };
            let error = result.unwrap_err();
            assert!(error.contains("conflict"), "{error}");
            assert!(
                read_information(&File::open(&path).unwrap(), 7 | 0x20).unwrap() == expected,
                "must preserve the other actor's resource properties"
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
    fn transaction_preserves_windows_creation_time_and_attributes() {
        use std::os::windows::fs::MetadataExt;
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
                info: *const BasicInfo,
                size: u32,
            ) -> i32;
        }
        let root = tempfile::tempdir().unwrap();
        let directory =
            crate::runtime::cache::directory::Directory::open(root.path(), false).unwrap();
        let file = directory.stage_file("metadata.txt").unwrap();
        let expected = BasicInfo {
            created: 132_000_000_000_000_000,
            accessed: 0,
            written: 0,
            changed: 0,
            attributes: 2 | 4 | 32 | 8192,
        };
        // SAFETY: live writable handle and correctly sized FILE_BASIC_INFO.
        assert_ne!(
            unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle(),
                    0,
                    &expected,
                    std::mem::size_of::<BasicInfo>() as u32,
                )
            },
            0
        );
        drop(file);
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        for proposal in [
            ProposedChange::write("metadata.txt", b"after".to_vec()),
            ProposedChange::delete("metadata.txt"),
        ] {
            let preview = manager.preview(vec![proposal]).unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            if root.path().join("metadata.txt").exists() {
                let metadata = std::fs::metadata(root.path().join("metadata.txt")).unwrap();
                assert_eq!(metadata.creation_time(), expected.created as u64);
                assert_eq!(metadata.file_attributes(), expected.attributes);
            }
            manager.rollback(&preview.id, &|_| Ok(()), None).unwrap();
            let metadata = std::fs::metadata(root.path().join("metadata.txt")).unwrap();
            assert_eq!(metadata.creation_time(), expected.created as u64);
            assert_eq!(metadata.file_attributes(), expected.attributes);
        }
    }

    #[test]
    fn legacy_inheritance_descriptor_survives_transaction_rollback() {
        #[link(name = "advapi32")]
        extern "system" {
            fn SetKernelObjectSecurity(
                handle: *mut c_void,
                information: u32,
                descriptor: *const c_void,
            ) -> i32;
        }
        let root = tempfile::tempdir().unwrap();
        let directory =
            crate::runtime::cache::directory::Directory::open(root.path(), false).unwrap();
        let file = directory.stage_file("legacy.txt").unwrap();
        let initial = read(&file).unwrap();
        let legacy = initial.replacen("D:AI", "D:", 1);
        let words: Vec<u16> = legacy.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: live writable file handle and Windows-owned parsed descriptor.
        unsafe {
            assert_ne!(
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    words.as_ptr(),
                    1,
                    &mut descriptor,
                    ptr::null_mut()
                ),
                0
            );
            let _owned = Local(descriptor);
            assert_ne!(
                SetKernelObjectSecurity(file.as_raw_handle(), ACCESS_INFORMATION, descriptor),
                0,
                "{}",
                io::Error::last_os_error()
            );
        }
        let before = read(&file).unwrap();
        assert!(
            !before.contains("D:AI"),
            "fixture must use legacy inheritance"
        );
        drop(file);
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        for proposal in [
            ProposedChange::write("legacy.txt", b"after".to_vec()),
            ProposedChange::delete("legacy.txt"),
        ] {
            let preview = manager.preview(vec![proposal]).unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            manager.rollback(&preview.id, &|_| Ok(()), None).unwrap();
            assert!(
                read(&File::open(root.path().join("legacy.txt")).unwrap()).unwrap() == before,
                "rollback must preserve the exact legacy descriptor"
            );
            assert_eq!(
                manager.status(&preview.id).unwrap().state,
                crate::runtime::transactions::TransactionState::RolledBack
            );
        }
    }

    #[test]
    fn protected_dacl_survives_edit_delete_and_recovery() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let directory =
            crate::runtime::cache::directory::Directory::open(root.path(), false).unwrap();
        let mut file = directory.stage_file("protected.txt").unwrap();
        file.write_all(b"before").unwrap();
        let initial = read(&file).unwrap();
        assert!(initial.starts_with("O:"), "owner SID missing: {initial}");
        assert!(initial.contains("G:"), "group SID missing: {initial}");
        // A non-default primary group must survive staging; copying only the
        // DACL would silently replace this with the creating token's group.
        let group = initial.find("G:").unwrap();
        let dacl = initial.find("D:").unwrap();
        let custom = format!("{}G:BU{}", &initial[..group], &initial[dacl..]);
        apply(&file, &custom.replacen("D:", "D:P", 1)).unwrap();
        let protected = read(&file).unwrap();
        assert!(protected.contains("D:P"));
        assert!(protected.contains("G:BU"));
        drop(file);
        let manager =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        for proposed in [
            ProposedChange::write("protected.txt", b"after".to_vec()),
            ProposedChange::delete("protected.txt"),
        ] {
            let preview = manager.preview(vec![proposed]).unwrap();
            manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            manager.rollback(&preview.id, &|_| Ok(()), None).unwrap();
            assert_eq!(
                read(&File::open(root.path().join("protected.txt")).unwrap()).unwrap(),
                protected
            );
            assert_eq!(
                std::fs::read(root.path().join("protected.txt")).unwrap(),
                b"before"
            );
        }
    }

    #[test]
    fn group_only_change_conflicts_before_apply_and_rollback() {
        use std::os::windows::fs::OpenOptionsExt;
        for rollback in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("file.txt");
            std::fs::write(&path, b"before").unwrap();
            let manager =
                TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
            let preview = manager
                .preview(vec![ProposedChange::write("file.txt", b"after".to_vec())])
                .unwrap();
            if rollback {
                manager.apply(&preview.id, &|_| Ok(()), None).unwrap();
            }
            let file = std::fs::OpenOptions::new()
                .access_mode(0xe0000)
                .open(&path)
                .unwrap();
            let current = read(&file).unwrap();
            let group = current.find("G:").unwrap();
            let dacl = current.find("D:").unwrap();
            let replacement = if &current[group..dacl] == "G:BU" {
                "BA"
            } else {
                "BU"
            };
            let changed = format!("{}G:{}{}", &current[..group], replacement, &current[dacl..]);
            apply(&file, &changed).unwrap();
            let expected_access = read(&file).unwrap();
            drop(file);
            let outcome = if rollback {
                manager.rollback(&preview.id, &|_| Ok(()), None)
            } else {
                manager.apply(&preview.id, &|_| Ok(()), None)
            }
            .unwrap_err();
            assert!(outcome.contains("conflict"), "{outcome}");
            assert_eq!(
                std::fs::read(&path).unwrap(),
                if rollback {
                    b"after".as_slice()
                } else {
                    b"before".as_slice()
                }
            );
            assert_eq!(read(&File::open(&path).unwrap()).unwrap(), expected_access);
        }
    }
}
