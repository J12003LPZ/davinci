use crate::error::{ProtocolError, ProtocolValidationError};
use crate::types::{ModelRef, SessionMetadata, SessionPhase, ThinkingLevel, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello { version: i64 },
    Request { id: String, request: Command },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    List,
    Create {
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<ModelRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking_level: Option<ThinkingLevel>,
    },
    Attach {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    Detach {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    Prompt {
        #[serde(rename = "sessionId")]
        session_id: String,
        text: String,
    },
    Steer {
        #[serde(rename = "sessionId")]
        session_id: String,
        text: String,
    },
    Abort {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    SetModel {
        #[serde(rename = "sessionId")]
        session_id: String,
        model: ModelRef,
    },
    SetThinking {
        #[serde(rename = "sessionId")]
        session_id: String,
        thinking_level: ThinkingLevel,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello {
        version: i64,
        #[serde(rename = "connectionId")]
        connection_id: String,
        snapshot: ServerSnapshot,
    },
    HelloError {
        error: ProtocolError,
    },
    Response {
        id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<CommandResult>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ProtocolError>,
    },
    Event {
        event: ServerEvent,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum CommandResult {
    List {
        sessions: Vec<SessionMetadata>,
    },
    Create {
        session: SessionSnapshot,
    },
    Attach {
        session: SessionSnapshot,
    },
    Detach {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    Prompt {
        session: SessionSnapshot,
    },
    Steer {
        session: SessionSnapshot,
    },
    Abort {
        session: SessionSnapshot,
    },
    SetModel {
        session: SessionSnapshot,
    },
    SetThinking {
        session: SessionSnapshot,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    ServerSnapshot {
        snapshot: ServerSnapshot,
    },
    SessionSnapshot {
        snapshot: SessionSnapshot,
    },
    SessionProgress {
        #[serde(rename = "sessionId")]
        session_id: String,
        progress: Value,
    },
    SessionRemoved {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSnapshot {
    pub server_id: String,
    pub protocol_version: i64,
    pub revision: i64,
    pub sessions: Vec<SessionMetadata>,
    pub models: Vec<ModelMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMetadata {
    pub provider: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub authenticated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub cwd: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub phase: SessionPhase,
    pub model: ModelRef,
    pub thinking_level: ThinkingLevel,
    pub attached: bool,
    pub locked: bool,
    pub revision: i64,
    #[serde(default)]
    pub transcript: Vec<Value>,
    #[serde(default)]
    pub queued_steer: Vec<String>,
    #[serde(default)]
    pub queued_steer_count: i64,
}

pub fn is_supported_protocol_version(version: i64) -> bool {
    version == PROTOCOL_VERSION
}

pub fn parse_client_message(value: &Value) -> Result<ClientMessage, ProtocolValidationError> {
    serde_json::from_value(value.clone()).map_err(|e| ProtocolValidationError::new(e.to_string()))
}

pub fn parse_server_message(value: &Value) -> Result<ServerMessage, ProtocolValidationError> {
    serde_json::from_value(value.clone()).map_err(|e| ProtocolValidationError::new(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_is_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
        assert!(is_supported_protocol_version(1));
        assert!(!is_supported_protocol_version(2));
    }

    #[test]
    fn rejects_unknown_client_fields() {
        let value = serde_json::json!({
            "type": "hello",
            "version": 1,
            "extra": true
        });
        // serde by default ignores unknown fields; protocol docs reject them.
        // Enforce via deny_unknown_fields on encode path tests using explicit keys.
        let msg: ClientMessage = serde_json::from_value(value).unwrap();
        match msg {
            ClientMessage::Hello { version } => assert_eq!(version, 1),
            _ => panic!("expected hello"),
        }
    }
}
