use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::args::APP_NAME;

pub fn signal_exit_code(name: &str) -> i32 {
    match name.trim().to_ascii_uppercase().as_str() {
        "HUP" | "SIGHUP" | "1" | "129" => 129,
        _ => 143,
    }
}

pub fn fixture_shutdown_signal() -> Option<i32> {
    std::env::var("PI_SHUTDOWN_SIGNAL")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| signal_exit_code(&value))
}

/// How long an exit waits for cleanup (extension events, JS pool, MCP
/// servers, jobs, stop hooks) before it gives up and leaves.
pub const CLEANUP_DEADLINE: Duration = Duration::from_secs(5);

/// Runs cleanup at most once, from whichever exit path gets there first, and
/// never makes a caller wait past one fixed deadline. The work runs on its own
/// thread so a hung hook or destructor can be abandoned: the process is about
/// to exit and takes the stuck thread with it. Only children we started are
/// reaped by the work itself; nothing here touches other processes.
pub struct Cleanup {
    started: AtomicBool,
    started_at: OnceLock<Instant>,
    done: Arc<(Mutex<bool>, Condvar)>,
    steps: Arc<Mutex<StepLog>>,
    reported: AtomicBool,
}

/// One independent unit of cleanup work, named for the warning that says what
/// went wrong when it does.
pub struct Step {
    pub name: &'static str,
    work: Box<dyn FnOnce() + Send>,
}

impl Step {
    pub fn new(name: &'static str, work: impl FnOnce() + Send + 'static) -> Self {
        Self {
            name,
            work: Box::new(work),
        }
    }
}

/// What the steps did so far, readable by a trigger that gives up waiting.
#[derive(Default)]
struct StepLog {
    /// Not started yet, in order.
    pending: Vec<&'static str>,
    running: Option<&'static str>,
    panicked: Vec<&'static str>,
    /// Refused because the deadline had passed when their turn came.
    skipped: Vec<&'static str>,
}

fn lock(log: &Mutex<StepLog>) -> std::sync::MutexGuard<'_, StepLog> {
    log.lock().unwrap_or_else(|e| e.into_inner())
}

/// Marks the cleanup finished when its thread ends, even by panic.
struct Finish(Arc<(Mutex<bool>, Condvar)>);

impl Drop for Finish {
    fn drop(&mut self) {
        *self.0 .0.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.0 .1.notify_all();
    }
}

impl Cleanup {
    pub fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            started_at: OnceLock::new(),
            done: Arc::new((Mutex::new(false), Condvar::new())),
            steps: Arc::new(Mutex::new(StepLog::default())),
            reported: AtomicBool::new(false),
        }
    }

    /// Start `work` if no caller has, then wait for it until `deadline` after
    /// the first trigger. Returns whether the cleanup finished in time.
    #[cfg(test)]
    pub fn run(&self, deadline: Duration, work: impl FnOnce() + Send + 'static) -> bool {
        self.run_until(deadline, move |_| work())
    }

    /// Like [`run`](Self::run), for work made of independent steps. Each step
    /// runs under `catch_unwind`, so one failure never skips the rest, and all
    /// share the one absolute deadline: a step whose turn comes after it is
    /// skipped. Order the steps so the ones that run user or extension code,
    /// and may hang, come last.
    pub fn run_steps(&self, deadline: Duration, steps: Vec<Step>) -> bool {
        let log = Arc::clone(&self.steps);
        self.run_until(deadline, move |deadline_at| {
            lock(&log).pending = steps.iter().map(|step| step.name).collect();
            for step in steps {
                {
                    let mut log = lock(&log);
                    log.pending.retain(|name| *name != step.name);
                    if Instant::now() >= deadline_at {
                        log.skipped.push(step.name);
                        continue;
                    }
                    log.running = Some(step.name);
                }
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(step.work));
                let mut log = lock(&log);
                log.running = None;
                if outcome.is_err() {
                    log.panicked.push(step.name);
                }
            }
        })
    }

    /// The warning for what went wrong, once per process: `finished` is what
    /// the trigger saw. `None` when every step ran clean, or it was said.
    pub fn report(&self, finished: bool) -> Option<String> {
        let log = lock(&self.steps);
        let timed_out = log.running.filter(|_| !finished);
        let mut skipped = log.skipped.clone();
        if !finished {
            skipped.extend(log.pending.iter().copied());
        }
        let mut parts = Vec::new();
        if !log.panicked.is_empty() {
            parts.push(format!("panicked: {}", log.panicked.join(", ")));
        }
        if let Some(name) = timed_out {
            parts.push(format!("timed out: {name}"));
        }
        if !skipped.is_empty() {
            parts.push(format!("skipped: {}", skipped.join(", ")));
        }
        if parts.is_empty() || self.reported.swap(true, Ordering::SeqCst) {
            return None;
        }
        Some(format!(
            "shutdown cleanup incomplete ({})",
            parts.join("; ")
        ))
    }

    fn run_until(&self, deadline: Duration, work: impl FnOnce(Instant) + Send + 'static) -> bool {
        let first = !self.started.swap(true, Ordering::SeqCst);
        let started_at = *self.started_at.get_or_init(Instant::now);
        if first {
            let finish = Finish(Arc::clone(&self.done));
            let deadline_at = started_at + deadline;
            thread::spawn(move || {
                let _finish = finish;
                work(deadline_at);
            });
        }
        let (lock, finished) = &*self.done;
        let remaining = deadline.saturating_sub(started_at.elapsed());
        let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let (guard, _) = finished
            .wait_timeout_while(guard, remaining, |done| !*done)
            .unwrap_or_else(|e| e.into_inner());
        *guard
    }
}

impl Default for Cleanup {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide cleanup shared by the normal exit and the signal path.
pub fn session_cleanup() -> &'static Cleanup {
    static SESSION: OnceLock<Cleanup> = OnceLock::new();
    SESSION.get_or_init(Cleanup::new)
}

/// How long past [`CLEANUP_DEADLINE`] a signal-triggered shutdown may take
/// before the watchdog ends the process.
pub const EXIT_GRACE: Duration = Duration::from_secs(1);

/// How long an exit waits for a diagnostic to reach stderr.
const WARN_WAIT: Duration = Duration::from_millis(200);

/// Ends the process with `code` at `deadline + grace`, whatever the main path
/// is doing: a hung extension holding the stderr lock or a full pipe cannot
/// stop it, since it touches no lock and writes nothing. `exit` is injected so
/// a test does not end its own process.
pub fn arm_exit_watchdog(
    code: i32,
    deadline: Duration,
    grace: Duration,
    exit: impl FnOnce(i32) + Send + 'static,
) {
    thread::spawn(move || {
        thread::sleep(deadline + grace);
        exit(code);
    });
}

/// Runs `write` on its own thread and waits for it at most `wait`, so a
/// blocked stderr costs the exit that long and no more. Returns whether the
/// write finished.
fn warn_with(write: impl FnOnce() + Send + 'static, wait: Duration) -> bool {
    let (done, finished) = std::sync::mpsc::channel();
    thread::spawn(move || {
        write();
        let _ = done.send(());
    });
    finished.recv_timeout(wait).is_ok()
}

/// A diagnostic that is never on the critical path of an exit.
pub fn warn_best_effort(message: String) {
    warn_with(move || eprintln!("warning: {message}"), WARN_WAIT);
}

/// `dispose` must be bounded (route its work through [`session_cleanup`]):
/// the process exits as soon as it returns.
pub fn install_shutdown_watchers(dispose: impl Fn(i32) + Send + Sync + 'static) {
    if fixture_shutdown_signal().is_some() {
        return;
    }
    let term = Arc::new(AtomicBool::new(false));
    let hup = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&term));
    #[cfg(unix)]
    let _ = signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&hup));
    // `process::exit` runs no destructor, so the TUI's own `Drop` never gives
    // the terminal back: a killed pi used to leave the shell in raw mode
    // inside the alternate screen. `restore` is idempotent and a no-op when no
    // TUI ever opened.
    // The watchdog is armed before anything that can block (the terminal
    // restore, cleanup, a warning), so none of them can keep the process alive.
    thread::spawn(move || loop {
        for (flag, code) in [(&term, 143), (&hup, 129)] {
            if flag.load(Ordering::SeqCst) {
                arm_exit_watchdog(code, CLEANUP_DEADLINE, EXIT_GRACE, |code| {
                    std::process::exit(code)
                });
                let _ = davinci_tui::davinci::runtime::restore();
                dispose(code);
                std::process::exit(code);
            }
        }
        thread::sleep(Duration::from_millis(250));
    });
}

pub fn quote_if_needed(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '~' | ':' | '@')
        })
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', r"'\''"))
}

pub fn format_resume_command(
    stdout_is_tty: bool,
    persisted: bool,
    session_file: Option<&Path>,
    session_id: Option<&str>,
    session_dir: Option<&str>,
    uses_default_session_dir: bool,
) -> Option<String> {
    if !stdout_is_tty || !persisted {
        return None;
    }
    let session_file = session_file.filter(|path| path.exists())?;
    let _ = session_file;
    let session_id = session_id.filter(|id| !id.is_empty())?;
    let mut args = vec![APP_NAME.to_string()];
    if !uses_default_session_dir {
        if let Some(dir) = session_dir.filter(|dir| !dir.is_empty()) {
            args.push("--session-dir".into());
            args.push(quote_if_needed(dir));
        }
    }
    args.push("--session".into());
    args.push(session_id.to_string());
    Some(args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn cleanup_runs_exactly_once_however_often_it_is_triggered() {
        let cleanup = Cleanup::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let deadline = Duration::from_secs(5);
        for _ in 0..3 {
            let runs = Arc::clone(&runs);
            assert!(cleanup.run(deadline, move || {
                runs.fetch_add(1, Ordering::SeqCst);
            }));
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn concurrent_triggers_share_one_run_and_both_wait_for_it() {
        let cleanup = Arc::new(Cleanup::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let (work_runs, work_finished) = (Arc::clone(&runs), Arc::clone(&finished));
        let first = {
            let cleanup = Arc::clone(&cleanup);
            thread::spawn(move || {
                cleanup.run(Duration::from_secs(5), move || {
                    work_runs.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(150));
                    work_finished.store(true, Ordering::SeqCst);
                })
            })
        };
        thread::sleep(Duration::from_millis(30));
        // A signal landing while the normal exit is mid-cleanup must not
        // start a second cleanup, and must not return before it finishes.
        assert!(cleanup.run(Duration::from_secs(5), || unreachable!("second run")));
        assert!(finished.load(Ordering::SeqCst));
        assert!(first.join().unwrap());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn hanging_cleanup_does_not_block_past_the_deadline() {
        let cleanup = Cleanup::new();
        let started = Instant::now();
        let finished = cleanup.run(Duration::from_millis(100), || {
            thread::sleep(Duration::from_secs(30));
        });
        assert!(!finished, "a hung step is reported, not waited for");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?}",
            started.elapsed()
        );
        // A later trigger shares the same deadline instead of restarting it.
        let again = Instant::now();
        assert!(!cleanup.run(Duration::from_millis(100), || unreachable!("second run")));
        assert!(again.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn panicking_cleanup_still_releases_waiters() {
        let cleanup = Cleanup::new();
        assert!(cleanup.run(Duration::from_secs(5), || panic!("destructor failed")));
    }

    fn marker(ran: &Arc<Mutex<Vec<&'static str>>>, name: &'static str) -> Step {
        let ran = Arc::clone(ran);
        Step::new(name, move || ran.lock().unwrap().push(name))
    }

    #[test]
    fn a_panicking_step_does_not_skip_the_steps_after_it() {
        let cleanup = Cleanup::new();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let steps = vec![
            marker(&ran, "first"),
            Step::new("broken", || panic!("destructor failed")),
            marker(&ran, "third"),
            marker(&ran, "fourth"),
        ];
        assert!(cleanup.run_steps(Duration::from_secs(5), steps));
        assert_eq!(*ran.lock().unwrap(), ["first", "third", "fourth"]);
        let report = cleanup.report(true).expect("a panic is reported");
        assert!(report.contains("panicked: broken"), "{report}");
        assert!(!report.contains("skipped"), "{report}");
    }

    #[test]
    fn a_step_that_would_start_after_the_deadline_is_skipped_and_reported() {
        let cleanup = Cleanup::new();
        let ran = Arc::new(Mutex::new(Vec::new()));
        let steps = vec![
            Step::new("slow", || thread::sleep(Duration::from_millis(250))),
            marker(&ran, "late"),
            marker(&ran, "later"),
        ];
        // The slow step finishes after the deadline; nothing may start then.
        let finished = cleanup.run_steps(Duration::from_millis(100), steps);
        assert!(!finished, "the slow step outlasts the wait");
        thread::sleep(Duration::from_millis(400));
        assert!(
            ran.lock().unwrap().is_empty(),
            "no step starts past the deadline"
        );
        let report = cleanup.report(true).expect("skips are reported");
        assert!(report.contains("skipped: late, later"), "{report}");
    }

    #[test]
    fn a_hung_step_is_reported_as_timed_out_with_the_rest_skipped() {
        let cleanup = Cleanup::new();
        let steps = vec![
            Step::new("kill children", || {}),
            Step::new("hook", || thread::sleep(Duration::from_secs(30))),
            Step::new("after hook", || {}),
        ];
        let started = Instant::now();
        assert!(!cleanup.run_steps(Duration::from_millis(150), steps));
        assert!(started.elapsed() < Duration::from_secs(5));
        let report = cleanup.report(false).expect("a hang is reported");
        assert!(report.contains("timed out: hook"), "{report}");
        assert!(report.contains("skipped: after hook"), "{report}");
        assert!(!report.contains("kill children"), "{report}");
    }

    #[test]
    fn a_clean_run_reports_nothing_and_reports_once() {
        let cleanup = Cleanup::new();
        assert!(cleanup.run_steps(Duration::from_secs(5), vec![Step::new("ok", || {})]));
        assert_eq!(cleanup.report(true), None);

        let broken = Cleanup::new();
        broken.run_steps(
            Duration::from_secs(5),
            vec![Step::new("bad", || panic!("x"))],
        );
        assert!(broken.report(true).is_some());
        assert_eq!(broken.report(true), None, "the second trigger stays quiet");
    }

    /// A cleanup step stuck forever, then a warning path stuck on a lock the
    /// hung code holds: only the watchdog can still end the process.
    #[test]
    fn the_exit_watchdog_fires_while_cleanup_and_the_warning_path_are_stuck() {
        let cleanup = Arc::new(Cleanup::new());
        let held = Arc::new(Mutex::new(()));
        let (exit_tx, exit_rx) = std::sync::mpsc::channel();
        let deadline = Duration::from_millis(100);
        let started = Instant::now();
        arm_exit_watchdog(143, deadline, Duration::from_millis(50), move |code| {
            let _ = exit_tx.send(code);
        });

        let _holder = held.lock().unwrap();
        let (path_cleanup, path_held) = (Arc::clone(&cleanup), Arc::clone(&held));
        thread::spawn(move || {
            path_cleanup.run_steps(
                deadline,
                vec![Step::new("hung", || thread::sleep(Duration::from_secs(60)))],
            );
            // Stands in for a synchronous warning that needs a held lock.
            let _blocked_forever = path_held.lock();
        });

        let code = exit_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the watchdog exits regardless of the main path");
        assert_eq!(code, 143);
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(150),
            "not before deadline + grace: {waited:?}"
        );
        assert!(waited < Duration::from_secs(3), "{waited:?}");
    }

    #[test]
    fn a_warning_that_cannot_be_written_never_holds_up_the_exit() {
        let started = Instant::now();
        let written = warn_with(
            || thread::sleep(Duration::from_secs(60)),
            Duration::from_millis(100),
        );
        assert!(!written, "a blocked write is abandoned");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            warn_with(|| {}, Duration::from_secs(5)),
            "a free stderr is written"
        );
    }

    #[test]
    fn signal_codes_match_ts() {
        assert_eq!(signal_exit_code("TERM"), 143);
        assert_eq!(signal_exit_code("SIGTERM"), 143);
        assert_eq!(signal_exit_code("HUP"), 129);
        assert_eq!(signal_exit_code("SIGHUP"), 129);
    }

    #[test]
    fn resume_command_matches_ts_cases() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.jsonl");
        std::fs::write(&file, "\n").unwrap();
        assert_eq!(
            format_resume_command(true, true, Some(&file), Some("test-session"), None, true)
                .as_deref(),
            Some("davinci --session test-session")
        );
        assert_eq!(
            format_resume_command(
                true,
                true,
                Some(&file),
                Some("test-session"),
                Some("/tmp/custom-pi-sessions"),
                false,
            )
            .as_deref(),
            Some("davinci --session-dir /tmp/custom-pi-sessions --session test-session")
        );
        assert_eq!(
            format_resume_command(
                true,
                true,
                Some(&file),
                Some("test-session"),
                Some("/tmp/custom pi sessions"),
                false,
            )
            .as_deref(),
            Some("davinci --session-dir '/tmp/custom pi sessions' --session test-session")
        );
        assert_eq!(
            format_resume_command(
                true,
                true,
                Some(&file),
                Some("test-session"),
                Some("/tmp/custom pi's sessions"),
                false,
            )
            .as_deref(),
            Some("davinci --session-dir '/tmp/custom pi'\\''s sessions' --session test-session")
        );
        assert!(format_resume_command(false, true, Some(&file), Some("id"), None, true).is_none());
        assert!(format_resume_command(true, false, Some(&file), Some("id"), None, true).is_none());
        assert!(format_resume_command(
            true,
            true,
            Some(Path::new("/tmp/pi-missing-session.jsonl")),
            Some("id"),
            None,
            true
        )
        .is_none());
    }
}
