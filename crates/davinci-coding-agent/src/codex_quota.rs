//! ChatGPT plan usage for `openai-codex` models, read the way Codex's own
//! usage screen reads it: one `codex app-server` child over stdio JSON-RPC,
//! `account/rateLimits/read` at start, after each turn (at most one read per
//! [`MIN_GAP`], a refresh inside the gap is deferred, not dropped) and every
//! minute, with `account/rateLimits/updated` notifications merged between
//! reads. The child uses the Codex CLI's own ChatGPT login; DaVinci passes it
//! no token. Results land in [`davinci_ai::codex_usage`], which the shell
//! draws.
//!
//! The child runs only while the row is shown ([`set_active`]): hiding the
//! row (another provider, `/config` → Plan usage off) stops it. A child that
//! leaves a read unanswered for [`ANSWER_TIMEOUT`] is killed and restarted.
//! It also exits when DaVinci does: its stdin closes. `DAVINCI_CODEX_BIN`
//! names another executable (tests use a fixture); `off` disables the meter.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use davinci_ai::codex_usage;
use serde_json::{json, Value};

/// A full read at least this often.
const PERIOD: Duration = Duration::from_secs(60);
/// No two reads closer than this, however many turns end.
const MIN_GAP: Duration = Duration::from_secs(10);
/// A read with no answer for this long means the child is stuck.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);
/// After the child dies, wait this long before starting another.
const RESTART_AFTER: Duration = Duration::from_secs(60);

struct Monitor {
    wake: SyncSender<()>,
    active: AtomicBool,
}

static MONITOR: OnceLock<Monitor> = OnceLock::new();

/// Show or hide the row. The first `true` starts the monitor thread; `false`
/// stops the child until the row is shown again.
pub fn set_active(active: bool) {
    if !active && MONITOR.get().is_none() {
        return;
    }
    let monitor = MONITOR.get_or_init(|| {
        // One slot is enough: a wake-up only says "look again".
        let (wake, requests) = sync_channel::<()>(1);
        std::thread::Builder::new()
            .name("codex-usage".into())
            .spawn(move || run(requests))
            .ok();
        Monitor {
            wake,
            active: AtomicBool::new(false),
        }
    });
    if monitor.active.swap(active, Ordering::SeqCst) != active {
        let _ = monitor.wake.try_send(());
    }
}

fn active() -> bool {
    MONITOR
        .get()
        .is_some_and(|monitor| monitor.active.load(Ordering::SeqCst))
}

/// Ask for a fresh read soon (after a turn). Does nothing while the row is
/// hidden.
pub fn refresh() {
    if let Some(monitor) = MONITOR.get() {
        let _ = monitor.wake.try_send(());
    }
}

/// Where to find Codex: `DAVINCI_CODEX_BIN`, or the installed CLI.
enum Binary {
    Off,
    Named(PathBuf),
    Installed,
}

fn binary() -> Binary {
    match std::env::var("DAVINCI_CODEX_BIN") {
        Ok(value) if value.trim().eq_ignore_ascii_case("off") => Binary::Off,
        Ok(value) if !value.trim().is_empty() => Binary::Named(PathBuf::from(value.trim())),
        _ => Binary::Installed,
    }
}

fn executable_names() -> &'static [&'static str] {
    if cfg!(windows) {
        // A bare `codex` only finds `codex.exe`; an npm install is `codex.cmd`.
        &["codex.exe", "codex.cmd"]
    } else {
        &["codex"]
    }
}

/// The first `names` file in `path`'s absolute entries. A relative entry
/// (`.`, an empty one) resolves against the working directory, and a
/// `codex.exe` in a project being opened must not run just because the row
/// is shown.
fn find_on_path(path: &std::ffi::OsStr, names: &[&str]) -> Option<PathBuf> {
    names.iter().find_map(|name| {
        std::env::split_paths(path)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// `codex` on PATH, else where the Codex installers put it. Never a bare
/// name: that would let the OS search relative PATH entries after all.
fn resolve_codex() -> Option<PathBuf> {
    let names = executable_names();
    let path = std::env::var_os("PATH").unwrap_or_default();
    if let Some(found) = find_on_path(&path, names) {
        return Some(found);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let installed = [
        home.map(|home| PathBuf::from(home).join(".codex/packages/standalone/current/bin")),
        std::env::var_os("LOCALAPPDATA")
            .filter(|_| cfg!(windows))
            .map(|local| PathBuf::from(local).join("Programs/OpenAI/Codex/bin")),
    ];
    installed
        .into_iter()
        .flatten()
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(names[0]))
        .find(|candidate| candidate.is_file())
}

fn run(requests: Receiver<()>) {
    let named = match binary() {
        Binary::Off => return,
        Binary::Named(path) => Some(path),
        Binary::Installed => None,
    };
    loop {
        // Hidden: no child, wait to be shown.
        while !active() {
            if requests.recv().is_err() {
                return;
            }
        }
        // Looked up each time, so installing Codex mid-session is picked up.
        let Some(binary) = named.clone().or_else(resolve_codex) else {
            codex_usage::set_unavailable(
                "Codex CLI not found · install it and run `codex login` to see plan usage",
            );
            std::thread::sleep(RESTART_AFTER);
            continue;
        };
        let session = match Session::spawn(&binary) {
            Ok(session) => session,
            Err(reason) => {
                codex_usage::set_unavailable(reason);
                std::thread::sleep(RESTART_AFTER);
                continue;
            }
        };
        match session.drive(&requests) {
            Ended::Hidden => continue,
            Ended::Disconnected => return,
            Ended::Stopped(reason) => {
                codex_usage::set_unavailable(reason);
                std::thread::sleep(RESTART_AFTER);
            }
        }
    }
}

enum Ended {
    /// The row was hidden: the child was stopped on purpose.
    Hidden,
    /// DaVinci is shutting down.
    Disconnected,
    /// The child died or stopped answering.
    Stopped(&'static str),
}

/// What the reader thread tells the driver.
struct Shared {
    alive: AtomicBool,
    /// The highest read id answered (result or error).
    answered: AtomicU64,
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    next_id: u64,
    shared: Arc<Shared>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Session {
    fn spawn(binary: &std::path::Path) -> Result<Self, String> {
        let mut command = Command::new(binary);
        command
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "Codex CLI not found · install it and run `codex login` to see plan usage"
                    .to_string()
            } else {
                format!("could not start Codex app-server: {error}")
            }
        })?;
        let stdout = child.stdout.take();
        let stdin = child.stdin.take();
        let shared = Arc::new(Shared {
            alive: AtomicBool::new(true),
            answered: AtomicU64::new(0),
        });
        // Own the child before anything else can fail, so Drop kills it.
        let mut session = match stdin {
            Some(stdin) => Self {
                child,
                stdin,
                next_id: 1,
                shared: shared.clone(),
            },
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("no stdin for Codex app-server".into());
            }
        };
        let stdout = stdout.ok_or("no stdout for Codex app-server")?;
        std::thread::Builder::new()
            .name("codex-usage-reader".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if let Some(id) = handle_line(&line) {
                        shared.answered.fetch_max(id, Ordering::SeqCst);
                    }
                }
                shared.alive.store(false, Ordering::SeqCst);
            })
            .map_err(|error| error.to_string())?;
        session.send(&json!({
            "id": 0,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "davinci",
                    "title": "DaVinci",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": { "experimentalApi": true },
            }
        }))?;
        session.send(&json!({ "method": "initialized" }))?;
        Ok(session)
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("Codex app-server closed: {error}"))
    }

    fn read(&mut self) -> Result<u64, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({ "id": id, "method": "account/rateLimits/read" }))?;
        Ok(id)
    }

    fn drive(mut self, requests: &Receiver<()>) -> Ended {
        const STOPPED: Ended = Ended::Stopped("Codex app-server stopped · retrying");
        let Ok(mut sent) = self.read() else {
            return STOPPED;
        };
        let mut last = Instant::now();
        let mut pending = false;
        loop {
            if !active() {
                return Ended::Hidden;
            }
            if !self.shared.alive.load(Ordering::SeqCst) {
                return STOPPED;
            }
            let unanswered = self.shared.answered.load(Ordering::SeqCst) < sent;
            if unanswered && last.elapsed() >= ANSWER_TIMEOUT {
                return Ended::Stopped("Codex app-server is not answering · retrying");
            }
            let due = if pending { MIN_GAP } else { PERIOD };
            if last.elapsed() >= due && !unanswered {
                match self.read() {
                    Ok(id) => sent = id,
                    Err(_) => return STOPPED,
                }
                last = Instant::now();
                pending = false;
                continue;
            }
            // Wake for the next read or a request; while a read is out, look
            // for its answer once a second (it usually takes under one).
            let wait = if unanswered {
                Duration::from_secs(1)
            } else {
                due.saturating_sub(last.elapsed())
            };
            match requests.recv_timeout(wait.clamp(Duration::from_millis(100), PERIOD)) {
                Ok(()) => pending = true,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ended::Disconnected,
            }
        }
    }
}

/// One line from the app-server: a read's result or error, or a rate-limit
/// notification. Returns the id of the read it answers. Everything else (the
/// initialize reply, other notifications, other limits) is ignored.
pub(crate) fn handle_line(line: &str) -> Option<u64> {
    let message = serde_json::from_str::<Value>(line).ok()?;
    if message.get("method").and_then(Value::as_str) == Some("account/rateLimits/updated") {
        let limits = message.pointer("/params/rateLimits")?;
        // Only the main `codex` limit is the plan's; other ids are other
        // buckets with windows of their own.
        let limit_id = limits.get("limitId").and_then(Value::as_str);
        if limit_id.is_none_or(|id| id == "codex") {
            if let Some(update) = codex_usage::parse_app_server_rate_limits(limits) {
                codex_usage::record_update(update);
            }
        }
        return None;
    }
    let id = message.get("id").and_then(Value::as_u64)?;
    // id 0 is `initialize`; reads start at 2.
    if id < 2 {
        if let Some(error) = message.get("error") {
            codex_usage::set_unavailable(describe_error(error));
        }
        return None;
    }
    if let Some(snapshot) = message
        .pointer("/result/rateLimits")
        .and_then(codex_usage::parse_app_server_rate_limits)
    {
        codex_usage::record(snapshot);
    } else if let Some(error) = message.get("error") {
        codex_usage::set_unavailable(describe_error(error));
    }
    Some(id)
}

/// A read's error as the one line the usage row shows.
pub(crate) fn describe_error(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let lower = message.to_ascii_lowercase();
    if lower.contains("token_expired") || lower.contains("401 unauthorized") {
        "Codex login expired · run `codex login` to see plan usage".into()
    } else if [
        "not logged in",
        "not signed in",
        "login required",
        "no auth",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
    {
        "Codex is not signed in · run `codex login` to see plan usage".into()
    } else {
        let first = message.lines().next().unwrap_or("unknown error");
        let short: String = first.chars().take(80).collect();
        format!("plan usage unavailable: {short}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_notifications_feed_one_snapshot() {
        handle_line(
            &json!({"id": 2, "result": {"rateLimits": {
                "primary": {"usedPercent": 27, "windowDurationMins": 300, "resetsAt": 1791349200},
                "secondary": {"usedPercent": 59, "windowDurationMins": 10080, "resetsAt": 1791741600},
                "planType": "plus"
            }}})
            .to_string(),
        );
        let snapshot = codex_usage::latest().unwrap();
        assert_eq!(snapshot.five_hour().unwrap().remaining_percent(), 73.0);
        assert_eq!(codex_usage::unavailable(), None);
        // A sparse update moves the 5-hour window and keeps the weekly one.
        handle_line(
            &json!({"method": "account/rateLimits/updated", "params": {"rateLimits": {
                "primary": {"usedPercent": 31, "windowDurationMins": 300}
            }}})
            .to_string(),
        );
        let snapshot = codex_usage::latest().unwrap();
        assert_eq!(snapshot.five_hour().unwrap().used_percent, 31.0);
        assert_eq!(snapshot.weekly().unwrap().used_percent, 59.0);
        assert_eq!(snapshot.plan_type.as_deref(), Some("plus"));
        // Another limit's update (its own bucket) leaves the plan alone.
        handle_line(
            &json!({"method": "account/rateLimits/updated", "params": {"rateLimits": {
                "limitId": "codex_other",
                "primary": {"usedPercent": 99, "windowDurationMins": 300}
            }}})
            .to_string(),
        );
        assert_eq!(
            codex_usage::latest()
                .unwrap()
                .five_hour()
                .unwrap()
                .used_percent,
            31.0
        );
        // A failed read says why; the initialize reply and noise are ignored,
        // and only replies to reads count as answers.
        assert_eq!(
            handle_line(r#"{"id": 0, "result": {"userAgent": "codex"}}"#),
            None
        );
        assert_eq!(handle_line("not json"), None);
        assert_eq!(
            handle_line(
                &json!({"id": 3, "error": {"message": "401 Unauthorized token_expired"}})
                    .to_string(),
            ),
            Some(3)
        );
        assert_eq!(
            codex_usage::unavailable().as_deref(),
            Some("Codex login expired · run `codex login` to see plan usage")
        );
    }

    #[test]
    fn codex_is_found_only_in_absolute_path_entries() {
        let id = std::process::id();
        let trusted = std::env::temp_dir().join(format!("davinci-codex-path-{id}"));
        // A project directory under the working directory, named relatively:
        // what `.` in PATH means when DaVinci is opened inside a project.
        let project = PathBuf::from(format!("davinci-codex-planted-{id}"));
        std::fs::create_dir_all(&trusted).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let name = executable_names()[0];
        std::fs::write(project.join(name), b"planted").unwrap();
        assert!(project.is_relative() && project.join(name).is_file());
        // The relative entry comes first and names a real file; it is skipped.
        let path = std::env::join_paths([project.clone(), trusted.clone()]).unwrap();
        let before = find_on_path(&path, &[name]);
        std::fs::write(trusted.join(name), b"real").unwrap();
        let after = find_on_path(&path, &[name]);
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&trusted);
        assert_eq!(before, None);
        assert_eq!(after, Some(trusted.join(name)));
    }

    #[test]
    fn expired_and_missing_logins_say_what_to_run() {
        let expired = json!({
            "code": -32603,
            "message": "failed to fetch codex rate limits: GET https://chatgpt.com/backend-api/wham/usage failed: 401 Unauthorized; body={\"error\":{\"code\":\"token_expired\"}}"
        });
        assert_eq!(
            describe_error(&expired),
            "Codex login expired · run `codex login` to see plan usage"
        );
        assert_eq!(
            describe_error(&json!({"message": "Not logged in"})),
            "Codex is not signed in · run `codex login` to see plan usage"
        );
        // A 401 elsewhere in an unrelated message is not a login problem.
        assert_eq!(
            describe_error(&json!({"message": "quota 401 tokens over"})),
            "plan usage unavailable: quota 401 tokens over"
        );
        let other = json!({"message": "backend unavailable\nsecond line"});
        assert_eq!(
            describe_error(&other),
            "plan usage unavailable: backend unavailable"
        );
        assert_eq!(
            describe_error(&json!({})),
            "plan usage unavailable: unknown error"
        );
    }
}
