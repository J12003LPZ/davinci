//! WOR-280/281/282: exercise the production library in an isolated child.
//! Invalid disposable launchers prevent any browser or clipboard side effect.
use std::{fs, process::Command};

fn probe(kind: &str) {
    let fixture = tempfile::tempdir().unwrap();
    let executable = fixture
        .path()
        .join(if cfg!(windows) { "probe.exe" } else { "probe" });
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let (browser, _) = davinci_tui::open_browser_argv("https://example.test");
    let clipboard = if cfg!(target_os = "macos") {
        "pbcopy"
    } else if cfg!(windows) {
        "clip"
    } else {
        "xclip"
    };
    for command in [browser, clipboard] {
        let path = fixture.path().join(if cfg!(windows) {
            format!("{command}.exe")
        } else {
            command.into()
        });
        fs::write(&path, b"invalid executable fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    let output = Command::new(executable)
        .args(["--exact", "production_launcher_probe", "--nocapture"])
        .current_dir(fixture.path())
        .env("PATH", fixture.path())
        .env("DAVINCI_LAUNCHER_PROBE", kind)
        .env_remove("PI_OPEN_BROWSER_DRY_RUN")
        .env_remove("PI_COPY_DRY_RUN")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("production probe exercised"));
}

#[test]
fn browser_spawn_failure_preserves_the_manual_url() {
    probe("browser");
}

#[test]
fn clipboard_spawn_failure_never_claims_a_copy() {
    probe("clipboard");
}

#[test]
fn fixture_substring_does_not_skip_the_production_launcher() {
    probe("fixture-url");
}

#[test]
fn production_launcher_probe() {
    let Ok(kind) = std::env::var("DAVINCI_LAUNCHER_PROBE") else {
        return;
    };
    assert!(!davinci_tui::open_browser_dry_run());
    if kind == "clipboard" {
        let result = davinci_tui::copy_text("synthetic clipboard fixture");
        assert!(result.starts_with("clipboard copy failed:"), "{result}");
    } else {
        let url = if kind == "fixture-url" {
            "https://example.test/oauth/pi-fixture/callback"
        } else {
            "https://example.test/oauth/callback"
        };
        let result = davinci_tui::open_browser(url);
        assert!(result.starts_with("browser launch failed"), "{result}");
        assert!(result.contains(url), "{result}");
    }
    println!("production probe exercised");
}
