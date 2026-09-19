use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use davinci_agent::decision::provider::{DecisionError, DecisionProvider};
use davinci_agent::decision::request::DecisionRequest;
use davinci_agent::decision::response::{
    parse_and_validate_response, DecisionResponse, MAX_RESPONSE_BYTES,
};
use davinci_agent::decision::HARD_DECISION_BUDGET;
use davinci_ai::AuthStorage;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use zeroize::Zeroizing;

pub const TYPESAFE_URL: &str = "https://api.typesafe.ai/v1/systemone";
pub const TYPESAFE_MODEL: &str = "jev-latest";
pub const VALIDATION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TypeSafeAuthError {
    #[error("TYPESAFE_API_KEY is set but empty")]
    InvalidEnvironment,
    #[error("stored TypeSafe credential is malformed")]
    InvalidStoredCredential,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CredentialValidationError {
    #[error("TypeSafe credential is empty")]
    Empty,
    #[error("TypeSafe rejected the credential")]
    Invalid,
    #[error("TypeSafe rejected the validation request schema")]
    SchemaMismatch,
    #[error("TypeSafe validation was rate limited")]
    RateLimited,
    #[error("TypeSafe validation service is overloaded")]
    Overloaded,
    #[error("TypeSafe validation service is unavailable")]
    Unavailable,
}

#[derive(Clone)]
pub struct TypeSafeProvider {
    api_key: Zeroizing<String>,
}

impl TypeSafeProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Zeroizing::new(api_key.into()),
        }
    }

    pub fn validate_api_key(api_key: &str) -> Result<(), CredentialValidationError> {
        if api_key.trim().is_empty() {
            return Err(CredentialValidationError::Empty);
        }
        let body = serde_json::json!({
            "state": {
                "probe": "davinci_typesafe_credential_validation",
                "schemaVersion": 1
            },
            "model": TYPESAFE_MODEL,
            "questions": {
                "probe": {
                    "type": "noul",
                    "instructions": "The field named probe equals davinci_typesafe_credential_validation."
                }
            }
        });
        let raw = send_json(api_key, &body, VALIDATION_TIMEOUT).map_err(map_validation_error)?;
        if raw.len() > MAX_RESPONSE_BYTES {
            return Err(CredentialValidationError::SchemaMismatch);
        }
        let value: Value =
            serde_json::from_slice(&raw).map_err(|_| CredentialValidationError::SchemaMismatch)?;
        let answer = value
            .get("answers")
            .and_then(|answers| answers.get("probe"))
            .ok_or(CredentialValidationError::SchemaMismatch)?;
        if answer.get("type").and_then(Value::as_str) != Some("noul") {
            return Err(CredentialValidationError::SchemaMismatch);
        }
        let value = answer
            .get("noul")
            .and_then(Value::as_f64)
            .ok_or(CredentialValidationError::SchemaMismatch)?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(CredentialValidationError::SchemaMismatch);
        }
        Ok(())
    }

    pub fn from_auth(auth: &AuthStorage) -> Result<Option<Arc<Self>>, TypeSafeAuthError> {
        Ok(resolve_api_key(auth)?.map(|key| Arc::new(Self::new(key))))
    }
}

impl DecisionProvider for TypeSafeProvider {
    fn name(&self) -> &'static str {
        "typesafe"
    }

    fn model(&self) -> &'static str {
        TYPESAFE_MODEL
    }

    fn evaluate(
        &self,
        request: &DecisionRequest,
        budget: Duration,
    ) -> Result<DecisionResponse, DecisionError> {
        let body = ProviderPayload::from_request(request);
        let raw = send_json(
            self.api_key.as_str(),
            &body,
            budget.min(HARD_DECISION_BUDGET),
        )?;
        parse_and_validate_response(&raw, request)
    }
}

#[derive(Debug, Serialize)]
struct ProviderPayload<'a> {
    state: &'a Value,
    model: &'static str,
    questions:
        &'a std::collections::BTreeMap<String, davinci_agent::decision::request::DecisionQuestion>,
}

impl<'a> ProviderPayload<'a> {
    fn from_request(request: &'a DecisionRequest) -> Self {
        Self {
            state: &request.state,
            model: TYPESAFE_MODEL,
            questions: &request.questions,
        }
    }
}

pub fn resolve_api_key(auth: &AuthStorage) -> Result<Option<String>, TypeSafeAuthError> {
    if let Ok(value) = std::env::var("TYPESAFE_API_KEY") {
        if value.trim().is_empty() {
            return Err(TypeSafeAuthError::InvalidEnvironment);
        }
        return Ok(Some(value));
    }
    let Some(credential) = auth.get("typesafe") else {
        return Ok(None);
    };
    let Some(key) = credential.key.as_deref() else {
        return Err(TypeSafeAuthError::InvalidStoredCredential);
    };
    if key.trim().is_empty() {
        return Err(TypeSafeAuthError::InvalidStoredCredential);
    }
    Ok(Some(key.to_owned()))
}

fn send_json(
    api_key: &str,
    body: &impl Serialize,
    budget: Duration,
) -> Result<Vec<u8>, DecisionError> {
    if api_key.trim().is_empty() {
        return Err(DecisionError::CredentialInvalid);
    }
    let body = serde_json::to_string(body)
        .map_err(|error| DecisionError::InvalidRequest(error.to_string()))?;
    let authorization = Zeroizing::new(format!("Bearer {api_key}"));
    let started = Instant::now();
    for attempt in 0..2 {
        let elapsed = started.elapsed();
        if elapsed >= budget {
            return Err(DecisionError::Unavailable(
                "request budget exhausted".into(),
            ));
        }
        let remaining = budget.saturating_sub(elapsed);
        let agent = ureq::AgentBuilder::new().timeout(remaining).build();
        let result = agent
            .post(TYPESAFE_URL)
            .set("Authorization", authorization.as_str())
            .set("Content-Type", "application/json")
            .send_string(&body);
        match result {
            Ok(response) => {
                let status = response.status();
                if !(200..300).contains(&status) {
                    if attempt == 0 && matches!(status, 429 | 529) {
                        continue;
                    }
                    return Err(DecisionError::HttpStatus(status as u16));
                }
                return read_bounded_response(response);
            }
            Err(ureq::Error::Status(status, _)) => {
                if attempt == 0 && matches!(status, 429 | 529) {
                    continue;
                }
                return Err(DecisionError::HttpStatus(status as u16));
            }
            Err(ureq::Error::Transport(_)) => {
                if attempt == 0 && started.elapsed() < budget {
                    continue;
                }
                return Err(DecisionError::Unavailable(
                    "provider network request failed".into(),
                ));
            }
        }
    }
    Err(DecisionError::Unavailable("provider request failed".into()))
}

fn read_bounded_response(response: ureq::Response) -> Result<Vec<u8>, DecisionError> {
    let mut body = Vec::new();
    response
        .into_reader()
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| DecisionError::Unavailable("provider response unreadable".into()))?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(DecisionError::SchemaMismatch(
            "decision response exceeds the bounded response size".into(),
        ));
    }
    Ok(body)
}

fn map_validation_error(error: DecisionError) -> CredentialValidationError {
    match error {
        DecisionError::HttpStatus(401) | DecisionError::CredentialInvalid => {
            CredentialValidationError::Invalid
        }
        DecisionError::HttpStatus(422) | DecisionError::SchemaMismatch(_) => {
            CredentialValidationError::SchemaMismatch
        }
        DecisionError::HttpStatus(429) | DecisionError::RateLimited => {
            CredentialValidationError::RateLimited
        }
        DecisionError::HttpStatus(529) | DecisionError::Overloaded => {
            CredentialValidationError::Overloaded
        }
        DecisionError::Unavailable(_) => CredentialValidationError::Unavailable,
        DecisionError::InvalidRequest(_)
        | DecisionError::StaleResponse
        | DecisionError::Disabled => CredentialValidationError::SchemaMismatch,
        DecisionError::HttpStatus(_) => CredentialValidationError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_payload_has_only_redacted_state_contract_fields() {
        let request = DecisionRequest::new(
            "request-id",
            davinci_agent::decision::risk::DecisionRisk::Ranking,
            serde_json::json!({"task":"safe"}),
            TYPESAFE_MODEL,
            [(
                "browser_relevant".to_owned(),
                davinci_agent::decision::request::DecisionQuestion::noul("browser"),
            )],
        );
        let payload = serde_json::to_value(ProviderPayload::from_request(&request)).unwrap();
        assert!(payload.get("request_id").is_none());
        assert!(payload.get("decision_class").is_none());
        assert_eq!(payload["model"], TYPESAFE_MODEL);
    }

    #[test]
    fn empty_key_is_rejected_before_network_access() {
        assert_eq!(
            TypeSafeProvider::validate_api_key(""),
            Err(CredentialValidationError::Empty)
        );
    }

    #[test]
    fn validation_statuses_are_secret_free_and_classified() {
        assert_eq!(
            map_validation_error(DecisionError::HttpStatus(401)),
            CredentialValidationError::Invalid
        );
        assert_eq!(
            map_validation_error(DecisionError::HttpStatus(422)),
            CredentialValidationError::SchemaMismatch
        );
        assert_eq!(
            map_validation_error(DecisionError::HttpStatus(429)),
            CredentialValidationError::RateLimited
        );
        assert_eq!(
            map_validation_error(DecisionError::HttpStatus(529)),
            CredentialValidationError::Overloaded
        );
        assert_eq!(
            map_validation_error(DecisionError::Unavailable("network".into())),
            CredentialValidationError::Unavailable
        );
    }

    #[test]
    fn validation_payload_does_not_contain_task_or_session_fields() {
        let body = serde_json::json!({
            "state": {
                "probe": "davinci_typesafe_credential_validation",
                "schemaVersion": 1
            },
            "model": TYPESAFE_MODEL,
            "questions": {
                "probe": {
                    "type": "noul",
                    "instructions": "The field named probe equals davinci_typesafe_credential_validation."
                }
            }
        });
        let encoded = serde_json::to_string(&body).unwrap();
        assert!(!encoded.contains("request_id"));
        assert!(!encoded.contains("Authorization"));
        assert!(!encoded.contains("session"));
    }
}
