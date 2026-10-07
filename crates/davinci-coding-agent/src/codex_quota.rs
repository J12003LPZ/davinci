//! ChatGPT plan usage for `openai-codex` models, read the way Codex's own
//! usage screen reads it: one `codex app-server` child over stdio JSON-RPC,
//! `account/rateLimits/read` at start, after each turn (debounced) and every
//! minute, with `account/rateLimits/updated` notifications merged between
//! reads. The child uses the Codex CLI's own ChatGPT login; DaVinci passes it
//! no token. Results land in [`davinci_ai::codex_usage`], which the shell
//! draws.
//!
//! The child exits when DaVinci does: its stdin closes. `DAVINCI_CODEX_BIN`
//! names another executable (tests use a fixture); `off` disables the meter.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use davinci_ai::codex_usage;
use serde_json::{json, Value};

/// A full read at least this often.
const PERIOD: Duration = Duration::from_secs(60);
/// No two reads closer than this, however many turns end.
const MIN_GAP: Duration = Duration::from_secs(10);
/// After the child dies, wait this long before starting another.
const RESTART_AFTER: Duration = Duration::from_secs(60);

static MONITOR: OnceLock<Mutex<Sender<()>>> = OnceLock::new();

/// Start the monitor once per process. Later calls do nothing.
pub fn start() {
    MONITOR.get_or_init(|| {
        let (tx, rx) = channel::<()>();
        std::thread::Builder::new()
            .name("codex-usage".into())
            .spawn(move || run(rx))
            .ok();
        Mutex::new(tx)
    });
}

/// Ask for a fresh read soon (after a turn). Does nothing before [`start`].
pub fn refresh() {
    if let Some(sender) = MONITOR.get() {
        let _ = sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .send(());
    }
}

fn binary() -> Option<String> {
    match std::env::var("DAVINCI_CODEX_BIN") {
        Ok(value) if value.trim().eq_ignore_ascii_case("off") => None,
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => Some("codex".into()),
    }
}

fn run(requests: std::sync::mpsc::Receiver<()>) {
    let Some(binary) = binary() else {
        return;
    };
    loop {
        let session = match Session::spawn(&binary) {
            Ok(session) => session,
            Err(reason) => {
                codex_usage::set_unavailable(reason);
                std::thread::sleep(RESTART_AFTER);
                continue;
            }
        };
        session.drive(&requests);
        // The child went away: say so, then try again later.
        codex_usage::set_unavailable("Codex app-server stopped · retrying");
        std::thread::sleep(RESTART_AFTER);
    }
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    next_id: u64,
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Session {
    fn spawn(binary: &str) -> Result<Self, String> {
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
        let stdin = child.stdin.take().ok_or("no stdin for Codex app-server")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("no stdout for Codex app-server")?;
        let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let reader_alive = alive.clone();
        std::thread::Builder::new()
            .name("codex-usage-reader".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    handle_line(&line);
                }
                reader_alive.store(false, std::sync::atomic::Ordering::SeqCst);
            })
            .map_err(|error| error.to_string())?;
        let mut session = Self {
            child,
            stdin,
            next_id: 1,
            alive,
        };
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

    fn read(&mut self) -> Result<(), String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({ "id": id, "method": "account/rateLimits/read" }))
    }

    fn drive(mut self, requests: &std::sync::mpsc::Receiver<()>) {
        let mut last = Instant::now();
        if self.read().is_err() {
            return;
        }
        loop {
            if !self.alive.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let wait = PERIOD.saturating_sub(last.elapsed());
            match requests.recv_timeout(wait.max(Duration::from_millis(100))) {
                Ok(()) if last.elapsed() < MIN_GAP => continue,
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if last.elapsed() < MIN_GAP {
                continue;
            }
            last = Instant::now();
            if self.read().is_err() {
                return;
            }
        }
    }
}

/// One line from the app-server: a read's result or error, or a rate-limit
/// notification. Everything else (the initialize reply, other
/// notifications) is ignored.
pub(crate) fn handle_line(line: &str) {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if message.get("method").and_then(Value::as_str) == Some("account/rateLimits/updated") {
        if let Some(update) = message
            .pointer("/params/rateLimits")
            .and_then(codex_usage::parse_app_server_rate_limits)
        {
            codex_usage::record_update(update);
        }
        return;
    }
    // id 0 is `initialize`; reads start at 2.
    if message.get("id").and_then(Value::as_u64).unwrap_or(0) < 2 {
        if let Some(error) = message.get("error") {
            codex_usage::set_unavailable(describe_error(error));
        }
        return;
    }
    if let Some(snapshot) = message
        .pointer("/result/rateLimits")
        .and_then(codex_usage::parse_app_server_rate_limits)
    {
        codex_usage::record(snapshot);
    } else if let Some(error) = message.get("error") {
        codex_usage::set_unavailable(describe_error(error));
    }
}

/// A read's error as the one line the usage row shows.
pub(crate) fn describe_error(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let lower = message.to_ascii_lowercase();
    if lower.contains("token_expired")
        || lower.contains("401")
        || lower.contains("not logged in")
        || lower.contains("no auth")
        || lower.contains("login")
    {
        "Codex login expired · run `codex login` to see plan usage".into()
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
        // A failed read says why; the initialize reply and noise are ignored.
        handle_line(r#"{"id": 0, "result": {"userAgent": "codex"}}"#);
        handle_line("not json");
        handle_line(
            &json!({"id": 3, "error": {"message": "401 Unauthorized token_expired"}}).to_string(),
        );
        assert_eq!(
            codex_usage::unavailable().as_deref(),
            Some("Codex login expired · run `codex login` to see plan usage")
        );
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
