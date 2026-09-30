#![cfg(target_os = "macos")]
//! Real macOS partial-policy fixtures plus fail-closed ownership negotiation.
//! These reduced-capability policies are never used by session settings.
use davinci_agent::{
    jobs::supervisor::{self, ProcessConfig, ProcessEvent, Supervisor, SupervisorCommand},
    sandbox::sanitize_environment_from,
};
use davinci_protocol::{MountAccess, MountRule, NetworkPolicy, SandboxBackendKind, SandboxSpec};
use std::{
    collections::BTreeMap,
    fs,
    net::{TcpListener, TcpStream, UdpSocket},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn host() -> SupervisorCommand {
    SupervisorCommand {
        executable: std::env::current_exe().unwrap(),
        argv: vec![
            "--exact".into(),
            "supervisor_fixture_entry".into(),
            "--nocapture".into(),
        ],
    }
}

#[test]
fn supervisor_fixture_entry() {
    if std::env::var_os("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").is_some() {
        supervisor::run();
    }
}

#[test]
fn seatbelt_command_fixture() {
    let Ok(action) = std::env::var("SEATBELT_ACTION") else {
        return;
    };
    let workspace = std::env::current_dir().unwrap();
    if action == "heartbeat" || action == "detached-heartbeat" {
        let detached = action == "detached-heartbeat";
        if detached {
            assert!(unsafe { libc::setsid() } >= 0);
        }
        // Detached test fixtures self-terminate even if their test owner dies.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !detached || Instant::now() < deadline {
            fs::write(workspace.join("heartbeat"), format!("{:?}", Instant::now())).unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        return;
    }
    if action == "observe" {
        let outside = PathBuf::from(std::env::var("SEATBELT_OUTSIDE").unwrap());
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let temp = PathBuf::from(std::env::var("TMPDIR").unwrap());
        let value = serde_json::json!({
            "workspace_write": fs::write(workspace.join("allowed"), "allowed").is_ok(),
            "workspace_read": fs::read(workspace.join("readable")).is_ok(),
            "outside_read": fs::read(outside.join("secret")).is_ok(),
            "data_volume_read": fs::read(Path::new("/System/Volumes/Data").join(outside.canonicalize().unwrap().strip_prefix("/").unwrap()).join("secret")).is_ok(),
            "outside_write": fs::write(outside.join("escaped"), "bad").is_ok(),
            "symlink_read": fs::read(workspace.join("escape/secret")).is_ok(),
            "symlink_write": fs::write(workspace.join("escape/escaped"), "bad").is_ok(),
            "git_write": fs::write(workspace.join(".git/config"), "bad").is_ok(),
            "git_hardlink_write": fs::hard_link(workspace.join(".git/config"), workspace.join("git-alias")).and_then(|_| fs::write(workspace.join("git-alias"), "bad")).is_ok(),
            "new_control": fs::create_dir(workspace.join(".davinci")).is_ok(),
            "new_legacy_control": fs::create_dir(workspace.join(".pi")).is_ok(),
            "metadata_alias_write": fs::write(workspace.join("metadata/config"), "bad").is_ok(),
            "hidden_read": fs::read(workspace.join("hidden/secret")).is_ok(),
            "hidden_write": fs::write(workspace.join("hidden/secret"), "bad").is_ok(),
            "home_write": fs::write(home.join("state"), "ok").is_ok(),
            "temp_write": fs::write(temp.join("state"), "ok").is_ok(),
            "secret_env": std::env::var_os("AWS_SECRET_ACCESS_KEY").is_some(),
            "home": home, "temp": temp,
            "setsid": fork_can_setsid(),
            "setpgid": unsafe { libc::setpgid(0, 0) } == 0,
        });
        println!("SEATBELT_RESULT {value}");
    } else if action == "network" {
        let ipv4 = std::env::var("SEATBELT_IPV4").unwrap();
        let ipv6 = std::env::var("SEATBELT_IPV6").unwrap();
        let dns = std::env::var("SEATBELT_DNS").unwrap();
        let socket = workspace.join("fixture.sock");
        let value = serde_json::json!({
            "ipv4": TcpStream::connect(ipv4).is_ok(),
            "ipv6": TcpStream::connect(ipv6).is_ok(),
            "dns_udp": UdpSocket::bind("127.0.0.1:0").and_then(|socket| socket.send_to(&[0; 12], dns)).is_ok(),
            "unix": UnixStream::connect(socket).is_ok(),
        });
        println!("SEATBELT_RESULT {value}");
    } else if action == "descendants" || action == "wait" || action == "detached-descendants" {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "seatbelt_command_fixture", "--nocapture"])
            .env(
                "SEATBELT_ACTION",
                if action == "detached-descendants" {
                    "detached-heartbeat"
                } else {
                    "heartbeat"
                },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let pid = spawn_orphan_fixture(child);
        fs::write(workspace.join("descendant.pid"), pid.to_string()).unwrap();
        while !workspace.join("heartbeat").exists() {
            std::thread::sleep(Duration::from_millis(5));
        }
        println!(
            "SEATBELT_RESULT {}",
            serde_json::json!({"temp":std::env::var("TMPDIR").unwrap()})
        );
        if action == "wait" {
            loop {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

// Deliberately exit the command parent without waiting, to exercise group
// cleanup and the detached ownership gap. Detached fixtures also self-expire.
#[allow(clippy::zombie_processes)]
fn spawn_orphan_fixture(mut command: Command) -> u32 {
    command.spawn().unwrap().id()
}

fn fork_can_setsid() -> bool {
    let child = unsafe { libc::fork() };
    if child == 0 {
        let code = if unsafe { libc::setsid() } >= 0 { 0 } else { 1 };
        unsafe { libc::_exit(code) };
    }
    if child < 0 {
        return false;
    }
    let mut status = 0;
    (unsafe { libc::waitpid(child, &mut status, 0) }) == child
        && libc::WIFEXITED(status)
        && libc::WEXITSTATUS(status) == 0
}

fn spec(workspace: &Path) -> SandboxSpec {
    let workspace = workspace.canonicalize().unwrap();
    let text = workspace.to_string_lossy().into_owned();
    let mut spec: SandboxSpec = serde_json::from_value(serde_json::json!({
        "id":"seatbelt-native", "mode":"workspace_write", "backend":"macos_seatbelt", "workspace":text,
        "process":{"allow_background":true},
        "required_capabilities":{"filesystem_isolation":true,"network_denied":true,"environment_isolation":true}
    })).unwrap();
    for path in [
        PathBuf::from("/System"),
        PathBuf::from("/usr"),
        PathBuf::from("/bin"),
        PathBuf::from("/sbin"),
        std::env::current_exe().unwrap(),
    ] {
        let path = path.canonicalize().unwrap().to_string_lossy().into_owned();
        spec.filesystem.mounts.push(MountRule {
            source: Some(path.clone()),
            target: path,
            access: MountAccess::ReadOnly,
        });
    }
    let cache = Path::new("/System/Volumes/Preboot/Cryptexes/OS/System");
    if let Ok(cache) = cache.canonicalize() {
        let cache = cache.to_string_lossy().into_owned();
        spec.filesystem.mounts.push(MountRule {
            source: Some(cache.clone()),
            target: cache,
            access: MountAccess::ReadOnly,
        });
    }
    spec.filesystem.mounts.push(MountRule {
        source: Some(spec.workspace.clone()),
        target: spec.workspace.clone(),
        access: MountAccess::ReadWrite,
    });
    spec
}

fn spawn(
    spec: SandboxSpec,
    action: &str,
    extra: BTreeMap<String, String>,
) -> (Supervisor, Arc<Mutex<Vec<u8>>>) {
    let mut parent = extra;
    parent.insert("AWS_SECRET_ACCESS_KEY".into(), "must-not-inherit".into());
    let mut policy = spec.environment.clone();
    policy
        .inject
        .insert("SEATBELT_ACTION".into(), action.into());
    // Fixture addresses only, never production credentials.
    policy.inject.extend(
        parent
            .iter()
            .filter(|(name, _)| name.starts_with("SEATBELT_"))
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    let environment = sanitize_environment_from(&policy, &parent).unwrap();
    let mut config = ProcessConfig::new(
        std::env::current_exe().unwrap(),
        vec![
            "--exact".into(),
            "seatbelt_command_fixture".into(),
            "--nocapture".into(),
        ],
        PathBuf::from(&spec.workspace),
        environment,
    )
    .with_sandbox(spec);
    if action == "wait" {
        config = config.as_background();
    }
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = output.clone();
    let process = Supervisor::spawn(
        &host(),
        config,
        Arc::new(move |event| {
            if let ProcessEvent::Output(bytes) = event {
                captured.lock().unwrap().extend(bytes);
            }
        }),
    )
    .expect("native Seatbelt must be usable on macOS CI");
    (process, output)
}

fn result(bytes: &Arc<Mutex<Vec<u8>>>) -> serde_json::Value {
    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    let line = output
        .lines()
        .find_map(|line| line.split_once("SEATBELT_RESULT ").map(|(_, value)| value))
        .unwrap_or_else(|| panic!("missing fixture result: {output}"));
    serde_json::from_str(line).unwrap()
}

fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "fixture timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn native_workspace_secrets_control_paths_symlinks_temp_and_restricted_mode() {
    assert_eq!(
        davinci_agent::sandbox::detected_native_backend(Path::new("/")),
        None
    );
    for restricted in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), "host secret").unwrap();
        fs::write(root.path().join("readable"), "data").unwrap();
        fs::create_dir(root.path().join("metadata")).unwrap();
        fs::write(root.path().join("metadata/config"), "protected").unwrap();
        std::os::unix::fs::symlink("metadata", root.path().join(".git")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        fs::create_dir(root.path().join("hidden")).unwrap();
        fs::write(root.path().join("hidden/secret"), "hidden").unwrap();
        let mut spec = spec(root.path());
        spec.filesystem.mounts.push(MountRule {
            source: None,
            target: root
                .path()
                .canonicalize()
                .unwrap()
                .join("hidden")
                .to_string_lossy()
                .into_owned(),
            access: MountAccess::Hidden,
        });
        if restricted {
            spec.mode = davinci_protocol::SandboxMode::Restricted;
            spec.filesystem
                .mounts
                .iter_mut()
                .filter(|mount| mount.target == spec.workspace)
                .for_each(|mount| mount.access = MountAccess::ReadOnly);
        }
        let (process, output) = spawn(
            spec,
            "observe",
            BTreeMap::from([(
                "SEATBELT_OUTSIDE".into(),
                outside.path().to_string_lossy().into_owned(),
            )]),
        );
        let exit = process.wait(Duration::from_secs(10)).unwrap();
        assert_eq!(
            exit.code,
            Some(0),
            "{exit:?}: {}",
            String::from_utf8_lossy(&output.lock().unwrap())
        );
        assert!(exit.error.is_none(), "{exit:?}");
        assert_eq!(
            exit.sandbox.unwrap().backend,
            SandboxBackendKind::MacosSeatbelt
        );
        let value = result(&output);
        assert_eq!(value["workspace_write"], !restricted);
        for allowed in ["workspace_read", "home_write", "temp_write"] {
            assert_eq!(value[allowed], true, "{allowed}: {value}");
        }
        for denied in [
            "outside_read",
            "data_volume_read",
            "outside_write",
            "symlink_read",
            "symlink_write",
            "git_write",
            "git_hardlink_write",
            "new_control",
            "new_legacy_control",
            "metadata_alias_write",
            "hidden_read",
            "hidden_write",
            "secret_env",
        ] {
            assert_eq!(value[denied], false, "{denied}: {value}");
        }
        assert_eq!(
            value["setsid"], true,
            "XNU does not mediate setsid: {value}"
        );
        assert_eq!(
            value["setpgid"], true,
            "XNU does not mediate setpgid: {value}"
        );
        assert!(!Path::new(value["temp"].as_str().unwrap()).exists());
        assert_eq!(
            fs::read_to_string(root.path().join("metadata/config")).unwrap(),
            "protected"
        );
        assert!(!outside.path().join("escaped").exists());
    }
}

#[test]
fn native_network_denies_ipv4_ipv6_dns_datagrams_and_unix_sockets() {
    let root = tempfile::tempdir().unwrap();
    let ipv4 = TcpListener::bind("127.0.0.1:0").unwrap();
    let ipv6 = TcpListener::bind("[::1]:0").unwrap();
    let dns = UdpSocket::bind("127.0.0.1:0").unwrap();
    let _unix = UnixListener::bind(root.path().join("fixture.sock")).unwrap();
    let addresses = BTreeMap::from([
        (
            "SEATBELT_IPV4".into(),
            ipv4.local_addr().unwrap().to_string(),
        ),
        (
            "SEATBELT_IPV6".into(),
            ipv6.local_addr().unwrap().to_string(),
        ),
        ("SEATBELT_DNS".into(), dns.local_addr().unwrap().to_string()),
    ]);
    for allowed in [false, true] {
        let mut spec = spec(root.path());
        if allowed {
            spec.network = NetworkPolicy::Unrestricted;
            spec.required_capabilities.network_denied = false;
        }
        let (process, output) = spawn(spec, "network", addresses.clone());
        let exit = process.wait(Duration::from_secs(10)).unwrap();
        assert_eq!(exit.code, Some(0), "{exit:?}");
        let value = result(&output);
        for key in ["ipv4", "ipv6", "dns_udp", "unix"] {
            assert_eq!(value[key], allowed, "{key}: {value}");
        }
    }
}

#[test]
fn native_group_members_are_cleaned_and_parallel_temp_is_separate() {
    for cancel in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut policy = spec(root.path());
        policy.process.allow_background = true;
        let (process, output) = spawn(
            policy,
            if cancel { "wait" } else { "descendants" },
            BTreeMap::new(),
        );
        wait_for(|| String::from_utf8_lossy(&output.lock().unwrap()).contains("SEATBELT_RESULT"));
        if cancel {
            process.stop();
        }
        let exit = process.wait(Duration::from_secs(10)).unwrap();
        assert!(exit.error.is_none(), "{exit:?}");
        let temp = result(&output)["temp"].as_str().unwrap().to_string();
        assert!(!Path::new(&temp).exists());
        let heartbeat = fs::read(root.path().join("heartbeat")).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            fs::read(root.path().join("heartbeat")).unwrap(),
            heartbeat,
            "descendant survived"
        );
    }
    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    let (first, first_output) = spawn(spec(first_root.path()), "wait", BTreeMap::new());
    let (second, second_output) = spawn(spec(second_root.path()), "wait", BTreeMap::new());
    wait_for(|| {
        String::from_utf8_lossy(&first_output.lock().unwrap()).contains("SEATBELT_RESULT")
            && String::from_utf8_lossy(&second_output.lock().unwrap()).contains("SEATBELT_RESULT")
    });
    let first_temp = result(&first_output)["temp"].as_str().unwrap().to_string();
    let second_temp = result(&second_output)["temp"].as_str().unwrap().to_string();
    assert_ne!(first_temp, second_temp);
    first.stop();
    assert!(first.wait(Duration::from_secs(5)).unwrap().error.is_none());
    assert!(!Path::new(&first_temp).exists());
    assert!(Path::new(&second_temp).exists());
    second.stop();
    assert!(second.wait(Duration::from_secs(5)).unwrap().error.is_none());
    assert!(!Path::new(&second_temp).exists());
}

#[test]
fn seatbelt_parent_loss_fixture() {
    let Ok(root) = std::env::var("SEATBELT_PARENT_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let (process, output) = spawn(spec(root), "wait", BTreeMap::new());
    wait_for(|| String::from_utf8_lossy(&output.lock().unwrap()).contains("SEATBELT_RESULT"));
    fs::write(
        root.join("owned-temp.json"),
        serde_json::to_vec(&result(&output)).unwrap(),
    )
    .unwrap();
    // The test's owner is deliberately killed without destructors. Retain the
    // Supervisor so this exercises its helper's parent-lifeline EOF path.
    std::hint::black_box(&process);
    loop {
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct FixtureOwner(std::process::Child);
impl Drop for FixtureOwner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn native_parent_loss_cleans_temp_and_group_members() {
    let root = tempfile::tempdir().unwrap();
    let mut owner = FixtureOwner(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "seatbelt_parent_loss_fixture", "--nocapture"])
            .env("SEATBELT_PARENT_ROOT", root.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for(|| root.path().join("owned-temp.json").exists());
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("owned-temp.json")).unwrap()).unwrap();
    let temp = PathBuf::from(value["temp"].as_str().unwrap());
    assert!(temp.exists());
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    wait_for(|| !temp.exists());
    std::thread::sleep(Duration::from_millis(100));
    let heartbeat = fs::read(root.path().join("heartbeat")).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        fs::read(root.path().join("heartbeat")).unwrap(),
        heartbeat,
        "descendant survived parent loss"
    );
}

#[test]
fn native_mandatory_process_ownership_fails_before_launch() {
    use davinci_agent::sandbox::{MacosSeatbeltBackend, SandboxBackend};
    let root = tempfile::tempdir().unwrap();
    let mut policy = spec(root.path());
    policy.required_capabilities.process_tree_isolation = true;
    policy.required_capabilities.deterministic_teardown = true;
    let request = davinci_protocol::ExecutionRequest {
        sandbox_id: policy.id.clone(),
        executable: "/bin/sh".into(),
        argv: vec!["-c".into(), "touch escaped".into()],
        cwd: policy.workspace.clone(),
        launch_id: None,
    };
    let failure = MacosSeatbeltBackend::new(None)
        .prepare(&policy, &request, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(
        failure.code,
        davinci_protocol::SandboxErrorCode::CapabilityUnavailable
    );
    assert!(!root.path().join("escaped").exists());
}

struct DetachedFixturePid(libc::pid_t);
impl Drop for DetachedFixturePid {
    fn drop(&mut self) {
        // A known fixture PID, never a production process enumeration policy.
        unsafe { libc::kill(self.0, libc::SIGKILL) };
    }
}

#[test]
fn native_detached_descendant_survives_group_teardown_so_auto_is_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let (process, output) = spawn(spec(root.path()), "detached-descendants", BTreeMap::new());
    wait_for(|| root.path().join("descendant.pid").exists());
    let pid = fs::read_to_string(root.path().join("descendant.pid"))
        .unwrap()
        .parse()
        .unwrap();
    let _fixture = DetachedFixturePid(pid);
    let exit = process.wait(Duration::from_secs(10)).unwrap();
    assert_eq!(exit.code, Some(0), "{exit:?}");
    let receipt = exit.sandbox.unwrap();
    assert_eq!(receipt.backend, SandboxBackendKind::MacosSeatbelt);
    let capabilities = receipt.capabilities;
    assert!(!capabilities.process_tree_isolation);
    assert!(!capabilities.deterministic_teardown);
    let before = fs::read(root.path().join("heartbeat")).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_ne!(
        fs::read(root.path().join("heartbeat")).unwrap(),
        before,
        "the ownership blocker changed; strengthen backend only with native evidence"
    );
    assert!(!Path::new(result(&output)["temp"].as_str().unwrap()).exists());
}
