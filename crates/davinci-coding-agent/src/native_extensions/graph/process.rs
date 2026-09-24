//! Child-process supervision shared by the worker and verification nodes.
//!
//! One helper, one contract: stream a child's output line by line while
//! honouring an abort flag and an OPTIONAL deadline. A `timeout_ms` of `0`
//! means the child runs until it exits on its own or the operator aborts —
//! the graph never imposes a clock the operator did not ask for.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// How long the final drain will keep taking already-queued output.
const DRAIN_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChildOutcome {
    pub exit_code: i32,
    pub timed_out: bool,
    pub run_deadline_exceeded: bool,
    pub aborted: bool,
    pub pid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkerDeadline {
    pub run_deadline: Option<Instant>,
    pub role_timeout: Option<Duration>,
}

impl WorkerDeadline {
    pub fn effective_deadline(&self, started: Instant) -> Option<Instant> {
        let role_deadline = self.role_timeout.map(|d| started + d);
        match (self.run_deadline, role_deadline) {
            (Some(run), Some(role)) => Some(run.min(role)),
            (Some(run), None) => Some(run),
            (None, Some(role)) => Some(role),
            (None, None) => None,
        }
    }
}

enum Line {
    Stdout(String),
    Stderr(String),
}

fn pump<R: Read + Send + 'static>(reader: R, sender: mpsc::Sender<Line>, wrap: fn(String) -> Line) {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buffer).trim_end().to_string();
                    if sender.send(wrap(text)).is_err() {
                        return;
                    }
                }
            }
        }
    });
}

pub fn terminate(child: &mut Child) {
    let pid = child.id();
    terminate_process_tree(pid);
    let _ = child.kill();
    let _ = child.wait();
}

pub fn terminate_process_tree(pid: u32) {
    davinci_sys::process::kill_tree(pid);
}

#[allow(dead_code)]
pub fn is_pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}")])
            .output();
        match output {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                text.lines().any(|line| {
                    line.split_whitespace()
                        .any(|word| word == pid.to_string().as_str())
                })
            }
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}

/// Run `command` to completion with a composite deadline (run deadline + role timeout),
/// streaming every line and terminating the process tree if either deadline expires.
pub fn run_child_with_deadline(
    mut command: Command,
    abort: &Arc<AtomicBool>,
    deadline: WorkerDeadline,
    mut on_stdout: impl FnMut(&str),
    mut on_stderr: impl FnMut(&str),
) -> std::io::Result<ChildOutcome> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let child_pid = child.id();
    let (sender, receiver) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, sender.clone(), Line::Stdout);
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, sender.clone(), Line::Stderr);
    }
    drop(sender);

    let started = Instant::now();
    let effective_deadline = deadline.effective_deadline(started);
    let mut outcome = ChildOutcome {
        pid: child_pid,
        ..ChildOutcome::default()
    };

    let mut pipes_closed = false;
    loop {
        if pipes_closed {
            thread::sleep(POLL_INTERVAL);
        } else {
            match receiver.recv_timeout(POLL_INTERVAL) {
                Ok(Line::Stdout(line)) => on_stdout(&line),
                Ok(Line::Stderr(line)) => on_stderr(&line),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => pipes_closed = true,
            }
        }
        // Pipe lifetime and process lifetime are independent. A child can close
        // its pipes early, or leave them inherited by a longer-lived descendant.
        if child.try_wait()?.is_some() {
            break;
        }
        if abort.load(Ordering::Relaxed) {
            outcome.aborted = true;
            terminate(&mut child);
            break;
        }
        let now = Instant::now();
        if let Some(effective) = effective_deadline {
            if now >= effective {
                if deadline.run_deadline.is_some_and(|rd| rd <= effective) {
                    outcome.run_deadline_exceeded = true;
                } else {
                    outcome.timed_out = true;
                }
                terminate(&mut child);
                break;
            }
        }
    }

    if !outcome.run_deadline_exceeded && !outcome.aborted {
        let drain_until = Instant::now() + DRAIN_GRACE;
        let mut pending = Vec::new();
        while Instant::now() < drain_until {
            match receiver.try_recv() {
                Ok(line) => pending.push(line),
                Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(5)),
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
        // Bound waiting for inherited pipes, not delivery of already collected
        // output. Slow callbacks must not drop a worker's final artifact.
        for line in pending {
            match line {
                Line::Stdout(line) => on_stdout(&line),
                Line::Stderr(line) => on_stderr(&line),
            }
        }
    }
    outcome.exit_code = match child.wait() {
        Ok(status) => {
            status
                .code()
                .unwrap_or(if outcome.aborted || outcome.run_deadline_exceeded {
                    130
                } else {
                    1
                })
        }
        Err(_) => 1,
    };
    Ok(outcome)
}

/// Run `command` to completion, feeding every stdout line to `on_stdout` and
/// every stderr line to `on_stderr` as they arrive.
pub fn run_child(
    command: Command,
    abort: &Arc<AtomicBool>,
    timeout_ms: u64,
    on_stdout: impl FnMut(&str),
    on_stderr: impl FnMut(&str),
) -> std::io::Result<ChildOutcome> {
    let role_timeout = (timeout_ms > 0).then(|| Duration::from_millis(timeout_ms));
    run_child_with_deadline(
        command,
        abort,
        WorkerDeadline {
            run_deadline: None,
            role_timeout,
        },
        on_stdout,
        on_stderr,
    )
}

/// Build the platform's shell invocation for a free-form command string.
pub fn shell_command(command: &str, cwd: &std::path::Path) -> Command {
    let mut process = if cfg!(windows) {
        let mut process = Command::new("cmd");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // cmd.exe owns parsing of the command line. Avoid MSVC argv
            // escaping, which changes nested quotes before cmd sees them.
            process.raw_arg("/C").raw_arg(command);
        }
        process
    } else {
        let mut process = Command::new("sh");
        process.arg("-c").arg(command);
        process
    };
    process.current_dir(cwd);
    davinci_sys::process::set_own_process_group(&mut process);
    process
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn echo(text: &str) -> Command {
        shell_command(&format!("echo {text}"), std::path::Path::new("."))
    }

    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "native_extensions::graph::process::tests::pipe_lifecycle_fixture",
            "--ignored",
            "--nocapture",
        ]);
        command.env("DAVINCI_TEST_PIPE_MODE", mode);
        command
    }

    #[test]
    #[ignore = "subprocess fixture invoked by pipe lifecycle tests"]
    fn pipe_lifecycle_fixture() {
        match std::env::var("DAVINCI_TEST_PIPE_MODE").as_deref() {
            Ok("close") => {
                #[cfg(windows)]
                unsafe {
                    #[link(name = "kernel32")]
                    extern "system" {
                        fn GetStdHandle(kind: u32) -> *mut std::ffi::c_void;
                        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
                    }
                    CloseHandle(GetStdHandle(-11i32 as u32));
                    CloseHandle(GetStdHandle(-12i32 as u32));
                }
                #[cfg(not(windows))]
                unsafe {
                    libc::close(1);
                    libc::close(2);
                }
                thread::sleep(Duration::from_secs(3));
            }
            Ok("descendant") => {
                let mut child = fixture("sleep").spawn().unwrap();
                // The fixture parent exits while this descendant still owns its pipes.
                thread::spawn(move || child.wait().unwrap());
            }
            Ok("sleep") => thread::sleep(Duration::from_secs(3)),
            Ok("burst") => {
                for index in 0..30 {
                    println!("burst-line-{index}");
                }
            }
            _ => panic!("fixture mode required"),
        }
    }

    #[test]
    fn exited_child_preserves_queued_output_with_slow_callbacks() {
        let mut lines = Vec::new();
        let outcome = run_child(
            fixture("burst"),
            &Arc::new(AtomicBool::new(false)),
            0,
            |line| {
                thread::sleep(Duration::from_millis(20));
                lines.push(line.to_string());
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(outcome.exit_code, 0);
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("burst-line-"))
                .count(),
            30
        );
    }

    #[test]
    fn closed_output_pipes_do_not_disable_the_deadline() {
        let started = Instant::now();
        let outcome = run_child(
            fixture("close"),
            &Arc::new(AtomicBool::new(false)),
            300,
            |_| {},
            |_| {},
        )
        .unwrap();
        assert!(outcome.timed_out, "{outcome:?}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn closed_output_pipes_do_not_disable_cancellation() {
        let abort = Arc::new(AtomicBool::new(false));
        let flag = abort.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            flag.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let outcome = run_child(fixture("close"), &abort, 0, |_| {}, |_| {}).unwrap();
        canceller.join().unwrap();
        assert!(outcome.aborted, "{outcome:?}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn exited_child_does_not_wait_for_descendant_output_pipes() {
        let started = Instant::now();
        let outcome = run_child(
            fixture("descendant"),
            &Arc::new(AtomicBool::new(false)),
            0,
            |_| {},
            |_| {},
        )
        .unwrap();
        assert_eq!(outcome.exit_code, 0);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "waited for descendant pipe handles"
        );
    }

    #[cfg(windows)]
    #[test]
    fn cmd_receives_quoted_arguments_intact() {
        let dir = tempfile::tempdir().unwrap();
        let output = shell_command(r#"echo "a b""#, dir.path()).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), r#""a b""#);
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_the_shells_children() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("still-running");
        let script = format!("(sleep 2; touch {}) & wait", marker.display());
        let abort = Arc::new(AtomicBool::new(false));
        let _ = run_child(shell_command(&script, dir.path()), &abort, 300, |_| {}, |_| {})
            .unwrap();
        thread::sleep(Duration::from_secs(3));
        assert!(!marker.exists(), "the backgrounded child survived the timeout");
    }

    #[test]
    fn a_child_streams_its_output_and_reports_success() {
        let abort = Arc::new(AtomicBool::new(false));
        let mut lines = Vec::new();
        let outcome = run_child(
            echo("hello"),
            &abort,
            0,
            |line| lines.push(line.to_string()),
            |_| {},
        )
        .expect("spawns");
        assert_eq!(outcome.exit_code, 0);
        assert!(!outcome.timed_out);
        assert!(!outcome.aborted);
        assert!(lines.iter().any(|line| line.contains("hello")));
    }

    #[test]
    fn a_failing_child_reports_its_exit_code() {
        let abort = Arc::new(AtomicBool::new(false));
        let command = shell_command("exit 3", std::path::Path::new("."));
        let outcome = run_child(command, &abort, 0, |_| {}, |_| {}).expect("spawns");
        assert_eq!(outcome.exit_code, 3);
    }

    #[test]
    fn zero_means_no_deadline_at_all() {
        let abort = Arc::new(AtomicBool::new(false));
        let command = shell_command("exit 0", std::path::Path::new("."));
        let outcome = run_child(command, &abort, 0, |_| {}, |_| {}).expect("spawns");
        assert!(!outcome.timed_out);
    }

    #[test]
    fn a_positive_deadline_still_stops_a_long_child() {
        let abort = Arc::new(AtomicBool::new(false));
        let sleeper = if cfg!(windows) {
            "ping -n 20 127.0.0.1 > nul"
        } else {
            "sleep 20"
        };
        let command = shell_command(sleeper, std::path::Path::new("."));
        let outcome = run_child(command, &abort, 300, |_| {}, |_| {}).expect("spawns");
        assert!(outcome.timed_out);
    }

    #[test]
    fn an_abort_flag_terminates_the_child() {
        let abort = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&abort);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::Relaxed);
        });
        let sleeper = if cfg!(windows) {
            "ping -n 20 127.0.0.1 > nul"
        } else {
            "sleep 20"
        };
        let command = shell_command(sleeper, std::path::Path::new("."));
        let outcome = run_child(command, &abort, 0, |_| {}, |_| {}).expect("spawns");
        assert!(outcome.aborted);
    }
}
