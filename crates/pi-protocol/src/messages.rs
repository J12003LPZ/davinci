use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cbor::{decode_cbor, encode_cbor, CborError, CborValue};
use crate::framing::{encode_frame, FrameDecoder, FrameError};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolErrorCode {
    Version,
    Busy,
    SessionLocked,
    NotFound,
    InvalidRequest,
    NotImplemented,
    InternalError,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ProtocolErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Idle,
    Turn,
    Compaction,
    BranchSummary,
    Retry,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt", default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    #[serde(
        rename = "parentSessionId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_session_id: Option<String>,
    #[serde(
        rename = "sessionName",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub session_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub cwd: String,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
    pub phase: SessionPhase,
    pub model: ModelRef,
    #[serde(rename = "thinkingLevel")]
    pub thinking_level: String,
    pub attached: bool,
    pub locked: bool,
    pub revision: u64,
    pub transcript: Vec<Value>,
    #[serde(rename = "queuedSteer")]
    pub queued_steer: Vec<Value>,
    #[serde(rename = "queuedSteerCount")]
    pub queued_steer_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerSnapshot {
    #[serde(rename = "serverId")]
    pub server_id: String,
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    pub revision: u64,
    pub sessions: Vec<SessionMetadata>,
    pub models: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    List,
    Create {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<ModelRef>,
        #[serde(
            rename = "thinkingLevel",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        thinking_level: Option<String>,
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
        #[serde(rename = "thinkingLevel")]
        thinking_level: String,
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
pub enum ClientMessage {
    Hello { version: u32 },
    Request { id: String, request: Command },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello {
        version: u32,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<CommandResult>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ProtocolError>,
    },
    Event {
        event: ServerEvent,
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

#[derive(Debug, Clone, PartialEq)]
pub enum ProtocolValidationError {
    Cbor(CborError),
    Frame(FrameError),
    Schema(String),
}

impl std::fmt::Display for ProtocolValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "{error}"),
            Self::Frame(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ProtocolValidationError {}

pub fn is_supported_protocol_version(version: u32) -> bool {
    version == PROTOCOL_VERSION
}

pub fn encode_client_message(message: &ClientMessage) -> Result<Vec<u8>, ProtocolValidationError> {
    encode_message(message)
}

pub fn encode_server_message(message: &ServerMessage) -> Result<Vec<u8>, ProtocolValidationError> {
    encode_message(message)
}

fn encode_message<T: Serialize>(message: &T) -> Result<Vec<u8>, ProtocolValidationError> {
    let json = serde_json::to_value(message)
        .map_err(|error| ProtocolValidationError::Schema(error.to_string()))?;
    let cbor = CborValue::from_json(&json).map_err(ProtocolValidationError::Cbor)?;
    let payload = encode_cbor(&cbor).map_err(ProtocolValidationError::Cbor)?;
    encode_frame(&payload).map_err(ProtocolValidationError::Frame)
}

pub fn parse_client_message(value: &CborValue) -> Result<ClientMessage, ProtocolValidationError> {
    serde_json::from_value(value.to_json())
        .map_err(|error| ProtocolValidationError::Schema(error.to_string()))
}

pub fn parse_server_message(value: &CborValue) -> Result<ServerMessage, ProtocolValidationError> {
    serde_json::from_value(value.to_json())
        .map_err(|error| ProtocolValidationError::Schema(error.to_string()))
}

pub struct ClientMessageDecoder {
    frames: FrameDecoder,
}

impl ClientMessageDecoder {
    pub fn new() -> Self {
        Self {
            frames: FrameDecoder::new(None).expect("default frame limit"),
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ClientMessage>, ProtocolValidationError> {
        let frames = self
            .frames
            .push(chunk)
            .map_err(ProtocolValidationError::Frame)?;
        frames
            .into_iter()
            .map(|payload| {
                let value = decode_cbor(&payload).map_err(ProtocolValidationError::Cbor)?;
                parse_client_message(&value)
            })
            .collect()
    }

    pub fn end(&mut self) -> Result<(), ProtocolValidationError> {
        self.frames.end().map_err(ProtocolValidationError::Frame)
    }
}

impl Default for ClientMessageDecoder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ServerMessageDecoder {
    frames: FrameDecoder,
}

impl ServerMessageDecoder {
    pub fn new() -> Self {
        Self {
            frames: FrameDecoder::new(None).expect("default frame limit"),
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ServerMessage>, ProtocolValidationError> {
        let frames = self
            .frames
            .push(chunk)
            .map_err(ProtocolValidationError::Frame)?;
        frames
            .into_iter()
            .map(|payload| {
                let value = decode_cbor(&payload).map_err(ProtocolValidationError::Cbor)?;
                parse_server_message(&value)
            })
            .collect()
    }

    pub fn end(&mut self) -> Result<(), ProtocolValidationError> {
        self.frames.end().map_err(ProtocolValidationError::Frame)
    }
}

impl Default for ServerMessageDecoder {
    fn default() -> Self {
        Self::new()
    }
}
