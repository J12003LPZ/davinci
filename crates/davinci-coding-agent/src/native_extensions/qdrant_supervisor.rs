//! Start DaVinci's own Qdrant container, once, only when the user approved it.
//!
//! [`reconcile_owned_qdrant`] runs `docker start <id>` for exactly one
//! container, and only when every one of these holds:
//!
//! - Qdrant is the chosen backend and memory is on (a disabled intent stays
//!   stopped and costs no Docker call);
//! - the URL is on this machine;
//! - autostart is on in the setup profile;
//! - the user consented to `allow_start_owned_qdrant_container` in the agent
//!   directory (folder trust is never consulted);
//! - nothing already answers at the URL (an existing Qdrant is reused);
//! - the Docker engine answers (Docker itself is never started and its
//!   settings are never touched);
//! - the container is DaVinci's: its id is the one the user recorded, or it
//!   carries DaVinci's label. Any other container, Qdrant or not, is left
//!   exactly as it is;
//! - no other DaVinci is starting it (a lock file in the agent directory).
//!
//! No restart policy is changed, no container is created or removed, and
//! nothing is installed. Docker is behind two injectable traits so no test
//! starts a real container.

use super::docker_discovery::{
    discover, probe_service, DockerRunner, EngineState, Owner, ServiceState, DISCOVERY_TIMEOUT,
};
use super::service_authority::{ServiceAction, ServiceAuthorization};
use super::service_supervisor::{
    find_in_paths, is_local_url, StartLock, CONSENT_REQUIRED, SERVICE_START_FAILED, START_DEADLINE,
};
use super::setup_health::{ComponentHealth, HealthState, DOCKER_UNAVAILABLE, QDRANT_OFFLINE};
use super::setup_profile::EffectiveSetupConfig;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

const COMPONENT: &str = "qdrant";
const LOCK_FILE: &str = "qdrant-start.lock";
const START_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(250);

/// The one mutation: start a container by id. Everything else Docker can do
/// is unreachable from here.
pub trait ContainerStarter: Send + Sync + std::fmt::Debug {
    fn start(&self, container_id: &str, timeout: Duration) -> Result<(), String>;
}

/// The real `docker start`, from an absolute PATH directory.
#[derive(Debug)]
pub struct SystemContainerStarter;

impl ContainerStarter for SystemContainerStarter {
    fn start(&self, container_id: &str, timeout: Duration) -> Result<(), String> {
        // The id goes to a process argument: only a full or short hex id.
        let valid = (12..=64).contains(&container_id.len())
            && container_id.chars().all(|ch| ch.is_ascii_hexdigit());
        if !valid {
            return Err(format!("`{container_id}` is not a container id"));
        }
        let program = std::env::var_os("PATH")
            .and_then(|path| find_in_paths("docker", &path))
            .ok_or_else(|| "docker is not installed".to_string())?;
        let mut command = std::process::Command::new(program);
        command
            .args(["start", container_id])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let mut stderr = child.stderr.take().ok_or("no stderr pipe")?;
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        });
        let Ok(bytes) = receiver.recv_timeout(timeout) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("docker start did not finish in time".into());
        };
        match child.wait() {
            Ok(status) if status.success() => Ok(()),
            _ => Err(String::from_utf8_lossy(&bytes)
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("docker start failed")
                .trim()
                .to_string()),
        }
    }
}

impl ContainerStarter for super::docker_discovery::AbsentDocker {
    fn start(&self, _: &str, _: Duration) -> Result<(), String> {
        Err("docker is not installed".into())
    }
}

/// Whether a start is worth attempting this session, without asking Docker.
/// False for the default (local) setup.
pub fn start_wanted(agent_dir: &Path, effective: &EffectiveSetupConfig) -> bool {
    effective.memory.enabled
        && effective.memory.config.projection.backend
            == super::memory_backends::MemoryProjectionBackend::Qdrant
        && effective.profile.memory.autostart
        && is_local_url(&effective.memory.config.qdrant_url)
        && ServiceAuthorization::check(agent_dir, ServiceAction::StartOwnedQdrantContainer).is_ok()
}

fn health(state: HealthState, detail: impl Into<String>) -> ComponentHealth {
    ComponentHealth::new(COMPONENT, state, detail)
}

/// Bring "Qdrant answers" about if the user allowed DaVinci to start its own
/// container. Blocking, bounded by `deadline`: run it off the UI thread.
pub fn reconcile_owned_qdrant(
    agent_dir: &Path,
    effective: &EffectiveSetupConfig,
    docker: &dyn DockerRunner,
    starter: &dyn ContainerStarter,
    deadline: Duration,
) -> ComponentHealth {
    let config = &effective.memory.config;
    if !effective.memory.enabled
        || config.projection.backend != super::memory_backends::MemoryProjectionBackend::Qdrant
    {
        return health(HealthState::Disabled, "Qdrant projection is off");
    }
    let url = &config.qdrant_url;
    if probe_service(url, config.projection.allow_remote, DISCOVERY_TIMEOUT)
        == ServiceState::Reachable
    {
        return health(HealthState::Ready, format!("Qdrant answers at {url}"));
    }
    let offline = |detail: String| {
        health(HealthState::Degraded, detail)
            .with_code(QDRANT_OFFLINE)
            .with_remediation("Start Qdrant yourself, or run /setup qdrant.")
    };
    if !is_local_url(url) {
        return offline(format!(
            "{url} does not answer; it is not a local server, so DaVinci does not start it"
        ));
    }
    if !effective.profile.memory.autostart {
        return offline(format!(
            "{url} does not answer and autostart is off in the setup profile"
        ));
    }
    if let Err(reason) =
        ServiceAuthorization::check(agent_dir, ServiceAction::StartOwnedQdrantContainer)
    {
        return health(HealthState::ActionRequired, reason)
            .with_code(CONSENT_REQUIRED)
            .with_remediation(format!(
                "Run /setup approve {} to let DaVinci start its own Qdrant container, or start Qdrant yourself.",
                super::docker_discovery::START_QDRANT
            ));
    }

    let recorded = effective
        .profile
        .memory
        .qdrant_setup
        .owned_container
        .as_ref()
        .map(|container| container.id.as_str());
    let found = discover(url, config.projection.allow_remote, recorded, docker);
    match &found.docker_engine {
        EngineState::Running { .. } => {}
        EngineState::NotInstalled => {
            return found.health(effective.profile.memory.qdrant_setup.keep_running)
        }
        _ => {
            return health(
                HealthState::ActionRequired,
                format!("{url} does not answer and Docker is not responding"),
            )
            .with_code(DOCKER_UNAVAILABLE)
            .with_remediation("Start Docker yourself; DaVinci does not change Docker Desktop.")
        }
    }
    let (Owner::DaVinci, Some(container)) = (found.owner, &found.container) else {
        // Missing, or someone else's: report it, touch nothing.
        return found.health(effective.profile.memory.qdrant_setup.keep_running);
    };
    if container.is_running() {
        return wait_until_answers(
            url,
            config.projection.allow_remote,
            deadline,
            format!("DaVinci's container {} is running", container.name),
        );
    }

    // One start at a time, across launches.
    let Some(lock) = StartLock::acquire_named(agent_dir, LOCK_FILE) else {
        return wait_until_answers(
            url,
            config.projection.allow_remote,
            deadline,
            "another DaVinci is starting it".into(),
        );
    };
    let outcome = starter.start(&container.id, START_COMMAND_TIMEOUT);
    let result = match outcome {
        Ok(()) => wait_until_answers(
            url,
            config.projection.allow_remote,
            deadline,
            format!("started DaVinci's container {}", container.name),
        ),
        Err(error) => health(
            HealthState::Failed,
            format!("could not start container {}: {error}", container.name),
        )
        .with_code(SERVICE_START_FAILED)
        .with_remediation(format!(
            "Run `docker start {}` yourself and check its log.",
            container.name
        )),
    };
    drop(lock);
    result
}

fn wait_until_answers(
    url: &str,
    allow_remote: bool,
    deadline: Duration,
    how: String,
) -> ComponentHealth {
    let started = Instant::now();
    loop {
        if probe_service(url, allow_remote, Duration::from_secs(2)) == ServiceState::Reachable {
            return health(
                HealthState::Ready,
                format!("{how}; Qdrant answers at {url}"),
            );
        }
        if started.elapsed() >= deadline {
            return health(
                HealthState::Failed,
                format!(
                    "{how}, but {url} did not answer within {}s",
                    deadline.as_secs()
                ),
            )
            .with_code(SERVICE_START_FAILED)
            .with_remediation("Check the container log (docker logs), then run /setup qdrant.");
        }
        std::thread::sleep(POLL.min(deadline));
    }
}

/// The deadline `/setup` and the relaunch recovery use.
pub const DEADLINE: Duration = START_DEADLINE;
