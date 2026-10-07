//! The Windows AppContainer launcher, end to end with real processes: what a
//! confined child can read, write and reach, and that it dies with the
//! launcher. No design runtime is needed; `cmd.exe` and `curl.exe` ship with
//! Windows.
#![cfg(windows)]

use davinci_agent::sandbox::appcontainer::{process_confinement, LaunchPlan, LAUNCHER_ARG};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    read: PathBuf,
    write: PathBuf,
    outside: PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let root = davinci_agent::strip_verbatim_prefix(&root);
    let read = root.join("read");
    let write = root.join("write");
    let outside = root.join("outside");
    for directory in [&read, &write, &outside] {
        std::fs::create_dir(directory).unwrap();
    }
    std::fs::write(read.join("note.txt"), "readable-note").unwrap();
    std::fs::write(outside.join("secret.txt"), "TOP-SECRET").unwrap();
    Fixture {
        _temp: temp,
        root,
        read,
        write,
        outside,
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn plan(fixture: &Fixture, executable: &str, argv: &[&str]) -> LaunchPlan {
    LaunchPlan {
        executable: executable.into(),
        argv: argv.iter().map(|arg| arg.to_string()).collect(),
        cwd: text(&fixture.write),
        read: vec![text(&fixture.read)],
        write: vec![text(&fixture.write)],
        hidden: vec![],
        protected: vec![],
    }
}

fn cmd() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!(r"{root}\System32\cmd.exe")
}

fn launch(plan: &LaunchPlan) -> Output {
    Command::new(env!("CARGO_BIN_EXE_davinci"))
        .args([LAUNCHER_ARG, &plan.encode()])
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn shell(fixture: &Fixture, line: &str) -> Output {
    launch(&plan(fixture, &cmd(), &["/d", "/c", line]))
}

#[test]
fn a_confined_child_reads_and_writes_only_its_mounts() {
    let f = fixture();
    let note = shell(&f, &format!("type {}", text(&f.read.join("note.txt"))));
    assert!(note.status.success(), "{note:?}");
    assert_eq!(
        String::from_utf8_lossy(&note.stdout).trim(),
        "readable-note"
    );

    let secret = shell(&f, &format!("type {}", text(&f.outside.join("secret.txt"))));
    assert!(!secret.status.success(), "{secret:?}");
    assert!(!String::from_utf8_lossy(&secret.stdout).contains("TOP-SECRET"));

    let wrote = shell(&f, "echo made> made.txt");
    assert!(wrote.status.success(), "{wrote:?}");
    assert_eq!(
        std::fs::read_to_string(f.write.join("made.txt"))
            .unwrap()
            .trim(),
        "made"
    );

    let refused = shell(&f, &format!("echo x> {}", text(&f.read.join("new.txt"))));
    assert!(!refused.status.success(), "{refused:?}");
    assert!(!f.read.join("new.txt").exists());
    let replaced = shell(&f, &format!("echo x> {}", text(&f.read.join("note.txt"))));
    assert!(!replaced.status.success(), "{replaced:?}");
    assert_eq!(
        std::fs::read_to_string(f.read.join("note.txt")).unwrap(),
        "readable-note"
    );
}

/// `icacls` of a path and what is under it: the ACLs as Windows reports them.
fn acls(path: &Path) -> String {
    let out = Command::new("icacls")
        .args([&text(path), "/t", "/q"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn hidden_protected_and_nested_mounts_fail_before_any_acl_change() {
    let f = fixture();
    let private = f.write.join("private");
    std::fs::create_dir(&private).unwrap();
    let before = acls(&private);
    let mut unsafe_plan = plan(&f, &cmd(), &["/d", "/c", "echo unsafe> file.txt"]);
    unsafe_plan.hidden = vec![text(&private)];
    let hidden = launch(&unsafe_plan);
    assert_eq!(hidden.status.code(), Some(125), "{hidden:?}");
    unsafe_plan.hidden.clear();
    unsafe_plan.protected = vec![text(&private)];
    let protected = launch(&unsafe_plan);
    assert_eq!(protected.status.code(), Some(125), "{protected:?}");
    unsafe_plan.protected.clear();
    unsafe_plan.read.push(text(&private));
    let overlap = launch(&unsafe_plan);
    assert_eq!(overlap.status.code(), Some(125), "{overlap:?}");
    assert_eq!(acls(&private), before);
    assert!(!f.write.join("file.txt").exists());
}

#[test]
fn an_ambient_write_grant_on_a_read_mount_fails_closed() {
    let f = fixture();
    let acl = Command::new("icacls")
        .args([&text(&f.read), "/grant", "*S-1-15-2-1:(OI)(CI)M"])
        .output()
        .unwrap();
    assert!(acl.status.success(), "{acl:?}");
    let before = acls(&f.write);
    let output = shell(&f, "echo should-not-run> unsafe.txt");
    assert_eq!(output.status.code(), Some(125), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("read mount is writable"));
    assert!(!f.write.join("unsafe.txt").exists());
    assert_eq!(acls(&f.write), before, "rejection mutated another mount");
}

#[test]
fn a_confined_child_reaches_no_network_not_even_the_host() {
    let f = fixture();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let curl = format!(r"{root}\System32\curl.exe");
    // The same request outside the container reaches the listener, so a
    // failure inside is the container's doing.
    let outside = Command::new(&curl)
        .args(["-s", "-m", "5", &format!("http://127.0.0.1:{port}/")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let accepted = loop {
        match listener.accept() {
            Ok((stream, _)) => break Some(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("{error}"),
        }
    };
    assert!(accepted.is_some(), "control request never arrived");
    drop(accepted);
    let _ = outside.wait_with_output();

    let inside = launch(&plan(
        &f,
        &curl,
        &["-s", "-m", "5", &format!("http://127.0.0.1:{port}/")],
    ));
    assert!(!inside.status.success(), "{inside:?}");
    let internet = launch(&plan(&f, &curl, &["-s", "-m", "5", "https://example.com/"]));
    assert!(!internet.status.success(), "{internet:?}");
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}

fn children_of(parent: u32) -> Vec<(u32, String)> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "Get-CimInstance Win32_Process -Filter 'ParentProcessId={parent}' | ForEach-Object {{ \"$($_.ProcessId)`t$($_.Name)\" }}"
            ),
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, name) = line.trim().split_once('\t')?;
            Some((pid.parse().ok()?, name.to_ascii_lowercase()))
        })
        .collect()
}

#[test]
fn the_child_is_an_appcontainer_without_network_and_dies_with_the_launcher() {
    let f = fixture();
    // A busy loop: cmd.exe needs nothing outside the container to keep going.
    let busy = plan(&f, &cmd(), &["/d", "/c", "for /l %i in (0,0,1) do @rem"]);
    let mut launcher = Command::new(env!("CARGO_BIN_EXE_davinci"))
        .args([LAUNCHER_ARG, &busy.encode()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let child = loop {
        if let Some((pid, _)) = children_of(launcher.id())
            .into_iter()
            .find(|(_, name)| name == "cmd.exe")
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "confined child never started");
        std::thread::sleep(Duration::from_millis(100));
    };
    let confinement = process_confinement(child).expect("child token");
    assert!(confinement.app_container);
    assert!(!confinement.network_capability);
    eprintln!("confined child integrity {:#x}", confinement.integrity);
    // The test's own process is not confined, so the check can tell.
    assert!(
        !process_confinement(std::process::id())
            .unwrap()
            .app_container
    );

    launcher.kill().unwrap();
    launcher.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while process_confinement(child).is_some() {
        assert!(
            Instant::now() < deadline,
            "confined child {child} survived its launcher"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn concurrent_launches_have_distinct_private_profiles() {
    let f = fixture();
    let first = plan(
        &f,
        &cmd(),
        &[
            "/d",
            "/c",
            "echo private> %TEMP%\\sentinel.txt & echo %TEMP% & for /l %i in (0,0,1) do @rem",
        ],
    );
    let mut launcher = Command::new(env!("CARGO_BIN_EXE_davinci"))
        .args([LAUNCHER_ARG, &first.encode()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut temp = String::new();
    std::io::BufReader::new(launcher.stdout.take().unwrap())
        .read_line(&mut temp)
        .unwrap();
    let sentinel = PathBuf::from(temp.trim()).join("sentinel.txt");
    assert!(sentinel.exists(), "first browser profile was not created");
    let second = shell(&f, &format!("type {}", text(&sentinel)));
    assert!(
        !second.status.success(),
        "second AppContainer read first one's temp: {second:?}"
    );
    assert!(!String::from_utf8_lossy(&second.stdout).contains("private"));
    launcher.kill().unwrap();
    launcher.wait().unwrap();
}

#[test]
fn a_launcher_given_no_valid_plan_starts_nothing() {
    let out = Command::new(env!("CARGO_BIN_EXE_davinci"))
        .args([LAUNCHER_ARG, "not-a-plan"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(125));
    assert!(String::from_utf8_lossy(&out.stderr).contains("launch plan"));
    let _ = fixture().root;
}
