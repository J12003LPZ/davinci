//! Phase 2: automatic local memory, setup profile, health, supervisor.
//!
//! Everything here is offline: temp directories, loopback fakes and an
//! injected launcher. No real `ollama`, no download, no real user profile.

use davinci_coding_agent::native_extensions::{MemoryKind, VectorMemory, VectorMemoryConfig};
use std::fs;

fn memory_in(cwd: &std::path::Path) -> VectorMemory {
    let config = VectorMemoryConfig {
        // A closed loopback port: nothing here may wait on a server.
        ollama_url: "http://127.0.0.1:9".into(),
        ..VectorMemoryConfig::default()
    };
    let memory = VectorMemory::with_config(cwd.to_path_buf(), config);
    memory.mark_dense_offline();
    memory
}

fn learn(memory: &mut VectorMemory, text: &str) -> Result<(), String> {
    memory
        .index_learning_memory(text, MemoryKind::Fact, 0.8, 0.9, "session", 1, None)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[test]
fn corrupt_or_unreadable_local_store_not_emptied() {
    let dir = tempfile::tempdir().unwrap();
    let path = memory_in(dir.path()).local_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    // A record file that is not valid UTF-8: it cannot be read as text, but
    // it is somebody's data.
    let bytes: Vec<u8> = b"\xff\xfe{\"not\":\"utf8\"}\n".to_vec();
    fs::write(&path, &bytes).unwrap();

    let mut memory = memory_in(dir.path());
    let health = memory.store_health();
    assert!(health.is_unreadable(), "{health:?}");
    assert_eq!(health.code(), Some("LOCAL_STORE_UNREADABLE"));
    assert_eq!(
        memory.status()["storeHealth"]["code"],
        "LOCAL_STORE_UNREADABLE"
    );

    // Every write path refuses, and says why.
    let error = learn(&mut memory, "remember this").unwrap_err();
    assert!(error.contains("LOCAL_STORE_UNREADABLE"), "{error}");
    assert!(memory
        .index_messages(&[davinci_coding_agent::native_extensions::MemoryMessage {
            role: "user".into(),
            content: "We decided to always use the unreadable-store policy.".into(),
        }])
        .is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "the unreadable file changed"
    );

    // Once the file is readable again a reload recovers and writes work.
    fs::write(&path, "").unwrap();
    memory.reindex().unwrap();
    assert!(!memory.store_health().is_unreadable());
    learn(&mut memory, "remember this").unwrap();
}

#[test]
fn an_absent_store_is_missing_not_unreadable_and_is_created_on_first_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut memory = memory_in(dir.path());
    assert!(!memory.store_health().is_unreadable());
    assert_eq!(memory.store_health().code(), None);
    learn(&mut memory, "first record").unwrap();
    assert!(memory.local_path().exists());
}

#[test]
fn corrupt_json_lines_and_foreign_records_are_kept_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let path = memory_in(dir.path()).local_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "this line is not json\n").unwrap();
    let mut memory = memory_in(dir.path());
    assert!(!memory.store_health().is_unreadable());
    learn(&mut memory, "a new record").unwrap();
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.starts_with("this line is not json\n"), "{raw}");
    assert!(raw.contains("a new record"), "{raw}");
}

// ---- Task 1: setup profile and effective config -------------------------

use davinci_coding_agent::native_extensions::setup_profile::{
    load_effective_setup, EnabledSource, ProfileState, SetupProfile, CONFIG_CORRUPT, ENV_CONFLICT,
    PROFILE_CORRUPT,
};
use davinci_coding_agent::native_extensions::ConfigFileState;

fn no_env(_: &str) -> Option<String> {
    None
}

#[test]
fn setup_profile_defaults_precedence_and_roundtrip() {
    let agent = tempfile::tempdir().unwrap();

    // Defaults: local Ollama + embeddinggemma/768, autostart and safe repair
    // on, Qdrant off, nothing switched off.
    let effective = load_effective_setup(agent.path(), &no_env);
    assert_eq!(effective.profile_state, ProfileState::Missing);
    assert_eq!(effective.profile.memory.backend, "local");
    assert!(effective.profile.memory.autostart && effective.profile.memory.safe_repair);
    assert!(!effective.profile.memory.qdrant);
    assert!(effective.memory.enabled && effective.governor.enabled);
    assert_eq!(effective.memory.config.embedding_model, "embeddinggemma");
    assert_eq!(effective.memory.config.embedding_dimensions, 768);
    assert_eq!(effective.memory.enabled_source, EnabledSource::Default);
    assert!(effective.issues().is_empty());

    // Existing user configuration is kept, and the environment is shown as an
    // override instead of silently winning.
    fs::write(
        agent.path().join("vector-memory.json"),
        r#"{"embeddingModel":"my-model","embeddingDimensions":512}"#,
    )
    .unwrap();
    let env =
        |name: &str| (name == "PI_MEMORY_OLLAMA_URL").then(|| "http://127.0.0.1:1".to_string());
    let effective = load_effective_setup(agent.path(), &env);
    assert_eq!(effective.memory.config.embedding_model, "my-model");
    assert_eq!(effective.memory.config.embedding_dimensions, 512);
    assert_eq!(effective.memory.config.ollama_url, "http://127.0.0.1:1");
    let vars: Vec<_> = effective
        .memory
        .overrides
        .iter()
        .map(|o| o.var.as_str())
        .collect();
    assert_eq!(vars, ["PI_MEMORY_OLLAMA_URL"]);
    assert_eq!(effective.memory.enabled_source, EnabledSource::ConfigFile);

    // Round trip, schema-versioned, keeps the previous file as .bak.
    let mut profile = SetupProfile::default();
    profile.memory.autostart = false;
    profile.save(agent.path()).unwrap();
    profile.memory.safe_repair = false;
    profile.save(agent.path()).unwrap();
    let (loaded, state) = SetupProfile::load(agent.path());
    assert_eq!(state, ProfileState::Present);
    assert_eq!(loaded, profile);
    assert_eq!(loaded.schema, 1);
    assert!(agent.path().join("setup-profile.json.bak").exists());
}

#[test]
fn a_profile_switch_off_beats_the_environment_and_is_reported() {
    let agent = tempfile::tempdir().unwrap();
    let mut profile = SetupProfile::default();
    profile.memory.enabled = Some(false);
    profile.save(agent.path()).unwrap();
    let env = |name: &str| (name == "PI_MEMORY_ENABLED").then(|| "1".to_string());
    let effective = load_effective_setup(agent.path(), &env);
    assert!(!effective.memory.enabled && !effective.memory.config.enabled);
    assert_eq!(effective.memory.enabled_source, EnabledSource::Profile);
    assert!(effective
        .issues()
        .iter()
        .any(|issue| issue.code == ENV_CONFLICT));
}

#[test]
fn corrupt_config_and_profile_are_action_required_and_never_reset() {
    let agent = tempfile::tempdir().unwrap();
    // A corrupt memory config must not fall back to enabled defaults.
    fs::write(agent.path().join("vector-memory.json"), "{ not json").unwrap();
    fs::write(agent.path().join("token-governor.json"), "\u{0}\u{0}").unwrap();
    let effective = load_effective_setup(agent.path(), &no_env);
    assert!(matches!(effective.memory.file, ConfigFileState::Corrupt(_)));
    assert!(!effective.memory.enabled && !effective.governor.enabled);
    assert_eq!(effective.memory.enabled_source, EnabledSource::FailClosed);
    let codes: Vec<_> = effective.issues().iter().map(|issue| issue.code).collect();
    assert_eq!(codes, [CONFIG_CORRUPT, CONFIG_CORRUPT]);

    // A corrupt profile is reported and survives a save attempt untouched.
    let path = agent.path().join("setup-profile.json");
    fs::write(&path, "{ broken").unwrap();
    let effective = load_effective_setup(agent.path(), &no_env);
    assert!(effective
        .issues()
        .iter()
        .any(|issue| issue.code == PROFILE_CORRUPT));
    let error = SetupProfile::default().save(agent.path()).unwrap_err();
    assert!(error.contains(PROFILE_CORRUPT), "{error}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "{ broken");
}

#[test]
fn a_utf8_bom_in_a_config_file_is_tolerated() {
    let agent = tempfile::tempdir().unwrap();
    fs::write(
        agent.path().join("vector-memory.json"),
        "\u{feff}{\"enabled\":false}",
    )
    .unwrap();
    let effective = load_effective_setup(agent.path(), &no_env);
    assert_eq!(effective.memory.file, ConfigFileState::Present);
    assert!(!effective.memory.enabled);
    assert_eq!(effective.memory.enabled_source, EnabledSource::ConfigFile);
    assert!(effective.issues().is_empty());
}

// ---- Task 2: health from probes ------------------------------------------

use davinci_coding_agent::native_extensions::setup_health::{
    assess_memory, probe_memory_store, probe_ollama, EmbedOutcome, HealthState, MemoryFacts,
    OllamaProbe, ProbeTarget, EMBEDDING_MISMATCH, INSTALL_REQUIRED, MODEL_MISSING,
    PROJECTION_STALE, PROVIDER_OFFLINE,
};
use davinci_coding_agent::native_extensions::LocalStoreHealth;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A loopback Ollama: `/api/tags` lists `models`, `/api/embed` answers with
/// vectors of `dimensions`. Counts the requests it serves.
struct FakeOllama {
    url: String,
    requests: Arc<AtomicUsize>,
}

fn fake_ollama(models: &'static [&'static str], dimensions: usize) -> FakeOllama {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let counter = Arc::clone(&counter);
            std::thread::spawn(move || {
                let mut buffer = vec![0u8; 8192];
                let mut read = 0;
                // Read until the headers (and a short body) are in.
                while let Ok(n) = stream.read(&mut buffer[read..]) {
                    if n == 0 {
                        break;
                    }
                    read += n;
                    if buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") && read > 40 {
                        break;
                    }
                }
                counter.fetch_add(1, Ordering::SeqCst);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let body = if request.starts_with("GET /api/tags") {
                    let list: Vec<_> = models
                        .iter()
                        .map(|name| format!("{{\"name\":\"{name}\"}}"))
                        .collect();
                    format!("{{\"models\":[{}]}}", list.join(","))
                } else if request.starts_with("POST /api/embed") {
                    let vector: Vec<_> = (0..dimensions).map(|i| format!("{}.5", i % 7)).collect();
                    format!("{{\"embeddings\":[[{}]]}}", vector.join(","))
                } else {
                    "{}".to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            });
        }
    });
    FakeOllama { url, requests }
}

fn target(url: &str, dimensions: usize) -> ProbeTarget {
    ProbeTarget {
        url: url.into(),
        model: "embeddinggemma".into(),
        dimensions,
        timeout: Duration::from_secs(5),
    }
}

fn facts<'a>(probe: Option<&'a OllamaProbe>, store: &'a LocalStoreHealth) -> MemoryFacts<'a> {
    MemoryFacts {
        enabled: true,
        config_error: None,
        store,
        probe,
        ollama_installed: None,
        starting: false,
        model: "embeddinggemma",
        configured_dimensions: 768,
        records: 3,
        lag: 0,
    }
}

#[test]
fn health_does_not_claim_dense_ready_when_ollama_down() {
    let store = LocalStoreHealth::Ok;

    // Nothing listens: Degraded with PROVIDER_OFFLINE. The dense cooldown
    // flag is not an input at all.
    let down = probe_ollama(&target("http://127.0.0.1:9", 768));
    assert!(!down.reachable);
    let health = assess_memory(&facts(Some(&down), &store));
    assert_eq!(health.state, HealthState::Degraded);
    assert_eq!(health.code, Some(PROVIDER_OFFLINE));

    // No ollama on the machine either: install is required.
    let mut f = facts(Some(&down), &store);
    f.ollama_installed = Some(false);
    assert_eq!(assess_memory(&f).code, Some(INSTALL_REQUIRED));
    assert_eq!(assess_memory(&f).state, HealthState::ActionRequired);

    // Server up, model missing: ActionRequired MODEL_MISSING, and the probe
    // did not embed.
    let server = fake_ollama(&["other-model:latest"], 768);
    let missing = probe_ollama(&target(&server.url, 768));
    assert_eq!(missing.model_installed, Some(false));
    assert_eq!(missing.embed, EmbedOutcome::NotAttempted);
    let health = assess_memory(&facts(Some(&missing), &store));
    assert_eq!(
        (health.state, health.code),
        (HealthState::ActionRequired, Some(MODEL_MISSING))
    );
    // Listing the models is the only request: a probe never pulls.
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);

    // Everything works: Ready. A stale record makes it Degraded.
    let server = fake_ollama(&["embeddinggemma:latest"], 768);
    let up = probe_ollama(&target(&server.url, 768));
    assert!(matches!(
        up.embed,
        EmbedOutcome::Ok {
            dimensions: 768,
            ..
        }
    ));
    assert_eq!(
        assess_memory(&facts(Some(&up), &store)).state,
        HealthState::Ready
    );
    let mut f = facts(Some(&up), &store);
    f.lag = 2;
    let health = assess_memory(&f);
    assert_eq!(
        (health.state, health.code),
        (HealthState::Degraded, Some(PROJECTION_STALE))
    );

    // Disabled stays Disabled, whatever the probe says; no probe is Checking.
    let mut f = facts(Some(&up), &store);
    f.enabled = false;
    assert_eq!(assess_memory(&f).state, HealthState::Disabled);
    assert_eq!(
        assess_memory(&facts(None, &store)).state,
        HealthState::Checking
    );
}

#[test]
fn a_model_with_the_wrong_vector_size_is_an_embedding_mismatch() {
    let server = fake_ollama(&["embeddinggemma"], 512);
    let probe = probe_ollama(&target(&server.url, 768));
    assert_eq!(probe.embed, EmbedOutcome::Mismatch { actual: 512 });
    let store = LocalStoreHealth::Ok;
    let health = assess_memory(&facts(Some(&probe), &store));
    assert_eq!(
        (health.state, health.code),
        (HealthState::ActionRequired, Some(EMBEDDING_MISMATCH))
    );
    assert!(health
        .remediation
        .unwrap()
        .contains("\"embeddingDimensions\": 512"));
}

#[test]
fn an_unreadable_store_is_action_required_even_when_ollama_is_fine() {
    let store = LocalStoreHealth::Unreadable("cannot read records.jsonl".into());
    let health = assess_memory(&facts(None, &store));
    assert_eq!(health.code, Some("LOCAL_STORE_UNREADABLE"));
    assert_eq!(
        probe_memory_store(&store).state,
        HealthState::ActionRequired
    );
    assert_eq!(
        probe_memory_store(&LocalStoreHealth::Ok).state,
        HealthState::Ready
    );
}

// ---- Task 4: approved service supervisor ----------------------------------

use davinci_coding_agent::native_extensions::service_authority::{
    ServiceAction, ServiceConsentStore,
};
use davinci_coding_agent::native_extensions::service_supervisor::{
    ManagedServiceSupervisor, OllamaLauncher, OwnedProcess, CONSENT_REQUIRED,
    OLLAMA_IDENTITY_CHANGED,
};
use davinci_coding_agent::native_extensions::setup_profile::OllamaIdentity;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// Reserve a loopback port nothing listens on.
fn free_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

/// Serve a fake Ollama on exactly `url`.
fn serve_fake_ollama_at(url: &str, models: &'static [&'static str], dimensions: usize) {
    let address = url.trim_start_matches("http://").to_string();
    let listener = loop {
        match TcpListener::bind(&address) {
            Ok(listener) => break listener,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut buffer = vec![0u8; 8192];
                let mut read = 0;
                while let Ok(n) = stream.read(&mut buffer[read..]) {
                    if n == 0 {
                        break;
                    }
                    read += n;
                    if buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") && read > 40 {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let body = if request.starts_with("GET /api/tags") {
                    let list: Vec<_> = models
                        .iter()
                        .map(|name| format!("{{\"name\":\"{name}\"}}"))
                        .collect();
                    format!("{{\"models\":[{}]}}", list.join(","))
                } else if request.starts_with("POST /api/embed") {
                    let vector: Vec<_> = (0..dimensions).map(|i| format!("{}.5", i % 7)).collect();
                    format!("{{\"embeddings\":[[{}]]}}", vector.join(","))
                } else {
                    "{}".to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            });
        }
    });
}

struct FakeProcess {
    killed: Arc<AtomicBool>,
}

impl OwnedProcess for FakeProcess {
    fn id(&self) -> u32 {
        4242
    }
    fn has_exited(&mut self) -> bool {
        self.killed.load(Ordering::SeqCst)
    }
    fn kill(&mut self) {
        self.killed.store(true, Ordering::SeqCst);
    }
}

/// A launcher that never starts a real process: "starting" brings up a fake
/// server on the configured address.
struct FakeLauncher {
    path: Option<PathBuf>,
    version: String,
    url: String,
    spawns: Arc<AtomicUsize>,
    killed: Arc<AtomicBool>,
}

impl FakeLauncher {
    fn new(url: &str, path: &str, version: &str) -> Self {
        Self {
            path: Some(PathBuf::from(path)),
            version: version.into(),
            url: url.into(),
            spawns: Arc::new(AtomicUsize::new(0)),
            killed: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl OllamaLauncher for FakeLauncher {
    fn find(&self) -> Option<PathBuf> {
        self.path.clone()
    }
    fn version(&self, _: &Path) -> Option<String> {
        Some(self.version.clone())
    }
    fn spawn(&self, _: &Path, _: &Path) -> Result<Box<dyn OwnedProcess>, String> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        serve_fake_ollama_at(&self.url, &["embeddinggemma:latest"], 768);
        Ok(Box::new(FakeProcess {
            killed: Arc::clone(&self.killed),
        }))
    }
}

struct Machine {
    agent: tempfile::TempDir,
    url: String,
}

impl Machine {
    fn new() -> Self {
        let machine = Self {
            agent: tempfile::tempdir().unwrap(),
            url: free_url(),
        };
        fs::write(
            machine.agent.path().join("vector-memory.json"),
            format!("{{\"ollamaUrl\":\"{}\"}}", machine.url),
        )
        .unwrap();
        machine
    }
    fn consent(&self) {
        ServiceConsentStore::open(self.agent.path())
            .set(ServiceAction::StartLocalOllama, true)
            .unwrap();
    }
    fn approve(&self, path: &str, version: &str) {
        let mut profile = SetupProfile::default();
        profile.ollama = Some(OllamaIdentity {
            path: path.into(),
            version: version.into(),
        });
        profile.save(self.agent.path()).unwrap();
    }
    fn effective(
        &self,
    ) -> davinci_coding_agent::native_extensions::setup_profile::EffectiveSetupConfig {
        load_effective_setup(self.agent.path(), &no_env)
    }
}

#[test]
fn supervisor_starts_only_approved_ollama_once() {
    // Without consent: ActionRequired, nothing started.
    let machine = Machine::new();
    machine.approve("/usr/bin/ollama", "ollama version 0.5.0");
    let launcher = FakeLauncher::new(&machine.url, "/usr/bin/ollama", "ollama version 0.5.0");
    let spawns = Arc::clone(&launcher.spawns);
    let supervisor = ManagedServiceSupervisor::new(Box::new(launcher));
    let health = supervisor.reconcile(machine.agent.path(), &machine.effective());
    assert_eq!(
        (health.state, health.code),
        (HealthState::ActionRequired, Some(CONSENT_REQUIRED))
    );
    assert_eq!(spawns.load(Ordering::SeqCst), 0);

    // Consent but no approved executable recorded: still nothing started.
    let machine = Machine::new();
    machine.consent();
    let launcher = FakeLauncher::new(&machine.url, "/usr/bin/ollama", "ollama version 0.5.0");
    let spawns = Arc::clone(&launcher.spawns);
    let supervisor = ManagedServiceSupervisor::new(Box::new(launcher));
    let health = supervisor.reconcile(machine.agent.path(), &machine.effective());
    assert_eq!(health.code, Some(CONSENT_REQUIRED));
    assert_eq!(spawns.load(Ordering::SeqCst), 0);

    // A different binary than the approved one (a planted copy, or an update
    // nobody approved) is refused.
    for (found_path, found_version) in [
        ("/work/project/ollama", "ollama version 0.5.0"),
        ("/usr/bin/ollama", "ollama version 9.9.9"),
    ] {
        let machine = Machine::new();
        machine.consent();
        machine.approve("/usr/bin/ollama", "ollama version 0.5.0");
        let launcher = FakeLauncher::new(&machine.url, found_path, found_version);
        let spawns = Arc::clone(&launcher.spawns);
        let supervisor = ManagedServiceSupervisor::new(Box::new(launcher));
        let health = supervisor.reconcile(machine.agent.path(), &machine.effective());
        assert_eq!(
            (health.state, health.code),
            (HealthState::ActionRequired, Some(OLLAMA_IDENTITY_CHANGED)),
            "{found_path} {found_version}"
        );
        assert_eq!(spawns.load(Ordering::SeqCst), 0);
    }

    // Approved and consented: two simultaneous launches start it once.
    let machine = Machine::new();
    machine.consent();
    machine.approve("/usr/bin/ollama", "ollama version 0.5.0");
    let launcher = FakeLauncher::new(&machine.url, "/usr/bin/ollama", "ollama version 0.5.0");
    let spawns = Arc::clone(&launcher.spawns);
    let killed = Arc::clone(&launcher.killed);
    let supervisor = Arc::new(ManagedServiceSupervisor::new(Box::new(launcher)));
    let effective = machine.effective();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let supervisor = Arc::clone(&supervisor);
            let effective = effective.clone();
            let dir = machine.agent.path().to_path_buf();
            std::thread::spawn(move || supervisor.reconcile(&dir, &effective))
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap().state, HealthState::Ready);
    }
    assert_eq!(spawns.load(Ordering::SeqCst), 1, "started more than once");

    // A third reconcile finds it healthy and starts nothing.
    let health = supervisor.reconcile(machine.agent.path(), &effective);
    assert_eq!(health.state, HealthState::Ready);
    assert_eq!(spawns.load(Ordering::SeqCst), 1);

    // It stops only what it started.
    assert_eq!(supervisor.owned_pid(), Some(4242));
    assert!(supervisor.shutdown_owned());
    assert!(killed.load(Ordering::SeqCst));
    assert!(!supervisor.shutdown_owned());
}

#[test]
fn a_healthy_ollama_is_reused_and_never_stopped() {
    let machine = Machine::new();
    machine.consent();
    machine.approve("/usr/bin/ollama", "ollama version 0.5.0");
    serve_fake_ollama_at(&machine.url, &["embeddinggemma:latest"], 768);
    let launcher = FakeLauncher::new(&machine.url, "/usr/bin/ollama", "ollama version 0.5.0");
    let spawns = Arc::clone(&launcher.spawns);
    let killed = Arc::clone(&launcher.killed);
    let supervisor = ManagedServiceSupervisor::new(Box::new(launcher));
    let health = supervisor.reconcile(machine.agent.path(), &machine.effective());
    assert_eq!(health.state, HealthState::Ready);
    assert_eq!(spawns.load(Ordering::SeqCst), 0);
    assert_eq!(supervisor.owned_pid(), None);
    assert!(!supervisor.shutdown_owned());
    assert!(
        !killed.load(Ordering::SeqCst),
        "a server DaVinci did not start was killed"
    );
}

#[test]
fn a_disabled_memory_is_never_probed_or_started() {
    let machine = Machine::new();
    machine.consent();
    machine.approve("/usr/bin/ollama", "ollama version 0.5.0");
    let mut profile = SetupProfile::load(machine.agent.path()).0;
    profile.memory.enabled = Some(false);
    profile.save(machine.agent.path()).unwrap();
    let server = fake_ollama(&["embeddinggemma"], 768);
    fs::write(
        machine.agent.path().join("vector-memory.json"),
        format!("{{\"ollamaUrl\":\"{}\"}}", server.url),
    )
    .unwrap();
    let launcher = FakeLauncher::new(&machine.url, "/usr/bin/ollama", "ollama version 0.5.0");
    let spawns = Arc::clone(&launcher.spawns);
    let supervisor = ManagedServiceSupervisor::new(Box::new(launcher));
    let health = supervisor.reconcile(machine.agent.path(), &machine.effective());
    assert_eq!(health.state, HealthState::Disabled);
    assert_eq!(spawns.load(Ordering::SeqCst), 0);
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        0,
        "a disabled feature was probed"
    );
}
