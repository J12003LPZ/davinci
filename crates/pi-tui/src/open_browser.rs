//! Open a URL without a shell, matching TS `open-browser.ts`.

use std::process::{Command, Stdio};

pub fn open_browser_argv(target: &str) -> (&'static str, Vec<String>) {
    if cfg!(target_os = "macos") {
        ("open", vec![target.to_string()])
    } else if cfg!(target_os = "windows") {
        (
            "rundll32",
            vec!["url.dll,FileProtocolHandler".into(), target.to_string()],
        )
    } else {
        ("xdg-open", vec![target.to_string()])
    }
}

pub fn open_browser_dry_run() -> bool {
    matches!(
        std::env::var("PI_OPEN_BROWSER_DRY_RUN").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) || std::env::var("PI_OAUTH_FIXTURE").is_ok()
}

pub fn copy_text_dry_run() -> bool {
    matches!(
        std::env::var("PI_COPY_DRY_RUN").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) || open_browser_dry_run()
}

/// Copy text without a shell, matching TS clipboard helpers (`pbcopy` / `xclip` / `clip`).
pub fn copy_text(text: &str) -> String {
    if copy_text_dry_run() {
        return format!("copy:{text}");
    }
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("pbcopy", vec![])
    } else if cfg!(target_os = "windows") {
        ("clip", vec![])
    } else {
        ("xclip", vec!["-selection", "clipboard"])
    };
    let _ = Command::new(cmd)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            child.wait()
        });
    format!("{cmd} clipboard")
}

pub fn open_browser(target: &str) -> String {
    let (cmd, args) = open_browser_argv(target);
    let launched = format!("{cmd} {}", args.join(" "));
    if open_browser_dry_run() || target.contains("pi-fixture") {
        return launched;
    }
    let _ = Command::new(cmd)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    launched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_matches_ts_platforms() {
        let (cmd, args) = open_browser_argv("https://example.test/auth");
        if cfg!(target_os = "macos") {
            assert_eq!(cmd, "open");
            assert_eq!(args, ["https://example.test/auth"]);
        } else if cfg!(target_os = "windows") {
            assert_eq!(cmd, "rundll32");
            assert_eq!(args[0], "url.dll,FileProtocolHandler");
        } else {
            assert_eq!(cmd, "xdg-open");
            assert_eq!(args, ["https://example.test/auth"]);
        }
        std::env::set_var("PI_OPEN_BROWSER_DRY_RUN", "1");
        assert!(open_browser("https://example.test/auth").contains("https://example.test/auth"));
        std::env::remove_var("PI_OPEN_BROWSER_DRY_RUN");
    }
}
