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
    cfg!(test)
        || matches!(
            std::env::var("PI_OPEN_BROWSER_DRY_RUN").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        )
}

pub fn copy_text_dry_run() -> bool {
    matches!(
        std::env::var("PI_COPY_DRY_RUN").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) || open_browser_dry_run()
}

/// Copy text without a shell, matching TS clipboard helpers (`pbcopy` / `xclip` / `clip`).
/// Returns a description of what happened; a failure says so instead of
/// claiming the clipboard holds the text.
pub fn copy_text(text: &str) -> String {
    match try_copy_text(text) {
        Ok(done) => done,
        Err(err) => format!("clipboard copy failed: {err}"),
    }
}

pub fn try_copy_text(text: &str) -> Result<String, String> {
    if copy_text_dry_run() {
        return Ok(format!("copy:{text}"));
    }
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("pbcopy", vec![])
    } else if cfg!(target_os = "windows") {
        ("clip", vec![])
    } else {
        ("xclip", vec!["-selection", "clipboard"])
    };
    let mut child = Command::new(cmd)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("could not run {cmd}: {err}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(text.as_bytes())
            .map_err(|err| format!("could not write to {cmd}: {err}"))?;
    }
    let status = child
        .wait()
        .map_err(|err| format!("could not wait for {cmd}: {err}"))?;
    if !status.success() {
        return Err(format!("{cmd} exited with {status}"));
    }
    Ok(format!("{cmd} clipboard"))
}

/// Opens `target`; the returned text names the command, or says the launch
/// failed so the caller can show the URL for manual use.
pub fn open_browser(target: &str) -> String {
    match try_open_browser(target) {
        Ok(launched) => launched,
        Err(err) => format!("browser launch failed ({err}); open this URL manually: {target}"),
    }
}

pub fn try_open_browser(target: &str) -> Result<String, String> {
    let (cmd, args) = open_browser_argv(target);
    let launched = format!("{cmd} {}", args.join(" "));
    // Only the explicit dry-run switch (or unit tests) suppresses the launch;
    // the shape of the URL never does.
    if open_browser_dry_run() {
        return Ok(launched);
    }
    Command::new(cmd)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("could not run {cmd}: {err}"))?;
    Ok(launched)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixture_looking_url_is_still_opened_outside_dry_run() {
        // Dry run is explicit; the URL's content must not decide it.
        assert!(open_browser_dry_run());
        let launched = try_open_browser("https://example.com/oauth/pi-fixture/callback").unwrap();
        assert!(launched.contains("pi-fixture"));
    }

    #[test]
    fn unit_tests_never_open_a_real_browser() {
        assert!(open_browser_dry_run());
    }

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
