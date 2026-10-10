//! Supervised child processes.
//!
//! `run_bounded` runs a helper with bounded time, captured output, and
//! cancellation. It writes stdin independently, drains both output streams,
//! terminates the process tree when needed, and gives inherited pipes a short
//! grace period to close after the direct child exits. The tree is owned from
//! spawn (a Unix process group, a Windows job), so that cleanup still reaches
//! descendants once the direct child is gone.

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
/// descendants. Windows `kill_tree` uses `taskkill /T`, which needs the root
/// alive; `run_bounded` uses a job object instead.
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
///
/// Nothing here is looked up on PATH: cleanup runs during cancellation and
/// timeouts, often for a repository-controlled process, and a PATH entry the
/// repository or user environment planted must not get a `kill` or
/// `taskkill` of its own run. Unix signals the group directly; Windows runs
/// `taskkill.exe` from the system directory.
pub fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        // pid 0 would signal our own group, and a value past i32::MAX would
        // wrap to a negative group id: neither is a child of ours.
        let Ok(group) = i32::try_from(pid) else {
            return;
        };
        if group > 0 {
            // SAFETY: plain syscall; a negative pid selects the process group.
            unsafe { libc::kill(-group, libc::SIGTERM) };
        }
    }
    #[cfg(windows)]
    {
        if let Some(taskkill) = system_program("taskkill.exe") {
            let _ = Command::new(taskkill)
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = pid;
}

/// `name` in the Windows system directory (`GetSystemDirectoryW`, normally
/// `C:\Windows\System32`), when it exists. Not read from `%SystemRoot%`,
/// which the environment controls just like PATH.
#[cfg(windows)]
pub fn system_program(name: &str) -> Option<PathBuf> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 260];
    loop {
        // SAFETY: the buffer is valid for `len` u16s.
        let len = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if len == 0 {
            return None;
        }
        if (len as usize) < buffer.len() {
            buffer.truncate(len as usize);
            break;
        }
        // Too small: `len` is the size needed, terminator included.
        buffer.resize(len as usize, 0);
    }
    let candidate = PathBuf::from(OsString::from_wide(&buffer)).join(name);
    candidate.is_file().then_some(candidate)
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

/// What `run_bounded` owns of the child's process tree.
///
/// On Windows the child starts suspended and joins a fresh job before its
/// first instruction, so every descendant is a member. Terminating the job
/// still reaches them after the direct child has exited, which
/// `taskkill /PID <root> /T` cannot: it walks the tree from a root that no
/// longer exists. Without a job (assignment refused), it falls back to that.
struct Tree {
    #[cfg(windows)]
    job: Option<windows_job::Job>,
}

impl Tree {
    fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        #[cfg(windows)]
        {
            windows_job::spawn(command).map(|(child, job)| (child, Self { job }))
        }
        #[cfg(not(windows))]
        {
            command.spawn().map(|child| (child, Self {}))
        }
    }

    /// Kill every remaining process of the tree, including after the direct
    /// child has been reaped.
    fn kill(&self, child: &Child) {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
            return;
        }
        kill_tree(child.id());
    }
}

fn stop_child(child: &mut Child, tree: &Tree) {
    tree.kill(child);
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
    let (mut child, tree) = Tree::spawn(&mut command)?;

    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A child that exits without reading gives the writer BrokenPipe.
        if let Err(err) = std::thread::Builder::new()
            .name("davinci-child-stdin".into())
            .spawn(move || {
                let _ = pipe.write_all(&bytes);
            })
        {
            stop_child(&mut child, &tree);
            return Err(err);
        }
    }

    let stdout = match child.stdout.take() {
        Some(pipe) => pipe,
        None => {
            stop_child(&mut child, &tree);
            return Err(io::Error::other("child stdout pipe was not available"));
        }
    };
    let stderr = match child.stderr.take() {
        Some(pipe) => pipe,
        None => {
            stop_child(&mut child, &tree);
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
            stop_child(&mut child, &tree);
            return Err(err);
        }
    };
    let err = match drain(stderr, limits.output_cap, done_tx, "davinci-child-stderr") {
        Ok(err) => err,
        Err(err) => {
            stop_child(&mut child, &tree);
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
                stop_child(&mut child, &tree);
                return Err(err);
            }
        }
        let was_cancelled = cancelled();
        if was_cancelled || started.elapsed() >= limits.timeout {
            result.cancelled = was_cancelled;
            result.timed_out = !was_cancelled;
            stop_child(&mut child, &tree);
            break;
        }
        std::thread::sleep(POLL);
    }

    let grace_started = Instant::now();
    let mut closed = 0;
    while closed < 2 {
        let left = READER_GRACE.saturating_sub(grace_started.elapsed());
        if left.is_zero() {
            tree.kill(&child);
            break;
        }
        match done_rx.recv_timeout(left) {
            Ok(()) => closed += 1,
            Err(_) => {
                // A descendant still holds a pipe. Kill the whole tree and
                // return instead of joining a reader thread without a bound.
                tree.kill(&child);
                break;
            }
        }
    }
    (result.stdout, result.stdout_truncated) = take(&out);
    (result.stderr, result.stderr_truncated) = take(&err);
    Ok(result)
}

#[cfg(windows)]
mod windows_job {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};

    const CREATE_SUSPENDED: u32 = 0x0000_0004;
    const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
    const THREAD_SUSPEND_RESUME: u32 = 0x0002;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[repr(C)]
    struct ThreadEntry32 {
        size: u32,
        usage: u32,
        thread_id: u32,
        owner_process_id: u32,
        base_priority: i32,
        delta_priority: i32,
        flags: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn TerminateJobObject(job: *mut c_void, code: u32) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> *mut c_void;
        fn Thread32First(snapshot: *mut c_void, entry: *mut ThreadEntry32) -> i32;
        fn Thread32Next(snapshot: *mut c_void, entry: *mut ThreadEntry32) -> i32;
        fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> *mut c_void;
        fn ResumeThread(thread: *mut c_void) -> u32;
    }

    /// An unnamed, non-inheritable job that only this process can reach.
    /// No kill-on-close: like the Unix process group, members outlive the
    /// handle unless `run_bounded` decides to terminate them.
    pub(super) struct Job(OwnedHandle);

    impl Job {
        pub(super) fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.0.as_raw_handle(), 1);
            }
        }
    }

    /// Spawn suspended, join a job, then resume. The child runs no code
    /// before it is a member, so it cannot start a descendant outside it.
    /// Overrides any creation flags already set on `command`.
    pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, Option<Job>)> {
        command.creation_flags(CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        let job = assign(&child);
        if let Err(error) = resume(child.id()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok((child, job))
    }

    fn assign(child: &Child) -> Option<Job> {
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return None;
        }
        let job = Job(unsafe { OwnedHandle::from_raw_handle(raw) });
        let joined =
            unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), child.as_raw_handle()) } != 0;
        joined.then_some(job)
    }

    /// Resume the suspended child's threads. A process created suspended has
    /// exactly its primary thread, and std does not keep that handle, so it
    /// is found by owner process id. The id is not stale: the child is
    /// unreaped, so its id cannot be reused yet.
    fn resume(process_id: u32) -> io::Result<()> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot as isize == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return Err(io::Error::last_os_error());
        }
        let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
        let mut entry = ThreadEntry32 {
            size: std::mem::size_of::<ThreadEntry32>() as u32,
            usage: 0,
            thread_id: 0,
            owner_process_id: 0,
            base_priority: 0,
            delta_priority: 0,
            flags: 0,
        };
        let mut resumed = 0;
        let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
        while more {
            if entry.owner_process_id == process_id {
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.thread_id) };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                resumed += 1;
            }
            more = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
        }
        if resumed == 0 {
            return Err(io::Error::other("suspended child has no thread to resume"));
        }
        Ok(())
    }
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
        let name = file
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
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
