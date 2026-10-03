//! Frozen measurements of the pre-P2 JobBook; no proposed manager API is used.
use davinci_agent::{jobs, runtime::CancellationToken, JobBook};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize, Serialize)]
struct FixtureProcess {
    pid: u32,
    port: u16,
    label: String,
    depth: usize,
    instance: String,
}

fn start(book: &Arc<Mutex<JobBook>>, directory: &Path, label: &str) -> u32 {
    let mut command = Command::new("node");
    command
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/process_fixture.cjs"))
        .args([directory.as_os_str(), label.as_ref(), "2".as_ref()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    book.lock()
        .unwrap()
        .register("node process_fixture.cjs", command.spawn().unwrap())
}

fn wait_for(mut condition: impl FnMut() -> bool, description: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out: {description}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn records(directory: &Path, label: &str, previous: &[FixtureProcess]) -> Vec<FixtureProcess> {
    let mut records = Vec::new();
    wait_for(
        || {
            records = fs::read_dir(directory)
                .unwrap()
                .take(16)
                .filter_map(|entry| fs::read(entry.ok()?.path()).ok())
                .filter_map(|bytes| serde_json::from_slice::<FixtureProcess>(&bytes).ok())
                .filter(|r| r.label == label && !previous.iter().any(|p| p.instance == r.instance))
                .collect();
            records.len() == 3
        },
        "fixture server ready",
    );
    records
}

fn listening(process: &FixtureProcess) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, process.port)),
        Duration::from_millis(100),
    )
    .is_ok()
}

#[test]
fn process_baseline_abrupt_host_helper() {
    let Some(directory) = std::env::var_os("DAVINCI_PROCESS_BASELINE_HELPER") else {
        return;
    };
    let book = Arc::new(Mutex::new(JobBook::default()));
    start(&book, Path::new(&directory), "abrupt");
    records(Path::new(&directory), "abrupt", &[]);
    // Deliberately skip Drop, as a hard host exit does.
    std::process::exit(0);
}

#[test]
#[ignore = "explicit local Node process lifecycle evaluation"]
fn process_manager_existing_jobbook_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let book = Arc::new(Mutex::new(JobBook::default()));
    let cancellation = CancellationToken::new();
    cancellation.attach_job_book(book.clone());
    let started = Instant::now();
    let first = start(&book, directory.path(), "equivalent");
    let mut processes = records(directory.path(), "equivalent", &[]);
    let first_ready_ms = started.elapsed().as_secs_f64() * 1000.0;
    let second_started = Instant::now();
    let second = start(&book, directory.path(), "equivalent");
    processes.extend(records(directory.path(), "equivalent", &processes));
    let second_ready_ms = second_started.elapsed().as_secs_f64() * 1000.0;
    assert_ne!(first, second);
    assert_eq!(book.lock().unwrap().running(), 2);
    assert!(processes.iter().all(listening));
    book.lock()
        .unwrap()
        .write_stdin(first, "roundtrip\n")
        .unwrap();
    wait_for(
        || {
            book.lock()
                .unwrap()
                .get(first)
                .unwrap()
                .output(None)
                .contains("ECHO roundtrip")
        },
        "stdin round trip",
    );
    book.lock().unwrap().write_stdin(first, "burst\n").unwrap();
    wait_for(
        || book.lock().unwrap().get(first).unwrap().output(Some(1)) == "BURST_DONE",
        "bounded output burst",
    );
    let output = book.lock().unwrap().get(first).unwrap().output(None);
    assert!(output.starts_with("[earlier output dropped]"));
    assert!(output.len() <= 4 * 1024 * 1024 + 64);
    let stop_started = Instant::now();
    cancellation.cancel();
    wait_for(
        || processes.iter().all(|p| !listening(p)),
        "cancel all descendants",
    );
    let stop_ms = stop_started.elapsed().as_secs_f64() * 1000.0;

    let helper = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "process_baseline_abrupt_host_helper"])
        .env("DAVINCI_PROCESS_BASELINE_HELPER", directory.path())
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(helper.success());
    let abrupt = records(directory.path(), "abrupt", &[]);
    std::thread::sleep(Duration::from_millis(200));
    let surviving_after_host_exit = abrupt.iter().filter(|p| listening(p)).count();
    // Only this isolated fixture tree is stopped; each process also has a 20s failsafe.
    let root = abrupt.iter().find(|p| p.depth == 2).unwrap();
    if listening(root) {
        jobs::kill_tree(root.pid);
    }
    wait_for(
        || abrupt.iter().all(|p| !listening(p)),
        "abrupt fixture cleanup",
    );
    let artifact = json!({
        "schema": 1, "platform": std::env::consts::OS,
        "measurement": "pre-P2 JobBook; real loopback Node server with child and grandchild",
        "equivalent_requests": 2, "root_startups": 2, "job_ids": [first, second],
        "fixture_processes": processes.len(), "first_ready_ms": first_ready_ms,
        "second_ready_ms": second_ready_ms, "stdin_roundtrip": true,
        "output_emitted_bytes": 5 * 1024 * 1024, "retained_output_bytes": output.len(),
        "output_dropped_marker": true, "cancellation_cleanup_ms": stop_ms,
        "descendants_after_cancellation": 0,
        "survivors_200ms_after_abrupt_host_exit": surviving_after_host_exit,
        "fixture_processes_after_explicit_cleanup": 0,
        "notes": ["Startup timing includes both descendants becoming ready.",
            "The 200ms host-exit observation is a bounded sample, not an indefinite-lifetime claim."]
    });
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
    if let Some(path) = std::env::var_os("DAVINCI_PROCESS_BASELINE_ARTIFACT") {
        fs::write(path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    }
}
