//! Clipboard helpers matching TypeScript `utils/clipboard.ts`.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::io::Write;
use std::process::{Command, Stdio};

const MAX_OSC52_ENCODED_LENGTH: usize = 100_000;

pub fn is_remote_session() -> bool {
    std::env::var("SSH_CONNECTION").is_ok()
        || std::env::var("SSH_CLIENT").is_ok()
        || std::env::var("MOSH_CONNECTION").is_ok()
}

pub fn osc52_payload(text: &str) -> Option<String> {
    let encoded = STANDARD.encode(text.as_bytes());
    if encoded.len() > MAX_OSC52_ENCODED_LENGTH {
        return None;
    }
    Some(format!("\u{1b}]52;c;{encoded}\u{07}"))
}

fn write_osc52(text: &str) -> bool {
    match osc52_payload(text) {
        Some(payload) => {
            let _ = std::io::stdout().write_all(payload.as_bytes());
            let _ = std::io::stdout().flush();
            true
        }
        None => false,
    }
}

fn pipe_to(command: &str, args: &[&str], text: &str) -> bool {
    let mut child = match Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(text.as_bytes()).is_err() {
            return false;
        }
    }
    child.wait().map(|status| status.success()).unwrap_or(false)
}

/// Copy text the way TypeScript `copyToClipboard` does: platform tools, then OSC 52.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut copied = false;
    if cfg!(target_os = "macos") {
        copied = pipe_to("pbcopy", &[], text);
    } else if cfg!(target_os = "windows") {
        copied = pipe_to("clip", &[], text);
    } else {
        if std::env::var("TERMUX_VERSION").is_ok() {
            copied = pipe_to("termux-clipboard-set", &[], text);
        }
        if !copied && std::env::var("WAYLAND_DISPLAY").is_ok() {
            copied = pipe_to("wl-copy", &[], text);
        }
        if !copied && std::env::var("DISPLAY").is_ok() {
            copied = pipe_to("xclip", &["-selection", "clipboard"], text)
                || pipe_to("xsel", &["--clipboard", "--input"], text);
        }
    }
    if is_remote_session() || !copied {
        copied = write_osc52(text) || copied;
    }
    if copied {
        Ok(())
    } else {
        Err("Failed to copy to clipboard".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_encodes_fixture_text() {
        let payload = osc52_payload("hi").unwrap();
        assert!(payload.starts_with("\u{1b}]52;c;"));
        assert!(payload.ends_with('\u{07}'));
        assert!(payload.contains(&STANDARD.encode("hi".as_bytes())));
    }
}
