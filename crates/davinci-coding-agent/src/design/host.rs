//! Supervised, bounded private-pipe transport. The caller retains the session writer.
use super::{admission::*, error::*, runtime::*};
use davinci_agent::jobs::supervisor::{ProcessConfig, ProcessEvent, Supervisor};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

pub const MAX_FRAME: usize = 1024 * 1024;
#[derive(Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}
impl FrameDecoder {
    pub fn receive(&mut self, chunk: &[u8]) -> DesignResult<Vec<Value>> {
        let mut messages = Vec::new();
        for byte in chunk {
            self.buffer.push(*byte);
            if self.buffer.len() < 4 {
                continue;
            }
            let length =
                u32::from_be_bytes(self.buffer[..4].try_into().expect("four bytes")) as usize;
            if length == 0 || length > MAX_FRAME {
                return Err(DesignError::BudgetExceeded("host frame limit".into()));
            }
            if self.buffer.len() == length + 4 {
                let value: Value = serde_json::from_slice(&self.buffer[4..])?;
                if value["version"] != 1
                    || value["id"].as_str().is_none_or(|id| {
                        id.is_empty()
                            || id.len() > 64
                            || !id
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                    })
                {
                    return Err(DesignError::InvalidInput("invalid host envelope".into()));
                }
                messages.push(value);
                self.buffer.clear();
                if messages.len() > 8 {
                    return Err(DesignError::BudgetExceeded("host queue limit".into()));
                }
            }
        }
        Ok(messages)
    }
}

pub struct DesignHostLease {
    supervisor: Arc<Supervisor>,
    receiver: mpsc::Receiver<DesignResult<Value>>,
    outstanding: BTreeSet<String>,
    directory: PathBuf,
    closed: bool,
    ready: bool,
    cancellations: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
}
impl DesignHostLease {
    pub fn start(
        ctx: &AuthorizedDesignContext,
        runtime: &TrustedDesignRuntime,
    ) -> DesignResult<Self> {
        ctx.check("design_host", &json!({"runtime":runtime.fingerprint()}))?;
        runtime.verify()?;
        let assets: Value =
            serde_json::from_slice(&fs::read(runtime.root.join("ui/manifest.json"))?)?;
        Self::spawn(
            ctx,
            runtime,
            "host/main.cjs",
            json!({"assetRoot":runtime.root.join("ui"),"assets":assets}),
        )
    }
    pub(crate) fn spawn(
        ctx: &AuthorizedDesignContext,
        runtime: &TrustedDesignRuntime,
        script: &str,
        config: Value,
    ) -> DesignResult<Self> {
        let host = ctx.tools().foreground_supervisor.as_ref().ok_or_else(|| {
            DesignError::MissingCapability("trusted process supervisor required".into())
        })?;
        let directory =
            std::env::temp_dir().join(format!("davinci-design-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        no_links(&directory)?;
        if directory.canonicalize()?.starts_with(ctx.workspace()) {
            return Err(DesignError::Denied(
                "private host directory must be outside workspace".into(),
            ));
        }
        let config_path = directory.join("config.json");
        fs::write(&config_path, serde_json::to_vec(&config)?)?;
        let (sender, receiver) = mpsc::sync_channel(8);
        let decoder = Arc::new(Mutex::new(FrameDecoder::default()));
        let stopped = Arc::new(Mutex::new(None::<std::sync::Weak<Supervisor>>));
        let owner = stopped.clone();
        let cancellations = Arc::new(Mutex::new(BTreeMap::<String, Arc<AtomicBool>>::new()));
        let request_flags = cancellations.clone();
        let seen = Mutex::new(BTreeSet::new());
        let mut environment = BTreeMap::new();
        for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR"] {
            if let Ok(value) = std::env::var(name) {
                environment.insert(name.into(), value);
            }
        }
        let result = Supervisor::spawn_with_stderr(
            host,
            ProcessConfig {
                executable: runtime.node.clone(),
                argv: vec![
                    node_path(&runtime.root.join(script)),
                    node_path(&config_path),
                ],
                cwd: directory.clone(),
                environment,
                sandbox: None,
                background: true,
                service: true,
                operation: None,
            },
            Arc::new(move |event| {
                let messages = match event {
                    ProcessEvent::Output(bytes) => decoder
                        .lock()
                        .map_err(|_| DesignError::IoFailure("host decoder unavailable".into()))
                        .and_then(|mut d| d.receive(&bytes)),
                    ProcessEvent::Finished(exit) => Err(DesignError::MissingCapability(format!(
                        "design host closed (exit {:?}, complete {})",
                        exit.code, exit.output_complete
                    ))),
                };
                let mut failed = false;
                match messages {
                    Ok(messages) => {
                        for message in messages {
                            let id = message["id"].as_str().expect("validated ID");
                            let Ok(mut flags) = request_flags.lock() else {
                                failed = true;
                                break;
                            };
                            if message["operation"] == "cancel" {
                                if let Some(flag) = flags.get(id) {
                                    flag.store(true, Ordering::SeqCst);
                                }
                                continue;
                            }
                            if id != "ready" {
                                let Ok(mut seen) = seen.lock() else {
                                    failed = true;
                                    break;
                                };
                                if !seen.insert(id.to_string())
                                    || seen.len() > 4096
                                    || flags.len() >= 8
                                {
                                    failed = true;
                                    break;
                                }
                                flags.insert(id.to_string(), Arc::new(AtomicBool::new(false)));
                            }
                            failed |= sender.try_send(Ok(message)).is_err();
                        }
                    }
                    Err(error) => {
                        let _ = sender.try_send(Err(error));
                        failed = true;
                    }
                }
                if failed {
                    if let Ok(flags) = request_flags.lock() {
                        for flag in flags.values() {
                            flag.store(true, Ordering::SeqCst);
                        }
                    }
                    if let Some(supervisor) = owner
                        .lock()
                        .ok()
                        .and_then(|o| o.as_ref().and_then(std::sync::Weak::upgrade))
                    {
                        supervisor.stop();
                    }
                }
            }),
            Arc::new(|_| {}),
        );
        match result {
            Ok(supervisor) => {
                let supervisor = Arc::new(supervisor);
                *stopped
                    .lock()
                    .map_err(|_| DesignError::IoFailure("host owner unavailable".into()))? =
                    Some(Arc::downgrade(&supervisor));
                Ok(Self {
                    supervisor,
                    receiver,
                    directory,
                    outstanding: BTreeSet::new(),
                    closed: false,
                    ready: false,
                    cancellations,
                })
            }
            Err(error) => {
                let _ = fs::remove_file(config_path);
                let _ = fs::remove_dir(directory);
                Err(DesignError::MissingCapability(error.to_string()))
            }
        }
    }
    pub fn receive(&mut self, timeout: Duration) -> DesignResult<Option<Value>> {
        if self.closed {
            return Err(DesignError::Cancelled);
        }
        match self
            .receiver
            .recv_timeout(timeout.min(Duration::from_secs(30)))
        {
            Ok(result) => {
                let value = result?;
                let id = value["id"].as_str().expect("validated ID").to_owned();
                if id == "ready" {
                    if self.ready {
                        self.close();
                        return Err(DesignError::Conflict("repeated host handshake".into()));
                    }
                    self.ready = true;
                } else if !self.ready {
                    self.close();
                    return Err(DesignError::Conflict(
                        "request before host handshake".into(),
                    ));
                }
                if id != "ready" && (!self.outstanding.insert(id) || self.outstanding.len() > 8) {
                    self.close();
                    return Err(DesignError::Conflict(
                        "replayed or excessive host request".into(),
                    ));
                }
                Ok(Some(value))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err(DesignError::MissingCapability(
                "design host disconnected".into(),
            )),
        }
    }
    pub fn respond(&mut self, id: &str, result: DesignResult<Value>) -> DesignResult<()> {
        if !self.outstanding.remove(id) {
            return Err(DesignError::Denied("response has no active request".into()));
        }
        self.cancellations
            .lock()
            .map_err(|_| DesignError::Cancelled)?
            .remove(id);
        let message = match result {
            Ok(value) => json!({"version":1,"id":id,"result":value}),
            Err(error) => json!({"version":1,"id":id,"error":error}),
        };
        let bytes = serde_json::to_vec(&message)?;
        let bytes = if bytes.len() > 256 * 1024 {
            serde_json::to_vec(
                &json!({"version":1,"id":id,"error":DesignError::BudgetExceeded("host response limit".into())}),
            )?
        } else {
            bytes
        };
        let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
        frame.extend(bytes);
        self.supervisor
            .write(&frame)
            .map_err(DesignError::IoFailure)?;
        Ok(())
    }
    pub fn cancellation(&self, id: &str) -> DesignResult<Arc<AtomicBool>> {
        self.cancellations
            .lock()
            .map_err(|_| DesignError::Cancelled)?
            .get(id)
            .cloned()
            .ok_or(DesignError::Cancelled)
    }
    pub fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            if let Ok(flags) = self.cancellations.lock() {
                for flag in flags.values() {
                    flag.store(true, Ordering::SeqCst);
                }
            }
            self.supervisor.stop();
        }
    }
    pub(crate) fn compiled_output(&self) -> PathBuf {
        self.directory.join("compiled.json")
    }
}
impl Drop for DesignHostLease {
    fn drop(&mut self) {
        self.close();
        if self.supervisor.wait(Duration::from_secs(2)).is_some() {
            clean_directory(&self.directory);
        } else {
            let supervisor = self.supervisor.clone();
            let directory = self.directory.clone();
            std::thread::spawn(move || {
                if supervisor.wait(Duration::from_secs(30)).is_some() {
                    clean_directory(&directory);
                }
            });
        }
    }
}
fn clean_directory(directory: &std::path::Path) {
    for name in ["config.json", "compiled.json"] {
        let _ = fs::remove_file(directory.join(name));
    }
    let _ = fs::remove_dir(directory);
}
