use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionErrorCode {
    NotFound,
    AlreadyExists,
    InvalidEntry,
    InvalidPayload,
    InvalidLane,
    InvalidQuery,
    InvalidForkTarget,
    Storage,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("{code:?}: {message}")]
pub struct SessionError {
    pub code: SessionErrorCode,
    pub message: String,
}

impl SessionError {
    pub fn new(code: SessionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn active_writer(session_id: &str) -> Self {
        Self::new(
            SessionErrorCode::Storage,
            format!("SQLite session {session_id} already has an active writer"),
        )
    }

    pub fn lost_writer(session_id: &str) -> Self {
        Self::new(
            SessionErrorCode::Storage,
            format!("SQLite session {session_id} writer lease was lost"),
        )
    }

    pub fn closed(session_id: &str) -> Self {
        Self::new(
            SessionErrorCode::Storage,
            format!("SQLite session {session_id} is closed"),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Error)]
#[serde(rename_all = "camelCase")]
#[error("{code:?}: {message}")]
pub struct ProtocolError {
    pub code: ProtocolErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ProtocolError {
    pub fn version() -> Self {
        Self {
            code: ProtocolErrorCode::Version,
            message: "Unsupported protocol version".to_string(),
            details: None,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: ProtocolErrorCode::InvalidRequest,
            message: message.into(),
            details: None,
        }
    }

    pub fn not_implemented() -> Self {
        Self {
            code: ProtocolErrorCode::NotImplemented,
            message: "Operation is not implemented".to_string(),
            details: None,
        }
    }

    pub fn internal() -> Self {
        Self {
            code: ProtocolErrorCode::InternalError,
            message: "Internal server error".to_string(),
            details: None,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: ProtocolErrorCode::NotFound,
            message: message.into(),
            details: None,
        }
    }

    pub fn busy(message: impl Into<String>) -> Self {
        Self {
            code: ProtocolErrorCode::Busy,
            message: message.into(),
            details: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum CborError {
    #[error("{0}")]
    Message(String),
}

impl CborError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("{0}")]
    Message(String),
}

impl FrameError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

#[derive(Debug, Error)]
pub enum ProtocolValidationError {
    #[error("{0}")]
    Message(String),
}

impl ProtocolValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}
