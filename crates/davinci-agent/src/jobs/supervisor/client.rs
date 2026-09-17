use super::{
    platform,
    wire::{self, Event, Request, MAX_INPUT, POLL},
    ProcessConfig, ProcessEvent, ProcessExit, SupervisorCommand,
};
use std::{
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    thread,
    time::Duration,
};

#[derive(Default)]
struct State {
    pid: Option<u32>,
    exit: Option<ProcessExit>,
    ack: Option<(u64, Result<usize, String>)>,
    sequence: u64,
}

struct Control {
    stop: AtomicBool,
    state: Mutex<State>,
    changed: Condvar,
    input: mpsc::SyncSender<Request>,
    write: Mutex<()>,
}

/// Holds one OS lifetime. Dropping the last owner requests cleanup; the monitor
/// retains the unreaped helper until its group/job has been terminated.
pub struct Supervisor {
    control: Arc<Control>,
    identity: uuid::Uuid,
    host_pid: u32,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("identity", &self.identity)
            .field("host_pid", &self.host_pid)
            .finish_non_exhaustive()
    }
}

impl Supervisor {
    pub fn spawn(
        host: &SupervisorCommand,
        config: ProcessConfig,
        event: Arc<dyn Fn(ProcessEvent) + Send + Sync>,
    ) -> Result<Self, String> {
        if serde_json::to_vec(&config)
            .map_err(|_| "invalid process configuration")?
            .len()
            > 64 * 1024
        {
            return Err("process configuration exceeds 64 KiB".into());
        }
        let mut command = Command::new(&host.executable);
        command.args(&host.argv).env_clear();
        // The trusted helper needs platform paths, never credentials or loader
        // injection variables. The requested process gets its own explicit env.
        for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("DAVINCI_INTERNAL_PROCESS_SUPERVISOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("supervisor launch failed: {error}"))?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stdin = child.stdin.take().expect("piped stdin");
        let (events, event_rx) = mpsc::sync_channel(64);
        thread::spawn(move || {
            if wire::handshake(&mut stdout).is_err() {
                return;
            }
            while let Ok(event) = wire::read::<Event>(&mut stdout) {
                if events.send(event).is_err() {
                    break;
                }
            }
        });
        // Until this handshake no configuration was sent, so the helper cannot
        // have spawned a command. Its OS ownership is established before Hello.
        if !matches!(event_rx.recv_timeout(Duration::from_secs(5)), Ok(Event::Hello { pid }) if pid == child.id())
        {
            platform::terminate_owned(&mut child);
            let _ = child.wait();
            return Err("supervisor ownership handshake failed".into());
        }
        let (input, input_rx) = mpsc::sync_channel(2);
        let control = Arc::new(Control {
            stop: AtomicBool::new(false),
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            input,
            write: Mutex::new(()),
        });
        let owner = Self {
            control: control.clone(),
            identity: uuid::Uuid::new_v4(),
            host_pid: child.id(),
        };
        let writer_control = Arc::downgrade(&control);
        thread::spawn(move || loop {
            let Some(control) = writer_control.upgrade() else {
                break;
            };
            if control.stop.load(Ordering::SeqCst) {
                break;
            }
            match input_rx.recv_timeout(POLL) {
                Ok(request) if wire::write(&mut stdin, &request).is_err() => {
                    control.stop.store(true, Ordering::SeqCst);
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
        });
        thread::spawn(move || monitor(child, event_rx, control, event));
        owner
            .control
            .input
            .try_send(Request::Configure(config))
            .map_err(|_| "supervisor stopped during startup")?;
        let state = owner
            .control
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (state, _) = owner
            .control
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |state| {
                state.pid.is_none() && state.exit.is_none()
            })
            .unwrap_or_else(|e| e.into_inner());
        if state.pid.is_none() {
            return Err(state
                .exit
                .as_ref()
                .and_then(|exit| exit.error.clone())
                .unwrap_or_else(|| "supervised command startup timed out".into()));
        }
        drop(state);
        Ok(owner)
    }

    /// Bytes are acknowledged only after the child's stdin write completes.
    /// A timeout is an unknown delivery outcome, never proof of zero delivery.
    pub fn write(&self, bytes: &[u8]) -> Result<usize, String> {
        if bytes.len() > MAX_INPUT {
            return Err("stdin exceeds 16 KiB".into());
        }
        let _single = self
            .control
            .write
            .try_lock()
            .map_err(|_| "another stdin write is pending")?;
        let mut state = self.control.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.exit.is_some() || self.control.stop.load(Ordering::SeqCst) {
            return Err("process is stopped".into());
        }
        state.sequence = state
            .sequence
            .checked_add(1)
            .ok_or("stdin sequence exhausted")?;
        let id = state.sequence;
        state.ack = None;
        self.control
            .input
            .try_send(Request::Write {
                id,
                bytes: bytes.to_vec(),
            })
            .map_err(|_| "stdin queue is unavailable")?;
        let (mut state, _) = self
            .control
            .changed
            .wait_timeout_while(state, Duration::from_millis(500), |state| {
                state.exit.is_none() && state.ack.as_ref().is_none_or(|(ack, _)| *ack != id)
            })
            .unwrap_or_else(|e| e.into_inner());
        match state.ack.take() {
            Some((ack, result)) if ack == id => result,
            _ => Err(
                "stdin acknowledgement timed out or process exited; delivery may be partial".into(),
            ),
        }
    }

    pub fn stop(&self) {
        self.control.stop.store(true, Ordering::SeqCst);
    }

    pub fn is_stopping(&self) -> bool {
        self.control.stop.load(Ordering::SeqCst)
    }

    pub fn wait(&self, timeout: Duration) -> Option<ProcessExit> {
        let state = self.control.state.lock().unwrap_or_else(|e| e.into_inner());
        let (state, _) = self
            .control
            .changed
            .wait_timeout_while(state, timeout, |state| state.exit.is_none())
            .unwrap_or_else(|e| e.into_inner());
        state.exit.clone()
    }

    pub fn child_pid(&self) -> u32 {
        self.control
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pid
            .unwrap_or(0)
    }

    /// Correlation only. Termination always uses the owned, unreaped OS lifetime.
    pub fn identity(&self) -> uuid::Uuid {
        self.identity
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn monitor(
    mut child: Child,
    events: mpsc::Receiver<Event>,
    control: Arc<Control>,
    callback: Arc<dyn Fn(ProcessEvent) + Send + Sync>,
) {
    let mut code = None;
    let mut exit_reported = false;
    let mut output_complete = false;
    let mut error = None;
    while !control.stop.load(Ordering::SeqCst) {
        match events.recv_timeout(POLL) {
            Ok(Event::Started { pid }) => {
                control.state.lock().unwrap_or_else(|e| e.into_inner()).pid = Some(pid);
                control.changed.notify_all();
            }
            Ok(Event::Output { bytes }) => {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    callback(ProcessEvent::Output(bytes))
                }))
                .is_err()
                {
                    error = Some("process output consumer failed".into());
                    break;
                }
            }
            Ok(Event::Written { id, count, failed }) => {
                let mut state = control.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.sequence == id {
                    state.ack = Some((
                        id,
                        if failed {
                            Err("process stdin delivery failed or queue is full".into())
                        } else {
                            Ok(count)
                        },
                    ));
                    control.changed.notify_all();
                }
            }
            Ok(Event::Exit {
                code: value,
                output_complete: complete,
            }) => {
                code = value;
                exit_reported = true;
                output_complete = complete;
            }
            Ok(Event::Failed | Event::Hello { .. }) => {
                error = Some("supervised command failed".into());
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let stopped = control.stop.swap(true, Ordering::SeqCst);
    // Never try_wait first: even a crashed Unix helper remains unreaped and pins
    // its PID/group. Windows Child owns a process handle, not a numeric lookup.
    platform::terminate_owned(&mut child);
    if child.wait().is_err() {
        error = Some("supervisor reap failed".into());
    }
    if !stopped && !exit_reported && error.is_none() {
        error = Some("supervisor exited without command status".into());
    }
    let exit = ProcessExit {
        code,
        stopped,
        error,
        output_complete,
    };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        callback(ProcessEvent::Finished(exit.clone()))
    }));
    control.state.lock().unwrap_or_else(|e| e.into_inner()).exit = Some(exit);
    control.changed.notify_all();
}
