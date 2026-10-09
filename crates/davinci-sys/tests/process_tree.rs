//! WOR-171: `run_bounded` owns the whole Windows process tree, so cleanup
//! reaches a pipe-holding grandchild after the direct child has exited.
#![cfg(windows)]

use davinci_sys::process::{run_bounded, RunLimits};
use std::process::Command;
use std::time::{Duration, Instant};

const ROLE: &str = "DAVINCI_PROCESS_TREE_ROLE";
const MARKER: &str = "DAVINCI_PROCESS_TREE_MARKER";

/// Grandchild: inherits the helper's pipes, then acts late. Finite lifetime,
/// so a failing run still cleans itself up.
#[test]
fn pipe_grandchild_fixture() {
    if std::env::var(ROLE).as_deref() != Ok("grandchild") {
        return;
    }
    let path = std::path::PathBuf::from(std::env::var_os(MARKER).unwrap());
    std::fs::write(path.with_extension("ready"), b"ready").unwrap();
    std::thread::sleep(Duration::from_secs(2));
    std::fs::write(path, b"grandchild continued after helper returned").unwrap();
}

/// Direct child: starts the grandchild, waits until it runs, then either
/// exits at once or (with `hang`) outlives the timeout.
#[test]
fn pipe_child_fixture() {
    let role = std::env::var(ROLE).unwrap_or_default();
    if role != "child" && role != "hang" {
        return;
    }
    let marker = std::path::PathBuf::from(std::env::var_os(MARKER).unwrap());
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "pipe_grandchild_fixture"])
        .env(ROLE, "grandchild")
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.with_extension("ready").exists() {
        assert!(Instant::now() < deadline, "grandchild did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(child);
    if role == "hang" {
        std::thread::sleep(Duration::from_secs(5));
    }
}

fn run(
    role: &str,
    marker: &std::path::Path,
    timeout: Duration,
) -> davinci_sys::process::BoundedOutput {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "pipe_child_fixture"])
        .env(ROLE, role)
        .env(MARKER, marker);
    run_bounded(
        command,
        None,
        RunLimits {
            timeout,
            output_cap: 4096,
        },
        &|| false,
    )
    .unwrap()
}

#[test]
fn inherited_pipe_cleanup_stops_descendant_after_direct_child_exit() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("late-effect.txt");
    let started = Instant::now();
    let result = run("child", &marker, Duration::from_secs(10));
    assert!(result.status.unwrap().success());
    assert!(marker.with_extension("ready").exists());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "helper waited for the grandchild instead of cleaning up"
    );
    std::thread::sleep(Duration::from_millis(2500));
    assert!(
        !marker.exists(),
        "grandchild survived inherited-pipe cleanup and produced a late effect"
    );
}

#[test]
fn timeout_stops_descendants_too() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("late-effect.txt");
    let result = run("hang", &marker, Duration::from_millis(1500));
    assert!(result.timed_out);
    assert!(marker.with_extension("ready").exists());
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!marker.exists(), "grandchild survived the timeout kill");
}
