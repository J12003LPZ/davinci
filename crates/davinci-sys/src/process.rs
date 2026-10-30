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

/// `git` for inspecting a repository the user does not fully trust. Repository
/// config can name programs, so the ones Git would start on its own are turned
/// off: `core.fsmonitor`, hooks (`core.hooksPath` is the null file, which also
/// stops `post-index-change` after a status refresh), the user and system
/// attributes files, lazy fetching of missing objects, and every transport.
/// System and global config are replaced by the null file, and inherited
/// environment overrides that inject config, reroute the repository or its
/// objects, or change the visible refs are removed. Discovery from the working
/// directory and the normal index are kept. Callers add their own
/// `current_dir`, arguments and, if they need one, `GIT_INDEX_FILE` (set after
/// this call, so it wins).
///
/// What this does not stop: a clean/smudge/process filter that the repository's
/// own `.git/config` defines and its attributes select. Git has no switch for
/// "every filter", so inspecting a repository whose `.git` is hostile still
/// needs a sandbox or an in-process implementation. Do not describe this
/// constructor as safe execution for such a repository.
pub fn isolated_git_command() -> Command {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut command = Command::new("git");
    command.args([
        "-c",
        "safe.bareRepository=explicit",
        "-c",
        "core.fsmonitor=false",
        "-c",
    ]);
    command.arg(format!("core.hooksPath={null}"));
    command.arg("-c");
    command.arg(format!("core.attributesFile={null}"));
    command.args(["-c", "protocol.allow=never"]);
    for (key, _) in std::env::vars_os() {
        let Some(key_str) = key.to_str() else {
            continue;
        };
        if key_str.starts_with("GIT_CONFIG_KEY_") || key_str.starts_with("GIT_CONFIG_VALUE_") {
            command.env_remove(&key);
        }
    }
    for name in [
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_SYSTEM",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_EXTERNAL_DIFF",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(name);
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null)
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_ALLOW_PROTOCOL", "");
    command
}

/// [`isolated_git_command`] for a command that will run in `dir`, with the
/// repository's own filter drivers neutralized.
///
/// A clean/smudge/process filter is a program the repository's config names
/// and its attributes select; `status` and `diff` run it while comparing
/// content. Git has no switch for "no filters", so the filters present are
/// listed (`git config` runs none of them, and `--includes` follows config
/// includes; system and global config are the null file) and each is overridden
/// with an empty command, which Git treats as "no filter", plus
/// `required=false` so a skipped filter is not an error. The overrides travel
/// as `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_<n>`/`GIT_CONFIG_VALUE_<n>` on the
/// child, key and value separate: a `-c name=value` argument is split at the
/// first `=`, so a driver named `x=y` would override the wrong key and the real
/// driver would still run. The list is read once, when the command is built: a
/// config edited between that and the run is not covered, which is the limit of
/// this approach.
///
/// Fails closed: when the filters cannot be listed or a listed key cannot be
/// taken apart, the returned command is poisoned. Spawning it still succeeds
/// but git exits 128 on an invalid config key whose text names the reason, so
/// the caller sees an ordinary git failure instead of an unneutralized run.
///
/// Diff drivers (`diff.<name>.textconv` and `.command`) cannot be neutralized
/// this way: Git tries to spawn an empty driver command and the diff dies
/// ("cannot spawn"). The supported switches are `--no-ext-diff` and
/// `--no-textconv`, and every `diff`/`log -p`/`show` run through this command
/// must pass both.
pub fn isolated_git_command_in(dir: &Path) -> Command {
    let mut command = isolated_git_command();
    command.current_dir(dir);
    match driver_overrides(dir) {
        Ok(settings) => {
            command.env("GIT_CONFIG_COUNT", settings.len().to_string());
            for (index, (key, value)) in settings.iter().enumerate() {
                command.env(format!("GIT_CONFIG_KEY_{index}"), key);
                command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
            }
        }
        Err(reason) => {
            // A key with a space in its section name is never valid, whatever
            // the reason text holds.
            let reason: String = reason.chars().filter(|c| !c.is_control()).collect();
            command
                .env("GIT_CONFIG_COUNT", "1")
                .env(
                    "GIT_CONFIG_KEY_0",
                    format!("davinci inspection refused: {reason}"),
                )
                .env("GIT_CONFIG_VALUE_0", "1");
        }
    }
    command
}

/// `(key, value)` settings that disable every repository-defined filter found
/// in the configuration visible from `dir`.
fn driver_overrides(dir: &Path) -> Result<Vec<(String, String)>, String> {
    let output = isolated_git_command()
        .current_dir(dir)
        .args([
            "config",
            "--includes",
            "--name-only",
            "-z",
            "--get-regexp",
            "^filter\\.",
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot list filter drivers: {error}"))?;
    // Exit 1 is "no matching key"; any other failure means the filters are
    // unknown, which is not the same as there being none.
    match output.status.code() {
        Some(0) => filter_overrides(&output.stdout),
        Some(1) => Ok(Vec::new()),
        _ => Err(format!(
            "cannot list filter drivers: git config exited with {}",
            output.status
        )),
    }
}

/// Parse the NUL-separated key list `git config -z --name-only` prints into
/// override settings for every `filter.<name>.*` driver.
fn filter_overrides(stdout: &[u8]) -> Result<Vec<(String, String)>, String> {
    let text =
        std::str::from_utf8(stdout).map_err(|_| "a config key is not valid UTF-8".to_string())?;
    let mut names = std::collections::BTreeSet::new();
    for key in text.split('\0').filter(|key| !key.is_empty()) {
        // section.subsection.variable; the subsection may itself contain dots.
        let (Some(first), Some(last)) = (key.find('.'), key.rfind('.')) else {
            return Err(format!("config key {key:?} has no section"));
        };
        if first == last {
            // section.variable: no subsection, so no driver name.
            continue;
        }
        let name = &key[first + 1..last];
        if name.contains(['\n', '\0']) {
            return Err(format!(
                "filter driver name {name:?} has a control character"
            ));
        }
        names.insert(name.to_string());
    }
    let mut settings = Vec::with_capacity(names.len() * 4);
    for name in names {
        for driver in ["clean", "smudge", "process"] {
            settings.push((format!("filter.{name}.{driver}"), String::new()));
        }
        settings.push((format!("filter.{name}.required"), "false".to_string()));
    }
    Ok(settings)
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

    #[test]
    fn audit_wp01_isolated_git_command_pins_config_and_strips_overrides() {
        let command = isolated_git_command();
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "-c",
                "safe.bareRepository=explicit",
                "-c",
                "core.fsmonitor=false",
                "-c",
                &format!(
                    "core.hooksPath={}",
                    if cfg!(windows) { "NUL" } else { "/dev/null" }
                ),
                "-c",
                &format!(
                    "core.attributesFile={}",
                    if cfg!(windows) { "NUL" } else { "/dev/null" }
                ),
                "-c",
                "protocol.allow=never",
            ]
            .map(OsStr::new)
        );
        let envs: std::collections::HashMap<_, _> = command.get_envs().collect();
        assert_eq!(
            envs[OsStr::new("GIT_CONFIG_NOSYSTEM")],
            Some(OsStr::new("1"))
        );
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        assert_eq!(
            envs[OsStr::new("GIT_CONFIG_GLOBAL")],
            Some(OsStr::new(null))
        );
        for removed in [
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
        ] {
            assert_eq!(envs[OsStr::new(removed)], None, "{removed} must be removed");
        }
        for (name, value) in [
            ("GIT_ATTR_NOSYSTEM", "1"),
            ("GIT_OPTIONAL_LOCKS", "0"),
            ("GIT_NO_LAZY_FETCH", "1"),
            ("GIT_ALLOW_PROTOCOL", ""),
        ] {
            assert_eq!(envs[OsStr::new(name)], Some(OsStr::new(value)), "{name}");
        }
    }

    fn git_in(dir: &std::path::Path, args: &[&str]) -> String {
        let mut command = Command::new("git");
        command
            .current_dir(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1");
        let output = command.output().expect("git is available");
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn commit_repo(dir: &std::path::Path, content: &str) -> String {
        git_in(dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("f.txt"), content).unwrap();
        git_in(dir, &["add", "."]);
        git_in(dir, &["commit", "-q", "-m", "c"]);
        git_in(dir, &["rev-parse", "HEAD"])
    }

    /// Serializes tests that edit this process's `GIT_*` environment.
    static GIT_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Review finding 1 (environment half): inherited repository-routing
    /// variables must not change which repository is inspected. The assertions
    /// read the repository's actual revision and status, not the command.
    #[test]
    fn audit_wp01_isolated_git_ignores_a_hostile_inherited_environment() {
        let _guard = GIT_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let victim = tempfile::tempdir().unwrap();
        let decoy = tempfile::tempdir().unwrap();
        let victim_head = commit_repo(victim.path(), "one\n");
        let decoy_head = commit_repo(decoy.path(), "decoy\n");
        assert_ne!(victim_head, decoy_head);
        std::fs::write(victim.path().join("f.txt"), "changed\n").unwrap();

        // GIT_COMMON_DIR only matters beside GIT_DIR, which is stripped too; the
        // two below change what a plain `git` resolves from the same directory.
        let names = [
            (
                "GIT_OBJECT_DIRECTORY",
                decoy.path().join(".git").join("objects"),
            ),
            ("GIT_NAMESPACE", std::path::PathBuf::from("hostile")),
        ];
        // Control: the hostile environment does change plain Git's answer.
        for (name, value) in &names {
            std::env::set_var(name, value);
        }
        let plain = Command::new("git")
            .current_dir(victim.path())
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .args(["rev-parse", "--verify", "-q", "refs/heads/main^{commit}"])
            .output()
            .unwrap();
        let plain_head = String::from_utf8_lossy(&plain.stdout).trim().to_owned();
        let isolated_head = isolated_git_command()
            .current_dir(victim.path())
            .args(["rev-parse", "--verify", "-q", "refs/heads/main^{commit}"])
            .output()
            .unwrap();
        let isolated_status = isolated_git_command()
            .current_dir(victim.path())
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        for (name, _) in &names {
            std::env::remove_var(name);
        }
        assert_ne!(
            plain_head, victim_head,
            "control: the hostile environment must change plain git's answer"
        );
        assert_eq!(
            String::from_utf8_lossy(&isolated_head.stdout).trim(),
            victim_head
        );
        assert_eq!(
            String::from_utf8_lossy(&isolated_status.stdout).trim(),
            "M f.txt"
        );
    }

    /// Review finding 1 (hook half): a `post-index-change` hook runs when a
    /// status refresh writes the index. It must not under the isolated command,
    /// and must under plain Git, or the canary proves nothing.
    #[test]
    fn audit_wp01_isolated_git_does_not_run_repository_hooks() {
        let _guard = GIT_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let repo = tempfile::tempdir().unwrap();
        commit_repo(repo.path(), "one\n");
        let hooks = repo.path().join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let marker = repo.path().join("hook-ran");
        let hook = hooks.join("post-index-change");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\necho ran > '{}'\n",
                marker.to_string_lossy().replace('\\', "/")
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let stale = |dir: &std::path::Path| {
            // A same-length rewrite makes the stat data stale so status must
            // refresh and write the index.
            std::fs::write(dir.join("f.txt"), "one\n").unwrap();
            let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
            let file = std::fs::File::options()
                .write(true)
                .open(dir.join("f.txt"))
                .unwrap();
            file.set_modified(later).unwrap();
        };

        stale(repo.path());
        let isolated = isolated_git_command()
            .current_dir(repo.path())
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(isolated.status.success(), "{isolated:?}");
        assert!(
            !marker.exists(),
            "the repository hook ran under the isolated command"
        );

        // Positive control: plain Git runs the hook for the same repository.
        stale(repo.path());
        let plain = Command::new("git")
            .current_dir(repo.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(plain.status.success(), "{plain:?}");
        assert!(
            marker.exists(),
            "control: plain git must run post-index-change"
        );
    }

    /// Planner item A: repository-defined filters are not run by `status` or
    /// `diff` through `isolated_git_command_in`, diff drivers are not run when
    /// the diff passes `--no-ext-diff --no-textconv`, and plain git runs the
    /// very same drivers (positive control).
    #[test]
    fn audit_wp01_isolated_git_does_not_run_repository_filters_or_diff_drivers() {
        let _guard = GIT_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let repo = tempfile::tempdir().unwrap();
        let dir = repo.path();
        git_in(dir, &["init", "-q", "-b", "main"]);
        std::fs::write(
            dir.join(".gitattributes"),
            "f.txt filter=canary\nh.txt filter=lazy\ng.txt diff=viewer\n",
        )
        .unwrap();
        for name in ["f.txt", "h.txt", "g.txt"] {
            std::fs::write(dir.join(name), "one\n").unwrap();
        }
        git_in(dir, &["add", "."]);
        git_in(dir, &["commit", "-q", "-m", "c"]);

        let slash = |name: &str| dir.join(name).to_string_lossy().replace('\\', "/");
        let marks = ["clean-ran", "process-ran", "textconv-ran", "command-ran"];
        // Configure after the commit so setup does not run them.
        git_in(
            dir,
            &[
                "config",
                "filter.canary.clean",
                &format!("echo ran >> '{}'; cat", slash(marks[0])),
            ],
        );
        git_in(dir, &["config", "filter.canary.smudge", "cat"]);
        git_in(
            dir,
            &[
                "config",
                "filter.lazy.process",
                &format!("echo ran >> '{}'", slash(marks[1])),
            ],
        );
        git_in(
            dir,
            &[
                "config",
                "diff.viewer.textconv",
                &format!("sh -c 'echo ran >> {}; cat \"$1\"' --", slash(marks[2])),
            ],
        );
        git_in(
            dir,
            &[
                "config",
                "diff.viewer.command",
                &format!("echo ran >> '{}'", slash(marks[3])),
            ],
        );

        let make_stale = || {
            // Same length, newer mtime: status must compare content (clean
            // filter); and a real change so diff converts both sides.
            for name in ["f.txt", "h.txt"] {
                std::fs::write(dir.join(name), "one\n").unwrap();
                let file = std::fs::File::options()
                    .write(true)
                    .open(dir.join(name))
                    .unwrap();
                file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
                    .unwrap();
            }
            std::fs::write(dir.join("g.txt"), "two\n").unwrap();
        };
        let run = |mut command: Command| {
            let out = command.current_dir(dir).output().unwrap();
            assert!(out.status.success(), "{out:?}");
        };
        let ran = |names: &[&str]| names.iter().filter(|n| dir.join(n).exists()).count();
        let clear = || {
            for mark in marks {
                let _ = std::fs::remove_file(dir.join(mark));
            }
        };
        let plain = |args: &[&str]| {
            let mut c = Command::new("git");
            c.env("GIT_CONFIG_NOSYSTEM", "1")
                .env(
                    "GIT_CONFIG_GLOBAL",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                )
                .args(args);
            c
        };

        // Control: plain git runs the clean filter and the diff drivers.
        make_stale();
        run(plain(&["status", "--porcelain"]));
        run(plain(&["diff", "--ext-diff", "--textconv"]));
        assert!(
            dir.join(marks[0]).exists(),
            "control: clean filter must run under plain git"
        );
        assert!(
            dir.join(marks[2]).exists() || dir.join(marks[3]).exists(),
            "control: a diff driver must run under plain git"
        );
        clear();

        // Isolated: none of them.
        make_stale();
        let mut status = isolated_git_command_in(dir);
        status.args(["status", "--porcelain"]);
        run(status);
        let mut diff = isolated_git_command_in(dir);
        diff.args(["diff", "--no-ext-diff", "--no-textconv"]);
        run(diff);
        assert_eq!(ran(&marks), 0, "a repository filter or diff driver ran");

        // The real answer is still reported.
        let out = isolated_git_command_in(dir)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("g.txt"));
    }

    fn plain_git(args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .args(args);
        command
    }

    /// B003: a filter driver whose name contains `=` made the old
    /// `-c filter.<name>.clean=` override set the wrong key (Git splits at the
    /// first `=`), so the real driver still ran. The overrides now travel as
    /// separate `GIT_CONFIG_KEY_<n>`/`GIT_CONFIG_VALUE_<n>` entries.
    #[test]
    fn audit_wp01_b003_filter_named_with_equals_is_neutralized() {
        let _guard = GIT_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let repo = tempfile::tempdir().unwrap();
        let dir = repo.path();
        git_in(dir, &["init", "-q", "-b", "main"]);
        std::fs::write(
            dir.join(".gitattributes"),
            "f.txt filter=x=y\nh.txt filter=p=q\n",
        )
        .unwrap();
        for name in ["f.txt", "h.txt"] {
            std::fs::write(dir.join(name), "one\n").unwrap();
        }
        git_in(dir, &["add", "."]);
        git_in(dir, &["commit", "-q", "-m", "c"]);

        let slash = |name: &str| dir.join(name).to_string_lossy().replace('\\', "/");
        let marks = ["clean-ran", "process-ran", "spaced-ran"];
        git_in(
            dir,
            &[
                "config",
                "filter.x=y.clean",
                &format!("echo ran >> '{}'; cat", slash(marks[0])),
            ],
        );
        git_in(
            dir,
            &[
                "config",
                "filter.p=q.process",
                &format!("echo ran >> '{}'", slash(marks[1])),
            ],
        );
        // Attributes cannot select a name with spaces, but it must still be
        // listed and overridden without breaking the command.
        git_in(
            dir,
            &[
                "config",
                "filter.with space.clean",
                &format!("echo ran >> '{}'; cat", slash(marks[2])),
            ],
        );

        let make_stale = || {
            for name in ["f.txt", "h.txt"] {
                std::fs::write(dir.join(name), "one\n").unwrap();
                let file = std::fs::File::options()
                    .write(true)
                    .open(dir.join(name))
                    .unwrap();
                file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
                    .unwrap();
            }
            std::fs::write(dir.join("f.txt"), "two\n").unwrap();
        };
        let run = |mut command: Command| {
            let out = command.current_dir(dir).output().unwrap();
            assert!(out.status.success(), "{out:?}");
        };
        let ran = || marks.iter().filter(|n| dir.join(n).exists()).count();

        // Control: plain git runs the driver named with `=`.
        make_stale();
        run(plain_git(&["status", "--porcelain"]));
        run(plain_git(&["diff"]));
        assert!(
            dir.join(marks[0]).exists(),
            "control: the clean filter named x=y must run under plain git"
        );
        for mark in marks {
            let _ = std::fs::remove_file(dir.join(mark));
        }

        // Isolated: none of them.
        make_stale();
        let mut status = isolated_git_command_in(dir);
        status.args(["status", "--porcelain"]);
        run(status);
        let mut diff = isolated_git_command_in(dir);
        diff.args(["diff", "--no-ext-diff", "--no-textconv"]);
        run(diff);
        assert_eq!(ran(), 0, "a filter named with `=` ran under isolation");
    }

    /// B003: when the filters cannot be listed the command must not run.
    #[test]
    fn audit_wp01_b003_unlistable_filters_fail_closed() {
        let _guard = GIT_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let repo = tempfile::tempdir().unwrap();
        let dir = repo.path();
        git_in(dir, &["init", "-q", "-b", "main"]);
        let canary = dir.join("clean-ran");
        std::fs::write(dir.join(".gitattributes"), "f.txt filter=canary\n").unwrap();
        std::fs::write(dir.join("f.txt"), "one\n").unwrap();
        // A driver name that is not UTF-8 cannot be carried in an override
        // key, so the listing is unusable although git reads it fine.
        let mut config = std::fs::read(dir.join(".git").join("config")).unwrap();
        config.extend_from_slice(b"[filter \"bad\xffname\"]\n\tclean = echo ran >> '");
        config.extend_from_slice(canary.to_string_lossy().replace('\\', "/").as_bytes());
        config.extend_from_slice(b"'; cat\n");
        std::fs::write(dir.join(".git").join("config"), config).unwrap();
        std::fs::write(dir.join(".gitattributes"), b"f.txt filter=bad\xffname\n").unwrap();
        let out = isolated_git_command_in(dir)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(!out.status.success(), "{out:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("inspection refused"), "{stderr}");
        assert!(
            !stderr.contains("not a git repository"),
            "must not look like a clean non-repository: {stderr}"
        );
        assert!(!canary.exists(), "the filter ran although listing failed");
    }

    /// B003: key parsing keeps key and value separate and rejects what it
    /// cannot take apart.
    #[test]
    fn audit_wp01_b003_filter_overrides_parse() {
        let settings =
            filter_overrides(b"filter.x=y.clean\0filter.a.b.smudge\0filter.plain\0").unwrap();
        assert!(settings.contains(&("filter.x=y.clean".to_string(), String::new())));
        assert!(settings.contains(&("filter.x=y.required".to_string(), "false".to_string())));
        assert!(settings.contains(&("filter.a.b.process".to_string(), String::new())));
        assert_eq!(settings.len(), 8, "two drivers, four settings each");
        assert!(filter_overrides(b"nodots\0").is_err());
        assert!(filter_overrides(b"filter.a\nb.clean\0").is_err());
        assert!(filter_overrides(b"filter.\xff.clean\0").is_err());
    }
}
