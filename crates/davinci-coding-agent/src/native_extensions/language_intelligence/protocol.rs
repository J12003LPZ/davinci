//! Small V1 wire boundary. Unknown LSP extensions remain opaque, bounded JSON.
//! We intentionally do not maintain a copy of the full LSP type specification.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct IntelligenceError {
    pub code: String,
    pub message: String,
}

impl IntelligenceError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, IntelligenceError>;
