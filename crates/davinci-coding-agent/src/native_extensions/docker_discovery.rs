//! Read-only discovery of an existing Qdrant and of Docker.
//!
//! What this file can do to the machine: run `docker version` and `docker ps`,
//! each with a timeout, and `GET /readyz` on the configured Qdrant URL. The
//! [`DockerQuery`] enum has two members and both read; there is no way to
//! express `run`, `start`, `stop`, `rm` or a Docker Desktop setting here, so
//! discovery cannot mutate anything by construction. Starting a container is a
//! separate, consent-gated path in `service_supervisor`.
//!
//! The answers are kept apart on purpose: whether the engine answers, whether
//! a Qdrant container exists, whether DaVinci owns it, and whether Qdrant
//! itself is reachable are four facts, and none implies another.

use super::service_authority::ServiceAction;
use super::service_supervisor::{find_in_paths, is_local_url};
use super::setup_health::{
    ComponentHealth, HealthState, DOCKER_UNAVAILABLE, INSTALL_REQUIRED, QDRANT_MISSING,
    QDRANT_NOT_OWNED, QDRANT_OFFLINE,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

/// Every Docker call and the Qdrant probe stop waiting after this.
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
/// The label DaVinci's own container carries (see [`docker_run_command`]).
pub const OWNER_LABEL: &str = "io.davinci.owner";
pub const OWNER_VALUE: &str = "davinci-memory";
/// The word after `/setup approve` that allows starting DaVinci's container.
pub const START_QDRANT: &str = "start-qdrant";
/// Name suggested in the command the user runs themselves.
pub const SUGGESTED_NAME: &str = "davinci-qdrant";

/// The only things discovery asks Docker. Both are reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerQuery {
    Version,
    ListContainers,
}

impl DockerQuery {
    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Version => &["version", "--format", "{{.Server.Version}}"],
            Self::ListContainers => &["ps", "--all", "--no-trunc", "--format", "{{json .}}"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerReply {
    Output(String),
    /// The CLI ran and said no (the daemon is not running, access denied).
    Failed(String),
    TimedOut,
    /// No `docker` executable in an absolute PATH directory.
    NotInstalled,
}

/// Injectable so tests never touch a real Docker.
pub trait DockerRunner: Send + Sync + std::fmt::Debug {
    fn run(&self, query: DockerQuery, timeout: Duration) -> DockerReply;
}

/// The real `docker` CLI, found only in absolute PATH directories (a relative
/// entry would resolve inside the project, which could plant it).
#[derive(Debug)]
pub struct SystemDockerRunner;

impl SystemDockerRunner {
    fn find() -> Option<PathBuf> {
        find_in_paths("docker", &std::env::var_os("PATH")?)
    }
}

impl DockerRunner for SystemDockerRunner {
    fn run(&self, query: DockerQuery, timeout: Duration) -> DockerReply {
        let Some(program) = Self::find() else {
            return DockerReply::NotInstalled;
        };
        let mut command = std::process::Command::new(program);
        command
            .args(query.args())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let Ok(mut child) = command.spawn() else {
            return DockerReply::NotInstalled;
        };
        let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            return DockerReply::Failed("no output pipes".into());
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let _ = stdout.read_to_end(&mut out);
            let _ = stderr.read_to_end(&mut err);
            let _ = sender.send((out, err));
        });
        let Ok((out, err)) = receiver.recv_timeout(timeout) else {
            let _ = child.kill();
            let _ = child.wait();
            return DockerReply::TimedOut;
        };
        let status = child.wait();
        if status.map(|status| status.success()).unwrap_or(false) {
            DockerReply::Output(String::from_utf8_lossy(&out).into_owned())
        } else {
            let text = String::from_utf8_lossy(&err);
            let line = text.lines().find(|line| !line.trim().is_empty());
            DockerReply::Failed(
                line.unwrap_or("docker exited with an error")
                    .trim()
                    .to_string(),
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum EngineState {
    NotInstalled,
    /// The CLI is there but the engine does not answer (Docker Desktop closed).
    Unreachable {
        detail: String,
    },
    TimedOut,
    Running {
        version: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    /// `running`, `exited`, `created`, ...
    pub state: String,
    pub ports: String,
    #[serde(skip)]
    pub labels: BTreeMap<String, String>,
}

impl ContainerInfo {
    pub fn is_running(&self) -> bool {
        self.state.eq_ignore_ascii_case("running")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum ServiceState {
    Reachable,
    Unreachable {
        detail: String,
    },
    /// Not probed: the URL is not on this machine and the user has not allowed that.
    NotProbed {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Owner {
    /// The recorded id, still carrying DaVinci's label. The only case a start
    /// may be offered for.
    DaVinci,
    /// A Qdrant container that is someone else's. Never touched.
    External,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QdrantDiscovery {
    pub url: String,
    pub service: ServiceState,
    pub docker_engine: EngineState,
    /// Every Qdrant container the engine listed (empty when the engine could
    /// not be asked, which `docker_engine` says).
    pub candidates: Vec<ContainerInfo>,
    /// The container this discovery considers relevant: the recorded one when
    /// it is listed, otherwise the first Qdrant container.
    pub container: Option<ContainerInfo>,
    pub owner: Owner,
}

/// The container recorded for DaVinci, by id or short id prefix (12+ chars).
fn matches_recorded(container: &ContainerInfo, recorded: &str) -> bool {
    let recorded = recorded.trim();
    recorded.len() >= 12
        && (container.id.starts_with(recorded) || recorded.starts_with(&container.id))
}

/// Whether the container carries DaVinci's label. Only a hint: the label alone
/// never makes a container DaVinci's.
pub fn has_owner_label(container: &ContainerInfo) -> bool {
    container.labels.get(OWNER_LABEL).map(String::as_str) == Some(OWNER_VALUE)
}

/// DaVinci owns a container only when its id is the one the user recorded AND
/// it still carries DaVinci's label. A labelled container nobody recorded is
/// someone else's; a recorded id that lost its label (or was recreated under
/// another id) is not started either.
pub fn is_owned(container: &ContainerInfo, recorded_id: Option<&str>) -> bool {
    recorded_id.is_some_and(|recorded| matches_recorded(container, recorded))
        && has_owner_label(container)
}

fn parse_labels(text: &str) -> BTreeMap<String, String> {
    text.split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect()
}

/// `docker ps --format '{{json .}}'`: one JSON object per line. Only Qdrant
/// containers are kept (by image name).
pub fn parse_containers(output: &str) -> Vec<ContainerInfo> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter_map(|row| {
            let text = |key: &str| row[key].as_str().unwrap_or_default().to_string();
            let image = text("Image");
            image
                .to_ascii_lowercase()
                .contains("qdrant")
                .then(|| ContainerInfo {
                    id: text("ID"),
                    name: text("Names"),
                    image,
                    state: text("State"),
                    ports: text("Ports"),
                    labels: parse_labels(&text("Labels")),
                })
        })
        .collect()
}

/// Is Qdrant answering at `url`? One `GET /readyz`, short timeout.
pub fn probe_service(url: &str, allow_remote: bool, timeout: Duration) -> ServiceState {
    if !is_local_url(url) && !allow_remote {
        return ServiceState::NotProbed {
            reason: "the URL is not on this machine and remote Qdrant is not allowed".into(),
        };
    }
    let agent = crate::http_client::agent_with_timeout(timeout);
    match agent
        .get(&format!("{}/readyz", url.trim_end_matches('/')))
        .call()
    {
        Ok(_) => ServiceState::Reachable,
        Err(error) => ServiceState::Unreachable {
            detail: error.to_string(),
        },
    }
}

/// Look, change nothing. Docker is asked only for `version`, and for `ps`
/// only if the engine answered.
pub fn discover(
    url: &str,
    allow_remote: bool,
    recorded_id: Option<&str>,
    runner: &dyn DockerRunner,
) -> QdrantDiscovery {
    let service = probe_service(url, allow_remote, DISCOVERY_TIMEOUT);
    let docker_engine = match runner.run(DockerQuery::Version, DISCOVERY_TIMEOUT) {
        DockerReply::Output(text) => EngineState::Running {
            version: text.trim().to_string(),
        },
        DockerReply::Failed(detail) => EngineState::Unreachable { detail },
        DockerReply::TimedOut => EngineState::TimedOut,
        DockerReply::NotInstalled => EngineState::NotInstalled,
    };
    let candidates = if matches!(docker_engine, EngineState::Running { .. }) {
        match runner.run(DockerQuery::ListContainers, DISCOVERY_TIMEOUT) {
            DockerReply::Output(text) => parse_containers(&text),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    // The recorded container comes first, wherever `docker ps` lists it.
    let recorded = recorded_id.and_then(|id| {
        candidates
            .iter()
            .find(|container| matches_recorded(container, id))
    });
    let owned = recorded.filter(|container| is_owned(container, recorded_id));
    let container = recorded.or_else(|| candidates.first()).cloned();
    let owner = match (&container, owned) {
        (None, _) => Owner::None,
        (Some(_), Some(_)) => Owner::DaVinci,
        (Some(_), None) => Owner::External,
    };
    QdrantDiscovery {
        url: url.to_string(),
        service,
        docker_engine,
        candidates,
        container,
        owner,
    }
}

/// The command the user can run to make a container DaVinci recognises. It is
/// shown, never executed: bound to loopback, labelled, with a named volume.
pub fn docker_run_command_with(keep_running: bool) -> String {
    let restart = if keep_running {
        "--restart unless-stopped "
    } else {
        ""
    };
    format!(
        "docker run -d --name {SUGGESTED_NAME} --label {OWNER_LABEL}={OWNER_VALUE} {restart}\
         -p 127.0.0.1:6333:6333 -v {SUGGESTED_NAME}:/qdrant/storage qdrant/qdrant"
    )
}

/// A machine with no Docker, for offline hosts and tests.
#[derive(Debug)]
pub struct AbsentDocker;

impl DockerRunner for AbsentDocker {
    fn run(&self, _: DockerQuery, _: Duration) -> DockerReply {
        DockerReply::NotInstalled
    }
}

impl QdrantDiscovery {
    pub fn service_reachable(&self) -> bool {
        self.service == ServiceState::Reachable
    }

    /// The step `/setup` shows: what was found and the next move, in plain
    /// words. It never says a container was started; this file cannot.
    pub fn health(&self, keep_running: bool) -> ComponentHealth {
        const QDRANT: &str = "qdrant";
        let who = match (&self.container, self.owner) {
            (Some(container), Owner::DaVinci) => {
                format!(" (container {}, DaVinci's)", container.name)
            }
            (Some(container), _) => {
                format!(" (container {}, not DaVinci's, left alone)", container.name)
            }
            _ => String::new(),
        };
        if self.service_reachable() {
            return ComponentHealth::new(
                QDRANT,
                HealthState::Ready,
                format!("Qdrant answers at {}{who}", self.url),
            );
        }
        let down = |detail: String, remediation: String, code: &'static str| {
            ComponentHealth::new(QDRANT, HealthState::ActionRequired, detail)
                .with_code(code)
                .with_remediation(remediation)
        };
        if let ServiceState::NotProbed { reason } = &self.service {
            return down(
                format!("Qdrant at {} was not checked: {reason}", self.url),
                "set projection.allowRemote only if you accept sending vectors there".into(),
                QDRANT_OFFLINE,
            );
        }
        match (&self.docker_engine, &self.container, self.owner) {
            (EngineState::NotInstalled, _, _) => down(
                format!("no Qdrant answers at {} and Docker is not installed", self.url),
                "install Docker, or point qdrantUrl at a Qdrant you already run; DaVinci does not install software".into(),
                INSTALL_REQUIRED,
            ),
            (EngineState::Unreachable { detail }, _, _) => down(
                format!("no Qdrant answers at {} and Docker is not running ({detail})", self.url),
                "start Docker yourself; DaVinci does not change Docker Desktop".into(),
                DOCKER_UNAVAILABLE,
            ),
            (EngineState::TimedOut, _, _) => down(
                format!("no Qdrant answers at {} and Docker did not answer in time", self.url),
                "check that Docker is responsive, then run /setup qdrant again".into(),
                DOCKER_UNAVAILABLE,
            ),
            (EngineState::Running { .. }, None, _) => down(
                format!("no Qdrant answers at {} and no Qdrant container exists", self.url),
                format!("create one yourself: {}", docker_run_command_with(keep_running)),
                QDRANT_MISSING,
            ),
            (EngineState::Running { .. }, Some(container), Owner::DaVinci) => down(
                format!(
                    "DaVinci's Qdrant container {} is {}",
                    container.name, container.state
                ),
                format!("/setup approve {START_QDRANT} lets DaVinci start that one container"),
                QDRANT_OFFLINE,
            ),
            (EngineState::Running { .. }, Some(container), _) => down(
                format!(
                    "a Qdrant container exists ({}, {}) but it is not DaVinci's",
                    container.name, container.state
                ),
                format!(
                    "start it yourself (docker start {}), or record it with /setup qdrant own {}",
                    container.name,
                    container.id.chars().take(12).collect::<String>()
                ),
                QDRANT_NOT_OWNED,
            ),
        }
    }
}

/// The consent that gates starting DaVinci's own container.
pub fn start_consent() -> ServiceAction {
    ServiceAction::StartOwnedQdrantContainer
}
