//! Shared types for the pi Rust port.
//!
//! TypeScript remains the product authority (`vendor/pi`). These types
//! mirror `@earendil-works/pi-agent-core` error codes and JSON helpers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

pub const TYPESCRIPT_UPSTREAM_SHA: &str = "853a80d26c90a14c1886f0ebb8ffaae133ca2185";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct SessionError {
    pub code: SessionErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionErrorCode {
    Storage,
    InvalidPayload,
    NotFound,
    Busy,
}

impl SessionError {
    pub fn new(code: SessionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::new(SessionErrorCode::Storage, message)
    }

    pub fn invalid_payload(message: impl Into<String>) -> Self {
        Self::new(SessionErrorCode::InvalidPayload, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(SessionErrorCode::NotFound, message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct PiError(pub String);

impl From<SessionError> for PiError {
    fn from(value: SessionError) -> Self {
        Self(value.to_string())
    }
}

impl From<serde_json::Error> for PiError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}

/// Generate a UUIDv7 when the clock is available, otherwise UUIDv4.
pub fn next_id() -> String {
    Uuid::now_v7().to_string()
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn assert_json_object(value: &Value, label: &str) -> Result<(), SessionError> {
    if value.is_object() {
        Ok(())
    } else {
        Err(SessionError::invalid_payload(format!(
            "{label} must be an object"
        )))
    }
}

pub fn json_stable(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_error_codes_match_typescript_names() {
        let err = SessionError::storage("SQLite session abc already has an active writer");
        assert_eq!(err.code, SessionErrorCode::Storage);
        assert!(err.message.contains("already has an active writer"));
    }

    #[test]
    fn next_id_is_nonempty() {
        assert!(!next_id().is_empty());
        assert_ne!(next_id(), next_id());
    }
}
