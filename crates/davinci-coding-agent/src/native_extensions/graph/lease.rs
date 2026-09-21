//! An OS-held workspace lease: process death releases ownership without stale-PID guesses.
use std::fs::{File, OpenOptions};
use std::path::Path;

pub struct WorkspaceLease {
    _file: File,
}

impl WorkspaceLease {
    pub fn acquire(cwd: &Path) -> Result<Self, String> {
        let cwd = cwd
            .canonicalize()
            .map_err(|error| format!("Cannot resolve graph workspace: {error}"))?;
        // The ownership namespace must not move when legacy runs are migrated.
        let parent = cwd.join(super::store::CONFIG_DIR).join("graph");
        // Reject redirected state directories before creating anything under
        // them. Both aliases of the workspace resolve to this same lock file.
        let mut directory = cwd.clone();
        for component in parent
            .strip_prefix(&cwd)
            .map_err(|error| error.to_string())?
            .components()
        {
            directory.push(component);
            match std::fs::symlink_metadata(&directory) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => return Err("Graph lease directory is not an ordinary directory".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match std::fs::create_dir(&directory) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => {
                            return Err(format!("Cannot create graph lease directory: {error}"))
                        }
                    }
                    let metadata =
                        std::fs::symlink_metadata(&directory).map_err(|error| error.to_string())?;
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        return Err("Graph lease directory was redirected".into());
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        let lock_path = parent.join("controller.lock");
        if let Ok(metadata) = std::fs::symlink_metadata(&lock_path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err("Graph lease file is not an ordinary file".into());
            }
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0).custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let unavailable = |error| {
            format!("Graph workspace ownership unavailable (another controller may be running): {error}")
        };
        let file = options.open(lock_path).map_err(unavailable)?;
        if !file.metadata().map_err(unavailable)?.is_file() {
            return Err("Graph lease file is not an ordinary file".into());
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: file owns a valid descriptor for this call; flock neither
            // takes ownership of it nor accesses Rust memory.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(unavailable(std::io::Error::last_os_error()));
            }
        }
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};
    use std::process::{Command, Stdio};

    #[test]
    fn workspace_lease_excludes_aliases_until_owner_drops() {
        let root = tempfile::tempdir().unwrap();
        let first = WorkspaceLease::acquire(root.path()).unwrap();
        assert!(WorkspaceLease::acquire(&root.path().join(".")).is_err());
        drop(first);
        assert!(WorkspaceLease::acquire(root.path()).is_ok());
    }

    #[test]
    fn workspace_lease_is_shared_across_legacy_and_current_run_stores() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".pi/graph/runs")).unwrap();
        let first = WorkspaceLease::acquire(root.path()).unwrap();
        std::fs::create_dir_all(root.path().join(".davinci/graph/runs")).unwrap();
        assert!(WorkspaceLease::acquire(root.path()).is_err());
        drop(first);
        assert!(WorkspaceLease::acquire(root.path()).is_ok());
    }

    #[test]
    fn workspace_lease_is_released_after_owner_process_is_killed() {
        const CHILD: &str = "DAVINCI_GRAPH_LEASE_FIXTURE";
        if let Some(root) = std::env::var_os(CHILD) {
            let _lease = WorkspaceLease::acquire(Path::new(&root)).unwrap();
            println!("LEASE_READY");
            std::io::stdout().flush().unwrap();
            let mut byte = [0];
            let _ = std::io::stdin().read_exact(&mut byte);
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native_extensions::graph::lease::tests::workspace_lease_is_released_after_owner_process_is_killed", "--nocapture"])
            .env(CHILD, root.path()).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            assert!(
                output.read_line(&mut line).unwrap() > 0,
                "child exited before acquiring ownership"
            );
            if line.contains("LEASE_READY") {
                break;
            }
            line.clear();
        }
        let refused = WorkspaceLease::acquire(root.path()).is_err();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(refused, "another process must not own the same workspace");
        assert!(WorkspaceLease::acquire(root.path()).is_ok());
    }
}
