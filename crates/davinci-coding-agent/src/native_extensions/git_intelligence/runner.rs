use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn executable() -> Result<PathBuf, String> {
    let path = std::env::var_os("PATH").ok_or("Git executable unavailable")?;
    for dir in std::env::split_paths(&path).filter(|dir| dir.is_absolute()) {
        let candidate = dir.join(if cfg!(windows) { "git.exe" } else { "git" });
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .map_err(|_| "cannot resolve Git executable".into());
        }
    }
    Err("Git executable unavailable".into())
}

pub fn git_root(cwd: &Path) -> Result<PathBuf, String> {
    let canonical = cwd
        .canonicalize()
        .map_err(|e| format!("invalid path: {e}"))?;
    for ancestor in canonical.ancestors() {
        let dot_git = ancestor.join(".git");
        if dot_git.exists() {
            return Ok(ancestor.to_path_buf());
        }
    }
    Err(format!("not a git repository: {}", cwd.display()))
}

pub fn is_git_repo(cwd: &Path) -> bool {
    git_root(cwd).is_ok()
}

pub fn is_shallow_repo(root: &Path) -> bool {
    if root.join(".git").join("shallow").exists() {
        return true;
    }
    match run(root, &["rev-parse", "--is-shallow-repository"]) {
        Ok(out) => std::str::from_utf8(&out)
            .map(|s| s.trim() == "true")
            .unwrap_or(false),
        Err(_) => false,
    }
}

pub fn validate_revision(rev: &str) -> Result<(), String> {
    if rev.is_empty() {
        return Err("revision cannot be empty".into());
    }
    if rev.starts_with('-') {
        return Err(format!(
            "invalid revision '{rev}': option injection rejected"
        ));
    }
    if rev.len() > 256 {
        return Err(format!(
            "invalid revision '{rev}': length exceeds 256 bytes"
        ));
    }
    if rev.chars().any(char::is_control) {
        return Err(format!(
            "invalid revision '{rev}': contains control characters"
        ));
    }
    Ok(())
}

pub fn validate_path(root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    if rel_path.is_empty() {
        return Ok(root.to_path_buf());
    }
    let p = Path::new(rel_path);
    if p.is_absolute() {
        return Err(format!(
            "path '{rel_path}' must be relative to repository root"
        ));
    }
    for comp in p.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err(format!("path traversal rejected in '{rel_path}'"));
        }
    }
    let target = root.join(p);
    Ok(target)
}

pub fn resolve_commit(root: &Path, rev: &str) -> Result<String, String> {
    validate_revision(rev)?;
    let expression = format!("{rev}^{{commit}}");
    let bytes = run(
        root,
        &["rev-parse", "--verify", "--end-of-options", &expression],
    )?;
    let id = std::str::from_utf8(&bytes)
        .map_err(|_| "invalid revision identity")?
        .trim();
    if ![40, 64].contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid commit SHA '{id}'"));
    }
    Ok(id.into())
}

pub fn current_head(root: &Path) -> Result<Option<String>, String> {
    match run_status(root, &["symbolic-ref", "--quiet", "HEAD"]) {
        Ok((bytes, status)) if status.success() => {
            let name = std::str::from_utf8(&bytes)
                .map_err(|_| "invalid HEAD reference")?
                .trim();
            if !name.starts_with("refs/heads/") || name.chars().any(char::is_control) {
                return Err("invalid HEAD branch reference".into());
            }
            let (_, exists) = run_status(root, &["show-ref", "--verify", "--quiet", name])?;
            if exists.code() == Some(1) {
                return Ok(None); // Unborn branch (empty repo)
            }
        }
        _ => {}
    }
    match resolve_commit(root, "HEAD") {
        Ok(sha) => Ok(Some(sha)),
        Err(_) => Ok(None),
    }
}

pub fn current_branch(root: &Path) -> Result<Option<String>, String> {
    let (bytes, status) = run_status(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    if status.success() {
        let branch = std::str::from_utf8(&bytes)
            .map_err(|_| "invalid branch name")?
            .trim();
        if !branch.is_empty() {
            return Ok(Some(branch.to_string()));
        }
    }
    Ok(None)
}

pub fn run(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    run_interruptible(root, args, &|| false, Duration::from_secs(10))
}

pub fn run_status(
    root: &Path,
    args: &[&str],
) -> Result<(Vec<u8>, std::process::ExitStatus), String> {
    run_status_interruptible(root, args, &|| false, Duration::from_secs(10))
}

pub fn run_interruptible(
    root: &Path,
    args: &[&str],
    cancelled: &dyn Fn() -> bool,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let (bytes, status) = run_status_interruptible(root, args, cancelled, timeout)?;
    if status.success() {
        Ok(bytes)
    } else {
        let msg = String::from_utf8_lossy(&bytes);
        Err(format!("git command failed: {}", msg.trim()))
    }
}

pub fn run_status_interruptible(
    root: &Path,
    args: &[&str],
    cancelled: &dyn Fn() -> bool,
    timeout: Duration,
) -> Result<(Vec<u8>, std::process::ExitStatus), String> {
    if cancelled() {
        return Err("git execution cancelled".into());
    }
    let exe = executable()?;
    if exe.starts_with(root) {
        return Err("repository-owned Git executable denied".into());
    }
    let mut command = Command::new(exe);
    command
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Sanitize environment
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.pager=cat",
        ])
        .args(args);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start Git process: {e}"))?;
    let stdout = child.stdout.take().ok_or("git stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("git stderr unavailable")?;

    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let res = bounded_read_combined(stdout, stderr, 16 * 1024 * 1024);
        let _ = tx.send(res);
    });

    let deadline = Instant::now() + timeout;
    loop {
        if cancelled() {
            terminate_child(&mut child);
            return Err("git execution cancelled".into());
        }
        if Instant::now() >= deadline {
            terminate_child(&mut child);
            return Err("git execution timed out".into());
        }
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(Ok(bytes)) => loop {
                if cancelled() {
                    terminate_child(&mut child);
                    return Err("git execution cancelled".into());
                }
                if let Some(status) = child
                    .try_wait()
                    .map_err(|e| format!("cannot inspect child: {e}"))?
                {
                    return Ok((bytes, status));
                }
                if Instant::now() >= deadline {
                    terminate_child(&mut child);
                    return Err("git execution timed out".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            },
            Ok(Err(error)) => {
                terminate_child(&mut child);
                return Err(error);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
            Err(_) => {
                terminate_child(&mut child);
                return Err("git process reader failed".into());
            }
        }
    }
}

fn terminate_child(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{}", child.id())])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn bounded_read_combined(
    stdout: impl Read,
    stderr: impl Read,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut out_reader = stdout.take(limit + 1);
    out_reader
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read git stdout: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err("git process stdout exceeded byte limit".into());
    }

    if bytes.is_empty() {
        let mut err_bytes = Vec::new();
        let mut err_reader = stderr.take(4096);
        let _ = err_reader.read_to_end(&mut err_bytes);
        if !err_bytes.is_empty() {
            return Ok(err_bytes);
        }
    }

    Ok(bytes)
}
