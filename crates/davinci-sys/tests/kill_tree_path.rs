//! `kill_tree` must not resolve its cleanup program through PATH. Its own
//! test binary because it rewrites PATH for the whole process.

use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn wait_exit(child: &mut Child, within: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < within {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[cfg(unix)]
#[test]
fn kill_tree_ignores_a_planted_kill_on_path() {
    use std::os::unix::fs::PermissionsExt;
    let shims = tempfile::tempdir().unwrap();
    let marker = shims.path().join("shim-ran");
    let shim = shims.path().join("kill");
    std::fs::write(&shim, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut command = Command::new("/bin/sleep");
    command.arg("30");
    davinci_sys::process::set_own_process_group(&mut command);
    let mut child = command.spawn().unwrap();
    std::env::set_var("PATH", shims.path());
    davinci_sys::process::kill_tree(child.id());
    let exited = wait_exit(&mut child, Duration::from_secs(5));
    if !exited {
        let _ = child.kill();
    }
    assert!(exited, "the process group was not signalled");
    assert!(!marker.exists(), "a kill found on PATH was executed");
}

#[cfg(windows)]
#[test]
fn kill_tree_runs_taskkill_from_the_system_directory() {
    let taskkill = davinci_sys::process::system_program("taskkill.exe").unwrap();
    assert!(taskkill.is_absolute());
    assert!(taskkill.ends_with("taskkill.exe"));
    let ping = davinci_sys::process::system_program("PING.EXE").unwrap();
    let mut child = Command::new(ping)
        .args(["-n", "30", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // An empty PATH: only a resolution that never consults it still works.
    let empty = tempfile::tempdir().unwrap();
    std::env::set_var("PATH", empty.path());
    davinci_sys::process::kill_tree(child.id());
    let exited = wait_exit(&mut child, Duration::from_secs(10));
    if !exited {
        let _ = child.kill();
    }
    assert!(exited, "taskkill did not stop the child");
}

#[test]
fn kill_tree_never_signals_our_own_group() {
    // pid 0 means "my group" to kill(2); reaching this line means we survived.
    davinci_sys::process::kill_tree(0);
}
