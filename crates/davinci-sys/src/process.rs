//! Supervised child processes.
//!
//! `run_bounded` runs a helper with bounded time, captured output, and
//! cancellation. It writes stdin independently, drains both output streams,
//! terminates the process tree when needed, and gives inherited pipes a short
//! grace period to close after the direct child exits.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(10);
const READER_GRACE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy)]
pub struct RunLimits {
    pub timeout: Duration,
    /// Per stream. Bytes beyond the cap are still read and discarded.
    pub output_cap: usize,
}

#[derive(Debug, Default)]
pub struct BoundedOutput {
    /// `None` when the child was killed for timeout or cancellation.
    pub status: Option<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub timed_out: bool,
    pub cancelled: bool,
}

/// Find `name` on PATH. On Windows, also try PATHEXT because `npm` is commonly
/// installed as `npm.cmd`. If no candidate exists, return `name` unchanged so
/// the eventual spawn error still identifies the requested program.
pub fn resolve_program(name: &str) -> PathBuf {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let pathext = if cfg!(windows) {
        Some(std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into()))
    } else {
        None
    };
    resolve_program_in(name, &path_var, pathext.as_deref()).unwrap_or_else(|| PathBuf::from(name))
}

/// Search the supplied PATH value, with optional Windows-style PATHEXT
/// expansion. Names that already contain a path are used as given, not
/// searched in PATH.
pub fn resolve_program_in(name: &str, path_var: &OsStr, pathext: Option<&str>) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return None;
    }
    let has_extension = Path::new(name).extension().is_some();
    for dir in std::env::split_paths(path_var) {
        // Empty PATH entries mean the current working directory on both Unix
        // and Windows. Never let an untrusted repository satisfy a trusted
        // program lookup through an empty component.
        if dir.as_os_str().is_empty() || !dir.is_absolute() {
            continue;
        }
        match pathext {
            Some(exts) if !has_extension => {
                for ext in exts.split(';').filter(|ext| !ext.is_empty()) {
                    let mut file = OsString::from(name);
                    file.push(ext.to_ascii_lowercase());
                    let candidate = dir.join(file);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
            _ => {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Put the child in its own Unix process group so `kill_tree` also reaches
/// descendants. Windows process-tree termination uses `taskkill /T`.
pub fn set_own_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

/// Kill `pid` and its descendants. On Unix, `pid` must be the leader of its
/// own process group (see [`set_own_process_group`]).
pub fn kill_tree(pid: u32) {
    if cfg!(windows) {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        let _ = Command::new("kill")
            // A negative PID selects a process group, not a signal option.
            .args(["-TERM", "--", &format!("-{pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[derive(Default)]
struct Captured {
    bytes: Vec<u8>,
    truncated: bool,
}

fn drain<R: Read + Send + 'static>(
    mut pipe: R,
    cap: usize,
    done: mpsc::Sender<()>,
    thread_name: &'static str,
) -> io::Result<Arc<Mutex<Captured>>> {
    let shared = Arc::new(Mutex::new(Captured::default()));
    let sink = Arc::clone(&shared);
    std::thread::Builder::new()
        .name(thread_name.into())
        .spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        let mut captured = sink.lock().unwrap_or_else(|err| err.into_inner());
                        let room = cap.saturating_sub(captured.bytes.len());
                        let keep = room.min(read);
                        captured.bytes.extend_from_slice(&chunk[..keep]);
                        if keep < read {
                            captured.truncated = true;
                        }
                    }
                }
            }
            let _ = done.send(());
        })?;
    Ok(shared)
}

fn take(shared: &Arc<Mutex<Captured>>) -> (Vec<u8>, bool) {
    let mut captured = shared.lock().unwrap_or_else(|err| err.into_inner());
    (std::mem::take(&mut captured.bytes), captured.truncated)
}

fn stop_child(child: &mut Child) {
    kill_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

/// Run a child while draining its pipes, bounding each captured stream, and
/// observing both the timeout and caller cancellation flag.
pub fn run_bounded(
    mut command: Command,
    stdin: Option<Vec<u8>>,
    limits: RunLimits,
    cancelled: &dyn Fn() -> bool,
) -> io::Result<BoundedOutput> {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    set_own_process_group(&mut command);
    let mut child = command.spawn()?;

    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A child that exits without reading gives the writer BrokenPipe.
        if let Err(err) = std::thread::Builder::new()
            .name("davinci-child-stdin".into())
            .spawn(move || {
                let _ = pipe.write_all(&bytes);
            })
        {
            stop_child(&mut child);
            return Err(err);
        }
    }

    let stdout = match child.stdout.take() {
        Some(pipe) => pipe,
        None => {
            stop_child(&mut child);
            return Err(io::Error::other("child stdout pipe was not available"));
        }
    };
    let stderr = match child.stderr.take() {
        Some(pipe) => pipe,
        None => {
            stop_child(&mut child);
            return Err(io::Error::other("child stderr pipe was not available"));
        }
    };
    let (done_tx, done_rx) = mpsc::channel();
    let out = match drain(
        stdout,
        limits.output_cap,
        done_tx.clone(),
        "davinci-child-stdout",
    ) {
        Ok(out) => out,
        Err(err) => {
            stop_child(&mut child);
            return Err(err);
        }
    };
    let err = match drain(stderr, limits.output_cap, done_tx, "davinci-child-stderr") {
        Ok(err) => err,
        Err(err) => {
            stop_child(&mut child);
            return Err(err);
        }
    };

    let started = Instant::now();
    let mut result = BoundedOutput::default();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                result.status = Some(status);
                break;
            }
            Ok(None) => {}
            Err(err) => {
                stop_child(&mut child);
                return Err(err);
            }
        }
        let was_cancelled = cancelled();
        if was_cancelled || started.elapsed() >= limits.timeout {
            result.cancelled = was_cancelled;
            result.timed_out = !was_cancelled;
            stop_child(&mut child);
            break;
        }
        std::thread::sleep(POLL);
    }

    let grace_started = Instant::now();
    let mut closed = 0;
    while closed < 2 {
        let left = READER_GRACE.saturating_sub(grace_started.elapsed());
        if left.is_zero() {
            kill_tree(child.id());
            break;
        }
        match done_rx.recv_timeout(left) {
            Ok(()) => closed += 1,
            Err(_) => {
                // A descendant still holds a pipe. Kill the process group and
                // return instead of joining a reader thread without a bound.
                kill_tree(child.id());
                break;
            }
        }
    }
    (result.stdout, result.stdout_truncated) = take(&out);
    (result.stderr, result.stderr_truncated) = take(&err);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn shell(script: &str) -> Command {
        if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", script]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            command
        }
    }

    fn limits(ms: u64) -> RunLimits {
        RunLimits {
            timeout: Duration::from_millis(ms),
            output_cap: 64 * 1024,
        }
    }

    #[test]
    fn captures_stdout_and_exit_status() {
        let out = run_bounded(shell("echo hello"), None, limits(10_000), &|| false).unwrap();
        assert!(out.status.unwrap().success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("hello"));
        assert!(!out.timed_out);
    }

    #[test]
    fn timeout_kills_the_child() {
        let script = if cfg!(windows) {
            "ping -n 30 127.0.0.1 >NUL"
        } else {
            "sleep 30"
        };
        let started = Instant::now();
        let out = run_bounded(shell(script), None, limits(300), &|| false).unwrap();
        assert!(out.timed_out);
        assert!(out.status.is_none());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn cancel_stops_the_child() {
        let script = if cfg!(windows) {
            "ping -n 30 127.0.0.1 >NUL"
        } else {
            "sleep 30"
        };
        let out = run_bounded(shell(script), None, limits(30_000), &|| true).unwrap();
        assert!(out.cancelled);
        assert!(!out.timed_out);
    }

    #[test]
    fn output_beyond_the_cap_is_truncated_not_blocking() {
        let script = if cfg!(windows) {
            "for /L %i in (1,1,300) do @echo 0123456789"
        } else {
            "i=0; while [ $i -lt 300 ]; do echo 0123456789; i=$((i+1)); done"
        };
        let limits = RunLimits {
            timeout: Duration::from_secs(20),
            output_cap: 100,
        };
        let out = run_bounded(shell(script), None, limits, &|| false).unwrap();
        assert_eq!(out.stdout.len(), 100);
        assert!(out.stdout_truncated);
        assert!(out.status.unwrap().success());
    }

    #[test]
    fn unread_large_stdin_does_not_block() {
        let payload = vec![b'x'; 4 * 1024 * 1024];
        let started = Instant::now();
        let out = run_bounded(shell("exit 0"), Some(payload), limits(10_000), &|| false).unwrap();
        assert!(out.status.unwrap().success());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn grandchild_holding_stdout_does_not_hang() {
        let started = Instant::now();
        let out = run_bounded(shell("sleep 30 & echo done"), None, limits(10_000), &|| {
            false
        })
        .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("done"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn resolve_program_in_uses_pathext_on_windows_style_lookup() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("npm.cmd"), "@echo off").unwrap();
        let path_var = std::env::join_paths([dir.path()]).unwrap();
        let found = resolve_program_in("npm", &path_var, Some(".COM;.EXE;.BAT;.CMD"));
        assert_eq!(found, Some(dir.path().join("npm.cmd")));
    }

    #[test]
    fn resolve_program_in_without_pathext_needs_exact_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tool"), "#!/bin/sh").unwrap();
        let path_var = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(
            resolve_program_in("tool", &path_var, None),
            Some(dir.path().join("tool"))
        );
        assert_eq!(resolve_program_in("missing", &path_var, None), None);
    }

    #[test]
    fn empty_path_entries_never_resolve_from_the_current_directory() {
        let file = tempfile::Builder::new()
            .prefix("davinci-path-entry-")
            .tempfile_in(".")
            .unwrap();
        let name = file
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(resolve_program_in(&name, OsStr::new(""), None), None);
    }

    #[test]
    fn relative_path_entries_never_resolve_from_the_current_directory() {
        let file = tempfile::Builder::new()
            .prefix("davinci-relative-path-entry-")
            .tempfile_in(".")
            .unwrap();
        let name = file.path().file_name().unwrap().to_string_lossy().into_owned();
        let relative = std::env::join_paths([std::path::PathBuf::from(".")]).unwrap();
        assert_eq!(resolve_program_in(&name, &relative, None), None);
    }

    #[test]
    fn names_with_a_separator_are_not_searched() {
        let path_var = std::ffi::OsString::new();
        assert_eq!(resolve_program_in("./npm", &path_var, Some(".CMD")), None);
        assert_eq!(resolve_program("./npm"), std::path::PathBuf::from("./npm"));
    }
}
