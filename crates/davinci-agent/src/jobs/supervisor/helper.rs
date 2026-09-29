use super::{
    platform::Ownership,
    wire::{self, Event, Request, MAX_INPUT, POLL},
    ProcessConfig,
};
use crate::sandbox::SandboxBroker;
#[cfg(unix)]
use davinci_protocol::ResourcePolicy;
use davinci_protocol::{ExecutionRequest, SandboxLifecycle, SandboxReceipt};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

type Message = (Event, Option<mpsc::SyncSender<()>>);

#[derive(Debug)]
struct OutputBudget {
    remaining: Mutex<Option<u64>>,
}

impl OutputBudget {
    fn new(limit: Option<u64>) -> Self {
        Self {
            remaining: Mutex::new(limit),
        }
    }

    fn take(&self, requested: usize) -> usize {
        let mut remaining = self
            .remaining
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(left) = *remaining else {
            return requested;
        };
        let allowed = requested.min(left.min(usize::MAX as u64) as usize);
        *remaining = Some(left.saturating_sub(allowed as u64));
        allowed
    }
}

pub(super) fn run() -> ! {
    let ownership = match Ownership::enter() {
        Ok(value) => value,
        Err(_) => std::process::exit(1),
    };
    let _ = run_owned();
    ownership.terminate()
}

fn run_owned() -> std::io::Result<()> {
    let mut output = std::io::stdout();
    output.write_all(wire::MAGIC)?;
    wire::write(
        &mut output,
        &Event::Hello {
            pid: std::process::id(),
        },
    )?;
    let mut input = std::io::stdin();
    // Only the parent holds this helper's stdin pipe, so the channel itself
    // authenticates the frames.
    let Request::Configure { identity, config } = wire::read(&mut input)? else {
        return Ok(());
    };
    if identity.operation != config.operation {
        wire::write(
            &mut output,
            &Event::LaunchFailed {
                identity,
                message: "process operation binding does not match its lifetime identity".into(),
            },
        )?;
        return Ok(());
    }
    let spawned = match spawn(config, &identity) {
        Ok(spawned) => spawned,
        Err(error) => {
            wire::write(
                &mut output,
                &Event::LaunchFailed {
                    identity,
                    message: format!("command launch failed before child start: {error}"),
                },
            )?;
            return Ok(());
        }
    };
    let sandbox_receipt = spawned.sandbox.clone();
    let output_budget = Arc::new(OutputBudget::new(spawned.max_output_bytes));
    let lifetime_deadline = spawned
        .max_lifetime_ms
        .map(|milliseconds| Instant::now() + Duration::from_millis(milliseconds));
    let mut child = spawned.child;
    wire::write(
        &mut output,
        &Event::Started {
            identity: identity.clone(),
            pid: child.id(),
            sandbox: sandbox_receipt,
        },
    )?;

    let stopped = Arc::new(AtomicBool::new(false));
    let (events, event_rx) = mpsc::sync_channel::<Message>(64);
    let writer_stop = stopped.clone();
    thread::spawn(move || {
        let mut output = std::io::stdout();
        while let Ok((event, ack)) = event_rx.recv() {
            if wire::write(&mut output, &event).is_err() {
                writer_stop.store(true, Ordering::SeqCst);
                break;
            }
            if let Some(ack) = ack {
                let _ = ack.try_send(());
            }
        }
    });
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .enumerate()
    .filter_map(|(index, pipe)| pipe.map(|pipe| (index == 1, pipe)))
    .map(|(stderr, pipe)| {
        let events = events.clone();
        let identity = identity.clone();
        let budget = Arc::clone(&output_budget);
        thread::spawn(move || forward_output(pipe, &events, &identity, stderr, &budget))
    })
    .collect();
    let (writes, write_rx) = mpsc::sync_channel::<(u64, Option<Vec<u8>>)>(1);
    let write_events = events.clone();
    let write_identity = identity.clone();
    let mut stdin = child.stdin.take();
    thread::spawn(move || {
        while let Ok((id, bytes)) = write_rx.recv() {
            let result = match bytes {
                Some(bytes) => stdin
                    .as_mut()
                    .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::BrokenPipe))
                    .and_then(|stdin| stdin.write_all(&bytes).and_then(|_| stdin.flush()))
                    .map(|_| bytes.len()),
                None => {
                    drop(stdin.take());
                    Ok(0)
                }
            };
            let failed = result.is_err();
            let count = result.unwrap_or(0);
            if write_events
                .send((
                    Event::Written {
                        identity: write_identity.clone(),
                        id,
                        count,
                        failed,
                    },
                    None,
                ))
                .is_err()
            {
                break;
            }
        }
    });
    let input_stop = stopped.clone();
    let input_events = events.clone();
    let input_identity = identity.clone();
    thread::spawn(move || {
        while let Ok(request) = wire::read(&mut input) {
            let (request_identity, id, bytes) = match request {
                Request::Write {
                    identity,
                    id,
                    bytes,
                } if bytes.len() <= MAX_INPUT => (identity, id, Some(bytes)),
                Request::CloseStdin { identity, id } => (identity, id, None),
                _ => break,
            };
            if request_identity != input_identity {
                break;
            }
            if writes.try_send((id, bytes)).is_err()
                && input_events
                    .send((
                        Event::Written {
                            identity: input_identity.clone(),
                            id,
                            count: 0,
                            failed: true,
                        },
                        None,
                    ))
                    .is_err()
            {
                break;
            }
        }
        // This pipe is the parent lifeline. EOF also covers SIGKILL/TerminateProcess.
        input_stop.store(true, Ordering::SeqCst);
    });

    while !stopped.load(Ordering::SeqCst) {
        if lifetime_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            let _ = events.try_send((
                Event::Failed {
                    identity: identity.clone(),
                },
                None,
            ));
            stopped.store(true, Ordering::SeqCst);
            break;
        }
        if let Some(status) = child.try_wait()? {
            // A grandchild may retain the pipes. Bound tail draining before
            // terminating the group instead of waiting for those pipes forever.
            let deadline = Instant::now() + Duration::from_millis(100);
            while readers.iter().any(|reader| !reader.is_finished()) && Instant::now() < deadline {
                thread::sleep(POLL);
            }
            flush_exit(&events, &identity, status.code(), readers_complete(readers));
            return Ok(());
        }
        thread::sleep(POLL);
    }
    Ok(())
}

fn forward_output(
    mut pipe: impl Read,
    events: &mpsc::SyncSender<Message>,
    identity: &super::ProcessIdentity,
    stderr: bool,
    budget: &OutputBudget,
) -> std::io::Result<()> {
    let mut bytes = [0; 8192];
    let mut truncated = false;
    loop {
        let count = match pipe.read(&mut bytes) {
            Ok(0) if truncated => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "sandbox output limit exceeded; output truncated",
                ))
            }
            Ok(0) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
            Ok(n) => n,
        };
        let allowed = budget.take(count);
        if allowed > 0
            && events
                .send((
                    Event::Output {
                        identity: identity.clone(),
                        bytes: bytes[..allowed].to_vec(),
                        stderr,
                    },
                    None,
                ))
                .is_err()
        {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        if allowed != count {
            // Past the ceiling: keep draining so the command is not blocked
            // on a full pipe, forward nothing more, and report the capture as
            // incomplete. Truncation is not a command failure.
            truncated = true;
        }
    }
}

fn readers_complete(readers: Vec<thread::JoinHandle<std::io::Result<()>>>) -> bool {
    let mut complete = true;
    for reader in readers {
        // Never join a descendant-held pipe that exceeded the drain deadline.
        complete &= reader.is_finished() && matches!(reader.join(), Ok(Ok(())));
    }
    complete
}

struct Spawned {
    child: std::process::Child,
    sandbox: Option<SandboxReceipt>,
    max_output_bytes: Option<u64>,
    max_lifetime_ms: Option<u64>,
}

fn spawn(config: ProcessConfig, identity: &super::ProcessIdentity) -> Result<Spawned, String> {
    // A service (an MCP server) lives as long as its session and frames its
    // own output; a cumulative output budget or lifetime would kill it midway.
    let max_output_bytes = config
        .sandbox
        .as_ref()
        .filter(|_| !config.service)
        .and_then(|spec| spec.resources.max_output_bytes);
    let max_lifetime_ms = config
        .sandbox
        .as_ref()
        .filter(|_| !config.service)
        .and_then(|spec| {
            if config.background {
                spec.process.max_background_lifetime_ms
            } else {
                spec.resources.timeout_ms
            }
        });
    #[cfg(unix)]
    let mut resource_policy = config.sandbox.as_ref().map(|spec| spec.resources.clone());
    let (executable, argv, cwd, environment, sandbox) = if let Some(spec) = config.sandbox.as_ref()
    {
        let request = ExecutionRequest {
            sandbox_id: spec.id.clone(),
            executable: config
                .executable
                .to_str()
                .ok_or("sandbox executable path is not UTF-8")?
                .to_string(),
            argv: config.argv.clone(),
            cwd: config
                .cwd
                .to_str()
                .ok_or("sandbox cwd is not UTF-8")?
                .to_string(),
            launch_id: Some(identity.lifetime.simple().to_string()),
        };
        let prepared = SandboxBroker
            .prepare(spec, &request, &config.environment)
            .map_err(|error| error.to_string())?;
        #[cfg(unix)]
        if prepared.backend == davinci_protocol::SandboxBackendKind::Container {
            // The container runtime enforces the limits inside the
            // container; rlimits here would only constrain its CLI.
            resource_policy = None;
        }
        let receipt = SandboxReceipt {
            sandbox_id: prepared.sandbox_id.clone(),
            spec_digest: prepared.spec_digest.clone(),
            backend: prepared.backend,
            capabilities: prepared.capabilities,
            lifecycle: SandboxLifecycle::Running,
        };
        (
            prepared.executable,
            prepared.argv,
            prepared.cwd,
            prepared.environment,
            Some(receipt),
        )
    } else {
        (
            config.executable,
            config.argv,
            config.cwd,
            config.environment,
            None,
        )
    };

    #[cfg(windows)]
    let cwd = {
        // Node and other runtimes cannot resolve relative files from a verbatim
        // current directory. Preserve the authorized location: simplify only
        // when both spellings resolve to the same canonical directory.
        let ordinary = crate::permission::strip_verbatim_prefix(&cwd);
        if ordinary.canonicalize().map_err(|error| error.to_string())?
            == cwd.canonicalize().map_err(|error| error.to_string())?
        {
            ordinary
        } else {
            cwd
        }
    };
    let mut command = Command::new(executable);
    command
        .args(argv)
        .current_dir(cwd)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    if let Some(resources) = resource_policy {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(move || apply_unix_resource_limits(&resources));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let child = command.spawn().map_err(|error| error.to_string())?;
    Ok(Spawned {
        child,
        sandbox,
        max_output_bytes,
        max_lifetime_ms,
    })
}

#[cfg(unix)]
fn apply_unix_resource_limits(policy: &ResourcePolicy) -> std::io::Result<()> {
    macro_rules! set_limit {
        ($resource:expr, $value:expr, $label:literal) => {{
            let value = $value as libc::rlim_t;
            let limit = libc::rlimit {
                rlim_cur: value,
                rlim_max: value,
            };
            if unsafe { libc::setrlimit($resource, &limit) } != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    concat!("failed to apply ", $label, " resource limit"),
                ));
            }
        }};
    }

    // RLIMIT_DATA, not RLIMIT_AS: Node, the JVM and Go reserve far more
    // address space than they use and fail outright under an address-space
    // cap. These limits apply per process, not to the whole tree.
    if let Some(bytes) = policy.max_memory_bytes {
        set_limit!(libc::RLIMIT_DATA, bytes, "memory");
    }
    if let Some(milliseconds) = policy.cpu_time_ms {
        let seconds = milliseconds.saturating_add(999) / 1000;
        set_limit!(libc::RLIMIT_CPU, seconds.max(1), "CPU");
    }
    if let Some(bytes) = policy.max_file_bytes {
        set_limit!(libc::RLIMIT_FSIZE, bytes, "file-size");
    }
    Ok(())
}

fn flush_exit(
    events: &mpsc::SyncSender<Message>,
    identity: &super::ProcessIdentity,
    code: Option<i32>,
    output_complete: bool,
) {
    let (ack, ack_rx) = mpsc::sync_channel(1);
    let mut message = (
        Event::Exit {
            identity: identity.clone(),
            code,
            output_complete,
        },
        Some(ack),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match events.try_send(message) {
            Ok(()) => {
                let _ = ack_rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
                break;
            }
            Err(mpsc::TrySendError::Full(returned)) if Instant::now() < deadline => {
                message = returned;
                thread::sleep(POLL);
            }
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct InterruptedThenData(bool);
    impl Read for InterruptedThenData {
        fn read(&mut self, _bytes: &mut [u8]) -> std::io::Result<usize> {
            if !self.0 {
                self.0 = true;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            Err(std::io::ErrorKind::Other.into())
        }
    }

    #[cfg(unix)]
    #[test]
    fn resource_limit_policy_rounds_subsecond_cpu_to_one_second() {
        let policy = ResourcePolicy {
            cpu_time_ms: Some(1),
            max_memory_bytes: Some(64 * 1024 * 1024),
            max_file_bytes: Some(1024),
            ..Default::default()
        };
        // This test exercises the policy shape without applying limits to the
        // test runner itself. Actual enforcement happens only in pre_exec.
        assert_eq!(policy.cpu_time_ms.unwrap().saturating_add(999) / 1000, 1);
        assert_eq!(policy.max_memory_bytes, Some(64 * 1024 * 1024));
        assert_eq!(policy.max_file_bytes, Some(1024));
    }

    #[test]
    fn supervisor_exit_waits_for_saturated_output_queue() {
        let (events, receiver) = mpsc::sync_channel::<Message>(1);
        let identity = super::super::ProcessIdentity::new(None);
        events
            .send((
                Event::Output {
                    identity: identity.clone(),
                    bytes: vec![b'x'],
                    stderr: false,
                },
                None,
            ))
            .unwrap();
        let drained = thread::spawn(move || {
            thread::sleep(Duration::from_millis(350));
            assert!(matches!(
                receiver.recv_timeout(Duration::from_secs(1)),
                Ok((Event::Output { .. }, None))
            ));
            let (event, ack) = receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("exit status must wait for output queue capacity");
            assert!(matches!(
                event,
                Event::Exit {
                    code: Some(0),
                    output_complete: true,
                    ..
                }
            ));
            ack.expect("exit event carries delivery acknowledgement")
                .send(())
                .unwrap();
        });
        flush_exit(&events, &identity, Some(0), true);
        drained.join().unwrap();
    }

    #[test]
    fn supervisor_capture_does_not_report_read_failure_as_eof() {
        let (events, _receiver) = mpsc::sync_channel(2);
        let identity = super::super::ProcessIdentity::new(None);
        let budget = OutputBudget::new(None);
        let result = forward_output(
            InterruptedThenData(false),
            &events,
            &identity,
            false,
            &budget,
        );
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Other);
    }

    #[test]
    fn supervisor_capture_requires_output_delivery() {
        let (events, receiver) = mpsc::sync_channel(2);
        drop(receiver);
        let identity = super::super::ProcessIdentity::new(None);
        let budget = OutputBudget::new(None);
        assert!(forward_output(&b"output"[..], &events, &identity, false, &budget,).is_err());
    }

    #[test]
    fn process_intent_selects_foreground_or_background_lifetime() {
        let policy = davinci_protocol::SandboxSpec {
            id: davinci_protocol::SandboxId("lifetime-test".into()),
            mode: davinci_protocol::SandboxMode::FullAccess,
            backend: davinci_protocol::SandboxBackendKind::Host,
            container: None,
            workspace: if cfg!(windows) {
                "C:\\workspace".into()
            } else {
                "/workspace".into()
            },
            filesystem: Default::default(),
            network: davinci_protocol::NetworkPolicy::Unrestricted,
            environment: Default::default(),
            resources: davinci_protocol::ResourcePolicy {
                timeout_ms: Some(1000),
                ..Default::default()
            },
            process: davinci_protocol::ProcessPolicy {
                allow_background: true,
                max_background_lifetime_ms: Some(5000),
            },
            required_capabilities: Default::default(),
        };
        let foreground = ProcessConfig::new(
            std::env::current_exe().unwrap(),
            vec![],
            std::env::current_dir().unwrap(),
            Default::default(),
        )
        .with_sandbox(policy.clone());
        let background = foreground.clone().as_background();
        let lifetime = |config: &ProcessConfig| {
            config.sandbox.as_ref().and_then(|spec| {
                if config.background {
                    spec.process.max_background_lifetime_ms
                } else {
                    spec.resources.timeout_ms
                }
            })
        };
        assert_eq!(lifetime(&foreground), Some(1000));
        assert_eq!(lifetime(&background), Some(5000));
    }

    #[test]
    fn output_past_the_budget_is_truncated_not_a_command_failure() {
        let (events, receiver) = mpsc::sync_channel(8);
        let identity = super::super::ProcessIdentity::new(None);
        let budget = OutputBudget::new(Some(3));
        let error = forward_output(&b"abcdef"[..], &events, &identity, false, &budget).unwrap_err();
        assert!(error.to_string().contains("truncated"), "{error}");
        drop(events);
        let events: Vec<_> = receiver.into_iter().map(|(event, _)| event).collect();
        assert_eq!(events.len(), 1, "only the allowed bytes are forwarded");
        assert!(matches!(&events[0], Event::Output { bytes, .. } if bytes == b"abc"));
    }

    #[test]
    fn output_budget_is_shared_across_streams_and_stops_at_limit() {
        let budget = OutputBudget::new(Some(5));
        assert_eq!(budget.take(3), 3);
        assert_eq!(budget.take(4), 2);
        assert_eq!(budget.take(1), 0);
    }

    #[test]
    fn supervisor_capture_completion_requires_successful_readers() {
        for failed in [false, true] {
            let reader = thread::spawn(move || {
                if failed {
                    Err(std::io::ErrorKind::Other.into())
                } else {
                    Ok(())
                }
            });
            while !reader.is_finished() {
                thread::yield_now();
            }
            assert_eq!(readers_complete(vec![reader]), !failed);
        }
        let reader = thread::spawn(|| -> std::io::Result<()> { panic!("reader fixture") });
        while !reader.is_finished() {
            thread::yield_now();
        }
        assert!(!readers_complete(vec![reader]));
        let (release, wait) = mpsc::sync_channel::<()>(1);
        let reader = thread::spawn(move || {
            let _ = wait.recv();
            Ok(())
        });
        assert!(!readers_complete(vec![reader]));
        drop(release);
    }
}
