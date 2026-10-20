//! Optional Qdrant projection: local mode contacts nothing, remote mode is a
//! rebuildable copy of the local store. Fakes only; no Docker, no real Qdrant.

use davinci_coding_agent::native_extensions::memory_backends::fake_qdrant::dead_url;
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
        ollama_url: dead_url(),
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
    assert!(!format!("{memory:?}").contains("k-123-secret"));
}

// ---- Task 3: lag, outage fallback and reconcile ----

use davinci_coding_agent::native_extensions::memory_backends::{QDRANT_OFFLINE, QDRANT_REFUSED};

fn instant_retry(mut config: VectorMemoryConfig) -> VectorMemoryConfig {
    config.projection.offline_backoff_seconds = 0;
    config
}

/// After `before` requests, the remote saw reads and the point count that
/// checks the ledger against it, but no write or delete.
fn assert_only_reads(fake: &FakeQdrant, before: usize, why: &str) {
    let later = &fake.requests()[before..];
    assert!(
        later
            .iter()
            .all(|request| request.method == "GET" || request.path.contains("/points/count")),
        "{why}: {later:?}"
    );
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
    assert_only_reads(&fake, before, "nothing left means nothing is written");
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
    assert_only_reads(&fake, before, "a restart does not re-send");
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

// ---- critic fixes: ownership marker, key transport, batch clamp ----

fn one_point(id: &str) -> ProjectionPoint {
    ProjectionPoint {
        id: id.into(),
        vector: vec![1.0, 0.0, 0.0],
        repo_id: "repo-x".into(),
    }
}

#[test]
fn a_collection_of_the_right_shape_without_the_marker_is_never_adopted() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let client = QdrantProjection::new(&qdrant_config(&fake, &embedder), "repo-x").unwrap();
    // Someone else made a collection with our name and our shape.
    fake.preload_collection(client.collection(), DIMS, "Cosine");

    let error = client.ensure_ready().unwrap_err();
    assert!(matches!(error, ProjectionError::Refused(_)), "{error}");
    assert!(error.to_string().contains("did not create"), "{error}");
    assert!(matches!(
        client.upsert(&[one_point("a")]),
        Err(ProjectionError::Refused(_))
    ));
    assert!(matches!(
        client.delete(&["a".into()], "repo-x"),
        Err(ProjectionError::Refused(_))
    ));
    assert!(matches!(
        client.clear_namespace("repo-x"),
        Err(ProjectionError::Refused(_))
    ));
    assert!(
        fake.requests()
            .iter()
            .filter(|request| request.path != "/readyz")
            .all(|request| request.method == "GET"),
        "only reads were sent: {:?}",
        fake.requests()
    );
    assert!(fake
        .collection(client.collection())
        .unwrap()
        .points
        .is_empty());
}

#[test]
fn a_collection_davinci_made_is_marked_and_the_marker_outlives_every_clear() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let client = QdrantProjection::new(&qdrant_config(&fake, &embedder), "repo-x").unwrap();
    client.upsert(&[one_point("a")]).unwrap();
    let marker = |fake: &FakeQdrant| fake.collection(client.collection()).unwrap().marker;
    assert_eq!(
        marker(&fake)
            .expect("created collections carry a marker")
            .payload["davinci_marker"],
        "davinci-memory"
    );
    // The marker has no repo id, so no query returns it and no clear removes it.
    let hits = client.query(&[1.0, 0.0, 0.0], "repo-x", 10).unwrap();
    assert_eq!(hits.len(), 1);
    client.clear_namespace("repo-x").unwrap();
    assert!(marker(&fake).is_some());
    // A second client (a restart) recognises its own collection.
    let again = QdrantProjection::new(&qdrant_config(&fake, &embedder), "repo-x").unwrap();
    again.upsert(&[one_point("b")]).unwrap();
}

#[test]
fn the_api_key_never_travels_over_plain_http_to_another_host() {
    let embedder = ollama();
    let fake = FakeQdrant::start();
    let mut config = qdrant_config(&fake, &embedder);
    config.projection.api_key = Secret::new("k-123-secret");
    config.projection.allow_remote = true;

    config.qdrant_url = "http://qdrant.example.com:6333".into();
    let error = QdrantProjection::new(&config, "r").err().expect("refused");
    assert!(matches!(error, ProjectionError::Refused(_)));
    assert!(error.to_string().contains("plain http"), "{error}");
    assert!(!error.to_string().contains("k-123-secret"));

    // https to another host, http on this machine, and no key over http are fine.
    config.qdrant_url = "HTTPS://qdrant.example.com:6333".into();
    assert!(QdrantProjection::new(&config, "r").is_ok());
    config.qdrant_url = fake.url();
    assert!(QdrantProjection::new(&config, "r").is_ok());
    config.qdrant_url = "http://qdrant.example.com:6333".into();
    config.projection.api_key = None;
    assert!(QdrantProjection::new(&config, "r").is_ok());
}

/// A server that answers every request with a redirect to `target`.
fn redirecting_server(target: String) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target}/readyz\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            );
        }
    });
    format!("http://{address}")
}

#[test]
fn a_redirect_is_not_followed_with_the_api_key() {
    let embedder = ollama();
    let elsewhere = FakeQdrant::start();
    let mut config = qdrant_config(&elsewhere, &embedder);
    config.qdrant_url = redirecting_server(elsewhere.url());
    config.projection.api_key = Secret::new("k-123-secret");
    let client = QdrantProjection::new(&config, "r").unwrap();
    assert!(client.ensure_ready().is_err());
    assert_eq!(
        elsewhere.request_count(),
        0,
        "the redirect target never saw a request, so never saw the key"
    );
}

#[test]
fn one_request_never_carries_more_than_the_client_maximum() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let mut config = qdrant_config(&fake, &embedder);
    config.projection.batch_size = 1_000_000;
    let client = QdrantProjection::new(&config, "repo-x").unwrap();
    let points: Vec<ProjectionPoint> = (0..600).map(|n| one_point(&format!("p{n}"))).collect();
    client.upsert(&points).unwrap();
    let sizes: Vec<usize> = fake
        .requests()
        .iter()
        .filter(|request| request.method == "PUT" && request.path.contains("wait=true"))
        .map(|request| request.body["points"].as_array().map_or(0, Vec::len))
        .collect();
    // The marker write, then 512 and 88.
    assert_eq!(sizes, vec![1, 512, 88], "{sizes:?}");
}

#[test]
fn a_clear_made_offline_empties_the_remote_after_a_restart() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config.clone());
    memory
        .index_messages(&[note("alpha one"), note("beta two")])
        .unwrap();
    assert_eq!(remote_points(&fake, &memory), 2);

    // Clearing the last records while Qdrant is down.
    fake.set_down(true);
    memory.clear().unwrap();
    assert!(memory.records().is_empty());
    assert_eq!(
        remote_points(&fake, &memory),
        2,
        "the remote still holds them"
    );
    assert_eq!(memory.status()["remoteClearPending"], true);
    assert!(memory.remote_projection_lag() >= 1);
    drop(memory);

    // A fresh process in the same directory still owes the delete.
    fake.set_down(false);
    let restarted = VectorMemory::with_config(dir.path().to_path_buf(), config);
    assert!(restarted.remote_clear_pending());
    let receipt = restarted.reconcile_remote(10);
    assert_eq!(receipt.failures, 0, "{receipt:?}");
    assert!(receipt.is_complete(), "{receipt:?}");
    assert_eq!(remote_points(&fake, &restarted), 0);
    assert!(!restarted.remote_clear_pending());
    assert_eq!(restarted.status()["remoteClearPending"], false);
    assert_eq!(restarted.remote_projection_lag(), 0);
}

#[test]
fn a_pending_clear_runs_before_anything_written_after_it() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config);
    memory.index_messages(&[note("alpha one")]).unwrap();

    fake.set_down(true);
    memory.clear().unwrap();
    memory.index_messages(&[note("gamma three")]).unwrap();
    assert_eq!(memory.records().len(), 1);

    // Back online: the old vector goes, the new one stays, and the next
    // reconcile finds nothing missing.
    fake.set_down(false);
    let before = fake.requests().len();
    memory.index_messages(&[note("delta four")]).unwrap();
    // The owed delete reaches Qdrant before any point written after the clear;
    // the other order would delete the new points with the old ones.
    let later = &fake.requests()[before..];
    let first_delete = later
        .iter()
        .position(|request| request.method == "POST" && request.path.contains("/points/delete"))
        .expect("the owed clear ran");
    let first_upload = later
        .iter()
        .position(|request| request.method == "PUT" && request.path.contains("/points?wait=true"))
        .expect("the new point was sent");
    assert!(
        first_delete < first_upload,
        "an upload ran before the owed clear: {later:?}"
    );
    let receipt = memory.reconcile_remote(10);
    assert!(receipt.is_complete(), "{receipt:?}");
    assert!(!memory.remote_clear_pending());
    assert_eq!(memory.records().len(), 2);
    assert_eq!(remote_points(&fake, &memory), 2);
}

#[test]
fn a_clear_is_refused_when_its_pending_marker_cannot_be_written() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config);
    memory.index_messages(&[note("alpha one")]).unwrap();
    // A directory where the ledger file goes: it cannot be written.
    let ledger = memory.local_path().with_file_name("projection-state.json");
    let _ = std::fs::remove_file(&ledger);
    std::fs::create_dir(&ledger).unwrap();
    let error = memory.clear().unwrap_err().to_string();
    assert!(error.contains("pending remote clear"), "{error}");
    assert_eq!(memory.records().len(), 1, "nothing was cleared");
    assert_eq!(remote_points(&fake, &memory), 1);
}

#[test]
fn a_recreated_collection_is_refilled_from_local_records() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let mut config = instant_retry(qdrant_config(&fake, &embedder));
    config.projection.batch_size = 2;
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config);
    let notes: Vec<MemoryMessage> = (0..5).map(|i| note(&format!("alpha note {i}"))).collect();
    memory.index_messages(&notes).unwrap();
    for _ in 0..3 {
        memory.reconcile_remote(2);
    }
    assert_eq!(remote_points(&fake, &memory), 5);
    assert_eq!(memory.remote_projection_lag(), 0, "the ledger says synced");

    // The collection is deleted and made again empty by the next use.
    fake.delete_collection(&memory.remote_collection_name());
    let mut sent = Vec::new();
    for _ in 0..6 {
        let pass = memory.reconcile_remote(2);
        assert_eq!(pass.failures, 0, "{pass:?}");
        sent.push(pass.indexed);
        if pass.lag == 0 {
            break;
        }
    }
    assert_eq!(
        sent,
        vec![2, 2, 1],
        "re-sent in batches of the configured size"
    );
    assert_eq!(remote_points(&fake, &memory), 5);
    assert_eq!(memory.remote_projection_lag(), 0);
}

use davinci_coding_agent::native_extensions::memory_backends::qdrant::point_id;
use davinci_coding_agent::native_extensions::setup_actions::finish_owed_remote_clear;
use std::sync::{Arc, Mutex};

fn foreign_point(id: &str, repo_id: &str) -> ProjectionPoint {
    ProjectionPoint {
        id: id.into(),
        vector: vec![0.0, 0.0, 1.0],
        repo_id: repo_id.into(),
    }
}

#[test]
fn another_clones_points_do_not_hide_a_recreated_collection() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config.clone());
    memory
        .index_messages(&[note("alpha one"), note("beta two"), note("alpha three")])
        .unwrap();
    assert_eq!(remote_points(&fake, &memory), 3);

    // The collection is deleted. Another clone of the same repository (same
    // repo id) writes to the recreated one before this clone looks: it holds
    // at least as many points as this clone's ledger, none of them ours.
    fake.delete_collection(&memory.remote_collection_name());
    let other = QdrantProjection::new(&config, &memory.repo_id).unwrap();
    let foreign: Vec<ProjectionPoint> = (0..4)
        .map(|n| foreign_point(&format!("foreign-{n}"), &memory.repo_id))
        .collect();
    other.upsert(&foreign).unwrap();
    assert_eq!(remote_points(&fake, &memory), 4);

    for _ in 0..4 {
        let pass = memory.reconcile_remote(10);
        assert_eq!(pass.failures, 0, "{pass:?}");
        if pass.lag == 0 {
            break;
        }
    }
    assert_eq!(
        remote_points(&fake, &memory),
        7,
        "ours came back, theirs stayed"
    );
    let collection = fake.collection(&memory.remote_collection_name()).unwrap();
    for record in memory.records() {
        assert!(
            collection.points.contains_key(&point_id(&record.id)),
            "{} was not re-sent",
            record.id
        );
    }
}

#[test]
fn counting_held_ids_ignores_other_points_and_is_chunked() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let client = QdrantProjection::new(&qdrant_config(&fake, &embedder), "repo-x").unwrap();
    let mut points: Vec<ProjectionPoint> = (0..600).map(|n| one_point(&format!("p{n}"))).collect();
    points.push(foreign_point("other-repo", "repo-y"));
    points.push(foreign_point("same-repo-not-asked", "repo-x"));
    client.upsert(&points).unwrap();

    let mut asked: Vec<String> = (0..600).map(|n| format!("p{n}")).collect();
    asked.push("never-sent".into());
    asked.push("other-repo".into());
    let before = fake.count_matching("POST", "/points/count");
    assert_eq!(client.count_held(&asked, "repo-x").unwrap(), 600);
    assert_eq!(
        fake.count_matching("POST", "/points/count") - before,
        2,
        "602 ids go out as two requests of at most 512"
    );
    assert_eq!(client.count_held(&[], "repo-x").unwrap(), 0);
}

#[test]
fn a_pending_clear_survives_a_change_of_collection() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config.clone());
    memory
        .index_messages(&[note("alpha one"), note("beta two")])
        .unwrap();
    let old_collection = memory.remote_collection_name();
    fake.set_down(true);
    memory.clear().unwrap();
    drop(memory);

    // A restart with another embedding model: a different collection name.
    fake.set_down(false);
    let mut changed = config;
    changed.embedding_model = "another-model".into();
    let restarted = VectorMemory::with_config(dir.path().to_path_buf(), changed);
    assert_ne!(restarted.remote_collection_name(), old_collection);
    assert!(
        restarted.remote_clear_pending(),
        "the owed clear was forgotten"
    );

    let receipt = restarted.reconcile_remote(10);
    assert_eq!(receipt.failures, 0, "{receipt:?}");
    assert!(!restarted.remote_clear_pending());
    assert_eq!(
        fake.collection(&old_collection).unwrap().points.len(),
        0,
        "the old collection was emptied, not the new one by mistake"
    );
}

#[test]
fn an_offline_clear_removes_only_what_it_cleared() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config.clone());
    memory
        .index_messages(&[note("alpha one"), note("beta two")])
        .unwrap();
    fake.set_down(true);
    memory.clear().unwrap();
    drop(memory);

    // Another clone uploads to the same namespace after the clear.
    fake.set_down(false);
    let restarted = VectorMemory::with_config(dir.path().to_path_buf(), config.clone());
    let other = QdrantProjection::new(&config, &restarted.repo_id).unwrap();
    other
        .upsert(&[foreign_point("their-memory", &restarted.repo_id)])
        .unwrap();
    assert_eq!(remote_points(&fake, &restarted), 3);

    let receipt = restarted.reconcile_remote(10);
    assert_eq!(receipt.failures, 0, "{receipt:?}");
    assert!(!restarted.remote_clear_pending());
    let collection = fake
        .collection(&restarted.remote_collection_name())
        .unwrap();
    assert_eq!(collection.points.len(), 1);
    assert!(collection.points.contains_key(&point_id("their-memory")));
}

#[test]
fn a_restart_delivers_an_owed_clear_without_waiting_for_a_write() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config.clone());
    memory.index_messages(&[note("alpha one")]).unwrap();
    fake.set_down(true);
    memory.clear().unwrap();
    drop(memory);
    fake.set_down(false);

    let restarted = Arc::new(Mutex::new(VectorMemory::with_config(
        dir.path().to_path_buf(),
        config.clone(),
    )));
    assert!(finish_owed_remote_clear(&restarted));
    let guard = restarted.lock().unwrap();
    assert!(!guard.remote_clear_pending());
    assert_eq!(remote_points(&fake, &guard), 0);

    // With nothing owed the startup path contacts nothing.
    let clean_dir = tempfile::tempdir().unwrap();
    let clean = Arc::new(Mutex::new(VectorMemory::with_config(
        clean_dir.path().to_path_buf(),
        config,
    )));
    let before = fake.request_count();
    assert!(finish_owed_remote_clear(&clean));
    assert_eq!(fake.request_count(), before);
}

#[test]
fn a_catch_up_snapshot_cannot_re_upload_what_a_clear_removed() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config);
    // Written while Qdrant was down, so the remote lacks them.
    fake.set_down(true);
    memory
        .index_messages(&[note("alpha one"), note("beta two")])
        .unwrap();
    fake.set_down(false);

    // The catch-up takes its snapshot, then the user clears.
    let snapshot = memory.clone();
    let epoch = memory.remote_clear_epoch();
    memory.clear().unwrap();

    let pass = snapshot.reconcile_remote_at(10, Some(epoch));
    assert_eq!(pass.indexed, 0, "{pass:?}");
    assert_eq!(
        remote_points(&fake, &memory),
        0,
        "cleared records came back"
    );
    assert_eq!(memory.remote_projection_lag(), 0);
}

#[test]
fn a_finished_clear_that_cannot_be_recorded_is_not_treated_as_finished() {
    let fake = FakeQdrant::start();
    let embedder = ollama();
    let dir = tempfile::tempdir().unwrap();
    let config = instant_retry(qdrant_config(&fake, &embedder));
    let mut memory = VectorMemory::with_config(dir.path().to_path_buf(), config);
    memory.index_messages(&[note("alpha one")]).unwrap();
    fake.set_down(true);
    memory.clear().unwrap();
    fake.set_down(false);

    // The state file can no longer be written.
    let ledger = memory.local_path().with_file_name("projection-state.json");
    std::fs::remove_file(&ledger).unwrap();
    std::fs::create_dir(&ledger).unwrap();
    let receipt = memory.reconcile_remote(10);
    assert_eq!(receipt.failures, 1, "{receipt:?}");
    assert!(
        memory.remote_clear_pending(),
        "claimed finished, disk disagrees"
    );
    assert!(memory.remote_projection_lag() >= 1);

    // Writable again: the repeat delete is harmless and the clear finishes.
    std::fs::remove_dir(&ledger).unwrap();
    let receipt = memory.reconcile_remote(10);
    assert_eq!(receipt.failures, 0, "{receipt:?}");
    assert!(!memory.remote_clear_pending());
}
