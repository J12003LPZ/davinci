//! Create and verify the same protected owner/System DACL on every reopen.
use std::{
    ffi::c_void,
    fs::OpenOptions,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    ptr,
};

const PRIVATE_DACL: &str = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)";

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

#[link(name = "advapi32")]
extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
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
}
#[link(name = "kernel32")]
extern "system" {
    fn CreateDirectoryW(path: *const u16, attributes: *const SecurityAttributes) -> i32;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: Windows allocated this descriptor/string with LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}

pub(super) fn private_directory(path: &Path, create: bool) -> Result<(), String> {
    if create {
        let sddl: Vec<u16> = PRIVATE_DACL.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: terminated UTF-16 input and initialized output pointer.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(format!(
                "cannot construct worker session ACL: {}",
                std::io::Error::last_os_error()
            ));
        }
        let _owned = Local(descriptor);
        let attributes = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            descriptor,
            inherit: 0,
        };
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: all referenced buffers remain live through this call.
        if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(format!("cannot create private worker directory: {error}"));
            }
        }
    }
    super::ordinary_directory(path)?;
    // READ_CONTROL; BACKUP_SEMANTICS opens a directory, OPEN_REPARSE_POINT
    // prevents following a replacement junction while verifying its handle.
    let file = OpenOptions::new()
        .access_mode(0x20000)
        .custom_flags(0x02200000)
        .open(path)
        .map_err(|error| format!("cannot inspect worker directory ACL: {error}"))?;
    {
        use std::os::windows::fs::MetadataExt;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err("worker directory handle is redirected".into());
        }
    }
    let mut descriptor = vec![0u32; 16 * 1024];
    let (mut needed, mut length) = (0, 0);
    let mut text = ptr::null_mut();
    // SAFETY: live handle, aligned 64KiB buffer, initialized output pointers.
    unsafe {
        if GetKernelObjectSecurity(
            file.as_raw_handle(),
            4,
            descriptor.as_mut_ptr().cast(),
            64 * 1024,
            &mut needed,
        ) == 0
        {
            return Err(format!(
                "cannot read worker directory ACL: {}",
                std::io::Error::last_os_error()
            ));
        }
        if ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.as_ptr().cast(),
            1,
            4,
            &mut text,
            &mut length,
        ) == 0
        {
            return Err(format!(
                "cannot encode worker directory ACL: {}",
                std::io::Error::last_os_error()
            ));
        }
        let _owned = Local(text.cast());
        if text.is_null() || length == 0 || length > 64 * 1024 {
            return Err("invalid worker directory ACL length".into());
        }
        let words = std::slice::from_raw_parts(text, length as usize);
        let end = words
            .iter()
            .position(|word| *word == 0)
            .unwrap_or(words.len());
        let actual = String::from_utf16(&words[..end]).map_err(|error| error.to_string())?;
        if actual != PRIVATE_DACL {
            return Err(
                "worker session directory ACL is not private to its owner and System".into(),
            );
        }
    }
    Ok(())
}
