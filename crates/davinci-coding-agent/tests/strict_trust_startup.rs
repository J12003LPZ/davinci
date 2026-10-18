//! Strict project trust at process start (phase 1 of the reliability plan).
//!
//! These tests run the real `davinci` binary against disposable projects with
//! a redirected agent directory. Nothing here talks to a provider, a network
//! or the user's own config.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const PROMPT_NO: &str = "No \u{2014} Exit DaVinci";
const PROMPT_YES: &str = "Yes, I trust this folder";

struct Fixture {
    _root: tempfile::TempDir,
    project: PathBuf,
    config: PathBuf,
    trace: PathBuf,
    mcp_marker: PathBuf,
    hook_marker: PathBuf,
}

/// A project whose `.pi` directory would, if trusted, spawn an MCP server and
/// run a session-start hook. Both write a marker file, so "did project code
/// run" is a file-exists check.
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(root.path()).unwrap();
    let project = base.join("project");
    let config = base.join("config");
    fs::create_dir_all(project.join(".pi")).unwrap();
    fs::create_dir_all(&config).unwrap();
    let mcp_marker = base.join("mcp-ran");
    let hook_marker = base.join("hook-ran");
    let (program, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let write_marker = |marker: &Path| format!("echo ran> \"{}\"", marker.display());
    fs::write(
        project.join(".pi").join("mcp.json"),
        serde_json::json!({"mcpServers": {"planted": {
            "command": program,
            "args": [flag, write_marker(&mcp_marker)],
        }}})
        .to_string(),
    )
    .unwrap();
    fs::write(
        project.join(".pi").join("hooks.json"),
        serde_json::json!({"sessionStart": [[program, flag, write_marker(&hook_marker)]]})
            .to_string(),
    )
    .unwrap();
    Fixture {
        trace: base.join("startup-trace.log"),
        _root: root,
        project,
        config,
        mcp_marker,
        hook_marker,
    }
}

impl Fixture {
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_davinci"));
        command.env_clear();
        for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "COMSPEC"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .current_dir(&self.project)
            .arg("--offline")
            .arg("--session-dir")
            .arg(self.project.join("..").join("sessions"))
            // The agent directory is redirected below. HOME stays the real
            // one so the user's own `~/.agents/skills` is recognised as
            // user-owned, not as a project resource of a temp dir under it.
            .envs(
                ["HOME", "USERPROFILE"]
                    .into_iter()
                    .filter_map(|key| std::env::var_os(key).map(|value| (key, value))),
            )
            .env("PI_CODING_AGENT_DIR", &self.config)
            .env("DAVINCI_CODING_AGENT_DIR", &self.config)
            .env("PI_OFFLINE", "1")
            .env("PI_DISABLE_NETWORK", "1")
            .env("PI_FORCE_INTERACTIVE", "1")
            // A launch that passes the gate stops right after the agent is
            // built instead of opening a terminal UI on a pipe.
            .env("PI_SHUTDOWN_SIGNAL", "SIGTERM")
            .env("DAVINCI_STARTUP_TRACE", &self.trace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn trace(&self) -> String {
        fs::read_to_string(&self.trace).unwrap_or_default()
    }

    fn trust_store(&self) -> Option<serde_json::Value> {
        let text = fs::read_to_string(self.config.join("trust.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// The stored decision for the project folder, whatever the key spelling.
    fn stored_decision(&self) -> Option<bool> {
        let store = self.trust_store()?;
        let project = fs::canonicalize(&self.project).unwrap();
        let wanted = davinci_coding_agent::trust::display_path(&project);
        store.as_object()?.get(&wanted)?.as_bool()
    }
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

type Captured = Arc<Mutex<Vec<u8>>>;

fn drain(mut pipe: impl Read + Send + 'static) -> Captured {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buffer);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(read) = pipe.read(&mut chunk) {
            if read == 0 {
                break;
            }
            sink.lock().unwrap().extend_from_slice(&chunk[..read]);
        }
    });
    buffer
}

fn text(buffer: &Captured) -> String {
    String::from_utf8_lossy(&buffer.lock().unwrap()).into_owned()
}

fn spawn(command: &mut Command) -> (Child, Captured, Captured) {
    let mut child = command.spawn().expect("spawn davinci");
    let out = drain(child.stdout.take().unwrap());
    let err = drain(child.stderr.take().unwrap());
    (child, out, err)
}

/// Feed `input` to stdin, close it, and wait for the process to end.
fn run_with_input(command: &mut Command, input: &str) -> Run {
    let (mut child, out, err) = spawn(command);
    {
        let mut stdin = child.stdin.take().unwrap();
        let _ = stdin.write_all(input.as_bytes());
    }
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            panic!(
                "davinci did not exit\nstdout:\n{}\nstderr:\n{}",
                text(&out),
                text(&err)
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // The reader threads finish once the pipes close.
    std::thread::sleep(Duration::from_millis(200));
    Run {
        code: status.code(),
        stdout: text(&out),
        stderr: text(&err),
    }
}

impl Run {
    fn describe(&self) -> String {
        format!(
            "status {:?}\nstdout:\n{}\nstderr:\n{}",
            self.code, self.stdout, self.stderr
        )
    }
}

#[test]
fn strict_trust_no_project_hook_before_prompt() {
    let fx = fixture();
    // Hold stdin open and never answer: the process must sit at the gate.
    let mut command = fx.command();
    let (mut child, out, err) = spawn(&mut command);
    let _stdin = child.stdin.take();
    let started = Instant::now();
    while !text(&out).contains(PROMPT_YES) {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the trust prompt never appeared\nstdout:\n{}\nstderr:\n{}",
            text(&out),
            text(&err)
        );
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "davinci exited ({status}) before asking\nstdout:\n{}\nstderr:\n{}",
                text(&out),
                text(&err)
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Give a wrongly placed gate time to have started its work anyway.
    std::thread::sleep(Duration::from_millis(1500));
    let trace = fx.trace();
    let _ = child.kill();
    let _ = child.wait();

    assert!(text(&out).contains(PROMPT_NO), "{}", text(&out));
    assert!(
        !trace.contains("agent built"),
        "the agent (MCP, hooks, extensions) was built before the answer:\n{trace}"
    );
    assert!(
        !fx.mcp_marker.exists(),
        "project MCP server ran at the gate"
    );
    assert!(!fx.hook_marker.exists(), "project hook ran at the gate");
    assert_eq!(
        fx.stored_decision(),
        None,
        "nothing is stored before an answer"
    );
}

#[test]
fn strict_trust_decline_exits_without_loading() {
    for answer in ["\n", "1\n", "n\n", "no\n", ""] {
        let fx = fixture();
        let run = run_with_input(&mut fx.command(), answer);
        assert_eq!(run.code, Some(0), "answer {answer:?}\n{}", run.describe());
        assert!(run.stdout.contains(PROMPT_NO), "{}", run.describe());
        assert!(run.stdout.contains(PROMPT_YES), "{}", run.describe());
        assert_eq!(fx.stored_decision(), Some(false), "answer {answer:?}");
        let trace = fx.trace();
        assert!(
            !trace.contains("agent built"),
            "answer {answer:?} still built the agent:\n{trace}"
        );
        assert!(!fx.mcp_marker.exists(), "answer {answer:?}: MCP ran");
        assert!(!fx.hook_marker.exists(), "answer {answer:?}: hook ran");
    }
}

#[test]
fn strict_trust_accept_persists_then_continues() {
    for answer in ["2\n", "y\n", "yes\n"] {
        let fx = fixture();
        let run = run_with_input(&mut fx.command(), answer);
        assert_eq!(
            fx.stored_decision(),
            Some(true),
            "answer {answer:?}\n{}",
            run.describe()
        );
        let trace = fx.trace();
        let gate = trace.find("trust gate").expect("gate mark");
        let built = trace.find("agent built").expect("the session continued");
        assert!(gate < built, "the gate must come first:\n{trace}");
    }
}

#[test]
fn strict_trust_stored_denial_survives_relaunch_without_asking() {
    let fx = fixture();
    let first = run_with_input(&mut fx.command(), "\n");
    assert_eq!(fx.stored_decision(), Some(false), "{}", first.describe());
    let _ = fs::remove_file(&fx.trace);
    let second = run_with_input(&mut fx.command(), "");
    assert!(
        !second.stdout.contains(PROMPT_YES),
        "a stored denial must not be asked again\n{}",
        second.describe()
    );
    assert_eq!(fx.stored_decision(), Some(false));
    assert!(!fx.mcp_marker.exists(), "a denied project's MCP server ran");
    assert!(!fx.hook_marker.exists(), "a denied project's hook ran");
}

#[test]
fn strict_trust_stored_grant_is_not_asked_again() {
    let fx = fixture();
    run_with_input(&mut fx.command(), "2\n");
    let _ = fs::remove_file(&fx.trace);
    let second = run_with_input(&mut fx.command(), "");
    assert!(!second.stdout.contains(PROMPT_YES), "{}", second.describe());
    assert!(fx.trace().contains("agent built"));
}

#[test]
fn strict_trust_is_silent_when_the_project_has_nothing_to_trust() {
    let fx = fixture();
    fs::remove_dir_all(fx.project.join(".pi")).unwrap();
    let run = run_with_input(&mut fx.command(), "");
    assert!(!run.stdout.contains(PROMPT_YES), "{}", run.describe());
    assert_eq!(fx.stored_decision(), None);
    assert!(fx.trace().contains("agent built"));
}

#[test]
fn strict_trust_cli_override_skips_the_question() {
    let fx = fixture();
    let run = run_with_input(fx.command().arg("--approve"), "");
    assert!(!run.stdout.contains(PROMPT_YES), "{}", run.describe());
    assert_eq!(fx.stored_decision(), None, "an override stores nothing");
    assert!(fx.trace().contains("agent built"));
}

#[test]
fn strict_trust_unreadable_store_is_reported_and_nothing_is_trusted() {
    let fx = fixture();
    fs::write(fx.config.join("trust.json"), "{ not json").unwrap();
    let run = run_with_input(&mut fx.command(), "");
    assert!(
        run.stderr.contains("trust store") && run.stderr.contains("trust.json"),
        "{}",
        run.describe()
    );
    assert!(!run.stdout.contains(PROMPT_YES), "{}", run.describe());
    assert!(!fx.mcp_marker.exists());
    assert!(!fx.hook_marker.exists());
}

// ---- Task 3: canonical identity, explicit deny, fail closed -------------

mod store {
    use davinci_coding_agent::trust::{
        canonicalize_trust_path, get_project_trust_options, resolve_project_trusted,
        ProjectTrustStore,
    };
    use std::{fs, path::Path};

    fn project_with_hooks(path: &Path) {
        fs::create_dir_all(path.join(".pi")).unwrap();
        fs::write(path.join(".pi").join("hooks.json"), "{}").unwrap();
    }

    /// A directory alias: a symlink, or a junction where Windows refuses
    /// symlinks to unprivileged users. `false` when neither can be made.
    fn link_dir(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link).is_ok()
                || std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(link)
                    .arg(target)
                    .output()
                    .is_ok_and(|output| output.status.success())
        }
    }

    fn resolve(agent: &Path, cwd: &Path) -> bool {
        resolve_project_trusted(agent, cwd, None, None, &[])
    }

    #[test]
    fn strict_trust_canonical_path_and_explicit_deny() {
        let root = tempfile::tempdir().unwrap();
        let agent = root.path().join("agent");
        let parent = root.path().join("work");
        let child = parent.join("repo");
        project_with_hooks(&child);
        let store = ProjectTrustStore::open(&agent);

        // An explicit child denial survives choosing "Trust parent folder".
        store.set(&child, Some(false)).unwrap();
        let options = get_project_trust_options(&child, false);
        let parent_option = options
            .iter()
            .find(|option| option.label.starts_with("Trust parent folder"))
            .expect("parent option");
        store.set_many(&parent_option.updates).unwrap();
        assert_eq!(
            store.try_get(&parent).unwrap(),
            Some(true),
            "parent granted"
        );
        assert_eq!(
            store.try_get(&child).unwrap(),
            Some(false),
            "child deny kept"
        );
        assert!(!resolve(&agent, &child));

        // Without a child entry the parent grant applies as before.
        store.set(&child, None).unwrap();
        assert!(resolve(&agent, &child));

        // ...and the denial also survives a relaunch: a new store handle on
        // the same directory sees it.
        store.set(&child, Some(false)).unwrap();
        assert!(!resolve(&agent, &child));
        assert_eq!(
            ProjectTrustStore::open(&agent).try_get(&child).unwrap(),
            Some(false)
        );
    }

    #[test]
    fn strict_trust_alias_and_real_path_share_one_identity() {
        let root = tempfile::tempdir().unwrap();
        let agent = root.path().join("agent");
        let real = root.path().join("real");
        project_with_hooks(&real);
        let alias = root.path().join("alias");
        if !link_dir(&real, &alias) {
            eprintln!("skipped: this host cannot create a directory symlink or junction");
            return;
        }
        let store = ProjectTrustStore::open(&agent);
        store.set(&alias, Some(false)).unwrap();
        assert_eq!(store.try_get(&real).unwrap(), Some(false));
        assert!(!resolve(&agent, &real));
        assert!(!resolve(&agent, &alias));
        assert_eq!(
            canonicalize_trust_path(&alias),
            canonicalize_trust_path(&real)
        );
        // One key, not one per spelling.
        let text = fs::read_to_string(agent.join("trust.json")).unwrap();
        let keys = serde_json::from_str::<serde_json::Value>(&text).unwrap();
        assert_eq!(keys.as_object().unwrap().len(), 1, "{text}");
        // Trust given through either spelling replaces the one entry.
        store.set(&real, Some(true)).unwrap();
        assert!(resolve(&agent, &alias));
    }

    #[test]
    #[cfg(windows)]
    fn strict_trust_drive_letter_case_is_one_identity() {
        let root = tempfile::tempdir().unwrap();
        let agent = root.path().join("agent");
        let project = root.path().join("repo");
        project_with_hooks(&project);
        let text = project.to_string_lossy().into_owned();
        let mut chars = text.chars();
        let drive = chars.next().unwrap();
        let swapped = if drive.is_ascii_uppercase() {
            drive.to_ascii_lowercase()
        } else {
            drive.to_ascii_uppercase()
        };
        let other_case = std::path::PathBuf::from(format!("{swapped}{}", chars.as_str()));
        let store = ProjectTrustStore::open(&agent);
        store.set(&other_case, Some(false)).unwrap();
        assert_eq!(store.try_get(&project).unwrap(), Some(false));
        assert!(!resolve(&agent, &project));
    }

    #[test]
    fn strict_trust_corrupt_store_fails_closed_with_an_actionable_error() {
        let root = tempfile::tempdir().unwrap();
        let agent = root.path().join("agent");
        let project = root.path().join("repo");
        project_with_hooks(&project);
        fs::create_dir_all(&agent).unwrap();
        for corrupt in ["{ not json", "[1,2]", r#"{"x": "yes"}"#] {
            fs::write(agent.join("trust.json"), corrupt).unwrap();
            let store = ProjectTrustStore::open(&agent);
            let error = store.try_get(&project).unwrap_err();
            assert!(error.contains("trust.json"), "names the file: {error}");
            assert!(!resolve_project_trusted(
                &agent,
                &project,
                None,
                Some("always"),
                &[]
            ));
            // A write must not paper over the damage either.
            assert!(store.set(&project, Some(true)).is_err());
            assert_eq!(
                fs::read_to_string(agent.join("trust.json")).unwrap(),
                corrupt
            );
        }
    }
}
