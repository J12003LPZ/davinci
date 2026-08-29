use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Agent error: {0}")]
    Agent(String),

    #[error("Tool execution failed: {0}")]
    ToolError(String),

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("AI error: {0}")]
    Ai(#[from] pi_ai::Error),

    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Aborted")]
    Aborted,
}

pub type Result<T> = std::result::Result<T, Error>;
