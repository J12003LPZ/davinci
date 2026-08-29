use pi_protocol::{decode_cbor, encode_cbor, ProtocolMessage};
use pi_session::manager::SessionManager;
use pi_session::types::{SessionEntry, CURRENT_SESSION_VERSION};
use tempfile::tempdir;

pub fn assert_cbor_roundtrip() {
    let hello = ProtocolMessage::Hello {
        version: CURRENT_SESSION_VERSION,
        client_id: "test-client-123".to_string(),
    };
    let encoded = encode_cbor(&hello).expect("CBOR encoding failed");
    let decoded = decode_cbor(&encoded).expect("CBOR decoding failed");
    assert_eq!(hello, decoded);
}

pub fn assert_session_parity() {
    let tmp = tempdir().expect("tempdir");
    let mgr = SessionManager::new(tmp.path());
    let (id, _path) = mgr
        .create_session("/workspace", None)
        .expect("create session");

    let entry = SessionEntry::ThinkingLevelChange {
        id: "entry-1".to_string(),
        parent_id: None,
        timestamp: "2026-08-29T00:00:00Z".to_string(),
        thinking_level: "high".to_string(),
    };

    mgr.append_entry(&id, &entry).expect("append entry");
    let entries = mgr.read_entries(&id).expect("read entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], entry);
}
