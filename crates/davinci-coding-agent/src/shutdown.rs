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
        }
    }

    /// Start `work` if no caller has, then wait for it until `deadline` after
    /// the first trigger. Returns whether the cleanup finished in time.
    pub fn run(&self, deadline: Duration, work: impl FnOnce() + Send + 'static) -> bool {
        let first = !self.started.swap(true, Ordering::SeqCst);
        let started_at = *self.started_at.get_or_init(Instant::now);
        if first {
            let finish = Finish(Arc::clone(&self.done));
            thread::spawn(move || {
                let _finish = finish;
                work();
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
    thread::spawn(move || loop {
        if term.load(Ordering::SeqCst) {
            let _ = davinci_tui::davinci::runtime::restore();
            dispose(143);
            std::process::exit(143);
        }
        if hup.load(Ordering::SeqCst) {
            let _ = davinci_tui::davinci::runtime::restore();
            dispose(129);
            std::process::exit(129);
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
