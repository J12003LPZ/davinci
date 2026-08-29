//! Shared types, CBOR subset, framing, and protocol messages for the Pi Rust port.
//! TypeScript packages remain the schema authority until the Phase 8 gate.

pub mod cbor;
pub mod error;
pub mod framing;
pub mod protocol;
pub mod types;

pub use cbor::{
    decode_cbor, decode_json, encode_cbor, encode_json, CborValue, DEFAULT_MAX_CBOR_BYTE_LENGTH,
    DEFAULT_MAX_CBOR_CONTAINER_LENGTH, DEFAULT_MAX_CBOR_DEPTH,
};
pub use error::{
    CborError, FrameError, ProtocolError, ProtocolErrorCode, ProtocolValidationError, SessionError,
    SessionErrorCode,
};
pub use framing::{
    assert_complete_frame, encode_frame, FrameDecoder, FrameDecoderOptions,
    DEFAULT_MAX_FRAME_LENGTH,
};
pub use protocol::{
    is_supported_protocol_version, parse_client_message, parse_server_message, ClientMessage,
    Command, CommandResult, ModelMetadata, ServerEvent, ServerMessage, ServerSnapshot,
    SessionSnapshot,
};
pub use types::{
    AgentEvent, AgentMessage, AssistantMessageEvent, Entry, Message, ModelRef, Role,
    SessionLeaseMode, SessionMetadata, SessionPhase, StopReason, ThinkingLevel, ToolCall, Usage,
    UsageCost, WriterLease, WriterLeaseOptions, DEFAULT_WRITER_LEASE_HEARTBEAT_MS,
    DEFAULT_WRITER_LEASE_TTL_MS, PROTOCOL_VERSION,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_lease_serde_matches_typescript() {
        let lease = WriterLease {
            owner_id: "owner-1".into(),
            fence: 2,
            expires_at_ms: 1_700_000_030_000,
        };
        let json = serde_json::to_value(&lease).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "ownerId": "owner-1",
                "fence": 2,
                "expiresAtMs": 1_700_000_030_000_i64
            })
        );
        let back: WriterLease = serde_json::from_value(json).unwrap();
        assert_eq!(back, lease);
    }

    #[test]
    fn session_metadata_omits_optional_nulls() {
        let meta = SessionMetadata {
            id: "sess-1".into(),
            created_at: 10,
            updated_at: None,
            parent_session_id: None,
            session_name: None,
            cwd: Some("/tmp".into()),
            path: None,
            metadata: None,
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["id"], "sess-1");
        assert_eq!(json["createdAt"], 10);
        assert_eq!(json["cwd"], "/tmp");
        assert!(json.get("sessionName").is_none());
    }
}
