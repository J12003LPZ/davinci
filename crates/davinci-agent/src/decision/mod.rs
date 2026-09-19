pub mod audit;
pub mod calibration;
pub mod policy;
pub mod provider;
pub mod request;
pub mod response;
pub mod risk;
pub mod telemetry;

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use audit::{record_for, DecisionAuditLog, DecisionAuditOutcome};
use provider::{DecisionError, DecisionProvider, DecisionProviderHealth};
use request::DecisionRequest;
use response::DecisionResponse;
use telemetry::DecisionTelemetry;

pub const NORMAL_DECISION_BUDGET: Duration = Duration::from_millis(800);
pub const HARD_DECISION_BUDGET: Duration = Duration::from_millis(1500);
const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(30);
const OVERLOAD_COOLDOWN: Duration = Duration::from_secs(10);
const NETWORK_COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HealthState {
    health: DecisionProviderHealth,
    blocked_until: Option<Instant>,
}

impl Default for HealthState {
    fn default() -> Self {
        Self {
            health: DecisionProviderHealth::Disabled,
            blocked_until: None,
        }
    }
}

pub struct DecisionRuntime {
    provider: RwLock<Arc<dyn DecisionProvider>>,
    enabled: AtomicBool,
    generation: AtomicU64,
    health: Mutex<HealthState>,
    last_reported_health: Mutex<DecisionProviderHealth>,
    telemetry: Arc<DecisionTelemetry>,
    audit: Arc<DecisionAuditLog>,
}

impl fmt::Debug for DecisionRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecisionRuntime")
            .field("enabled", &self.is_enabled())
            .field("generation", &self.generation())
            .field("health", &self.health())
            .finish_non_exhaustive()
    }
}

impl DecisionRuntime {
    pub fn new(provider: Arc<dyn DecisionProvider>) -> Self {
        Self {
            provider: RwLock::new(provider),
            enabled: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            health: Mutex::new(HealthState::default()),
            last_reported_health: Mutex::new(DecisionProviderHealth::Disabled),
            telemetry: Arc::new(DecisionTelemetry::default()),
            audit: Arc::new(DecisionAuditLog::default()),
        }
    }

    pub fn enable(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.enabled.store(true, Ordering::SeqCst);
        *self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = HealthState {
            health: DecisionProviderHealth::Ready,
            blocked_until: None,
        };
        *self
            .last_reported_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = DecisionProviderHealth::Ready;
        generation
    }

    pub fn disable(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.enabled.store(false, Ordering::SeqCst);
        *self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = HealthState::default();
        *self
            .last_reported_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = DecisionProviderHealth::Disabled;
        generation
    }

    pub fn replace_provider(&self, provider: Arc<dyn DecisionProvider>) -> u64 {
        *self
            .provider
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = provider;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if self.enabled.load(Ordering::SeqCst) {
            *self
                .health
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = HealthState {
                health: DecisionProviderHealth::Ready,
                blocked_until: None,
            };
            *self
                .last_reported_health
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = DecisionProviderHealth::Ready;
        }
        generation
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn health(&self) -> DecisionProviderHealth {
        self.health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .health
    }

    pub fn telemetry(&self) -> Arc<DecisionTelemetry> {
        Arc::clone(&self.telemetry)
    }

    pub fn audit(&self) -> Arc<DecisionAuditLog> {
        Arc::clone(&self.audit)
    }

    /// Returns a health state once, only when it differs from the last state
    /// observed by the host. The host can turn this into a secret-free notice
    /// without putting provider payloads or credentials into the transcript.
    pub fn take_health_transition(&self) -> Option<DecisionProviderHealth> {
        let current = self.health();
        let mut reported = self
            .last_reported_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *reported == current {
            return None;
        }
        *reported = current;
        Some(current)
    }

    pub fn evaluate(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        if !self.is_enabled() {
            return Err(DecisionError::Disabled);
        }
        request.validate_size()?;
        if let Some(error) = self.cooldown_error() {
            self.telemetry.record_fallback();
            return Err(error);
        }

        let generation = self.generation();
        let provider = self
            .provider
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let provider_name = provider.name().to_owned();
        let model = provider.model().to_owned();
        let started = Instant::now();
        self.telemetry.record_request();
        let result = provider.evaluate(request, NORMAL_DECISION_BUDGET.min(HARD_DECISION_BUDGET));
        let latency_ms = started.elapsed().as_millis() as u64;

        if self.generation() != generation {
            self.telemetry.record_fallback();
            self.audit.push(record_for(
                request,
                provider_name,
                model,
                latency_ms,
                DecisionAuditOutcome::Stale,
            ));
            return Err(DecisionError::StaleResponse);
        }

        match result {
            Ok(response) => {
                *self
                    .health
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = HealthState {
                    health: DecisionProviderHealth::Ready,
                    blocked_until: None,
                };
                self.telemetry.record_success(
                    latency_ms,
                    response.input_tokens,
                    response.output_tokens,
                );
                self.telemetry.record_answers(&response, false, true);
                self.audit.push(record_for(
                    request,
                    provider_name,
                    model,
                    latency_ms,
                    DecisionAuditOutcome::Success,
                ));
                Ok(response)
            }
            Err(error) => {
                self.update_health(&error);
                if matches!(error, DecisionError::Unavailable(_))
                    && latency_ms >= NORMAL_DECISION_BUDGET.as_millis() as u64
                {
                    self.telemetry.record_timeout();
                }
                self.telemetry.record_failure(&error, latency_ms);
                self.telemetry.record_fallback();
                self.audit.push(record_for(
                    request,
                    provider_name,
                    model,
                    latency_ms,
                    DecisionAuditOutcome::Fallback,
                ));
                Err(error)
            }
        }
    }

    fn cooldown_error(&self) -> Option<DecisionError> {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(blocked_until) = health.blocked_until else {
            return match health.health {
                // A 401 invalidates the session credential. Keep the runtime
                // fail-closed until an explicit enable or credential replace
                // resets the generation and health state.
                DecisionProviderHealth::CredentialInvalid => Some(DecisionError::CredentialInvalid),
                DecisionProviderHealth::Disabled => Some(DecisionError::Disabled),
                _ => None,
            };
        };
        if Instant::now() < blocked_until {
            return Some(match health.health {
                DecisionProviderHealth::RateLimited => DecisionError::RateLimited,
                DecisionProviderHealth::Overloaded => DecisionError::Overloaded,
                DecisionProviderHealth::Unavailable => {
                    DecisionError::Unavailable("provider cooldown active".to_owned())
                }
                DecisionProviderHealth::CredentialInvalid => DecisionError::CredentialInvalid,
                DecisionProviderHealth::SchemaMismatch => {
                    DecisionError::SchemaMismatch("provider schema mismatch".to_owned())
                }
                DecisionProviderHealth::Disabled => DecisionError::Disabled,
                DecisionProviderHealth::Ready => return None,
            });
        }
        health.blocked_until = None;
        health.health = DecisionProviderHealth::Ready;
        None
    }

    fn update_health(&self, error: &DecisionError) {
        let cooldown = match error.health() {
            DecisionProviderHealth::RateLimited => Some(RATE_LIMIT_COOLDOWN),
            DecisionProviderHealth::Overloaded => Some(OVERLOAD_COOLDOWN),
            DecisionProviderHealth::Unavailable => Some(NETWORK_COOLDOWN),
            _ => None,
        };
        *self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = HealthState {
            health: error.health(),
            blocked_until: cooldown.map(|duration| Instant::now() + duration),
        };
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::policy::{add_optional_capabilities, DecisionRollout, OptionalCapability};
    use super::provider::{DecisionError, DecisionProvider};
    use super::request::{DecisionQuestion, DecisionRequest};
    use super::response::{parse_and_validate_response, DecisionAnswer};
    use super::{DecisionRuntime, HARD_DECISION_BUDGET};

    struct FixtureProvider {
        calls: AtomicUsize,
        response: Result<Vec<u8>, DecisionError>,
    }

    impl DecisionProvider for FixtureProvider {
        fn name(&self) -> &'static str {
            "fixture"
        }

        fn model(&self) -> &'static str {
            "fixture-model"
        }

        fn evaluate(
            &self,
            request: &DecisionRequest,
            _budget: std::time::Duration,
        ) -> Result<super::response::DecisionResponse, DecisionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let raw = self.response.clone()?;
            parse_and_validate_response(&raw, request)
        }
    }

    fn request() -> DecisionRequest {
        DecisionRequest::new(
            "test-request",
            super::risk::DecisionRisk::Ranking,
            json!({"task":"redacted"}),
            "fixture-model",
            [(
                "browser_relevant".to_owned(),
                DecisionQuestion::noul("browser"),
            )],
        )
    }

    #[test]
    fn off_runtime_does_not_call_provider() {
        let provider = std::sync::Arc::new(FixtureProvider {
            calls: AtomicUsize::new(0),
            response: Ok(
                br#"{"answers":{"browser_relevant":{"type":"noul","noul":1.0}}}"#.to_vec(),
            ),
        });
        let runtime = DecisionRuntime::new(provider.clone());
        assert_eq!(runtime.evaluate(&request()), Err(DecisionError::Disabled));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn disable_invalidates_in_flight_generation() {
        let provider = std::sync::Arc::new(FixtureProvider {
            calls: AtomicUsize::new(0),
            response: Ok(
                br#"{"answers":{"browser_relevant":{"type":"noul","noul":1.0}}}"#.to_vec(),
            ),
        });
        let runtime = DecisionRuntime::new(provider);
        let generation = runtime.enable();
        assert!(generation > 0);
        assert!(runtime.is_enabled());
        runtime.disable();
        assert!(!runtime.is_enabled());
        assert!(runtime.generation() > generation);
    }

    #[test]
    fn credential_invalid_stops_repeated_calls_until_reset() {
        let provider = std::sync::Arc::new(FixtureProvider {
            calls: AtomicUsize::new(0),
            response: Err(DecisionError::CredentialInvalid),
        });
        let runtime = DecisionRuntime::new(provider.clone());
        runtime.enable();
        assert_eq!(
            runtime.evaluate(&request()),
            Err(DecisionError::CredentialInvalid)
        );
        assert_eq!(
            runtime.evaluate(&request()),
            Err(DecisionError::CredentialInvalid)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

        runtime.enable();
        assert_eq!(
            runtime.evaluate(&request()),
            Err(DecisionError::CredentialInvalid)
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn additive_policy_never_removes_or_vetoes_deterministic_requirements() {
        let deterministic = vec!["mandatory_verification".to_owned()];
        let mut vetoed = OptionalCapability::new("dangerous", 1.0);
        vetoed.vetoed = true;
        let optional = vec![vetoed, OptionalCapability::new("browser", 0.90)];
        assert_eq!(
            add_optional_capabilities(&deterministic, &optional, DecisionRollout::GuardedAdditive),
            vec!["mandatory_verification", "browser"]
        );
        assert_eq!(
            add_optional_capabilities(&deterministic, &optional, DecisionRollout::Shadow),
            deterministic
        );
    }

    #[test]
    fn response_validation_rejects_bad_probability_and_unknown_fields() {
        let request = request();
        let bad = br#"{"answers":{"browser_relevant":{"type":"noul","noul":1.2}}}"#;
        assert!(matches!(
            parse_and_validate_response(bad, &request),
            Err(DecisionError::SchemaMismatch(_))
        ));
        let unknown =
            br#"{"answers":{"browser_relevant":{"type":"noul","noul":1.0,"secret":"x"}}}"#;
        assert!(matches!(
            parse_and_validate_response(unknown, &request),
            Err(DecisionError::SchemaMismatch(_))
        ));
    }

    #[test]
    fn response_parser_preserves_noul_answer_without_raw_payload() {
        let parsed = parse_and_validate_response(
            br#"{"answers":{"browser_relevant":{"type":"noul","noul":0.9}}}"#,
            &request(),
        )
        .expect("valid fixture response");
        assert!(matches!(
            parsed.answers.get("browser_relevant"),
            Some(DecisionAnswer::Noul { value }) if (*value - 0.9).abs() < 0.001
        ));
        assert!(HARD_DECISION_BUDGET.as_millis() <= 1500);
    }
}
