//! A context page written by one process must load in another that opens the
//! same agent directory (a cold resume).

use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};
use davinci_agent::runtime::context_vm::{
    CheckpointState, ContextObject, ContextObjectStore, ProvenanceRef, StateValue,
};

#[test]
fn checkpoint_page_loads_from_a_second_cache_on_the_same_directory() {
    let dir = tempfile::tempdir().expect("dir");
    let writer = ContextObjectStore::new(CacheRuntime::new(
        CacheConfig::default(),
        Some(dir.path().to_path_buf()),
    ));
    let reader = ContextObjectStore::new(CacheRuntime::new(
        CacheConfig::default(),
        Some(dir.path().to_path_buf()),
    ));
    let object = ContextObject::Checkpoint(CheckpointState::default());
    let page = writer.save(&object).expect("save");
    let loaded = reader
        .load(&page)
        .expect("a fresh cache finds the page on disk");
    assert_eq!(loaded, object);
}

#[test]
fn populated_checkpoint_page_loads_from_a_second_cache() {
    let dir = tempfile::tempdir().expect("dir");
    let open = || {
        ContextObjectStore::new(CacheRuntime::new(
            CacheConfig::default(),
            Some(dir.path().to_path_buf()),
        ))
    };
    let (writer, reader) = (open(), open());
    let mut state = CheckpointState::default();
    for n in 0..40 {
        state.goals.push(StateValue {
            value: format!("goal {n}: {}", "Keep the code readable. ".repeat(20)),
            provenance: vec![ProvenanceRef {
                kind: davinci_agent::runtime::ProvenanceKind::UserDecision,
                source_refs: vec![format!("session:{n}")],
                content_hash: "ab".repeat(32),
            }],
        });
    }
    let object = ContextObject::Checkpoint(state);
    let page = writer.save(&object).expect("save");
    let loaded = reader
        .load(&page)
        .expect("a fresh cache finds the page on disk");
    assert_eq!(loaded, object);
}
