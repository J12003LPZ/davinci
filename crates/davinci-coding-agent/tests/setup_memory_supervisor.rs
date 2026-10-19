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
