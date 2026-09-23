use davinci_agent::jobs::supervisor::{
    self, ProcessConfig, ProcessEvent, Supervisor, SupervisorCommand,
};
use davinci_agent::{jobs::managed::ManagedOwner, JobBook};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    process::{Command, Stdio},
    sync::{atomic::AtomicBool, Arc, Barrier, Mutex},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct Record {
    pid: u32,
    port: u16,
    depth: usize,
}

fn wait_for(mut condition: impl FnMut() -> bool, description: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out: {description}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn records(directory: &Path) -> Vec<Record> {
    let mut records = Vec::new();
    wait_for(
        || {
            records = fs::read_dir(directory)
                .unwrap()
                .take(16)
                .filter_map(|entry| fs::read(entry.ok()?.path()).ok())
                .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
                .collect();
            records.len() == 3
        },
        "three fixture processes",
    );
    records
}

fn listening(process: &Record) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, process.port)),
        Duration::from_millis(100),
    )
    .is_ok()
}

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

fn config(directory: &Path, argv: Vec<String>) -> ProcessConfig {
    let mut environment = BTreeMap::new();
    for name in ["PATH", "SystemRoot", "TEMP", "TMP", "HOME"] {
        if let Ok(value) = std::env::var(name) {
            environment.insert(name.into(), value);
        }
    }
    ProcessConfig::new("node".into(), argv, directory.into(), environment)
}

fn fixture(directory: &Path) -> (Supervisor, Arc<Mutex<Vec<u8>>>) {
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = output.clone();
    let process = Supervisor::spawn(
        &host(),
        config(
            directory,
            vec![
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/support/process_fixture.cjs")
                    .to_string_lossy()
                    .into(),
                directory.to_string_lossy().into(),
                "supervised".into(),
                "2".into(),
            ],
        ),
        Arc::new(move |event| {
            if let ProcessEvent::Output(bytes) = event {
                captured.lock().unwrap().extend(bytes);
            }
        }),
    )
    .unwrap();
    (process, output)
}

#[test]
fn supervisor_fixture_entry() {
    if std::env::var_os("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").is_some() {
        supervisor::run();
    }
}

#[test]
fn supervisor_host_loss_fixture() {
    let Some(directory) = std::env::var_os("DAVINCI_SUPERVISOR_HOST_LOSS_FIXTURE") else {
        return;
    };
    let (_process, _output) = fixture(Path::new(&directory));
    records(Path::new(&directory));
    fs::write(Path::new(&directory).join("ready"), b"ready").unwrap();
    std::thread::sleep(Duration::from_secs(20));
}

#[test]
fn supervisor_stdin_and_descendant_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let (process, output) = fixture(directory.path());
    let children = records(directory.path());
    assert_eq!(
        process.child_pid(),
        children.iter().find(|p| p.depth == 2).unwrap().pid
    );
    assert!(children.iter().all(listening));
    assert_eq!(process.write(b"roundtrip\n").unwrap(), 10);
    wait_for(
        || String::from_utf8_lossy(&output.lock().unwrap()).contains("ECHO roundtrip"),
        "stdin delivery",
    );
    process.stop();
    let exit = process
        .wait(Duration::from_secs(5))
        .expect("stop must finish");
    assert!(exit.stopped);
    wait_for(
        || children.iter().all(|p| !listening(p)),
        "stop all descendants",
    );
    process.stop();
    assert_eq!(process.wait(Duration::ZERO), Some(exit));
    assert!(process.write(b"after stop").is_err());
}

#[test]
fn supervisor_survives_call_and_cleans_up_after_forced_host_loss() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "supervisor_host_loss_fixture"])
        .env("DAVINCI_SUPERVISOR_HOST_LOSS_FIXTURE", directory.path())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    wait_for(|| directory.path().join("ready").exists(), "owner ready");
    let children = records(directory.path());
    assert!(children.iter().all(listening));
    owner.kill().unwrap();
    owner.wait().unwrap();
    wait_for(
        || children.iter().all(|p| !listening(p)),
        "host loss descendant cleanup",
    );
}

#[test]
fn supervisor_nonzero_exit_and_inherited_pipes_do_not_hang() {
    let directory = tempfile::tempdir().unwrap();
    let mut command = config(
        directory.path(),
        vec![
            "--exact".into(),
            "supervisor_pipe_holder_fixture".into(),
            "--nocapture".into(),
        ],
    );
    command.executable = std::env::current_exe().unwrap();
    command.environment.insert(
        "DAVINCI_PIPE_HOLDER_FIXTURE".into(),
        directory.path().to_string_lossy().into(),
    );
    let process = Supervisor::spawn(&host(), command, Arc::new(|_| {})).unwrap();
    let exit = process
        .wait(Duration::from_secs(5))
        .expect("held descendant pipes must close");
    assert_eq!(exit.code, Some(7));
    assert!(!exit.stopped);
    assert!(!exit.output_complete);
    let record: Record = fs::read_dir(directory.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path()).ok())
        .find_map(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap();
    wait_for(|| !listening(&record), "held-pipe descendant is stopped");
}

#[test]
fn supervisor_pipe_holder_fixture() {
    let Some(directory) = std::env::var_os("DAVINCI_PIPE_HOLDER_FIXTURE") else {
        return;
    };
    // Rust's Child drop does not kill on parent exit. Readiness below proves
    // that a live descendant holds these explicitly inherited output handles.
    let mut child = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/process_fixture.cjs"))
        .args([directory.as_os_str(), "held".as_ref(), "0".as_ref()])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    wait_for(
        || {
            fs::read_dir(&directory)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| {
                    fs::read(entry.path())
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<Record>(&bytes).ok())
                        .is_some()
                })
        },
        "pipe holder ready",
    );
    assert!(child.try_wait().unwrap().is_none());
    std::process::exit(7);
}

#[test]
fn supervisor_stdin_backpressure_does_not_block_stop() {
    let directory = tempfile::tempdir().unwrap();
    let process = Supervisor::spawn(
        &host(),
        config(
            directory.path(),
            vec!["-e".into(), "setTimeout(()=>{},20000)".into()],
        ),
        Arc::new(|_| {}),
    )
    .unwrap();
    let bytes = vec![b'x'; 16 * 1024];
    let start = Instant::now();
    for _ in 0..16 {
        if process.write(&bytes).is_err() {
            break;
        }
    }
    assert!(start.elapsed() < Duration::from_secs(3));
    process.stop();
    assert!(process.wait(Duration::from_secs(5)).unwrap().stopped);
    assert!(process.write(&vec![0; 16 * 1024 + 1]).is_err());
}

fn managed(directory: &Path) -> (ManagedOwner, Arc<Mutex<JobBook>>) {
    let book = Arc::new(Mutex::new(JobBook::default()));
    (
        ManagedOwner::new(directory, book.clone(), host()).unwrap(),
        book,
    )
}

fn managed_config(directory: &Path) -> ProcessConfig {
    config(directory, vec!["-e".into(), "process.stdout.write('ready\\n');process.stdin.pipe(process.stdout);setTimeout(()=>{},20000)".into()])
}

#[test]
fn managed_concurrent_equivalent_starts_use_one_job_and_owned_lifetime() {
    let directory = tempfile::tempdir().unwrap();
    let (owner, book) = managed(directory.path());
    let config = managed_config(directory.path());
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let (owner, config, barrier) = (owner.clone(), config.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                owner
                    .start_authorized(config, 0, &AtomicBool::new(false), || Ok(()))
                    .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(results.iter().all(|result| result.0 == results[0].0));
    assert_eq!(results.iter().filter(|result| !result.1).count(), 1);
    assert_eq!(book.lock().unwrap().running(), 1);
    let id = results[0].0;
    let snapshot = owner.snapshot(id).unwrap();
    assert_eq!(snapshot.active_leases, 1);
    assert_eq!(snapshot.owner, owner.id());
    assert_eq!(book.lock().unwrap().get(id).unwrap().pid, snapshot.pid);
    owner.release(id).unwrap();
    assert!(
        owner
            .wait(id, Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .stopped
    );
}

#[test]
fn managed_released_lifetime_is_never_reused() {
    let directory = tempfile::tempdir().unwrap();
    let (owner, _) = managed(directory.path());
    let config = managed_config(directory.path());
    let (id, _) = owner
        .start_authorized(config.clone(), 0, &AtomicBool::new(false), || Ok(()))
        .unwrap();
    owner.release(id).unwrap();
    // Deliberately do not wait for the asynchronous stop monitor.
    let (replacement, reused) = owner
        .start_authorized(config, 0, &AtomicBool::new(false), || Ok(()))
        .unwrap();
    assert_ne!(replacement, id);
    assert!(!reused);
    owner.release(replacement).unwrap();
    assert!(
        owner
            .wait(replacement, Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .stopped
    );
}

#[test]
fn managed_child_leases_share_only_after_explicit_parent_grant() {
    let directory = tempfile::tempdir().unwrap();
    let (parent, book) = managed(directory.path());
    let first = parent.child_lease();
    let second = parent.child_lease();
    let stranger = ManagedOwner::new(directory.path(), book.clone(), host()).unwrap();
    let config = managed_config(directory.path());
    let (id, _) = first
        .start_authorized(config.clone(), 0, &AtomicBool::new(false), || Ok(()))
        .unwrap();
    assert!(second.snapshot(id).is_err());
    assert!(stranger.snapshot(id).is_err());
    assert!(stranger.release(id).is_err());
    let (same, reused) = second
        .start_authorized(config, 0, &AtomicBool::new(false), || Ok(()))
        .unwrap();
    assert_eq!(same, id);
    assert!(reused);
    drop(first);
    assert!(second
        .wait(id, Duration::from_millis(30))
        .unwrap()
        .is_none());
    assert_eq!(second.snapshot(id).unwrap().active_leases, 1);
    assert!(
        davinci_agent::jobs::output_tool(&book, &serde_json::json!({"jobId":id}), None).is_err()
    );
    assert!(davinci_agent::jobs::kill_tool(&book, &serde_json::json!({"jobId":id})).is_err());
    assert!(
        davinci_agent::jobs::stdin_tool(&book, &serde_json::json!({"jobId":id,"input":"x"}))
            .is_err()
    );
    second.release(id).unwrap();
    assert!(
        second
            .wait(id, Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .stopped
    );
}

#[test]
fn managed_output_uses_the_existing_bounded_job_buffer_and_byte_cursors() {
    let directory = tempfile::tempdir().unwrap();
    let (owner, book) = managed(directory.path());
    let (id, _) = owner.start_authorized(config(directory.path(), vec!["-e".into(),
        "process.stdout.write(Buffer.alloc(5*1024*1024,'x'),()=>process.stdout.write('\\nDONE\\n'))".into()]),
        0, &AtomicBool::new(false), || Ok(())).unwrap();
    assert_eq!(
        owner
            .wait(id, Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .code,
        Some(0)
    );
    let first = owner.output(id, Some(0), 1024).unwrap();
    assert!(first.truncated);
    assert_eq!(first.total_bytes, 5 * 1024 * 1024 + 6);
    assert!(first.text.len() <= 1024);
    let mut next = first.next_cursor;
    while next < first.total_bytes {
        let page = owner.output(id, Some(next), 64 * 1024).unwrap();
        assert!(!page.truncated);
        assert!(page.next_cursor > next);
        next = page.next_cursor;
    }
    assert_eq!(next, first.total_bytes);
    assert!(book.lock().unwrap().get(id).unwrap().output(None).len() <= 4 * 1024 * 1024 + 64);
}

#[test]
fn managed_start_rechecks_authority_and_isolates_revision_and_environment() {
    let directory = tempfile::tempdir().unwrap();
    let (owner, book) = managed(directory.path());
    let config = managed_config(directory.path());
    let denied = owner.start_authorized(config.clone(), 0, &AtomicBool::new(false), || {
        Err("revoked".into())
    });
    assert_eq!(denied.unwrap_err(), "revoked");
    assert!(book.lock().unwrap().is_empty());
    let (first, _) = owner
        .start_authorized(config.clone(), 0, &AtomicBool::new(false), || Ok(()))
        .unwrap();
    assert!(owner
        .start_authorized(config.clone(), 0, &AtomicBool::new(false), || Err(
            "revoked".into()
        ))
        .is_err());
    let (revision, reused) = owner
        .start_authorized(config.clone(), 1, &AtomicBool::new(false), || Ok(()))
        .unwrap();
    assert_ne!(revision, first);
    assert!(!reused);
    let mut different = config;
    different
        .environment
        .insert("FIXTURE_VARIANT".into(), "other".into());
    let (environment, reused) = owner
        .start_authorized(different, 1, &AtomicBool::new(false), || Ok(()))
        .unwrap();
    assert_ne!(environment, revision);
    assert!(!reused);
    for id in [first, revision, environment] {
        owner.release(id).unwrap();
        assert!(owner.wait(id, Duration::from_secs(5)).unwrap().is_some());
    }
}

#[test]
fn managed_interrupted_start_cleans_unpublished_tree_and_reservation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let directory = tempfile::tempdir().unwrap();
    let (owner, book) = managed(directory.path());
    let mut command = config(
        directory.path(),
        vec![
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/support/process_fixture.cjs")
                .to_string_lossy()
                .into(),
            directory.path().to_string_lossy().into(),
            "cancelled".into(),
            "2".into(),
        ],
    );
    let abort = AtomicBool::new(false);
    let calls = AtomicUsize::new(0);
    let result = owner.start_authorized(command.clone(), 0, &abort, || {
        if calls.fetch_add(1, Ordering::SeqCst) == 2 {
            records(directory.path());
            abort.store(true, Ordering::SeqCst);
        }
        Ok(())
    });
    assert!(result.unwrap_err().contains("cancelled"));
    assert!(book.lock().unwrap().is_empty());
    let children = records(directory.path());
    wait_for(
        || children.iter().all(|p| !listening(p)),
        "unpublished tree cleanup",
    );

    // A panic in trusted host validation also releases its reservation.
    command.argv = vec!["-e".into(), "setTimeout(()=>{},20000)".into()];
    let calls = AtomicUsize::new(0);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        owner.start_authorized(command.clone(), 0, &AtomicBool::new(false), || {
            if calls.fetch_add(1, Ordering::SeqCst) == 1 {
                panic!("fixture validation panic");
            }
            Ok(())
        })
    }))
    .is_err());
    let (id, reused) = owner
        .start_authorized(command, 0, &AtomicBool::new(false), || Ok(()))
        .unwrap();
    assert!(!reused);
    owner.release(id).unwrap();
    assert!(owner.wait(id, Duration::from_secs(5)).unwrap().is_some());
}

#[test]
fn supervised_commands_preserve_literal_argv_and_only_explicit_environment() {
    let directory = tempfile::tempdir().unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = captured.clone();
    let literal = vec![
        "two words",
        "$(no-command)",
        "quote'\"value",
        "semi;colon",
        "line\nbreak",
    ];
    let mut argv = vec!["-e".into(), "process.stdout.write(JSON.stringify({argv:process.argv.slice(1),allowed:process.env.FIXTURE_ALLOWED,internal:process.env.DAVINCI_INTERNAL_PROCESS_SUPERVISOR??null}))".into(), "--".into()];
    argv.extend(literal.iter().map(|value| (*value).to_string()));
    let mut command = config(directory.path(), argv);
    command
        .environment
        .insert("FIXTURE_ALLOWED".into(), "explicit".into());
    let process = Supervisor::spawn(
        &host(),
        command,
        Arc::new(move |event| {
            if let ProcessEvent::Output(bytes) = event {
                output.lock().unwrap().extend(bytes);
            }
        }),
    )
    .unwrap();
    let exit = process.wait(Duration::from_secs(5)).unwrap();
    assert_eq!(exit.code, Some(0));
    assert!(exit.output_complete);
    let result: serde_json::Value = serde_json::from_slice(&captured.lock().unwrap()).unwrap();
    assert_eq!(result["argv"], serde_json::json!(literal));
    assert_eq!(result["allowed"], "explicit");
    assert!(result["internal"].is_null());
}

#[test]
fn supervisor_capture_preserves_streams_and_legacy_merge() {
    let directory = tempfile::tempdir().unwrap();
    for split in [true, false] {
        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let output = stdout.clone();
        let errors = stderr.clone();
        let command = config(
            directory.path(),
            vec![
                "-e".into(),
                "process.stdout.write('out');process.stderr.write('err')".into(),
            ],
        );
        let event = Arc::new(move |event| {
            if let ProcessEvent::Output(bytes) = event {
                output.lock().unwrap().extend(bytes);
            }
        });
        let process = if split {
            Supervisor::spawn_with_stderr(
                &host(),
                command,
                event,
                Arc::new(move |bytes| errors.lock().unwrap().extend(bytes)),
            )
        } else {
            Supervisor::spawn(&host(), command, event)
        }
        .unwrap();
        let exit = process.wait(Duration::from_secs(5)).unwrap();
        assert_eq!(exit.code, Some(0));
        assert!(exit.output_complete);
        if split {
            assert_eq!(*stdout.lock().unwrap(), b"out");
            assert_eq!(*stderr.lock().unwrap(), b"err");
        } else {
            let output = stdout.lock().unwrap();
            assert!(output.as_slice() == b"outerr" || output.as_slice() == b"errout");
            assert!(stderr.lock().unwrap().is_empty());
        }
    }
}

#[test]
fn supervisor_closes_stdin_after_acknowledged_input() {
    let directory = tempfile::tempdir().unwrap();
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = output.clone();
    let process = Supervisor::spawn(&host(), config(directory.path(), vec!["-e".into(),
        "let text='';process.stdin.on('data',b=>text+=b);process.stdin.on('end',()=>{process.stdout.write(text);setTimeout(()=>process.exit(0),200)})".into()]),
        Arc::new(move |event| { if let ProcessEvent::Output(bytes) = event { captured.lock().unwrap().extend(bytes); } })).unwrap();
    assert_eq!(process.write(b"first").unwrap(), 5);
    assert_eq!(process.write(b"second").unwrap(), 6);
    process.close_stdin().unwrap();
    assert!(process.write(b"too late").is_err());
    let exit = process.wait(Duration::from_secs(5)).unwrap();
    assert_eq!(exit.code, Some(0));
    assert!(exit.output_complete);
    assert_eq!(*output.lock().unwrap(), b"firstsecond");
}

#[test]
fn stopping_one_owned_lifetime_preserves_an_unrelated_process_tree() {
    let first_directory = tempfile::tempdir().unwrap();
    let second_directory = tempfile::tempdir().unwrap();
    let (first, _) = fixture(first_directory.path());
    let (second, _) = fixture(second_directory.path());
    let first_children = records(first_directory.path());
    let second_children = records(second_directory.path());
    first.stop();
    assert!(first.wait(Duration::from_secs(5)).unwrap().stopped);
    wait_for(
        || first_children.iter().all(|p| !listening(p)),
        "first tree cleanup",
    );
    assert!(second_children.iter().all(listening));
    second.stop();
    assert!(second.wait(Duration::from_secs(5)).unwrap().stopped);
    wait_for(
        || second_children.iter().all(|p| !listening(p)),
        "second tree cleanup",
    );
}
