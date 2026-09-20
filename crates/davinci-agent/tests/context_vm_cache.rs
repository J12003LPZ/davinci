use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};
use davinci_agent::runtime::context_vm::{
    events_from_messages, CheckpointState, ContextObject, ContextObjectStore, ContextPageKind,
    ContextVmConfig, ContextVmRuntime,
};
use davinci_agent::{ContextItem, ContextPacket};
use davinci_ai::ChatMessage;

#[test]
fn rebuilding_a_missing_page_is_not_a_retrieval_hit() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = ContextVmRuntime::new(
        ContextVmConfig::default(),
        CacheRuntime::new(CacheConfig::default(), Some(directory.path().to_path_buf())),
    );
    let events = events_from_messages(&[ChatMessage::text("user", "preserve this requirement")]);
    let image = runtime
        .compile(&events, &ContextPacket::empty(), 4_000)
        .unwrap();
    let mut missing_root = image.root;
    let page = missing_root.checkpoint.as_mut().unwrap();
    page.id = format!("ctx:checkpoint:{}", "0".repeat(64));
    page.content_hash = "0".repeat(64);
    runtime.install_root(missing_root, 1);

    let recovered = runtime
        .compile(&events, &ContextPacket::empty(), 4_000)
        .unwrap();
    assert!(recovered.messages.iter().any(|message| {
        davinci_ai::content_text(&message.content).contains("preserve this requirement")
    }));
    assert_eq!(
        runtime.metrics().page_fault_hits,
        0,
        "rebuilds are not retrieval hits"
    );
    let metrics = runtime.metrics();
    assert_eq!(metrics.page_lookup_misses, 1);
    assert_eq!(metrics.rebuild_attempts, 2);
    assert_eq!(metrics.rebuild_successes, 2);
    assert_eq!(metrics.rebuild_failures, 0);
    assert_eq!(metrics.semantic_page_faults, 0);
}

#[test]
fn identical_context_objects_are_content_addressed_and_reload_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let cache = CacheRuntime::new(CacheConfig::default(), Some(directory.path().to_path_buf()));
    let object = ContextObject::Checkpoint(CheckpointState {
        through_seq: 7,
        ..CheckpointState::default()
    });

    let first = ContextObjectStore::new(cache).save(&object).unwrap();
    let second = ContextObjectStore::new(CacheRuntime::new(
        CacheConfig::default(),
        Some(directory.path().to_path_buf()),
    ))
    .save(&object)
    .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(first.kind, ContextPageKind::Checkpoint);
    assert!(first.id.starts_with("ctx:checkpoint:"));

    let restarted = CacheRuntime::new(CacheConfig::default(), Some(directory.path().to_path_buf()));
    let loaded = ContextObjectStore::new(restarted).load(&first).unwrap();
    assert_eq!(loaded, object);
}

#[test]
fn changed_context_objects_get_new_immutable_page_ids() {
    let directory = tempfile::tempdir().unwrap();
    let store = ContextObjectStore::new(CacheRuntime::new(
        CacheConfig::default(),
        Some(directory.path().to_path_buf()),
    ));
    let first = store
        .save(&ContextObject::Checkpoint(CheckpointState {
            through_seq: 7,
            ..CheckpointState::default()
        }))
        .unwrap();
    let second = store
        .save(&ContextObject::Checkpoint(CheckpointState {
            through_seq: 8,
            ..CheckpointState::default()
        }))
        .unwrap();

    assert_ne!(first.id, second.id);
    assert_ne!(first.content_hash, second.content_hash);
}

#[test]
fn volatile_hot_tail_does_not_change_stable_cache_affinity() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = ContextVmRuntime::new(
        ContextVmConfig::default(),
        CacheRuntime::new(CacheConfig::default(), Some(directory.path().to_path_buf())),
    );
    let stable_packet = ContextPacket {
        items: vec![ContextItem {
            source: "repo:README.md".into(),
            content: "stable repository fact".into(),
            estimated_tokens: 4,
            priority: 100,
            stable_for_cache: true,
            provenance: serde_json::json!({"provenance_kind":"repository_fact"}),
        }],
        estimated_tokens: 4,
        cache_key: "stable".into(),
    };
    let initial = events_from_messages(&[ChatMessage::text("user", "keep the API")]);
    let first = runtime.compile(&initial, &stable_packet, 10_000).unwrap();
    assert!(first.entries.iter().any(|e| e.category == "broker_context"));
    let before = runtime.cache_affinity();

    let extended = events_from_messages(&[
        ChatMessage::text("user", "keep the API"),
        ChatMessage::text("assistant", "new volatile tail"),
    ]);
    runtime.append_delta(&extended).unwrap();
    let extended_image = runtime.compile(&extended, &stable_packet, 10_000).unwrap();
    assert!(extended_image
        .entries
        .iter()
        .any(|e| e.category == "broker_context"));
    assert_eq!(runtime.cache_affinity(), before);

    let changed_packet = ContextPacket {
        items: vec![ContextItem {
            content: "changed repository fact".into(),
            ..stable_packet.items[0].clone()
        }],
        ..stable_packet.clone()
    };
    runtime.compile(&extended, &changed_packet, 10_000).unwrap();
    assert_ne!(runtime.cache_affinity(), before);

    // Reserve the required state and newest event, leaving no room for the
    // optional broker item. A one-token budget now correctly rejects both.
    let tight_budget = runtime
        .compile(&extended, &ContextPacket::empty(), 10_000)
        .unwrap()
        .estimated_tokens;
    runtime
        .compile(&extended, &stable_packet, tight_budget)
        .unwrap();
    let tight_before = runtime.cache_affinity();
    runtime
        .compile(&extended, &changed_packet, tight_budget)
        .unwrap();
    assert_eq!(runtime.cache_affinity(), tight_before);
    assert!(runtime.compile(&extended, &stable_packet, 1).is_err());
    assert_eq!(runtime.cache_affinity(), tight_before);
}
