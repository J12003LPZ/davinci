use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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

pub fn install_shutdown_watchers(dispose: impl Fn(i32) + Send + Sync + 'static) {
    if fixture_shutdown_signal().is_some() {
        return;
    }
    let term = Arc::new(AtomicBool::new(false));
    let hup = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&term));
    #[cfg(unix)]
    let _ = signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&hup));
    thread::spawn(move || loop {
        if term.load(Ordering::SeqCst) {
            dispose(143);
            std::process::exit(143);
        }
        if hup.load(Ordering::SeqCst) {
            dispose(129);
            std::process::exit(129);
        }
        thread::sleep(Duration::from_millis(20));
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
            Some("pi --session test-session")
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
            Some("pi --session-dir /tmp/custom-pi-sessions --session test-session")
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
            Some("pi --session-dir '/tmp/custom pi sessions' --session test-session")
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
            Some("pi --session-dir '/tmp/custom pi'\\''s sessions' --session test-session")
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
