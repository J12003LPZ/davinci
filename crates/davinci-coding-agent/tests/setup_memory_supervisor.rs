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

// ---- Task 5: consented model download ---------------------------------------

use davinci_coding_agent::native_extensions::service_authority::ServiceAuthorization;
use davinci_coding_agent::native_extensions::setup::{
    cancel_pull, memory_step, pull_progress, SetupContext, StepState,
};
use davinci_coding_agent::native_extensions::setup_actions::{
    approve_action, execute_approved_action, SetupAction,
};
use std::sync::Mutex;

/// The download slot is process-wide, so tests that download take turns.
static PULL_TURN: Mutex<()> = Mutex::new(());

/// A stateful fake Ollama: `/api/pull` either installs the model (`succeed`)
/// or ends its stream without a verdict. Counts pulls.
struct PullServer {
    url: String,
    pulls: Arc<AtomicUsize>,
}

fn pull_server(installed: bool, dimensions: usize, succeed: bool) -> PullServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let have = Arc::new(AtomicBool::new(installed));
    let pulls = Arc::new(AtomicUsize::new(0));
    let (have_t, pulls_t) = (Arc::clone(&have), Arc::clone(&pulls));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (have, pulls) = (Arc::clone(&have_t), Arc::clone(&pulls_t));
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
                    if have.load(Ordering::SeqCst) {
                        "{\"models\":[{\"name\":\"embeddinggemma:latest\"}]}".to_string()
                    } else {
                        "{\"models\":[]}".to_string()
                    }
                } else if request.starts_with("POST /api/pull") {
                    pulls.fetch_add(1, Ordering::SeqCst);
                    if succeed {
                        have.store(true, Ordering::SeqCst);
                        "{\"status\":\"pulling manifest\"}\n{\"status\":\"success\"}\n".to_string()
                    } else {
                        "{\"status\":\"pulling manifest\"}\n".to_string()
                    }
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
    PullServer { url, pulls }
}

fn memory_for(dir: &Path, url: &str) -> Arc<Mutex<VectorMemory>> {
    let config = VectorMemoryConfig {
        ollama_url: url.into(),
        embedding_dimensions: 8,
        request_timeout_seconds: 5,
        ..VectorMemoryConfig::default()
    };
    Arc::new(Mutex::new(VectorMemory::with_config(
        dir.to_path_buf(),
        config,
    )))
}

struct Ctx {
    agent: tempfile::TempDir,
    supervisor: ManagedServiceSupervisor,
}

impl Ctx {
    fn new() -> Self {
        Self {
            agent: tempfile::tempdir().unwrap(),
            // No ollama on this "machine": nothing may be started.
            supervisor: ManagedServiceSupervisor::new(Box::new(NoOllamaHere)),
        }
    }
    fn get(&self) -> SetupContext<'_> {
        SetupContext {
            agent_dir: self.agent.path(),
            supervisor: &self.supervisor,
        }
    }
}

struct NoOllamaHere;
impl OllamaLauncher for NoOllamaHere {
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

fn wait_for_pull() {
    let started = std::time::Instant::now();
    while !pull_progress().is_some_and(|progress| progress.finished) {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the download never finished"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn ollama_model_present_skips_pull_missing_requires_consent() {
    let _turn = PULL_TURN.lock().unwrap_or_else(|e| e.into_inner());
    let project = tempfile::tempdir().unwrap();

    // The model is already there: nothing is downloaded, even on apply.
    let present = pull_server(true, 8, true);
    let ctx = Ctx::new();
    let memory = memory_for(project.path(), &present.url);
    let step = memory_step(&memory, true, &ctx.get());
    assert!(
        matches!(step.state, StepState::Ready | StepState::Done),
        "{}",
        step.detail
    );
    assert_eq!(present.pulls.load(Ordering::SeqCst), 0);

    // The model is missing and nobody said yes: a check and an apply both
    // leave it alone and say how to approve.
    let missing = pull_server(false, 8, true);
    let ctx = Ctx::new();
    let memory = memory_for(project.path(), &missing.url);
    let checked = memory_step(&memory, false, &ctx.get());
    assert_eq!(checked.code, Some("MODEL_MISSING"));
    let refused = memory_step(&memory, true, &ctx.get());
    assert_eq!(refused.state, StepState::Action);
    assert_eq!(refused.code, Some("CONSENT_REQUIRED"));
    assert!(refused
        .action
        .unwrap()
        .contains("/setup approve download-model"));
    assert_eq!(
        missing.pulls.load(Ordering::SeqCst),
        0,
        "downloaded without approval"
    );
    assert!(
        ServiceAuthorization::check(ctx.agent.path(), ServiceAction::DownloadEmbeddingModel)
            .is_err()
    );
    // The action itself refuses too.
    let refused = execute_approved_action(SetupAction::DownloadModel, &memory, &ctx.get());
    assert_eq!(refused.code, Some("CONSENT_REQUIRED"));
    assert_eq!(missing.pulls.load(Ordering::SeqCst), 0);

    // After the user's yes, the download runs once and the model is there.
    approve_action(SetupAction::DownloadModel, &ctx.get()).unwrap();
    let started = memory_step(&memory, true, &ctx.get());
    assert_eq!(started.state, StepState::Running, "{}", started.detail);
    wait_for_pull();
    assert_eq!(missing.pulls.load(Ordering::SeqCst), 1);
    let done = memory_step(&memory, true, &ctx.get());
    assert!(
        matches!(done.state, StepState::Ready | StepState::Done),
        "{}",
        done.detail
    );
    assert_eq!(missing.pulls.load(Ordering::SeqCst), 1, "pulled again");
}

#[test]
fn an_interrupted_download_is_reported_not_silently_restarted() {
    let _turn = PULL_TURN.lock().unwrap_or_else(|e| e.into_inner());
    let project = tempfile::tempdir().unwrap();
    // The stream ends without a verdict.
    let server = pull_server(false, 8, false);
    let ctx = Ctx::new();
    approve_action(SetupAction::DownloadModel, &ctx.get()).unwrap();
    let memory = memory_for(project.path(), &server.url);

    assert_eq!(
        memory_step(&memory, true, &ctx.get()).state,
        StepState::Running
    );
    wait_for_pull();
    assert_eq!(server.pulls.load(Ordering::SeqCst), 1);

    // The next apply reports the unknown outcome and does NOT pull again.
    let report = memory_step(&memory, true, &ctx.get());
    assert_eq!(report.state, StepState::Action, "{}", report.detail);
    assert!(
        report.detail.contains("did not finish"),
        "{}",
        report.detail
    );
    assert_eq!(server.pulls.load(Ordering::SeqCst), 1, "restarted silently");

    // Only the explicit run after that tries again.
    assert_eq!(
        memory_step(&memory, true, &ctx.get()).state,
        StepState::Running
    );
    wait_for_pull();
    assert_eq!(server.pulls.load(Ordering::SeqCst), 2);
    assert!(
        !cancel_pull(),
        "nothing is running, so there is nothing to cancel"
    );
}

// ---- Task 6: restart recovery and deduplicated notices ----------------------

use davinci_coding_agent::native_extensions::service_notifications::{
    backoff_delay, recovery_round, recovery_wanted, IncidentTracker, NoticeKind, RecoveryState,
    COOLDOWN,
};

/// Fails to start `fail_first` times, then works.
struct FlakyLauncher {
    inner: FakeLauncher,
    fail_first: usize,
    attempts: Arc<AtomicUsize>,
}

impl OllamaLauncher for FlakyLauncher {
    fn find(&self) -> Option<PathBuf> {
        self.inner.find()
    }
    fn version(&self, program: &Path) -> Option<String> {
        self.inner.version(program)
    }
    fn spawn(&self, program: &Path, agent_dir: &Path) -> Result<Box<dyn OwnedProcess>, String> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        if attempt < self.fail_first {
            return Err("the process exited immediately".into());
        }
        self.inner.spawn(program, agent_dir)
    }
}

#[test]
fn backoff_is_one_two_four_then_the_cooldown() {
    let seconds: Vec<u64> = (1..=6).map(|n| backoff_delay(n).as_secs()).collect();
    assert_eq!(seconds, [1, 2, 4, 120, 120, 120]);
    assert_eq!(backoff_delay(4), COOLDOWN);
}

#[test]
fn cold_relaunch_restores_ollama_and_deduplicates_alerts() {
    let machine = Machine::new();
    machine.consent();
    machine.approve("/usr/bin/ollama", "ollama version 0.5.0");
    assert!(recovery_wanted(machine.agent.path()));

    // A record written before the "reboot". Ollama is down, so search can
    // only use keywords, and it still finds the record.
    let project = tempfile::tempdir().unwrap();
    let mut memory = memory_in(project.path());
    learn(
        &mut memory,
        "We decided to use the tokio runtime everywhere.",
    )
    .unwrap();
    let hits = memory.search("tokio runtime", 3);
    assert!(
        !hits.is_empty(),
        "keyword search found nothing while Ollama is down"
    );

    let attempts = Arc::new(AtomicUsize::new(0));
    let launcher = FlakyLauncher {
        inner: FakeLauncher::new(&machine.url, "/usr/bin/ollama", "ollama version 0.5.0"),
        fail_first: 3,
        attempts: Arc::clone(&attempts),
    };
    let supervisor = ManagedServiceSupervisor::new(Box::new(launcher));
    let mut state = RecoveryState::default();
    let dir = machine.agent.path();

    // Three failed starts: one incident notice, then silence; the waits are
    // 1s, 2s, 4s.
    let first = recovery_round(&mut state, &supervisor, dir);
    assert_eq!(first.health.state, HealthState::Failed);
    assert_eq!(
        first.notice.as_ref().map(|n| n.kind.clone()),
        Some(NoticeKind::Incident)
    );
    assert_eq!(first.retry_in.map(|d| d.as_secs()), Some(1));
    let second = recovery_round(&mut state, &supervisor, dir);
    assert!(
        second.notice.is_none(),
        "the same failure was announced twice"
    );
    assert_eq!(second.retry_in.map(|d| d.as_secs()), Some(2));
    let third = recovery_round(&mut state, &supervisor, dir);
    assert!(third.notice.is_none());
    assert_eq!(third.retry_in.map(|d| d.as_secs()), Some(4));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);

    // The fourth attempt works: Ollama is back, announced exactly once.
    let fourth = recovery_round(&mut state, &supervisor, dir);
    assert_eq!(fourth.health.state, HealthState::Ready);
    assert_eq!(
        fourth.notice.as_ref().map(|n| n.kind.clone()),
        Some(NoticeKind::Recovered)
    );
    assert_eq!(fourth.retry_in, None);
    let after = recovery_round(&mut state, &supervisor, dir);
    assert!(after.notice.is_none(), "recovery announced twice");
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        4,
        "started again although it was healthy"
    );

    // Once the failures reach the cooldown, the wait stays at 120s.
    let mut state = RecoveryState::default();
    state.failures = 3;
    assert_eq!(backoff_delay(state.failures + 1), COOLDOWN);
}

#[test]
fn a_different_failure_is_announced_again_but_a_repeat_is_not() {
    use davinci_coding_agent::native_extensions::setup_health::ComponentHealth;
    let mut tracker = IncidentTracker::default();
    let failed = ComponentHealth::new("ollama", HealthState::Failed, "did not come up")
        .with_code("SERVICE_START_FAILED");
    assert!(tracker.observe(&failed).is_some());
    assert!(tracker.observe(&failed).is_none());
    let needs_user = ComponentHealth::new("ollama", HealthState::ActionRequired, "no consent")
        .with_code("CONSENT_REQUIRED");
    assert!(tracker.observe(&needs_user).is_some());
    // Checking or starting in between neither announces nor clears.
    assert!(tracker
        .observe(&ComponentHealth::new(
            "ollama",
            HealthState::Starting,
            "starting"
        ))
        .is_none());
    assert!(tracker.open_incident().is_some());
    let ready = ComponentHealth::new("ollama", HealthState::Ready, "answers");
    assert_eq!(
        tracker.observe(&ready).map(|n| n.kind),
        Some(NoticeKind::Recovered)
    );
    assert!(tracker.observe(&ready).is_none());
}

#[test]
fn recovery_is_not_wanted_until_the_user_set_it_up_and_never_when_disabled() {
    // Nothing approved: no thread, no probe, no notice.
    let machine = Machine::new();
    assert!(!recovery_wanted(machine.agent.path()));
    machine.consent();
    assert!(
        !recovery_wanted(machine.agent.path()),
        "no approved executable yet"
    );
    machine.approve("/usr/bin/ollama", "ollama version 0.5.0");
    assert!(recovery_wanted(machine.agent.path()));
    // Switched off: silent again, however much else is in place.
    let mut profile = SetupProfile::load(machine.agent.path()).0;
    profile.memory.enabled = Some(false);
    profile.save(machine.agent.path()).unwrap();
    assert!(!recovery_wanted(machine.agent.path()));
}

// ---- Task 7: safe projection repair -----------------------------------------

/// An embedding endpoint whose vector size can be changed while it runs.
struct EmbedServer {
    url: String,
    dimensions: Arc<AtomicUsize>,
}

fn embed_server(dimensions: usize) -> EmbedServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let dims = Arc::new(AtomicUsize::new(dimensions));
    let shared = Arc::clone(&dims);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let dims = Arc::clone(&shared);
            std::thread::spawn(move || {
                let mut buffer = vec![0u8; 65536];
                let mut read = 0;
                let mut expected = None;
                while let Ok(n) = stream.read(&mut buffer[read..]) {
                    if n == 0 {
                        break;
                    }
                    read += n;
                    let text = String::from_utf8_lossy(&buffer[..read]).to_string();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let length = text[..split]
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        expected = Some(split + 4 + length);
                    }
                    if expected.is_some_and(|total| read >= total) {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&buffer[..read]).to_string();
                let body = text.split("\r\n\r\n").nth(1).unwrap_or_default();
                let inputs = serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|value| value["input"].as_array().map(Vec::len))
                    .unwrap_or(1);
                let reply = if text.starts_with("GET /api/tags") {
                    "{\"models\":[{\"name\":\"embeddinggemma:latest\"}]}".to_string()
                } else {
                    let size = dims.load(Ordering::SeqCst);
                    let vector: Vec<_> = (0..size).map(|i| format!("{}.25", i % 5)).collect();
                    let vectors = vec![format!("[{}]", vector.join(",")); inputs];
                    format!("{{\"embeddings\":[{}]}}", vectors.join(","))
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                let _ = stream.write_all(response.as_bytes());
            });
        }
    });
    EmbedServer {
        url,
        dimensions: dims,
    }
}

fn memory_with_server(cwd: &Path, url: &str, model: &str) -> VectorMemory {
    VectorMemory::with_config(
        cwd.to_path_buf(),
        VectorMemoryConfig {
            ollama_url: url.into(),
            embedding_model: model.into(),
            embedding_dimensions: 8,
            ..VectorMemoryConfig::default()
        },
    )
}

fn record_ids(memory: &VectorMemory) -> Vec<String> {
    fs::read_to_string(memory.local_path())
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|value| value["id"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn repair_rebuilds_only_stale_vectors_without_deleting_records() {
    let project = tempfile::tempdir().unwrap();
    let server = embed_server(8);

    // Five records written while the service was down: no vectors yet.
    let mut down = memory_in(project.path());
    for n in 0..5 {
        learn(&mut down, &format!("fact number {n} about repair")).unwrap();
    }
    let mut memory = memory_with_server(project.path(), &server.url, "embeddinggemma");
    let ids = record_ids(&memory);
    assert_eq!(ids.len(), 5);
    // One record is retracted: it must stay in the file and stay unembedded.
    memory.tombstones.insert(ids[0].clone());

    // The model answers with the wrong size: nothing is dropped or changed.
    server.dimensions.store(12, Ordering::SeqCst);
    let before = fs::read(memory.local_path()).unwrap();
    let blocked = memory.repair_projection(2);
    assert_eq!(blocked.blocked, Some("EMBEDDING_MISMATCH"), "{blocked:?}");
    assert_eq!(blocked.reembedded, 0);
    assert_eq!(blocked.records_before, blocked.records_after);
    assert_eq!(
        fs::read(memory.local_path()).unwrap(),
        before,
        "a blocked repair wrote the store"
    );

    // Fixed: bounded batches of two, each safe to repeat.
    server.dimensions.store(8, Ordering::SeqCst);
    let first = memory.repair_projection(2);
    assert_eq!(
        (first.reembedded, first.error.clone()),
        (2, None),
        "{first:?}"
    );
    assert_eq!(first.stale_before, 4, "the retracted record is not counted");
    assert_eq!(first.remaining, 2);
    let second = memory.repair_projection(2);
    assert_eq!(second.reembedded, 2);
    assert!(second.is_complete(), "{second:?}");
    let again = memory.repair_projection(2);
    assert_eq!((again.reembedded, again.remaining), (0, 0));

    // Every record is still in the file, the retracted one included.
    assert_eq!(record_ids(&memory), ids);
    assert_eq!(second.records_before, 5);
    assert_eq!(second.records_after, 5);
}

#[test]
fn vectors_from_another_model_are_re_embedded_and_no_record_is_lost() {
    let project = tempfile::tempdir().unwrap();
    let server = embed_server(8);
    let mut first = memory_with_server(project.path(), &server.url, "model-a");
    learn(&mut first, "alpha fact kept across models").unwrap();
    learn(&mut first, "beta fact kept across models").unwrap();
    assert_eq!(first.repair_projection(8).remaining, 0);
    let ids = record_ids(&first);

    // A different embedding model: the old vectors no longer fit.
    let mut second = memory_with_server(project.path(), &server.url, "model-b");
    let receipt = second.repair_projection(8);
    assert_eq!(receipt.stale_before, 2, "{receipt:?}");
    assert_eq!(receipt.reembedded, 2);
    assert!(receipt.is_complete());
    assert_eq!(record_ids(&second), ids);
}

#[test]
fn repair_with_the_service_down_changes_nothing() {
    let project = tempfile::tempdir().unwrap();
    let mut memory = memory_in(project.path());
    learn(&mut memory, "a record waiting for its vector").unwrap();
    let before = fs::read(memory.local_path()).unwrap();
    let receipt = memory.repair_projection(8);
    assert_eq!(receipt.blocked, Some("PROVIDER_OFFLINE"), "{receipt:?}");
    assert_eq!(fs::read(memory.local_path()).unwrap(), before);
}
