use super::index::{RepoIndex, PARSER_VERSION, SCHEMA_VERSION};
use super::scanner::{linked, read_bounded, validate_relative, MAX_FILES};
use super::symbols::digest;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn directory(agent_dir: &Path, root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(agent_dir).map_err(|e| e.to_string())?;
    let base = agent_dir.canonicalize().map_err(|e| e.to_string())?;
    if base.starts_with(root) {
        return Err("index cache must be outside workspace".into());
    }
    let mut directory = base;
    for part in [
        "repo-index".into(),
        digest(root.to_string_lossy().as_bytes()),
    ] {
        directory.push(part);
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
        let meta = fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
        if linked(&meta) || !meta.is_dir() {
            return Err("linked cache path denied".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(directory)
}

/// OS-owned lease: released even if a worker crashes. No stale lockfile deletion.
pub(super) fn lease(directory: &Path) -> Result<File, String> {
    for _ in 0..1000 {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0).custom_flags(0x00200000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        match options.open(directory.join("refresh.lock")) {
            Ok(file) => {
                if linked(&file.metadata().map_err(|e| e.to_string())?) {
                    return Err("linked cache lease denied".into());
                }
                #[cfg(unix)]
                {
                    use std::os::fd::AsRawFd;
                    // SAFETY: the descriptor belongs to a live File.
                    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
                    {
                        let error = std::io::Error::last_os_error();
                        if error.kind() != std::io::ErrorKind::WouldBlock {
                            return Err(error.to_string());
                        }
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                }
                return Ok(file);
            }
            Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => {}
            Err(e) => return Err(e.to_string()),
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err("index_unavailable: refresh lease busy".into())
}

pub(super) fn load(directory: &Path, root: &str, config: &str) -> Option<RepoIndex> {
    let body = read_bounded(directory, Path::new("index.json"), MAX_CACHE_BYTES).ok()?;
    let index: RepoIndex = serde_json::from_str(&body).ok()?;
    if index.schema_version != SCHEMA_VERSION
        || index.parser_version != PARSER_VERSION
        || index.root != root
        || index.config_identity != config
        || index.files.len() > MAX_FILES
    {
        return None;
    }
    if index.files.iter().any(|(path, file)| {
        validate_relative(path).is_err()
            || &file.path != path
            || file.size > 1_000_000
            || file.symbols.len() + file.edges.len() > 20_000
            || file
                .symbols
                .iter()
                .any(|s| s.file != *path || s.id.len() != 64)
    }) || index.text_files.len() > MAX_FILES
        || index
            .text_files
            .iter()
            .chain(&index.metadata)
            .any(|p| validate_relative(p).is_err())
    {
        return None;
    }
    Some(index)
}

pub(super) fn save(directory: &Path, index: &RepoIndex) -> Result<(), String> {
    let bytes = serde_json::to_vec(index).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_CACHE_BYTES {
        return Err("cache byte limit".into());
    }
    let target = directory.join("index.json");
    if fs::symlink_metadata(&target).is_ok_and(|m| linked(&m)) {
        return Err("linked cache file denied".into());
    }
    let temporary = directory.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e: std::io::Error| e.to_string())
}
