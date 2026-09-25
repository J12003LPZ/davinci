//! Small V1 wire boundary. Unknown LSP extensions remain opaque, bounded JSON.
//! We intentionally do not maintain a copy of the full LSP type specification.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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


#[derive(Debug, Clone)]
pub struct RequestBudget {
    pub deadline: Instant,
    pub cancelled: Option<Arc<AtomicBool>>,
}
impl RequestBudget {
    pub fn from_timeout(timeout: Duration) -> Self {
        Self { deadline: Instant::now() + timeout, cancelled: None }
    }
    pub fn check(&self) -> Result<()> {
        if self.cancelled.as_ref().is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(IntelligenceError::new("request_cancelled", "Language-intelligence request was cancelled"));
        }
        if Instant::now() >= self.deadline {
            return Err(IntelligenceError::new("request_timeout", "Language-intelligence deadline exceeded"));
        }
        Ok(())
    }
    pub fn remaining(&self) -> Result<Duration> {
        self.check()?;
        self.deadline.checked_duration_since(Instant::now()).filter(|v| !v.is_zero())
            .ok_or_else(|| IntelligenceError::new("request_timeout", "Language-intelligence deadline exceeded"))
    }
}
