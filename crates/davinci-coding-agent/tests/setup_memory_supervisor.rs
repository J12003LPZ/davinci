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
