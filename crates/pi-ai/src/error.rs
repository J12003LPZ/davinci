use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Unsupported provider: {0}")]
    UnsupportedProvider(String),

    #[error("Unsupported API: {0}")]
    UnsupportedApi(String),

    #[error("No API key for provider: {0}")]
    NoApiKey(String),

    #[error("API request failed: {0}")]
    RequestFailed(String),

    #[error("Stream error: {0}")]
    StreamError(String),

    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Auth error: {0}")]
    AuthError(String),

    #[error("Aborted")]
    Aborted,

    #[error("{0}")]
    Custom(String),
}

pub type Result<T> = std::result::Result<T, Error>;
