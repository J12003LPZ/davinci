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

// ---- Task 2: the Qdrant client and the query path ----

use davinci_coding_agent::native_extensions::memory_backends::fake_ollama::FakeOllama;
use davinci_coding_agent::native_extensions::memory_backends::qdrant::{
    collection_name, QdrantProjection,
};
use davinci_coding_agent::native_extensions::memory_backends::{
    ProjectionError, ProjectionPoint, Secret, VectorProjection,
};
use davinci_coding_agent::native_extensions::vector_memory::EmbeddingIdentity;

const DIMS: usize = 3;

fn ollama() -> FakeOllama {
    FakeOllama::start(
        DIMS,
        vec![
            ("alpha".into(), vec![1.0, 0.0, 0.0]),
            ("beta".into(), vec![0.0, 1.0, 0.0]),
        ],
    )
}

fn qdrant_config(qdrant: &FakeQdrant, ollama: &FakeOllama) -> VectorMemoryConfig {
    let mut config = VectorMemoryConfig {
        qdrant_url: qdrant.url(),
        ollama_url: ollama.url(),
        embedding_dimensions: DIMS,
        embed_timeout_seconds: 10,
        request_timeout_seconds: 10,
        promotion: false,
        minimum_score: 0.0,
        ..VectorMemoryConfig::default()
    };
    config.projection.backend = MemoryProjectionBackend::Qdrant;
    config
}

fn note(text: &str) -> MemoryMessage {
    MemoryMessage {
        role: "user".into(),
        content: text.into(),
    }
}

#[test]
fn qdrant_query_filters_repo_scope_and_tombstones() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let mut memory =
        VectorMemory::with_config(dir.path().to_path_buf(), qdrant_config(&fake, &embedder));
    memory
        .index_messages(&[note("alpha deploy note"), note("beta cache note")])
        .unwrap();

    // The remote holds ids and numbers, never text.
    let collection = fake
        .collection(&collection_name(
            "davinci",
            &memory.repo_id,
            &EmbeddingIdentity::from_config(&memory.config),
        ))
        .expect("namespaced collection was created");
    assert_eq!(collection.points.len(), 2);
    for point in collection.points.values() {
        let keys: Vec<&String> = point.payload.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["record_id", "repo_id"]);
    }
    assert_eq!(fake.collection_names().len(), 1, "no global pi_memory_v1");

    let hits = memory.search("alpha", 3);
    assert_eq!(hits[0].record.text, "alpha deploy note");
    let queries: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|request| request.path.ends_with("/points/query"))
        .collect();
    assert_eq!(queries.len(), 1, "dense ranking came from the remote");
    assert_eq!(
        queries[0].body["filter"]["must"][0]["match"]["value"],
        memory.repo_id.as_str(),
        "the query is filtered to this repo"
    );

    // A tombstoned record still sits in the remote; it must not surface.
    let alpha = hits[0].record.id.clone();
    memory.add_tombstone(alpha.clone());
    let after = memory.search("alpha", 3);
    assert!(after.iter().all(|hit| hit.record.id != alpha));
}

#[test]
fn projection_queries_never_cross_repos_in_one_collection() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let config = qdrant_config(&fake, &embedder);
    let client = QdrantProjection::new(&config, "repo-x").unwrap();
    let point = |id: &str, repo: &str| ProjectionPoint {
        id: id.into(),
        vector: vec![1.0, 0.0, 0.0],
        repo_id: repo.into(),
    };
    client
        .upsert(&[point("id-x", "repo-x"), point("id-y", "repo-y")])
        .unwrap();
    let hits = client.query(&[1.0, 0.0, 0.0], "repo-x", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].id, "id-x",
        "non-UUID ids resolve back to the record id"
    );
    // Deleting is namespace-scoped too: repo-x cannot remove repo-y's point.
    client.delete(&["id-y".into()], "repo-x").unwrap();
    let all = fake.collection(client.collection()).unwrap();
    assert_eq!(all.points.len(), 2);
    client.delete(&["id-y".into()], "repo-y").unwrap();
    assert_eq!(
        fake.collection(client.collection()).unwrap().points.len(),
        1
    );
}

#[test]
fn a_foreign_collection_is_refused_and_left_alone() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let config = qdrant_config(&fake, &embedder);
    let client = QdrantProjection::new(&config, "repo-x").unwrap();
    // Same name, different shape: someone else's collection.
    fake.preload_collection(client.collection(), 512, "Dot");
    let error = client.ensure_ready().unwrap_err();
    assert!(matches!(error, ProjectionError::Refused(_)), "{error}");
    let upsert = client.upsert(&[ProjectionPoint {
        id: "a".into(),
        vector: vec![1.0, 0.0, 0.0],
        repo_id: "repo-x".into(),
    }]);
    assert!(matches!(upsert, Err(ProjectionError::Refused(_))));
    let kept = fake.collection(client.collection()).unwrap();
    assert_eq!((kept.size, kept.distance.as_str()), (512, "Dot"));
    assert!(kept.points.is_empty());
    assert!(
        fake.requests()
            .iter()
            .all(|request| request.method == "GET"),
        "only reads were sent to a foreign collection"
    );
}

#[test]
fn qdrant_not_ready_is_offline_and_creates_nothing() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    fake.set_not_ready(true);
    let client = QdrantProjection::new(&qdrant_config(&fake, &embedder), "r").unwrap();
    assert!(matches!(
        client.ensure_ready(),
        Err(ProjectionError::Offline(_))
    ));
    assert!(fake.collection_names().is_empty());
    assert_eq!(fake.request_count(), 1);
}

#[test]
fn api_key_goes_in_a_header_and_nowhere_else() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    fake.require_api_key("k-123-secret");
    let mut config = qdrant_config(&fake, &embedder);

    // No key: refused, not "offline" (retrying will not help).
    let client = QdrantProjection::new(&config, "r").unwrap();
    assert!(matches!(
        client.ensure_ready(),
        Err(ProjectionError::Refused(_))
    ));

    config.projection.api_key = Secret::new("k-123-secret");
    let dir = tempfile::tempdir().unwrap();
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config);
    memory.index_messages(&[note("alpha one")]).unwrap();
    assert!(fake
        .requests()
        .iter()
        .filter(|request| request.path != "/readyz")
        .any(|request| request.api_key.as_deref() == Some("k-123-secret")));
    assert_eq!(fake.collection_names().len(), 1);

    let status = memory.status().to_string();
    assert!(!status.contains("k-123-secret"));
    assert!(!format!("{:?}", memory.config).contains("k-123-secret"));
    assert!(!format!("{:?}", memory).contains("k-123-secret"));
}
