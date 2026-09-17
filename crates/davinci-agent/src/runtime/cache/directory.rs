//! Handle-bound cache filesystem operations. Only generated single-component names.
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};

pub(super) struct Directory {
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
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
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
    pub fn lease(&self) -> io::Result<File> {
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
        Ok(file)
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
    pub fn lease(&self) -> io::Result<File> {
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
        Ok(file)
    }
}

impl Directory {
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
