//! Bounded owner, group and DACL snapshots. No privilege changes.
use std::{ffi::c_void, fs::File, io, os::windows::io::AsRawHandle, ptr};

const DACL_INFORMATION: u32 = 4;
const ACCESS_INFORMATION: u32 = 1 | 2 | DACL_INFORMATION;
const MAX_DESCRIPTOR: usize = 64 * 1024;
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
    fn GetSecurityDescriptorControl(
        descriptor: *const c_void,
        control: *mut u16,
        revision: *mut u32,
    ) -> i32;
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
    fn SetSecurityInfo(
        handle: *mut c_void,
        kind: u32,
        information: u32,
        owner: *mut c_void,
        group: *mut c_void,
        dacl: *mut c_void,
        sacl: *mut c_void,
    ) -> u32;
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
    // Four-byte alignment is required for a self-relative security descriptor.
    let mut descriptor = vec![0u32; MAX_DESCRIPTOR / 4];
    let mut needed = 0;
    let mut text = ptr::null_mut();
    let mut length = 0;
    // SAFETY: live handle, aligned bounded buffer, and initialized output pointers.
    unsafe {
        if GetKernelObjectSecurity(
            file.as_raw_handle(),
            ACCESS_INFORMATION,
            descriptor.as_mut_ptr().cast(),
            MAX_DESCRIPTOR as u32,
            &mut needed,
        ) == 0
        {
            return Err(format!("read source DACL: {}", io::Error::last_os_error()));
        }
        if ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.as_ptr().cast(),
            1,
            ACCESS_INFORMATION,
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
    // and its ACL remain owned until SetSecurityInfo has completed.
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
        let (mut present, mut defaulted, mut control, mut revision) = (0, 0, 0, 0);
        let mut acl = ptr::null_mut();
        let mut owner = ptr::null_mut();
        let mut group = ptr::null_mut();
        if GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) == 0
            || present == 0
            || acl.is_null()
            || GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0
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
            | if owner.is_null() { 0 } else { 1 }
            | if group.is_null() { 0 } else { 2 };
        let inheritance = if control & 0x1000 != 0 {
            0x80000000
        } else {
            0x20000000
        };
        let error = SetSecurityInfo(
            file.as_raw_handle(),
            1,
            information | inheritance,
            owner,
            group,
            acl,
            ptr::null_mut(),
        );
        if error != 0 {
            return Err(format!(
                "restore source DACL: {}",
                io::Error::from_raw_os_error(error as i32)
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
