#![cfg(target_os = "linux")]
//! Real filesystem, network and environment checks of the production launch plan.
//! Process ownership/lifecycle is covered separately by supervisor tests.
use davinci_agent::sandbox::{sanitize_environment_from, LinuxBubblewrapBackend, SandboxBackend};
use davinci_protocol::{ExecutionRequest, MountAccess, MountRule, SandboxMode, SandboxSpec};
use std::{
    collections::BTreeMap,
    fs,
    net::{TcpListener, TcpStream},
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const PROBE: &str = r#"
import json, os, pathlib, socket, sys
outside, port = pathlib.Path(sys.argv[1]), int(sys.argv[2])
def allowed(operation):
    try:
        operation()
        return True
    except OSError:
        return False
def connect():
    with socket.create_connection(('127.0.0.1', port), timeout=1):
        pass
print(json.dumps({
    'workspace_read': pathlib.Path('readable').read_text() == 'fixture',
    'workspace_write': allowed(lambda: pathlib.Path('allowed').write_text('allowed')),
    'outside_read': allowed(lambda: (outside / 'secret').read_text()),
    'outside_write': allowed(lambda: (outside / 'escaped').write_text('bad')),
    'symlink_read': allowed(lambda: pathlib.Path('escape/secret').read_text()),
    'symlink_write': allowed(lambda: pathlib.Path('escape/escaped').write_text('bad')),
    'git_write': allowed(lambda: pathlib.Path('.git/config').write_text('bad')),
    'network': allowed(connect),
    'secret_env': 'DAVINCI_FIXTURE_SECRET' in os.environ,
    'private_temp': allowed(lambda: pathlib.Path('/tmp/private-fixture').write_text('ok')),
}))
"#;

fn spec(workspace: &Path, mode: SandboxMode) -> SandboxSpec {
    let workspace = workspace.to_string_lossy().into_owned();
    let mut spec: SandboxSpec = serde_json::from_value(serde_json::json!({
        "id":"bubblewrap-native", "mode":mode, "backend":"linux_bubblewrap",
        "workspace":workspace,
        "required_capabilities":{
            "filesystem_isolation":true,"network_denied":true,"environment_isolation":true
        }
    }))
    .unwrap();
    for path in ["/usr", "/bin", "/lib", "/lib64"] {
        if Path::new(path).exists() {
            spec.filesystem.mounts.push(MountRule {
                source: Some(path.into()),
                target: path.into(),
                access: MountAccess::ReadOnly,
            });
        }
    }
    spec.filesystem.mounts.push(MountRule {
        source: Some(workspace.clone()),
        target: workspace,
        access: if mode == SandboxMode::WorkspaceWrite {
            MountAccess::ReadWrite
        } else {
            MountAccess::ReadOnly
        },
    });
    spec
}

#[test]
#[ignore = "requires Linux bubblewrap, python3, and permission to create user/PID/network namespaces"]
fn native_bubblewrap_enforces_filesystem_network_and_environment() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(workspace.join("readable"), "fixture").unwrap();
    fs::write(workspace.join(".git/config"), "preserved").unwrap();
    fs::write(outside.join("secret"), "synthetic fixture").unwrap();
    symlink(&outside, workspace.join("escape")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    // Positive control: the same destination is reachable outside the sandbox.
    drop(TcpStream::connect(listener.local_addr().unwrap()).unwrap());
    let backend = LinuxBubblewrapBackend::new(PathBuf::from("/usr/bin/bwrap"));
    for mode in [SandboxMode::WorkspaceWrite, SandboxMode::Restricted] {
        let spec = spec(&workspace, mode);
        let environment = sanitize_environment_from(
            &spec.environment,
            &BTreeMap::from([("DAVINCI_FIXTURE_SECRET".into(), "synthetic".into())]),
        )
        .unwrap();
        let request = ExecutionRequest {
            sandbox_id: spec.id.clone(),
            executable: "/usr/bin/python3".into(),
            argv: vec![
                "-c".into(),
                PROBE.into(),
                outside.to_string_lossy().into_owned(),
                listener.local_addr().unwrap().port().to_string(),
            ],
            cwd: spec.workspace.clone(),
            launch_id: None,
        };
        let prepared = backend.prepare(&spec, &request, &environment).unwrap();
        let mut child = Command::new(prepared.executable)
            .args(prepared.argv)
            .current_dir(prepared.cwd)
            .env_clear()
            .envs(prepared.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("provision bubblewrap before running this opt-in test");
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("bubblewrap fixture timed out");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{mode:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            actual,
            serde_json::json!({
                "workspace_read":true, "workspace_write":mode == SandboxMode::WorkspaceWrite,
                "outside_read":false, "outside_write":false,
                "symlink_read":false, "symlink_write":false, "git_write":false,
                "network":false, "secret_env":false, "private_temp":true
            }),
            "{mode:?}"
        );
        assert!(!outside.join("escaped").exists());
        assert_eq!(
            fs::read_to_string(workspace.join(".git/config")).unwrap(),
            "preserved"
        );
        println!("{mode:?}: {actual}");
    }
}
