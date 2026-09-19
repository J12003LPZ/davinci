use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};
use davinci_agent::runtime::context_vm::{
    events_from_messages, CheckpointState, ContextObject, ContextObjectStore, ContextPageKind,
    ContextVmConfig, ContextVmRuntime,
};
use davinci_agent::{ContextItem, ContextPacket};
use davinci_ai::ChatMessage;

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
    runtime.compile(&initial, &stable_packet, 1_000).unwrap();
    let before = runtime.cache_affinity();

    let extended = events_from_messages(&[
        ChatMessage::text("user", "keep the API"),
        ChatMessage::text("assistant", "new volatile tail"),
    ]);
    runtime.append_delta(&extended).unwrap();
    runtime.compile(&extended, &stable_packet, 1_000).unwrap();
    assert_eq!(runtime.cache_affinity(), before);

    let changed_packet = ContextPacket {
        items: vec![ContextItem {
            content: "changed repository fact".into(),
            ..stable_packet.items[0].clone()
        }],
        ..stable_packet.clone()
    };
    runtime.compile(&extended, &changed_packet, 1_000).unwrap();
    assert_ne!(runtime.cache_affinity(), before);

    runtime.compile(&extended, &stable_packet, 1).unwrap();
    let tight_before = runtime.cache_affinity();
    runtime.compile(&extended, &changed_packet, 1).unwrap();
    assert_eq!(runtime.cache_affinity(), tight_before);
}
