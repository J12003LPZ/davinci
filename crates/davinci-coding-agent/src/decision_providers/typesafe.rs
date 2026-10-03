use std::sync::Arc;
use std::time::Duration;

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
    #[error(
        "TypeSafe rejected the credential; use an active direct TypeSafe API key from console.typesafe.ai"
    )]
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
    http: super::typesafe_http::TypeSafeHttp,
}

impl TypeSafeProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        let raw = Zeroizing::new(api_key.into());
        let api_key = normalize_api_key(raw.as_str())
            .unwrap_or_default()
            .to_owned();
        Self {
            api_key: Zeroizing::new(api_key),
            http: super::typesafe_http::TypeSafeHttp::new(TYPESAFE_URL),
        }
    }

    pub fn validate_api_key(api_key: &str) -> Result<(), CredentialValidationError> {
        let api_key = normalize_api_key(api_key).ok_or(CredentialValidationError::Empty)?;
        let request = validation_request();
        let raw = super::typesafe_http::TypeSafeHttp::new(TYPESAFE_URL)
            .send(
                api_key,
                &ProviderPayload::from_request(&request),
                VALIDATION_TIMEOUT,
                1,
            )
            .map_err(map_validation_error)?;
        validate_probe_response(&raw, &request)
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
        request.validate_size()?;
        let raw = self.http.send(
            self.api_key.as_str(),
            &body,
            budget.min(HARD_DECISION_BUDGET),
            2,
        )?;
        parse_and_validate_response(&raw, request)
    }

    fn evaluate_shadow(
        &self,
        request: &DecisionRequest,
        budget: Duration,
    ) -> Result<DecisionResponse, DecisionError> {
        request.validate_size()?;
        let raw = self.http.send(
            self.api_key.as_str(),
            &ProviderPayload::from_request(request),
            budget.min(HARD_DECISION_BUDGET),
            1,
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

/// The key probe exercises one Noul and one Score so that it hits the same
/// response parser as real requests: a response shape the runtime would
/// reject fails here, at key entry, instead of silently in every shadow call.
fn validation_request() -> DecisionRequest {
    use davinci_agent::decision::request::DecisionQuestion;
    DecisionRequest::new(
        "credential-validation",
        davinci_agent::decision::risk::DecisionClass::Ranking,
        serde_json::json!({
            "probe": "davinci_typesafe_credential_validation",
            "schemaVersion": 2
        }),
        TYPESAFE_MODEL,
        [
            (
                "probe".to_owned(),
                DecisionQuestion::noul(
                    "The field named probe equals davinci_typesafe_credential_validation.",
                ),
            ),
            (
                "probe_scale".to_owned(),
                DecisionQuestion::score(
                    "How long is the text in the field named probe?",
                    ["Under ten characters.", "Ten characters or more."],
                ),
            ),
        ],
    )
}

fn validate_probe_response(
    raw: &[u8],
    request: &DecisionRequest,
) -> Result<(), CredentialValidationError> {
    if raw.len() > MAX_RESPONSE_BYTES {
        return Err(CredentialValidationError::SchemaMismatch);
    }
    parse_and_validate_response(raw, request)
        .map(|_| ())
        .map_err(|_| CredentialValidationError::SchemaMismatch)
}

/// Normalize text copied from a credential field or an Authorization header.
///
/// TypeSafe expects the raw key in its Bearer header. Accepting the common
/// copied forms here prevents DaVinci from accidentally sending surrounding
/// whitespace or a duplicated `Bearer` scheme as part of the key.
pub fn normalize_api_key(api_key: &str) -> Option<&str> {
    let trimmed = api_key.trim();
    let credential = match (trimmed.get(..14), trimmed.get(14..)) {
        (Some(header), Some(rest)) if header.eq_ignore_ascii_case("authorization:") => rest.trim(),
        _ => trimmed,
    };
    let normalized = match (credential.get(..6), credential.get(6..)) {
        (Some(scheme), Some(rest)) if scheme.eq_ignore_ascii_case("bearer") && rest.is_empty() => {
            ""
        }
        (Some(scheme), Some(rest))
            if scheme.eq_ignore_ascii_case("bearer")
                && rest.chars().next().is_some_and(char::is_whitespace) =>
        {
            rest.trim()
        }
        _ => trimmed,
    };
    (!normalized.is_empty()).then_some(normalized)
}

pub fn resolve_api_key(auth: &AuthStorage) -> Result<Option<String>, TypeSafeAuthError> {
    if let Ok(value) = std::env::var("TYPESAFE_API_KEY") {
        let key = normalize_api_key(&value).ok_or(TypeSafeAuthError::InvalidEnvironment)?;
        return Ok(Some(key.to_owned()));
    }
    let Some(credential) = auth.get("typesafe") else {
        return Ok(None);
    };
    let Some(key) = credential.key.as_deref() else {
        return Err(TypeSafeAuthError::InvalidStoredCredential);
    };
    let key = normalize_api_key(key).ok_or(TypeSafeAuthError::InvalidStoredCredential)?;
    Ok(Some(key.to_owned()))
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
        DecisionError::Unavailable(_) | DecisionError::Busy => {
            CredentialValidationError::Unavailable
        }
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
    fn pasted_credentials_are_normalized_before_use() {
        for candidate in [
            "direct-key",
            " direct-key ",
            "Bearer direct-key",
            "bearer\tdirect-key\r\n",
            "Authorization: Bearer direct-key",
            "authorization:\tBEARER\tdirect-key\r\n",
        ] {
            assert_eq!(normalize_api_key(candidate), Some("direct-key"));
        }
        assert_eq!(normalize_api_key("  "), None);
        assert_eq!(normalize_api_key("Bearer  \r\n"), None);
        assert_eq!(normalize_api_key("Authorization: Bearer  \r\n"), None);

        let provider = TypeSafeProvider::new(" Bearer direct-key\r\n");
        assert_eq!(provider.api_key.as_str(), "direct-key");
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
        let request = validation_request();
        request
            .validate_size()
            .expect("probe matches the API shape");
        let encoded = serde_json::to_string(&ProviderPayload::from_request(&request)).unwrap();
        assert!(!encoded.contains("request_id"));
        assert!(!encoded.contains("credential-validation"));
        assert!(!encoded.contains("Authorization"));
        assert!(!encoded.contains("session"));
        assert!(!encoded.contains("\"task\""));
    }

    #[test]
    fn probe_accepts_the_documented_response_shape() {
        let raw = br#"{
  "model": "jev-1.13.0",
  "answers": {
    "probe": { "type": "noul", "noul": 0.97 },
    "probe_scale": {
      "type": "score",
      "score": 0.98,
      "legend": { "0": "Under ten characters.", "1": "Ten characters or more." },
      "probabilities": { "0": 0.02, "1": 0.98 },
      "confidence": 0.95
    }
  },
  "usage": { "input_tokens": 120, "output_tokens": 12 }
}"#;
        assert_eq!(validate_probe_response(raw, &validation_request()), Ok(()));
    }

    #[test]
    fn probe_rejects_a_response_the_runtime_parser_would_reject() {
        // Old probe only read answers.probe.noul, so a key could validate
        // while every real request failed the stricter runtime parser.
        let missing_score =
            br#"{"model":"jev-1.13.0","answers":{"probe":{"type":"noul","noul":0.9}}}"#;
        let unknown_top_level = br#"{"answers":{"probe":{"type":"noul","noul":0.9},"probe_scale":{"type":"score","score":1.0,"probabilities":{"0":0.0,"1":1.0},"confidence":1.0}},"extra":1}"#;
        for raw in [&missing_score[..], &unknown_top_level[..]] {
            assert_eq!(
                validate_probe_response(raw, &validation_request()),
                Err(CredentialValidationError::SchemaMismatch)
            );
        }
    }
}

#[cfg(test)]
mod state_contract {
    #[test]
    fn version_two_state_serializes_through_the_actual_provider_payload() {
        let request = crate::decision_state::build_request(
            "contract",
            "review this change",
            davinci_agent::decision::risk::DecisionClass::Ranking,
        );
        let wire = serde_json::to_value(super::ProviderPayload::from_request(&request)).unwrap();
        assert_eq!(wire["state"]["schemaVersion"], 2);
        assert_eq!(wire["state"]["workspaceDirty"], "unknown");
        assert!(wire["state"].get("hasUncommittedChanges").is_none());
        assert_eq!(wire["model"], super::TYPESAFE_MODEL);
        assert!(wire.get("request_id").is_none());
        assert!(wire["questions"]
            .as_object()
            .unwrap()
            .contains_key("test_impact_relevant"));
    }
}
