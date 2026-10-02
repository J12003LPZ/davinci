//! Actual packaged supervisor, owned Node app and persistent state. Offline.
use davinci_agent::{
    jobs::supervisor::SupervisorCommand, process_manager::ProcessManager, JobBook, PermissionMode,
    PermissionPolicy, PermissionState,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn manager(root: &Path) -> ProcessManager {
    ProcessManager::new(
        root,
        Arc::new(Mutex::new(JobBook::default())),
        Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        ))),
        SupervisorCommand {
            executable: env!("CARGO_BIN_EXE_davinci").into(),
            argv: vec!["--internal-process-supervisor".into()],
        },
    )
    .unwrap()
}

fn call(manager: &ProcessManager, root: &Path, name: &str, args: Value) -> Value {
    let result = manager.execute(root, name, &args, None, None).unwrap();
    assert!(!result.is_error, "{}", result.content);
    result.details.unwrap()
}

fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "owned app readiness deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn start(manager: &ProcessManager, root: &Path, mode: &str, instance: &str) -> u64 {
    let result = call(
        manager,
        root,
        "process_start",
        json!({"executable":"node", "argv":[Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/harness-product.cjs"), root, mode, instance]}),
    );
    result["process"]["id"].as_u64().unwrap()
}

fn address(root: &Path, instance: &str) -> SocketAddr {
    let file = root.join(format!("{instance}.json"));
    wait_for(|| {
        fs::read(&file)
            .ok()
            .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
            .is_some()
    });
    let value: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    assert_eq!(value["instance"], instance);
    format!("127.0.0.1:{}", value["port"].as_u64().unwrap())
        .parse()
        .unwrap()
}

fn request(
    address: SocketAddr,
    method: &str,
    route: &str,
    authorized: bool,
) -> std::io::Result<String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write!(stream, "{method} {route} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: 0\r\n{}\r\n", if authorized { "Authorization: Bearer synthetic-user\r\n" } else { "" })?;
    let mut result = String::new();
    stream.take(16384).read_to_string(&mut result)?;
    Ok(result)
}

#[test]
fn harness_journey_requires_real_persistence_and_auth_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path());
    start(&manager, root.path(), "normal", "first");
    let first = address(root.path(), "first");
    assert!(request(first, "POST", "/value", false)
        .unwrap()
        .starts_with("HTTP/1.1 401"));
    assert!(request(first, "GET", "/value", true)
        .unwrap()
        .contains("empty"));
    assert!(request(first, "POST", "/value", true)
        .unwrap()
        .starts_with("HTTP/1.1 201"));
    assert!(request(first, "GET", "/value", true)
        .unwrap()
        .contains("saved"));
    manager.shutdown();
    wait_for(|| TcpStream::connect_timeout(&first, Duration::from_millis(50)).is_err());
    let restarted = self::manager(root.path());
    start(&restarted, root.path(), "normal", "second");
    let second = address(root.path(), "second");
    assert!(request(second, "GET", "/value", true)
        .unwrap()
        .contains("saved"));
    assert!(request(second, "GET", "/value", false)
        .unwrap()
        .starts_with("HTTP/1.1 401"));
    restarted.shutdown();
}

#[test]
fn harness_operational_failures_have_observable_outcomes() {
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path());
    let failed = start(&manager, root.path(), "startup-failure", "failed");
    wait_for(|| {
        call(
            &manager,
            root.path(),
            "process_status",
            json!({"id":failed}),
        )["state"]
            == "exited"
    });
    assert_eq!(
        call(
            &manager,
            root.path(),
            "process_status",
            json!({"id":failed})
        )["exit_code"],
        12
    );
    assert!(call(
        &manager,
        root.path(),
        "process_output",
        json!({"id":failed})
    )["text"]
        .as_str()
        .unwrap()
        .contains("STARTUP_FAILED"));
    let running = start(&manager, root.path(), "normal", "running");
    let address = address(root.path(), "running");
    assert!(request(address, "POST", "/value", true)
        .unwrap()
        .contains("201"));
    assert!(request(address, "POST", "/migrate-fail", true)
        .unwrap()
        .starts_with("HTTP/1.1 500"));
    assert!(request(address, "GET", "/value", true)
        .unwrap()
        .contains("saved"));
    assert!(!root.path().join("data.json.staged").exists());
    fs::write(root.path().join("outage"), "fixture").unwrap();
    assert!(request(address, "GET", "/health", false)
        .unwrap()
        .starts_with("HTTP/1.1 503"));
    assert!(request(address, "GET", "/value", true)
        .unwrap()
        .starts_with("HTTP/1.1 503"));
    fs::remove_file(root.path().join("outage")).unwrap();
    assert!(request(address, "GET", "/health", false)
        .unwrap()
        .starts_with("HTTP/1.1 200"));
    assert!(request(address, "GET", "/hang", true).is_err());
    let logs = call(
        &manager,
        root.path(),
        "process_output",
        json!({"id":running}),
    )["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(logs.contains("MIGRATION_ROLLED_BACK") && logs.contains("DATABASE_UNAVAILABLE"));
    assert!(!logs.contains("synthetic-user"));
    manager.shutdown();
    wait_for(|| TcpStream::connect_timeout(&address, Duration::from_millis(50)).is_err());
}
