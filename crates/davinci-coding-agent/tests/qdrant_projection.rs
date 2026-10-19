//! Optional Qdrant projection: local mode contacts nothing, remote mode is a
//! rebuildable copy of the local store. Fakes only; no Docker, no real Qdrant.

use davinci_coding_agent::native_extensions::memory_backends::fake_qdrant::FakeQdrant;
use davinci_coding_agent::native_extensions::memory_backends::MemoryProjectionBackend;
use davinci_coding_agent::native_extensions::setup_profile::{
    load_effective_setup, QdrantChoices, SetupProfile,
};
use davinci_coding_agent::native_extensions::vector_memory::{
    MemoryMessage, VectorMemory, VectorMemoryConfig,
};

fn no_env(_: &str) -> Option<String> {
    None
}

fn local_memory(dir: &std::path::Path, qdrant_url: &str) -> VectorMemory {
    let config = VectorMemoryConfig {
        qdrant_url: qdrant_url.to_string(),
        // Nothing listens here: embedding falls back to lexical, quickly.
        ollama_url: "http://127.0.0.1:9".into(),
        embed_timeout_seconds: 1,
        request_timeout_seconds: 1,
        promotion: false,
        ..VectorMemoryConfig::default()
    };
    VectorMemory::with_config(dir.to_path_buf(), config)
}

#[test]
fn local_mode_never_contacts_qdrant() {
    let fake = FakeQdrant::start();
    let dir = tempfile::tempdir().unwrap();
    let mut memory = local_memory(dir.path(), &fake.url());
    assert_eq!(
        memory.config.projection.backend,
        MemoryProjectionBackend::Local
    );
    assert!(!memory.remote_projection_enabled());

    memory
        .index_messages(&[MemoryMessage {
            role: "user".into(),
            content: "the scheduler uses a bounded graph queue".into(),
        }])
        .unwrap();
    let hits = memory.search("scheduler", 3);
    assert_eq!(hits.len(), 1);
    let status = memory.status();
    assert_eq!(status["projectionBackend"], "local");
    assert_eq!(status["remoteProjectionEnabled"], false);
    // The explicit remote entry point is a no-op in local mode too.
    memory.upsert_remote(&[]).unwrap();
    memory
        .upsert_remote(&hits.into_iter().map(|hit| hit.record).collect::<Vec<_>>())
        .unwrap();

    assert_eq!(fake.request_count(), 0, "local mode made a Qdrant request");
}

#[test]
fn profile_choice_selects_qdrant_and_environment_can_turn_it_off() {
    let agent = tempfile::tempdir().unwrap();
    let mut profile = SetupProfile::default();
    profile.memory.backend = MemoryProjectionBackend::Qdrant;
    profile.memory.qdrant_setup = QdrantChoices {
        url: Some("http://127.0.0.1:7777".into()),
        allow_remote: false,
    };
    profile.save(agent.path()).unwrap();

    let effective = load_effective_setup(agent.path(), &no_env);
    assert_eq!(
        effective.memory.config.projection.backend,
        MemoryProjectionBackend::Qdrant
    );
    assert_eq!(effective.memory.config.qdrant_url, "http://127.0.0.1:7777");

    let off = |name: &str| (name == "PI_MEMORY_BACKEND").then(|| "local".to_string());
    let effective = load_effective_setup(agent.path(), &off);
    assert_eq!(
        effective.memory.config.projection.backend,
        MemoryProjectionBackend::Local
    );
}

#[test]
fn default_profile_and_config_are_local() {
    let agent = tempfile::tempdir().unwrap();
    let effective = load_effective_setup(agent.path(), &no_env);
    assert_eq!(
        effective.memory.config.projection.backend,
        MemoryProjectionBackend::Local
    );
}
