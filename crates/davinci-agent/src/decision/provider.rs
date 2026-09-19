use std::time::Duration;

use thiserror::Error;

use super::request::DecisionRequest;
use super::response::DecisionResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionProviderHealth {
    Disabled,
    Ready,
    CredentialInvalid,
    RateLimited,
    Overloaded,
    Unavailable,
    SchemaMismatch,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DecisionError {
    #[error("decision intelligence is disabled")]
    Disabled,
    #[error("decision intelligence credential is invalid")]
    CredentialInvalid,
    #[error("decision intelligence provider is rate limited")]
    RateLimited,
    #[error("decision intelligence provider is overloaded")]
    Overloaded,
    #[error("decision intelligence provider is unavailable: {0}")]
    Unavailable(String),
    #[error("decision intelligence response schema mismatch: {0}")]
    SchemaMismatch(String),
    #[error("decision intelligence request is invalid: {0}")]
    InvalidRequest(String),
    #[error("decision intelligence response was stale")]
    StaleResponse,
    #[error("decision intelligence provider returned HTTP status {0}")]
    HttpStatus(u16),
}

impl DecisionError {
    pub const fn health(&self) -> DecisionProviderHealth {
        match self {
            Self::Disabled => DecisionProviderHealth::Disabled,
            Self::CredentialInvalid | Self::HttpStatus(401) => {
                DecisionProviderHealth::CredentialInvalid
            }
            Self::RateLimited | Self::HttpStatus(429) => DecisionProviderHealth::RateLimited,
            Self::Overloaded | Self::HttpStatus(529) => DecisionProviderHealth::Overloaded,
            Self::SchemaMismatch(_) | Self::HttpStatus(422) => {
                DecisionProviderHealth::SchemaMismatch
            }
            Self::InvalidRequest(_) | Self::StaleResponse => DecisionProviderHealth::Unavailable,
            Self::Unavailable(_) | Self::HttpStatus(_) => DecisionProviderHealth::Unavailable,
        }
    }

    pub const fn retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited
                | Self::Overloaded
                | Self::Unavailable(_)
                | Self::HttpStatus(429)
                | Self::HttpStatus(529)
        )
    }
}

pub trait DecisionProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn model(&self) -> &'static str;
    fn evaluate(
        &self,
        request: &DecisionRequest,
        budget: Duration,
    ) -> Result<DecisionResponse, DecisionError>;
}
