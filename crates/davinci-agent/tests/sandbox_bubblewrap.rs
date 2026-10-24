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

const CONTROL_PATH_PROBE: &str = r#"
import json, os, pathlib, shutil
def allowed(operation):
    try:
        operation()
        return True
    except OSError:
        return False
def replace_with_dir(name):
    os.mkdir('replacement-' + name)
    os.rename('replacement-' + name, name)
result = {'new_top_level_file': allowed(lambda: pathlib.Path('created').write_text('ok'))}
for name in ['.git', '.davinci', '.pi']:
    result[name] = {
        'mkdir_inside': allowed(lambda: os.mkdir(name + '/x')),
        'write_inside': allowed(lambda: pathlib.Path(name + '/hooks.json').write_text('{}')),
        'rmtree': allowed(lambda: shutil.rmtree(name)),
        'rmdir': allowed(lambda: os.rmdir(name)),
        'rename_over': allowed(lambda: replace_with_dir(name)),
        'rename_away': allowed(lambda: os.rename(name, 'moved-' + name)),
    }
print(json.dumps(result))
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

#[test]
#[ignore = "requires Linux bubblewrap, python3, and permission to create user/PID/network namespaces"]
fn native_bubblewrap_protects_control_paths_that_did_not_exist() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let spec = spec(&workspace, SandboxMode::WorkspaceWrite);
    let request = ExecutionRequest {
        sandbox_id: spec.id.clone(),
        executable: "/usr/bin/python3".into(),
        argv: vec!["-c".into(), CONTROL_PATH_PROBE.into()],
        cwd: spec.workspace.clone(),
        launch_id: None,
    };
    let environment = sanitize_environment_from(&spec.environment, &BTreeMap::new()).unwrap();
    let prepared = LinuxBubblewrapBackend::new(PathBuf::from("/usr/bin/bwrap"))
        .prepare(&spec, &request, &environment)
        .unwrap();
    let output = Command::new(prepared.executable)
        .args(prepared.argv)
        .current_dir(prepared.cwd)
        .env_clear()
        .envs(prepared.environment)
        .stdin(Stdio::null())
        .output()
        .expect("provision bubblewrap before running this opt-in test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let denied = serde_json::json!({
        "mkdir_inside":false, "write_inside":false, "rmtree":false,
        "rmdir":false, "rename_over":false, "rename_away":false
    });
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        actual,
        serde_json::json!({
            "new_top_level_file":true, ".git":denied, ".davinci":denied, ".pi":denied
        })
    );
    // The host created the placeholders; the sandbox changed nothing in them.
    for name in [".git", ".davinci", ".pi"] {
        assert_eq!(
            fs::read_dir(workspace.join(name)).unwrap().count(),
            0,
            "{name}"
        );
    }
    assert!(workspace.join("created").is_file());
}

const GITFILE_TARGET_PROBE: &str = r#"
import json, os, pathlib
def allowed(operation):
    try:
        operation()
        return True
    except OSError:
        return False
print(json.dumps({
    'write_config': allowed(lambda: pathlib.Path('.metadata/config').write_text('[core]\nfsmonitor = evil')),
    'create_inside': allowed(lambda: pathlib.Path('.metadata/hooks').write_text('x')),
    'rename_away': allowed(lambda: os.rename('.metadata', 'moved')),
    'write_git_file': allowed(lambda: pathlib.Path('.git').write_text('gitdir: moved')),
    'write_elsewhere': allowed(lambda: pathlib.Path('other').write_text('ok')),
}))
"#;

#[test]
#[ignore = "requires Linux bubblewrap, python3, and permission to create user/PID/network namespaces"]
fn native_bubblewrap_protects_the_directory_a_git_file_points_at() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join(".metadata")).unwrap();
    fs::write(workspace.join(".git"), "gitdir: .metadata\n").unwrap();
    fs::write(workspace.join(".metadata/config"), "preserved").unwrap();
    let spec = spec(&workspace, SandboxMode::WorkspaceWrite);
    let request = ExecutionRequest {
        sandbox_id: spec.id.clone(),
        executable: "/usr/bin/python3".into(),
        argv: vec!["-c".into(), GITFILE_TARGET_PROBE.into()],
        cwd: spec.workspace.clone(),
        launch_id: None,
    };
    let environment = sanitize_environment_from(&spec.environment, &BTreeMap::new()).unwrap();
    let prepared = LinuxBubblewrapBackend::new(PathBuf::from("/usr/bin/bwrap"))
        .prepare(&spec, &request, &environment)
        .unwrap();
    let output = Command::new(prepared.executable)
        .args(prepared.argv)
        .current_dir(prepared.cwd)
        .env_clear()
        .envs(prepared.environment)
        .stdin(Stdio::null())
        .output()
        .expect("provision bubblewrap before running this opt-in test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        actual,
        serde_json::json!({
            "write_config":false, "create_inside":false, "rename_away":false,
            "write_git_file":false, "write_elsewhere":true
        })
    );
    assert_eq!(
        fs::read_to_string(workspace.join(".metadata/config")).unwrap(),
        "preserved"
    );
    assert_eq!(
        fs::read_dir(workspace.join(".metadata")).unwrap().count(),
        1
    );
}

#[test]
#[ignore = "requires Linux bubblewrap"]
fn native_bubblewrap_refuses_a_writable_mount_over_an_outside_git_directory() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let shared = temp.path().join("shared");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(shared.join("repo.git")).unwrap();
    fs::write(
        workspace.join(".git"),
        format!("gitdir: {}\n", shared.join("repo.git").display()),
    )
    .unwrap();
    let mut spec = spec(&workspace, SandboxMode::WorkspaceWrite);
    spec.filesystem.mounts.push(MountRule {
        source: Some(shared.to_string_lossy().into_owned()),
        target: "/shared".into(),
        access: MountAccess::ReadWrite,
    });
    let request = ExecutionRequest {
        sandbox_id: spec.id.clone(),
        executable: "/usr/bin/true".into(),
        argv: Vec::new(),
        cwd: spec.workspace.clone(),
        launch_id: None,
    };
    let error = LinuxBubblewrapBackend::new(PathBuf::from("/usr/bin/bwrap"))
        .prepare(&spec, &request, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("exposes workspace control path"),
        "{}",
        error.message
    );
}

const BARE_ROOT_PROBE: &str = r#"
import json, os, pathlib, sys
marker = sys.argv[1]
def allowed(operation):
    try:
        operation()
        return True
    except OSError:
        return False
def plant_head():
    pathlib.Path('HEAD').write_text('ref: refs/heads/main\n')
def plant_config():
    pathlib.Path('config').write_text(
        '[core]\n\trepositoryformatversion = 0\n\tbare = false\n\tworktree = ' + os.getcwd()
        + '\n\tfsmonitor = echo ran > ' + marker + ' #\n')
print(json.dumps({
    'head_file': allowed(plant_head),
    'head_rmdir': allowed(lambda: os.rmdir('HEAD')),
    'head_rename_away': allowed(lambda: os.rename('HEAD', 'moved-head')),
    'objects': allowed(lambda: os.mkdir('objects')),
    'refs': allowed(lambda: os.makedirs('refs/heads')),
    'config': allowed(plant_config),
}))
"#;

#[test]
#[ignore = "requires Linux bubblewrap, python3, git, and permission to create user/PID/network namespaces"]
fn native_bubblewrap_does_not_let_the_workspace_root_become_a_bare_repository() {
    let git_available = Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !git_available {
        eprintln!("git is unavailable: skipping the host `git status` half of this fixture");
    }
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let marker = temp.path().join("fsmonitor-ran");
    let spec = spec(&workspace, SandboxMode::WorkspaceWrite);
    let request = ExecutionRequest {
        sandbox_id: spec.id.clone(),
        executable: "/usr/bin/python3".into(),
        argv: vec![
            "-c".into(),
            BARE_ROOT_PROBE.into(),
            marker.to_string_lossy().into_owned(),
        ],
        cwd: spec.workspace.clone(),
        launch_id: None,
    };
    let environment = sanitize_environment_from(&spec.environment, &BTreeMap::new()).unwrap();
    let prepared = LinuxBubblewrapBackend::new(PathBuf::from("/usr/bin/bwrap"))
        .prepare(&spec, &request, &environment)
        .unwrap();
    let output = Command::new(prepared.executable)
        .args(prepared.argv)
        .current_dir(prepared.cwd)
        .env_clear()
        .envs(prepared.environment)
        .stdin(Stdio::null())
        .output()
        .expect("provision bubblewrap before running this opt-in test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        actual,
        serde_json::json!({
            "head_file":false, "head_rmdir":false, "head_rename_away":false,
            "objects":true, "refs":true, "config":true
        })
    );
    // The host placeholder survived: an empty directory is never a valid HEAD.
    assert!(workspace.join("HEAD").is_dir());
    assert_eq!(fs::read_dir(workspace.join("HEAD")).unwrap().count(), 0);

    // The workspace now holds everything a bare repository needs except a valid
    // HEAD. The host's own `git status` must not discover it and run fsmonitor.
    if git_available {
        let _ = Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(["status", "--porcelain"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            !marker.exists(),
            "host git ran core.fsmonitor planted by the sandbox"
        );
    }
}
