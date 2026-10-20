//! Start the local Ollama server, once, only when the user approved it.
//!
//! [`ManagedServiceSupervisor::reconcile`] answers "is Ollama up, and if it is
//! not, may and can we start it". It starts a process only when all of these
//! hold, and otherwise says which one failed:
//!
//! - the feature is enabled and the user's profile allows autostart;
//! - the configured URL is on this machine (a remote server is the user's);
//! - the user consented to `allow_start_local_ollama` in the agent directory
//!   (Phase 1's [`ServiceAuthorization`]); folder trust is never consulted;
//! - the `ollama` found in the machine's absolute `PATH` entries is the very
//!   executable and version the user approved (recorded in the setup profile).
//!   A different binary, or one a project could plant, is not started;
//! - nobody else is starting it: a lock file in the agent directory is held
//!   for the start, and the server is probed again after taking it, so two
//!   DaVinci launches at once cause at most one start.
//!
//! A healthy Ollama that is already running is reused and left alone. The
//! supervisor stops only a child it started itself ([`shutdown_owned`]); it
//! never touches a server it did not start. The launcher is injectable so no
//! test starts a real process.
//!
//! [`ServiceAuthorization`]: super::service_authority::ServiceAuthorization
//! [`shutdown_owned`]: ManagedServiceSupervisor::shutdown_owned

use super::service_authority::{ServiceAction, ServiceAuthorization};
use super::setup_health::{
    ComponentHealth, HealthState, ProbeTarget, INSTALL_REQUIRED, PROVIDER_OFFLINE,
};
use super::setup_profile::{EffectiveSetupConfig, OllamaIdentity, SetupProfile};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The user has not allowed DaVinci to start Ollama.
pub const CONSENT_REQUIRED: &str = "CONSENT_REQUIRED";
/// The `ollama` on this machine is not the one the user approved.
pub const OLLAMA_IDENTITY_CHANGED: &str = "OLLAMA_IDENTITY_CHANGED";
/// A start was attempted and the server did not come up.
pub const SERVICE_START_FAILED: &str = "SERVICE_START_FAILED";

/// How long a start waits for the server to answer.
pub const START_DEADLINE: Duration = Duration::from_secs(8);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// How long `ollama --version` may take before it is killed.
const VERSION_DEADLINE: Duration = Duration::from_secs(5);
/// A start lock older than this belongs to a process that died.
const LOCK_STALE: Duration = Duration::from_secs(60);
const LOCK_FILE: &str = "ollama-start.lock";

/// A process this supervisor started and may stop.
pub trait OwnedProcess: Send {
    fn id(&self) -> u32;
    /// True once the process has ended.
    fn has_exited(&mut self) -> bool;
    fn kill(&mut self);
}

/// Everything the supervisor needs from the machine. The real one looks in
/// `PATH`; tests inject a fake.
pub trait OllamaLauncher: Send + Sync {
    /// The `ollama` executable to use, if any.
    fn find(&self) -> Option<PathBuf>;
    /// Its version string, used as part of its identity.
    fn version(&self, program: &Path) -> Option<String>;
    /// Start `ollama serve` and hand back the child.
    fn spawn(&self, program: &Path, agent_dir: &Path) -> Result<Box<dyn OwnedProcess>, String>;
}

/// The real launcher: absolute `PATH` entries only, `ollama --version` for the
/// version, and `ollama serve` with a trimmed environment and the agent
/// directory as its working directory (never the project).
pub struct SystemLauncher;

struct ChildProcess(std::process::Child);

impl OwnedProcess for ChildProcess {
    fn id(&self) -> u32 {
        self.0.id()
    }
    fn has_exited(&mut self) -> bool {
        !matches!(self.0.try_wait(), Ok(None))
    }
    fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl OllamaLauncher for SystemLauncher {
    fn find(&self) -> Option<PathBuf> {
        find_in_paths("ollama", &std::env::var_os("PATH")?)
    }

    fn version(&self, program: &Path) -> Option<String> {
        run_with_deadline(program, &["--version"], VERSION_DEADLINE)
    }

    fn spawn(&self, program: &Path, agent_dir: &Path) -> Result<Box<dyn OwnedProcess>, String> {
        let mut command = std::process::Command::new(program);
        command
            .arg("serve")
            .current_dir(agent_dir)
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        for (key, value) in std::env::vars_os() {
            let name = key.to_string_lossy().to_ascii_uppercase();
            let keep = matches!(
                name.as_str(),
                "PATH"
                    | "HOME"
                    | "USERPROFILE"
                    | "APPDATA"
                    | "LOCALAPPDATA"
                    | "PROGRAMDATA"
                    | "SYSTEMROOT"
                    | "WINDIR"
                    | "TEMP"
                    | "TMP"
                    | "TMPDIR"
                    | "LD_LIBRARY_PATH"
                    | "HTTP_PROXY"
                    | "HTTPS_PROXY"
                    | "NO_PROXY"
            ) || ["OLLAMA_", "CUDA_", "HIP_", "ROCR_", "NVIDIA_", "XDG_"]
                .iter()
                .any(|prefix| name.starts_with(prefix));
            if keep {
                command.env(key, value);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
            .spawn()
            .map(|child| Box::new(ChildProcess(child)) as Box<dyn OwnedProcess>)
            .map_err(|error| error.to_string())
    }
}

/// The first non-empty line `program args` prints, or `None` when it fails or
/// does not finish in time (the child is killed then).
fn run_with_deadline(program: &Path, args: &[&str], deadline: Duration) -> Option<String> {
    use std::io::Read;
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = sender.send(bytes);
    });
    // A hung executable (a stalled GPU driver, say) must not hang setup.
    let Ok(bytes) = receiver.recv_timeout(deadline) else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let _ = child.wait();
    let text = String::from_utf8_lossy(&bytes);
    let line = text.lines().find(|line| !line.trim().is_empty())?;
    Some(line.trim().to_string())
}

/// The supervisor for this process. Process-wide so a `/reload` that rebuilds
/// the native host does not lose sight of the server this process started.
pub fn system_supervisor() -> &'static ManagedServiceSupervisor {
    static SUPERVISOR: std::sync::OnceLock<ManagedServiceSupervisor> = std::sync::OnceLock::new();
    SUPERVISOR.get_or_init(|| ManagedServiceSupervisor::new(Box::new(SystemLauncher)))
}

/// First `program` in `path_var`. Only absolute directories count: a relative
/// entry (`.`, or an empty one) resolves against the process working
/// directory, which is the project, so a project could plant the executable.
pub(crate) fn find_in_paths(program: &str, path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .filter(|directory| directory.is_absolute())
        .find_map(|directory| {
            let plain = directory.join(program);
            let mut candidates = vec![plain.clone()];
            if cfg!(windows) {
                for extension in ["exe", "cmd", "bat"] {
                    candidates.push(plain.with_extension(extension));
                }
            }
            candidates.into_iter().find(|candidate| candidate.is_file())
        })
}

/// Only a local Ollama is ever started; a remote URL is the user's server.
pub(crate) fn is_local_url(url: &str) -> bool {
    let Ok(url) = url::Url::parse(url) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "0.0.0.0")
        )
}

fn same_path(left: &str, right: &Path) -> bool {
    let normalize = |text: &str| {
        let canonical = std::fs::canonicalize(text)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| text.to_string());
        let trimmed = canonical
            .strip_prefix(r"\\?\")
            .unwrap_or(&canonical)
            .to_string();
        if cfg!(windows) {
            trimmed.replace('/', "\\").to_ascii_lowercase()
        } else {
            trimmed
        }
    };
    normalize(left) == normalize(&right.to_string_lossy())
}

/// Record the `ollama` that is on this machine as the approved one. Called
/// when the user approves the start (`/setup`), never by a background check.
pub fn approve_installed_ollama(
    agent_dir: &Path,
    launcher: &dyn OllamaLauncher,
) -> Result<OllamaIdentity, String> {
    let program = launcher
        .find()
        .ok_or_else(|| format!("{INSTALL_REQUIRED}: no ollama executable on PATH"))?;
    let version = launcher
        .version(&program)
        .ok_or_else(|| format!("could not read the version of {}", program.display()))?;
    let identity = OllamaIdentity {
        path: program.to_string_lossy().into_owned(),
        version,
    };
    let (mut profile, _) = SetupProfile::load(agent_dir);
    profile.ollama = Some(identity.clone());
    profile.save(agent_dir)?;
    Ok(identity)
}

pub(crate) struct StartLock {
    path: PathBuf,
}

impl StartLock {
    /// `None` when another live start holds it.
    fn acquire(agent_dir: &Path) -> Option<Self> {
        Self::acquire_named(agent_dir, LOCK_FILE)
    }

    /// The same lock under another file name (one lock per service).
    pub(crate) fn acquire_named(agent_dir: &Path, name: &str) -> Option<Self> {
        let path = agent_dir.join(name);
        let _ = std::fs::create_dir_all(agent_dir);
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    let _ = writeln!(file, "{}", std::process::id());
                    return Some(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if !stale {
                        return None;
                    }
                    let _ = std::fs::remove_file(&path);
                }
                Err(_) => return None,
            }
        }
        None
    }
}

impl Drop for StartLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

struct Owned {
    process: Box<dyn OwnedProcess>,
}

pub struct ManagedServiceSupervisor {
    launcher: Box<dyn OllamaLauncher>,
    owned: Mutex<Option<Owned>>,
    deadline: Duration,
}

impl std::fmt::Debug for ManagedServiceSupervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedServiceSupervisor")
    }
}

impl ManagedServiceSupervisor {
    pub fn new(launcher: Box<dyn OllamaLauncher>) -> Self {
        Self {
            launcher,
            owned: Mutex::new(None),
            deadline: START_DEADLINE,
        }
    }

    /// Shorter or longer wait for a start, mainly for tests.
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    pub fn launcher(&self) -> &dyn OllamaLauncher {
        self.launcher.as_ref()
    }

    /// Bring "Ollama answers" about if the user allowed it. Blocking, bounded
    /// by `deadline`: run it off the UI thread. Never pulls a model.
    pub fn reconcile(&self, agent_dir: &Path, effective: &EffectiveSetupConfig) -> ComponentHealth {
        const C: &str = "ollama";
        // A feature that is off gets no probe, no process and no message.
        if !effective.memory.enabled {
            return ComponentHealth::new(C, HealthState::Disabled, "memory is switched off");
        }
        let config = &effective.memory.config;
        let target = ProbeTarget::from_config(config);
        if probe_ollama_up(&target) {
            return ComponentHealth::new(
                C,
                HealthState::Ready,
                format!("answers at {}", target.url),
            );
        }
        let offline = |detail: String| {
            ComponentHealth::new(C, HealthState::Degraded, detail)
                .with_code(PROVIDER_OFFLINE)
                .with_remediation("Start Ollama, or run /setup.")
        };
        if !is_local_url(&config.ollama_url) {
            return offline(format!(
                "{} does not answer; it is not a local server, so DaVinci does not start it",
                target.url
            ));
        }
        if !effective.profile.memory.autostart {
            return offline(format!(
                "{} does not answer and autostart is off in the setup profile",
                target.url
            ));
        }
        if let Err(reason) = ServiceAuthorization::check(agent_dir, ServiceAction::StartLocalOllama)
        {
            return ComponentHealth::new(C, HealthState::ActionRequired, reason)
                .with_code(CONSENT_REQUIRED)
                .with_remediation(
                    "Run /setup and approve starting the local Ollama, or start it yourself.",
                );
        }
        let Some(approved) = &effective.profile.ollama else {
            return ComponentHealth::new(
                C,
                HealthState::ActionRequired,
                "no Ollama executable has been approved yet",
            )
            .with_code(CONSENT_REQUIRED)
            .with_remediation("Run /setup so the installed Ollama can be recorded and approved.");
        };
        let Some(program) = self.launcher.find() else {
            return ComponentHealth::new(
                C,
                HealthState::ActionRequired,
                "no ollama executable on PATH",
            )
            .with_code(INSTALL_REQUIRED)
            .with_remediation("Install Ollama from https://ollama.com/download, then run /setup.");
        };
        let version = self.launcher.version(&program);
        if !same_path(&approved.path, &program) || version.as_deref() != Some(&approved.version) {
            return ComponentHealth::new(
                C,
                HealthState::ActionRequired,
                format!(
                    "the ollama on this machine ({} {}) is not the approved one ({} {})",
                    program.display(),
                    version.as_deref().unwrap_or("unknown version"),
                    approved.path,
                    approved.version
                ),
            )
            .with_code(OLLAMA_IDENTITY_CHANGED)
            .with_remediation("Run /setup to approve the Ollama that is installed now.");
        }

        // One start at a time, across launches.
        let Some(lock) = StartLock::acquire(agent_dir) else {
            // Another launch is starting it: wait for it instead of racing.
            return self.wait_ready(&target, C, "another DaVinci is starting it");
        };
        // Someone may have finished while we waited for the lock.
        if probe_ollama_up(&target) {
            drop(lock);
            return ComponentHealth::new(
                C,
                HealthState::Ready,
                format!("answers at {}", target.url),
            );
        }
        // A server this supervisor already started and that is still alive is
        // waited for, never replaced: a second spawn would orphan the first.
        if self.owned_pid().is_some() {
            let health = self.wait_ready(&target, C, "started earlier and still starting");
            drop(lock);
            return health;
        }
        let child = match self.launcher.spawn(&program, agent_dir) {
            Ok(child) => child,
            Err(error) => {
                drop(lock);
                return ComponentHealth::new(
                    C,
                    HealthState::Failed,
                    format!("could not start {}: {error}", program.display()),
                )
                .with_code(SERVICE_START_FAILED)
                .with_remediation("Start Ollama yourself (`ollama serve`), then run /setup.");
            }
        };
        *self.owned.lock().unwrap_or_else(|e| e.into_inner()) = Some(Owned { process: child });
        let health = self.wait_ready(&target, C, "started");
        drop(lock);
        health
    }

    fn wait_ready(
        &self,
        target: &ProbeTarget,
        component: &'static str,
        how: &str,
    ) -> ComponentHealth {
        let started = Instant::now();
        loop {
            if probe_ollama_up(target) {
                return ComponentHealth::new(
                    component,
                    HealthState::Ready,
                    format!("{how}; answers at {}", target.url),
                );
            }
            if started.elapsed() >= self.deadline {
                return ComponentHealth::new(
                    component,
                    HealthState::Failed,
                    format!(
                        "{how}, but {} did not answer within {}s",
                        target.url,
                        self.deadline.as_secs()
                    ),
                )
                .with_code(SERVICE_START_FAILED)
                .with_remediation(
                    "Run `ollama serve` yourself and check its log, then run /setup.",
                );
            }
            std::thread::sleep(POLL_INTERVAL.min(self.deadline));
        }
    }

    /// The pid of the server this supervisor started and that still runs.
    pub fn owned_pid(&self) -> Option<u32> {
        let mut owned = self.owned.lock().unwrap_or_else(|e| e.into_inner());
        let entry = owned.as_mut()?;
        if entry.process.has_exited() {
            *owned = None;
            return None;
        }
        Some(entry.process.id())
    }

    /// Stop the server this supervisor started, if it did. A server DaVinci
    /// found already running, or that the user started, is never touched.
    pub fn shutdown_owned(&self) -> bool {
        let owned = self.owned.lock().unwrap_or_else(|e| e.into_inner()).take();
        match owned {
            Some(mut entry) => {
                if !entry.process.has_exited() {
                    entry.process.kill();
                }
                true
            }
            None => false,
        }
    }
}

/// True when `/api/tags` answers. (The model and the embedding are the
/// memory component's business, not the service's.)
fn probe_ollama_up(target: &ProbeTarget) -> bool {
    let quick = ProbeTarget {
        timeout: target.timeout.min(Duration::from_secs(3)),
        ..target.clone()
    };
    probe_tags_only(&quick)
}

fn probe_tags_only(target: &ProbeTarget) -> bool {
    // `probe_ollama` also embeds when the model is installed; for "is the
    // service up" only the model list matters, so ask just for that.
    let agent = ureq::AgentBuilder::new().timeout(target.timeout).build();
    agent
        .get(&format!("{}/api/tags", target.url))
        .call()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hung_version_probe_is_killed_at_its_deadline() {
        #[cfg(windows)]
        let (program, args): (&str, &[&str]) = ("ping", &["-n", "60", "127.0.0.1"]);
        #[cfg(not(windows))]
        let (program, args): (&str, &[&str]) = ("sleep", &["60"]);
        let started = Instant::now();
        let found = run_with_deadline(Path::new(program), args, Duration::from_millis(400));
        assert_eq!(found, None);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }
}
