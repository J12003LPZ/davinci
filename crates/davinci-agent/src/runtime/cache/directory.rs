//! Handle-bound cache filesystem operations. Only generated single-component names.
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};

pub(crate) struct DirectoryLease {
    _file: File,
}

#[cfg(unix)]
impl Drop for DirectoryLease {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // A concurrent fork can retain this open file description until exec,
        // even with CLOEXEC. Closing our descriptor alone need not release flock.
        // SAFETY: the lease still owns this live descriptor.
        unsafe { libc::flock(self._file.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[derive(Debug)]
pub(crate) struct Directory {
    pub path: PathBuf,
    #[cfg(unix)]
    handle: File,
    #[cfg(windows)]
    _pins: Vec<File>,
}
fn valid_name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
        || name == "."
        || name == ".."
    {
        return Err(io::Error::other("invalid cache object name"));
    }
    Ok(())
}
fn valid_source_name(name: &str) -> io::Result<()> {
    if name.is_empty() || name.contains(['/', '\\', ':']) || name == "." || name == ".." {
        return Err(io::Error::other("invalid source name"));
    }
    Ok(())
}
#[cfg(unix)]
impl Directory {
    pub fn open(path: &Path, create: bool) -> io::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::OpenOptionsExt;
        if !path.is_absolute() {
            return Err(io::Error::other("cache root must be absolute"));
        }
        let mut handle = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")?;
        for part in path.components() {
            let Component::Normal(name) = part else {
                if part == Component::RootDir {
                    continue;
                }
                return Err(io::Error::other("invalid cache root"));
            };
            let name = std::ffi::CString::new(name.as_bytes()).map_err(io::Error::other)?;
            if create {
                // SAFETY: live directory descriptor and valid NUL-terminated component.
                let status = unsafe { libc::mkdirat(handle.as_raw_fd(), name.as_ptr(), 0o700) };
                if status != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
                {
                    return Err(io::Error::last_os_error());
                }
                if status == 0 {
                    handle.sync_all()?;
                }
            }
            let fd = unsafe {
                libc::openat(
                    handle.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            handle = unsafe { File::from_raw_fd(fd) };
        }
        Ok(Self {
            path: path.into(),
            handle,
        })
    }
    pub fn file(&self, name: &str, create: bool) -> io::Result<File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        valid_name(name)?;
        let name = std::ffi::CString::new(name).map_err(io::Error::other)?;
        let flags = if create {
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
        } else {
            libc::O_RDONLY
        };
        let fd = unsafe {
            libc::openat(
                self.handle.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("cache object is not regular"));
        }
        Ok(file)
    }
    pub fn source_file(&self, name: &str) -> io::Result<File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        valid_source_name(name)?;
        let name = std::ffi::CString::new(name).map_err(io::Error::other)?;
        let fd = unsafe {
            libc::openat(
                self.handle.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("source is not regular"));
        }
        Ok(file)
    }
    pub fn publish(&self, temp: &str, name: &str) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        valid_name(temp)?;
        valid_name(name)?;
        let temp = std::ffi::CString::new(temp).map_err(io::Error::other)?;
        let name = std::ffi::CString::new(name).map_err(io::Error::other)?;
        let fd = self.handle.as_raw_fd();
        if unsafe { libc::linkat(fd, temp.as_ptr(), fd, name.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.handle.sync_all()
    }
    pub fn remove(&self, name: &str) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        valid_name(name)?;
        let name = std::ffi::CString::new(name).map_err(io::Error::other)?;
        if unsafe { libc::unlinkat(self.handle.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn lease(&self) -> io::Result<DirectoryLease> {
        use std::os::fd::AsRawFd;
        let file = match self.file("active.lock", true) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                self.file("active.lock", false)?
            }
            Err(e) => return Err(e),
        };
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(DirectoryLease { _file: file })
    }
}

#[cfg(windows)]
impl Directory {
    pub fn open(path: &Path, create: bool) -> io::Result<Self> {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        if !path.is_absolute() {
            return Err(io::Error::other("cache root must be absolute"));
        }
        let mut current = PathBuf::new();
        let mut pins = Vec::new();
        for component in path.components() {
            if matches!(component, Component::ParentDir | Component::CurDir) {
                return Err(io::Error::other("invalid cache root"));
            }
            current.push(component);
            if !matches!(component, Component::Normal(_)) {
                continue;
            }
            if create {
                match fs::create_dir(&current) {
                    Ok(()) => (),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                    Err(e) => return Err(e),
                }
            }
            // Pin each ancestor against replacement, with OPEN_REPARSE_POINT.
            let file = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02000000 | 0x00200000)
                .open(&current)?;
            let metadata = file.metadata()?;
            if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
                return Err(io::Error::other("linked cache directory denied"));
            }
            pins.push(file);
        }
        Ok(Self {
            path: path.into(),
            _pins: pins,
        })
    }
    pub fn file(&self, name: &str, create: bool) -> io::Result<File> {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        valid_name(name)?;
        let mut options = OpenOptions::new();
        options
            .read(!create)
            .write(create)
            .create_new(create)
            .share_mode(1 | 2 | 4)
            .custom_flags(0x00200000);
        let file = options.open(self.path.join(name))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("linked cache object denied"));
        }
        Ok(file)
    }
    pub fn source_file(&self, name: &str) -> io::Result<File> {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        valid_source_name(name)?;
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(self.path.join(name))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("linked source denied"));
        }
        Ok(file)
    }
    pub fn publish(&self, temp: &str, name: &str) -> io::Result<()> {
        valid_name(temp)?;
        valid_name(name)?;
        fs::hard_link(self.path.join(temp), self.path.join(name))
    }
    pub fn remove(&self, name: &str) -> io::Result<()> {
        valid_name(name)?;
        fs::remove_file(self.path.join(name))
    }
    pub fn lease(&self) -> io::Result<DirectoryLease> {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .custom_flags(0x00200000)
            .open(self.path.join("active.lock"))?;
        if file.metadata()?.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("linked cache lease denied"));
        }
        Ok(DirectoryLease { _file: file })
    }
}

impl Directory {
    pub(crate) fn identity(&self) -> io::Result<String> {
        #[cfg(unix)]
        {
            file_identity(&self.handle)
        }
        #[cfg(windows)]
        {
            let file = self
                ._pins
                .last()
                .ok_or_else(|| io::Error::other("directory has no identity handle"))?;
            file_identity(file)
        }
    }

    pub(crate) fn stage_file(&self, name: &str) -> io::Result<File> {
        #[cfg(unix)]
        {
            self.file(name, true)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            valid_name(name)?;
            // GENERIC_READ/WRITE (compression restore), READ_CONTROL, WRITE_DAC
            // WRITE_OWNER and DELETE (clear generated short name) only on our
            // exclusive new staging file.
            OpenOptions::new()
                .write(true)
                .access_mode(0xc00f0000)
                .create_new(true)
                .share_mode(1)
                .custom_flags(0x02200000)
                .open(self.path.join(name))
        }
    }

    pub(crate) fn check_current(&self) -> io::Result<()> {
        let current = Self::open(&self.path, false)?;
        if current.identity()? != self.identity()? {
            return Err(io::Error::other("directory identity changed"));
        }
        Ok(())
    }

    /// Replace a single ordinary source entry from an exclusively created sibling.
    /// Callers verify the destination identity immediately before this operation.
    pub(crate) fn replace_source(
        &self,
        temp: &str,
        name: &str,
        existing: bool,
        _staged_metadata: bool,
    ) -> io::Result<()> {
        valid_name(temp)?;
        valid_source_name(name)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let temp = std::ffi::CString::new(temp).map_err(io::Error::other)?;
            let name = std::ffi::CString::new(name).map_err(io::Error::other)?;
            let fd = self.handle.as_raw_fd();
            // SAFETY: live directory handle and validated NUL-terminated names.
            if existing {
                if unsafe { libc::renameat(fd, temp.as_ptr(), fd, name.as_ptr()) } != 0 {
                    return Err(io::Error::last_os_error());
                }
            } else {
                // One atomic no-clobber rename: a link/unlink pair leaves an
                // ambiguous two-link postimage if the host exits between calls.
                #[cfg(target_os = "linux")]
                let result = unsafe {
                    libc::syscall(
                        libc::SYS_renameat2,
                        fd,
                        temp.as_ptr(),
                        fd,
                        name.as_ptr(),
                        libc::RENAME_NOREPLACE,
                    )
                };
                #[cfg(target_os = "macos")]
                let result = unsafe {
                    libc::renameatx_np(fd, temp.as_ptr(), fd, name.as_ptr(), libc::RENAME_EXCL)
                };
                #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                let result = {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "atomic no-clobber source rename is unavailable",
                    ));
                };
                if result != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            self.handle.sync_all()
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            extern "system" {
                fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
                fn ReplaceFileW(
                    replaced: *const u16,
                    replacement: *const u16,
                    backup: *const u16,
                    flags: u32,
                    exclude: *mut std::ffi::c_void,
                    reserved: *mut std::ffi::c_void,
                ) -> i32;
            }
            let temp: Vec<u16> = self
                .path
                .join(temp)
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            let name: Vec<u16> = self
                .path
                .join(name)
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            // The transaction already stages and verifies its supported metadata
            // image. ReplaceFileW would merge/rewrite that DACL (including legacy
            // inheritance flags) after validation. Rename the staged image intact.
            // For creates, omit REPLACE_EXISTING to protect concurrent creators.
            let result = unsafe {
                if existing && !_staged_metadata {
                    // Journals do not stage metadata; retain their destination ACL.
                    ReplaceFileW(
                        name.as_ptr(),
                        temp.as_ptr(),
                        std::ptr::null(),
                        0,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                } else {
                    MoveFileExW(temp.as_ptr(), name.as_ptr(), 8 | u32::from(existing))
                }
            };
            if result == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    pub(crate) fn sync(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.handle.sync_all()
        }
        // Windows has no supported directory-fsync equivalent. File contents are
        // flushed separately; process-crash recovery does not promise power-loss atomicity.
        #[cfg(windows)]
        {
            Ok(())
        }
    }

    pub(crate) fn remove_source(&self, name: &str) -> io::Result<()> {
        valid_source_name(name)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let name = std::ffi::CString::new(name).map_err(io::Error::other)?;
            if unsafe { libc::unlinkat(self.handle.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                return Err(io::Error::last_os_error());
            }
            self.handle.sync_all()
        }
        #[cfg(windows)]
        {
            fs::remove_file(self.path.join(name))
        }
    }

    pub fn names(&self) -> io::Result<Vec<String>> {
        // Enumeration is advisory; every read/write/unlink is handle-confined.
        // Reject an overfull directory rather than perform unbounded maintenance.
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.path)?.take(65537) {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        if names.len() > 65536 {
            return Err(io::Error::other("cache directory entry limit exceeded"));
        }
        Ok(names)
    }
}

pub(crate) fn file_identity(file: &File) -> io::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
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
        let mut information = Information::default();
        // SAFETY: owned live handle and the documented BY_HANDLE_FILE_INFORMATION layout.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(format!(
            "{}:{}:{}",
            information.volume, information.index_high, information.index_low
        ))
    }
}

#[cfg(test)]
mod lease_tests {
    use super::*;

    #[test]
    fn directory_lease_excludes_contenders_until_drop() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let first = Directory::open(&path, false).unwrap();
        let second = Directory::open(&path, false).unwrap();
        let lease = first.lease().unwrap();
        assert!(second.lease().is_err());
        drop(lease);
        assert!(second.lease().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn directory_lease_releases_with_inherited_description_still_open() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let directory = Directory::open(&path, false).unwrap();
        let lease = directory.lease().unwrap();
        // dup shares the same open file description, just like a forked child.
        let inherited = lease._file.try_clone().unwrap();
        assert!(directory.lease().is_err());
        drop(lease);
        let next = directory
            .lease()
            .expect("the owner's scope releases its lock");
        drop(inherited);
        assert!(directory.lease().is_err());
        drop(next);
        assert!(directory.lease().is_ok());
    }
}
