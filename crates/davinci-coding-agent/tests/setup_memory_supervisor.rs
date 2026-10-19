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
