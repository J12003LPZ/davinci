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
        ..QdrantChoices::default()
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

// ---- Task 3: lag, outage fallback and reconcile ----

use davinci_coding_agent::native_extensions::memory_backends::{QDRANT_OFFLINE, QDRANT_REFUSED};

fn instant_retry(mut config: VectorMemoryConfig) -> VectorMemoryConfig {
    config.projection.offline_backoff_seconds = 0;
    config
}

fn remote_points(fake: &FakeQdrant, memory: &VectorMemory) -> usize {
    fake.collection(&memory.remote_collection_name())
        .map(|collection| collection.points.len())
        .unwrap_or(0)
}

#[test]
fn qdrant_rebuild_and_outage_keep_local_memory() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config.clone());
    memory.index_messages(&[note("alpha one")]).unwrap();
    assert_eq!(remote_points(&fake, &memory), 1);
    assert_eq!(memory.remote_projection_lag(), 0);

    // Outage: indexing still stores the record locally; the lag is counted.
    fake.set_down(true);
    memory.index_messages(&[note("beta two")]).unwrap();
    assert_eq!(memory.records().len(), 2, "no record was lost");
    assert_eq!(memory.remote_projection_lag(), 1);
    let local_file = std::fs::read_to_string(memory.local_path()).unwrap();
    assert!(local_file.contains("beta two"));

    // Search still answers, from the local index, and says why.
    let result = memory
        .search_tool(&serde_json::json!({"query": "beta"}))
        .unwrap();
    assert!(result.content.contains("beta two"), "{}", result.content);
    assert!(
        result.content.contains(QDRANT_OFFLINE),
        "{}",
        result.content
    );
    assert_eq!(memory.status()["projection"]["code"], QDRANT_OFFLINE);
    let offline_receipt = memory.reconcile_remote(10);
    assert_eq!(offline_receipt.code, Some(QDRANT_OFFLINE));
    assert_eq!((offline_receipt.indexed, offline_receipt.lag), (0, 1));
    assert_eq!(offline_receipt.failures, 1);

    // Recovery: one bounded pass catches up, a second finds nothing to do.
    fake.set_down(false);
    let receipt = memory.reconcile_remote(10);
    assert_eq!((receipt.indexed, receipt.lag, receipt.failures), (1, 0, 0));
    assert!(receipt.is_complete());
    assert_eq!(remote_points(&fake, &memory), 2);
    let before = fake.request_count();
    assert_eq!(memory.reconcile_remote(10).indexed, 0);
    assert_eq!(fake.request_count(), before, "nothing left means no calls");
    assert!(memory.status()["projection"]["code"].is_null());

    // A tombstone removes the remote point; the local record stays.
    let alpha = memory.search("alpha", 1)[0].record.id.clone();
    memory.add_tombstone(alpha.clone());
    assert_eq!(memory.remote_projection_lag(), 1);
    let receipt = memory.reconcile_remote(10);
    assert_eq!((receipt.removed, receipt.lag), (1, 0));
    assert_eq!(remote_points(&fake, &memory), 1);
    assert!(memory.records().iter().any(|record| record.id == alpha));

    // The remote loses its data: a rebuild restores it from local records
    // without touching a collection that belongs to someone else.
    fake.preload_collection("someone-elses", DIMS, "Cosine");
    fake.wipe_points();
    let rebuilt = memory.rebuild_remote();
    assert_eq!(rebuilt.failures, 0, "{rebuilt:?}");
    assert_eq!(remote_points(&fake, &memory), 1);
    assert!(fake.collection("someone-elses").unwrap().points.is_empty());
    assert!(fake
        .requests()
        .iter()
        .all(|request| !request.path.contains("someone-elses")));
}

#[test]
fn reconcile_is_bounded_and_survives_a_restart() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    fake.set_down(true);
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config.clone());
    let notes: Vec<MemoryMessage> = (0..5).map(|i| note(&format!("alpha note {i}"))).collect();
    memory.index_messages(&notes).unwrap();
    assert_eq!(memory.remote_projection_lag(), 5);

    fake.set_down(false);
    let first = memory.reconcile_remote(2);
    assert_eq!((first.indexed, first.lag), (2, 3));
    assert!(!first.is_complete());
    let second = memory.reconcile_remote(2);
    assert_eq!((second.indexed, second.lag), (2, 1));
    // Re-sending is harmless: point ids are stable.
    assert_eq!(remote_points(&fake, &memory), 4);
    let third = memory.reconcile_remote(2);
    assert_eq!((third.indexed, third.lag), (1, 0));
    assert_eq!(remote_points(&fake, &memory), 5);

    // A new process reads the sync file and does not re-send.
    let again = VectorMemory::with_config(dir.path().to_path_buf(), config);
    assert_eq!(again.remote_projection_lag(), 0);
    let before = fake.request_count();
    assert!(again.reconcile_remote(10).is_complete());
    assert_eq!(fake.request_count(), before);
}

#[test]
fn an_outage_costs_one_attempt_until_the_backoff_ends() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let mut config = qdrant_config(&fake, &embedder);
    config.projection.offline_backoff_seconds = 300;
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config);
    memory.index_messages(&[note("alpha one")]).unwrap();
    fake.set_down(true);
    memory.index_messages(&[note("beta two")]).unwrap();
    let after_failure = fake.request_count();
    for _ in 0..3 {
        let hits = memory.search("alpha", 2);
        assert!(!hits.is_empty(), "local search still answers");
    }
    memory.index_messages(&[note("alpha three")]).unwrap();
    assert_eq!(
        fake.request_count(),
        after_failure,
        "no request is made while backing off"
    );
    assert_eq!(memory.status()["projection"]["code"], QDRANT_OFFLINE);
}

#[test]
fn a_refused_remote_is_reported_as_refused_not_offline() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    fake.require_api_key("needed");
    let dir = tempfile::tempdir().unwrap();
    let mut memory = VectorMemory::with_config(
        dir.path().to_path_buf(),
        instant_retry(qdrant_config(&fake, &embedder)),
    );
    memory.index_messages(&[note("alpha one")]).unwrap();
    assert_eq!(memory.records().len(), 1);
    let receipt = memory.reconcile_remote(10);
    assert_eq!(receipt.code, Some(QDRANT_REFUSED));
    assert_eq!(memory.status()["projection"]["state"], "refused");
}

#[test]
fn setup_repair_catches_the_remote_up_and_says_so() {
    use davinci_coding_agent::native_extensions::setup_actions::repair_step;
    use std::sync::{Arc, Mutex};
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    fake.set_down(true);
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config);
    memory.index_messages(&[note("alpha one")]).unwrap();
    let shared = Arc::new(Mutex::new(memory));

    let step = repair_step(&shared);
    assert!(step.detail.contains(QDRANT_OFFLINE), "{}", step.detail);
    assert!(step.detail.contains("nothing was lost"), "{}", step.detail);

    fake.set_down(false);
    let step = repair_step(&shared);
    assert!(
        step.detail.contains("Qdrant copy in sync"),
        "{}",
        step.detail
    );
    assert_eq!(shared.lock().unwrap().remote_projection_lag(), 0);
}
