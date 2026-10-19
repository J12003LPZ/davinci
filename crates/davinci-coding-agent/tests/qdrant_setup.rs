//! `/setup` and the optional Qdrant: read-only discovery, ownership, consent.
//! Fake Docker and an in-process Qdrant only. No real Docker, no network.

use davinci_coding_agent::native_extensions::docker_discovery::{
    discover, DockerQuery, DockerReply, DockerRunner, EngineState, Owner, ServiceState,
};
use davinci_coding_agent::native_extensions::memory_backends::fake_qdrant::FakeQdrant;
use davinci_coding_agent::native_extensions::service_supervisor::{
    ManagedServiceSupervisor, OllamaLauncher, OwnedProcess,
};
use davinci_coding_agent::native_extensions::NativeExtensionHost;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DAVINCI_ID: &str = "aaaaaaaaaaaa1111111111111111111111111111111111111111111111111111";
const OTHER_ID: &str = "bbbbbbbbbbbb2222222222222222222222222222222222222222222222222222";

/// A Docker that records what it was asked and answers from a script.
#[derive(Debug)]
struct FakeDocker {
    version: DockerReply,
    ps: DockerReply,
    asked: Mutex<Vec<DockerQuery>>,
}

impl FakeDocker {
    fn new(version: DockerReply, ps: DockerReply) -> Arc<Self> {
        Arc::new(Self {
            version,
            ps,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn running_with(rows: &[String]) -> Arc<Self> {
        Self::new(
            DockerReply::Output("27.0.1\n".into()),
            DockerReply::Output(rows.join("\n")),
        )
    }

    fn asked(&self) -> Vec<DockerQuery> {
        self.asked.lock().unwrap().clone()
    }
}

impl DockerRunner for FakeDocker {
    fn run(&self, query: DockerQuery, _: Duration) -> DockerReply {
        self.asked.lock().unwrap().push(query);
        match query {
            DockerQuery::Version => self.version.clone(),
            DockerQuery::ListContainers => self.ps.clone(),
        }
    }
}

fn row(id: &str, name: &str, image: &str, state: &str, labels: &str) -> String {
    serde_json::json!({
        "ID": id, "Names": name, "Image": image, "State": state,
        "Ports": "127.0.0.1:6333->6333/tcp", "Labels": labels,
    })
    .to_string()
}

fn external_qdrant() -> String {
    row(
        OTHER_ID,
        "team-qdrant",
        "qdrant/qdrant:v1.12",
        "running",
        "",
    )
}

fn davinci_qdrant(state: &str) -> String {
    row(
        DAVINCI_ID,
        "davinci-qdrant",
        "qdrant/qdrant",
        state,
        "io.davinci.owner=davinci-memory",
    )
}

#[test]
fn existing_qdrant_detected_no_unapproved_docker_mutation() {
    let qdrant = FakeQdrant::start();
    let docker = FakeDocker::running_with(&[external_qdrant()]);

    let found = discover(&qdrant.url(), false, None, docker.as_ref());
    assert_eq!(found.service, ServiceState::Reachable);
    assert_eq!(
        found.docker_engine,
        EngineState::Running {
            version: "27.0.1".into()
        }
    );
    let container = found
        .container
        .as_ref()
        .expect("the external container is listed");
    assert_eq!(container.name, "team-qdrant");
    assert_eq!(found.owner, Owner::External, "never assumed to be ours");
    let health = found.health(false);
    assert_eq!(format!("{:?}", health.state), "Ready");
    assert!(health.detail.contains("not DaVinci's"), "{}", health.detail);

    // Discovery asked Docker two read questions and nothing else; the type
    // has no way to ask for more.
    assert_eq!(
        docker.asked(),
        vec![DockerQuery::Version, DockerQuery::ListContainers]
    );
    // And Qdrant got a readiness probe, no write.
    assert!(qdrant
        .requests()
        .iter()
        .all(|request| request.method == "GET" && request.path == "/readyz"));
}

#[test]
fn engine_reachable_and_container_exists_are_separate_facts() {
    let down_url = "http://127.0.0.1:9";

    // Docker missing: install guidance, no list call.
    let missing = FakeDocker::new(DockerReply::NotInstalled, DockerReply::NotInstalled);
    let found = discover(down_url, false, None, missing.as_ref());
    assert_eq!(found.docker_engine, EngineState::NotInstalled);
    assert_eq!(found.health(false).code, Some("INSTALL_REQUIRED"));
    assert_eq!(missing.asked(), vec![DockerQuery::Version]);

    // Docker installed, engine not running: tell the user, change nothing.
    let stopped = FakeDocker::new(
        DockerReply::Failed("Cannot connect to the Docker daemon".into()),
        DockerReply::NotInstalled,
    );
    let found = discover(down_url, false, None, stopped.as_ref());
    assert!(matches!(
        found.docker_engine,
        EngineState::Unreachable { .. }
    ));
    assert_eq!(found.health(false).code, Some("DOCKER_UNAVAILABLE"));
    assert!(found
        .health(false)
        .remediation
        .unwrap()
        .contains("does not change Docker Desktop"));
    assert_eq!(stopped.asked(), vec![DockerQuery::Version]);

    // A hung docker is a timeout, not a hang.
    let hung = FakeDocker::new(DockerReply::TimedOut, DockerReply::TimedOut);
    assert_eq!(
        discover(down_url, false, None, hung.as_ref()).docker_engine,
        EngineState::TimedOut
    );

    // Engine fine, no Qdrant container: the exact command is shown, not run.
    let empty = FakeDocker::running_with(&[]);
    let found = discover(down_url, false, None, empty.as_ref());
    let health = found.health(true);
    assert_eq!(health.code, Some("QDRANT_MISSING"));
    let command = health.remediation.unwrap();
    assert!(command.contains("docker run -d"), "{command}");
    assert!(command.contains("--restart unless-stopped"), "{command}");
    assert!(command.contains("127.0.0.1:6333:6333"), "{command}");
    assert!(!found
        .health(false)
        .remediation
        .unwrap()
        .contains("--restart"));
    assert_eq!(
        empty.asked(),
        vec![DockerQuery::Version, DockerQuery::ListContainers]
    );
}

#[test]
fn ownership_comes_from_the_label_or_a_recorded_id_only() {
    let down_url = "http://127.0.0.1:9";
    let both = FakeDocker::running_with(&[external_qdrant(), davinci_qdrant("exited")]);
    let found = discover(down_url, false, None, both.as_ref());
    assert_eq!(found.owner, Owner::DaVinci);
    assert_eq!(found.container.as_ref().unwrap().name, "davinci-qdrant");
    assert_eq!(found.candidates.len(), 2);
    let health = found.health(false);
    assert_eq!(health.code, Some("QDRANT_OFFLINE"));
    assert!(health
        .remediation
        .unwrap()
        .contains("/setup approve start-qdrant"));

    // A container with only a recorded id is ours; one with a similar name is not.
    let by_id = FakeDocker::running_with(&[row(DAVINCI_ID, "x", "qdrant/qdrant", "exited", "")]);
    let found = discover(down_url, false, Some(&DAVINCI_ID[..12]), by_id.as_ref());
    assert_eq!(found.owner, Owner::DaVinci);
    let lookalike = FakeDocker::running_with(&[row(
        OTHER_ID,
        "davinci-qdrant",
        "qdrant/qdrant",
        "exited",
        "",
    )]);
    let found = discover(down_url, false, Some(&DAVINCI_ID[..12]), lookalike.as_ref());
    assert_eq!(found.owner, Owner::External);
    assert_eq!(found.health(false).code, Some("QDRANT_NOT_OWNED"));
    // A recorded prefix shorter than 12 characters matches nothing.
    let found = discover(down_url, false, Some("aaaa"), by_id.as_ref());
    assert_eq!(found.owner, Owner::External);
}

#[test]
fn a_remote_qdrant_url_is_not_probed_without_consent() {
    let docker = FakeDocker::running_with(&[]);
    let found = discover("https://qdrant.example.com", false, None, docker.as_ref());
    assert!(matches!(found.service, ServiceState::NotProbed { .. }));
}

// ---- through /setup ----

struct NoOllama;
impl OllamaLauncher for NoOllama {
    fn find(&self) -> Option<PathBuf> {
        None
    }
    fn version(&self, _: &Path) -> Option<String> {
        None
    }
    fn spawn(&self, _: &Path, _: &Path) -> Result<Box<dyn OwnedProcess>, String> {
        Err("no process may be started in this test".into())
    }
}

fn host(agent: &Path, project: &Path, docker: Arc<FakeDocker>) -> NativeExtensionHost {
    NativeExtensionHost::new_with_agent_dir("qdrant-setup", project, Some(agent))
        .with_supervisor(Arc::new(ManagedServiceSupervisor::new(Box::new(NoOllama))))
        .with_docker(docker)
}

fn step<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == id)
        .unwrap_or_else(|| panic!("no {id} step in {report}"))
}

#[test]
fn local_mode_setup_never_asks_docker_anything() {
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let docker = FakeDocker::running_with(&[external_qdrant()]);
    let mut host = host(agent.path(), project.path(), docker.clone());
    for args in ["", "check", "check --json", "repair"] {
        let report = host.command("setup", args).unwrap().unwrap();
        if args.is_empty() || args.starts_with("check") {
            let qdrant = step(&report, "qdrant");
            assert_eq!(qdrant["state"], "skipped", "{args}: {qdrant}");
            assert!(qdrant["detail"].as_str().unwrap().contains("off"));
        }
    }
    assert!(docker.asked().is_empty(), "local mode called Docker");
}

#[test]
fn setup_qdrant_explains_then_enable_and_disable_round_trip() {
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let qdrant = FakeQdrant::start();
    std::fs::write(
        agent.path().join("vector-memory.json"),
        format!("{{\"qdrantUrl\":\"{}\"}}", qdrant.url()),
    )
    .unwrap();
    let docker = FakeDocker::running_with(&[external_qdrant()]);
    let mut host = host(agent.path(), project.path(), docker.clone());

    // Explaining is allowed to look; it shows benefits and downsides.
    let report = host.command("setup", "qdrant").unwrap().unwrap();
    let text = report["steps"][0]["detail"].as_str().unwrap().to_string();
    for needle in [
        "Benefit",
        "Downsides",
        "QDRANT_OFFLINE",
        "stays off unless you choose it",
    ] {
        assert!(text.contains(needle), "missing {needle}: {text}");
    }
    assert_eq!(report["steps"][1]["state"], "ready");

    // Enabling is explicit and persisted; the profile says Qdrant.
    let report = host.command("setup", "qdrant enable").unwrap().unwrap();
    assert_eq!(report["steps"][0]["state"], "done");
    let profile = std::fs::read_to_string(agent.path().join("setup-profile.json")).unwrap();
    assert!(profile.contains("\"backend\": \"qdrant\""), "{profile}");
    let default = host.command("setup", "").unwrap().unwrap();
    assert_eq!(step(&default, "qdrant")["state"], "ready");

    let report = host.command("setup", "qdrant disable").unwrap().unwrap();
    assert!(report["steps"][0]["detail"]
        .as_str()
        .unwrap()
        .contains("off for good"));
    let asked_before = docker.asked().len();
    let default = host.command("setup", "").unwrap().unwrap();
    assert_eq!(step(&default, "qdrant")["state"], "skipped");
    assert_eq!(
        docker.asked().len(),
        asked_before,
        "off again means no Docker calls"
    );

    assert!(host.command("setup", "qdrant frobnicate").is_err());
}

#[test]
fn own_records_only_a_container_the_engine_lists() {
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let docker = FakeDocker::running_with(&[external_qdrant()]);
    let mut host = host(agent.path(), project.path(), docker);

    for bad in ["qdrant own abc", "qdrant own ffffffffffff"] {
        let report = host.command("setup", bad).unwrap().unwrap();
        assert_eq!(report["steps"][0]["state"], "action", "{bad}: {report}");
    }
    assert!(!agent.path().join("setup-profile.json").exists());

    let report = host
        .command("setup", &format!("qdrant own {}", &OTHER_ID[..12]))
        .unwrap()
        .unwrap();
    assert_eq!(report["steps"][0]["state"], "done", "{report}");
    let profile = std::fs::read_to_string(agent.path().join("setup-profile.json")).unwrap();
    assert!(profile.contains(OTHER_ID), "{profile}");

    let report = host.command("setup", "qdrant release").unwrap().unwrap();
    assert_eq!(report["steps"][0]["state"], "done");
    let profile = std::fs::read_to_string(agent.path().join("setup-profile.json")).unwrap();
    assert!(!profile.contains(OTHER_ID));
}
