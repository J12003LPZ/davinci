//! Native packaged supervisor and repeatable offline server-reuse evaluation.
use davinci_agent::{
    jobs::supervisor::SupervisorCommand, process_manager::ProcessManager, JobBook, PermissionMode,
    PermissionPolicy, PermissionState,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn manager(cwd: &Path) -> ProcessManager {
    ProcessManager::new(
        cwd,
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

fn call(manager: &ProcessManager, cwd: &Path, name: &str, args: Value) -> Value {
    manager
        .execute(cwd, name, &args, None, None)
        .unwrap()
        .details
        .unwrap()
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "process fixture deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn packaged_process_supervisor_runs_without_normal_cli_initialization() {
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path());
    assert_eq!(
        call(&manager, root.path(), "process_list", json!({}))["processes"],
        json!([])
    );
    let start = call(
        &manager,
        root.path(),
        "process_start",
        json!({"executable":"node", "argv":["-e","console.log('PACKAGED_HELPER_OK');process.exit(7)"]}),
    );
    let id = start["process"]["id"].as_u64().unwrap();
    wait_for(|| {
        call(&manager, root.path(), "process_status", json!({"id":id}))["state"] == "exited"
    });
    assert_eq!(
        call(&manager, root.path(), "process_status", json!({"id":id}))["exit_code"],
        7
    );
    assert!(
        call(&manager, root.path(), "process_output", json!({"id":id}))["text"]
            .as_str()
            .unwrap()
            .contains("PACKAGED_HELPER_OK")
    );
    assert!(!root.path().join(".davinci").exists());
}

#[derive(Deserialize)]
struct FixtureProcess {
    port: u16,
    label: String,
}

fn records(directory: &Path, label: &str) -> Vec<FixtureProcess> {
    let mut records = vec![];
    wait_for(|| {
        records = fs::read_dir(directory)
            .unwrap()
            .take(16)
            .filter_map(|entry| fs::read(entry.ok()?.path()).ok())
            .filter_map(|bytes| serde_json::from_slice::<FixtureProcess>(&bytes).ok())
            .filter(|record| record.label == label)
            .collect();
        records.len() == 3
    });
    records
}

fn listening(process: &FixtureProcess) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, process.port)),
        Duration::from_millis(100),
    )
    .is_ok()
}

fn start(manager: &ProcessManager, directory: &Path, label: &str) -> Value {
    call(
        manager,
        directory,
        "process_start",
        json!({
            "executable":"node", "argv":[
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../davinci-agent/tests/support/process_fixture.cjs"),
                directory, label, "2"
            ]
        }),
    )
}

#[test]
fn managed_abrupt_host_helper() {
    let Some(directory) = std::env::var_os("DAVINCI_MANAGED_HOST_EXIT_FIXTURE") else {
        return;
    };
    let directory = Path::new(&directory);
    let manager = manager(directory);
    start(&manager, directory, "abrupt");
    records(directory, "abrupt");
    std::process::exit(0); // Deliberately skip host Drop and session shutdown.
}

#[test]
#[ignore = "explicit local Node process lifecycle evaluation"]
fn managed_process_reuse_and_cleanup_evaluation() {
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path());
    let began = Instant::now();
    let first = start(&manager, root.path(), "equivalent");
    let processes = records(root.path(), "equivalent");
    let first_ready_ms = began.elapsed().as_secs_f64() * 1000.0;
    let began = Instant::now();
    let second = start(&manager, root.path(), "equivalent");
    let second_ready_ms = began.elapsed().as_secs_f64() * 1000.0;
    let id = first["process"]["id"].as_u64().unwrap();
    assert_eq!(second["process"]["id"], id);
    assert_eq!(second["reused"], true);
    assert!(processes.iter().all(listening));
    call(
        &manager,
        root.path(),
        "process_write",
        json!({"id":id,"text":"roundtrip\n"}),
    );
    wait_for(|| {
        call(&manager, root.path(), "process_output", json!({"id":id}))["text"]
            .as_str()
            .unwrap()
            .contains("ECHO roundtrip")
    });
    call(
        &manager,
        root.path(),
        "process_write",
        json!({"id":id,"text":"burst\n"}),
    );
    wait_for(|| {
        call(&manager, root.path(), "process_output", json!({"id":id}))["text"]
            .as_str()
            .unwrap()
            .contains("BURST_DONE")
    });
    let output = call(
        &manager,
        root.path(),
        "process_output",
        json!({"id":id,"cursor":0}),
    );
    assert_eq!(output["truncated"], true);
    assert!(output["text"].as_str().unwrap().len() <= 65536);
    let metrics = call(&manager, root.path(), "process_list", json!({}))["metrics"].clone();
    assert_eq!(metrics["startups"], 1);
    assert_eq!(metrics["reuses"], 1);
    let began = Instant::now();
    manager.shutdown();
    wait_for(|| processes.iter().all(|process| !listening(process)));
    let shutdown_ms = began.elapsed().as_secs_f64() * 1000.0;
    let helper = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "managed_abrupt_host_helper"])
        .env("DAVINCI_MANAGED_HOST_EXIT_FIXTURE", root.path())
        .output()
        .unwrap();
    assert!(
        helper.status.success(),
        "{}",
        String::from_utf8_lossy(&helper.stderr)
    );
    let abrupt = records(root.path(), "abrupt");
    let began = Instant::now();
    std::thread::sleep(Duration::from_millis(200));
    let survivors = abrupt.iter().filter(|process| listening(process)).count();
    wait_for(|| abrupt.iter().all(|process| !listening(process)));
    let artifact = json!({
        "schema":1, "platform":std::env::consts::OS,
        "measurement":"P2 packaged supervisor; real loopback Node server with child and grandchild",
        "equivalent_requests":2,"root_startups":1,"job_ids":[id,id],"fixture_processes":processes.len(),
        "first_ready_ms":first_ready_ms,"second_ready_ms":second_ready_ms,
        "stdin_roundtrip":true,"output_emitted_bytes":5*1024*1024,
        "retained_page_bytes":output["text"].as_str().unwrap().len(),"earliest_cursor":output["earliest_cursor"],
        "output_truncated":true,"shutdown_cleanup_ms":shutdown_ms,"descendants_after_shutdown":0,
        "survivors_200ms_after_abrupt_host_exit":survivors,"descendants_after_host_loss":0,
        "host_loss_observation_ms":began.elapsed().as_secs_f64()*1000.0,
        "notes":["The cold start includes descendants becoming ready; the warm request reuses the same live server.","Output is a bounded page with byte cursors, not an unbounded saved log.","Timing is machine-specific; no provider usage is measured."]
    });
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
    if let Some(path) = std::env::var_os("DAVINCI_PROCESS_EVAL_ARTIFACT") {
        fs::write(path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    }
}
