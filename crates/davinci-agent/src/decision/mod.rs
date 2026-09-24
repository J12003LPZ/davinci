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

/// One finished shadow sample, reduced to metadata a host may persist and
/// later join with what the coding model actually did.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowOutcome {
    pub request_id: String,
    /// `ok`, or the provider health the failure mapped to (`RateLimited`,
    /// `Unavailable`, ...). Never an error message or payload.
    pub outcome: String,
    pub model: Option<String>,
    pub latency_ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub answers: Vec<telemetry::DecisionAnswerTelemetry>,
}

impl ShadowOutcome {
    /// Boundary marker for a turn whose sample was never admitted.
    pub fn not_admitted(error: &DecisionError) -> Self {
        Self {
            request_id: String::new(),
            outcome: match error {
                DecisionError::Busy => "Busy".to_owned(),
                other => format!("{:?}", other.health()),
            },
            model: None,
            latency_ms: 0,
            input_tokens: None,
            output_tokens: None,
            answers: Vec::new(),
        }
    }

    fn from_result(
        request: &DecisionRequest,
        result: &Result<DecisionResponse, DecisionError>,
        latency_ms: u64,
    ) -> Self {
        let mut outcome = Self {
            request_id: request.request_id.clone(),
            outcome: "ok".to_owned(),
            model: None,
            latency_ms,
            input_tokens: None,
            output_tokens: None,
            answers: Vec::new(),
        };
        match result {
            Ok(response) => {
                outcome.model = response.model.clone();
                outcome.input_tokens = response.input_tokens;
                outcome.output_tokens = response.output_tokens;
                outcome.answers = response
                    .answers
                    .iter()
                    .map(|(id, answer)| telemetry::answer_metadata(id, answer, false, true))
                    .collect();
            }
            Err(DecisionError::StaleResponse) => outcome.outcome = "Stale".to_owned(),
            Err(DecisionError::Busy) => outcome.outcome = "Busy".to_owned(),
            Err(error) => outcome.outcome = format!("{:?}", error.health()),
        }
        outcome
    }
}

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
    shadow_busy: AtomicBool,
    provider_busy: Arc<AtomicBool>,
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
            shadow_busy: AtomicBool::new(false),
            provider_busy: Arc::new(AtomicBool::new(false)),
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
        self.evaluate_generation(request, self.generation(), false)
    }

    /// At most one background call; busy shadow samples are dropped rather
    /// than adding a queue or blocking the user's model request.
    pub fn enqueue_shadow(self: &Arc<Self>, request: DecisionRequest) -> Result<(), DecisionError> {
        request.validate_size()?;
        self.enqueue_shadow_with(move || request)
    }

    /// Optional fact validation also belongs off the main-provider submit path.
    /// The same single-worker bound covers request preparation and inference.
    pub fn enqueue_shadow_with(
        self: &Arc<Self>,
        prepare: impl FnOnce() -> DecisionRequest + Send + 'static,
    ) -> Result<(), DecisionError> {
        self.enqueue_shadow_observed(prepare, |_| {})
    }

    /// Like `enqueue_shadow_with`, and hands the finished sample to
    /// `observe` on the worker thread. The outcome is metadata only (no
    /// task text, state, or payload), so the host may persist it.
    pub fn enqueue_shadow_observed(
        self: &Arc<Self>,
        prepare: impl FnOnce() -> DecisionRequest + Send + 'static,
        observe: impl FnOnce(ShadowOutcome) + Send + 'static,
    ) -> Result<(), DecisionError> {
        if !self.is_enabled() {
            return Err(DecisionError::Disabled);
        }
        let generation = self.generation();
        if self
            .shadow_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.telemetry.record_fallback();
            return Err(DecisionError::Busy);
        }
        let runtime = Arc::clone(self);
        std::thread::Builder::new()
            .name("jev-shadow".into())
            .spawn(move || {
                struct Release(Arc<DecisionRuntime>);
                impl Drop for Release {
                    fn drop(&mut self) {
                        self.0.shadow_busy.store(false, Ordering::Release);
                    }
                }
                let release = Release(runtime);
                let request = prepare();
                let started = Instant::now();
                let result = release.0.evaluate_generation(&request, generation, true);
                observe(ShadowOutcome::from_result(
                    &request,
                    &result,
                    started.elapsed().as_millis() as u64,
                ));
            })
            .map_err(|_| {
                self.shadow_busy.store(false, Ordering::Release);
                DecisionError::Unavailable("shadow worker could not start".into())
            })?;
        Ok(())
    }

    fn evaluate_generation(
        &self,
        request: &DecisionRequest,
        generation: u64,
        shadow: bool,
    ) -> Result<DecisionResponse, DecisionError> {
        request.validate_size()?;
        let provider = self
            .provider
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let provider_name = provider.name().to_owned();
        let model = provider.model().to_owned();
        if self.generation() != generation {
            self.telemetry.record_fallback();
            self.audit.push(record_for(
                request,
                provider_name,
                model,
                0,
                DecisionAuditOutcome::Stale,
            ));
            return Err(DecisionError::StaleResponse);
        }
        if !self.is_enabled() {
            return Err(DecisionError::Disabled);
        }
        if let Some(error) = self.cooldown_error() {
            self.telemetry.record_fallback();
            self.audit.push(record_for(
                request,
                provider_name,
                model,
                0,
                DecisionAuditOutcome::Fallback,
            ));
            return Err(error);
        }
        let started = Instant::now();
        self.telemetry.record_request();
        let result = self.call_provider(provider, request.clone(), shadow);
        if started.elapsed() >= NORMAL_DECISION_BUDGET {
            self.telemetry.record_soft_deadline_miss();
        }
        let result = if started.elapsed() >= HARD_DECISION_BUDGET {
            Err(DecisionError::Unavailable(
                "hard decision deadline exceeded".into(),
            ))
        } else {
            result
        };
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
                if error != DecisionError::Busy {
                    self.update_health(&error);
                }
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

    fn call_provider(
        &self,
        provider: Arc<dyn DecisionProvider>,
        request: DecisionRequest,
        shadow: bool,
    ) -> Result<DecisionResponse, DecisionError> {
        if self
            .provider_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(DecisionError::Busy);
        }
        let started = Instant::now();
        let busy = self.provider_busy.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("jev-provider".into())
            .spawn(move || {
                struct Release(Arc<AtomicBool>);
                impl Drop for Release {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }
                let release = Release(busy);
                let result = if shadow {
                    provider.evaluate_shadow(&request, HARD_DECISION_BUDGET)
                } else {
                    provider.evaluate(&request, HARD_DECISION_BUDGET)
                };
                drop(release);
                let _ = tx.send(result);
            })
            .map_err(|_| {
                self.provider_busy.store(false, Ordering::Release);
                DecisionError::Unavailable("provider worker could not start".into())
            })?;
        rx.recv_timeout(HARD_DECISION_BUDGET.saturating_sub(started.elapsed()))
            .map_err(|_| DecisionError::Unavailable("hard decision deadline exceeded".into()))?
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

    fn observe_one(response: Result<Vec<u8>, DecisionError>) -> super::ShadowOutcome {
        let runtime =
            std::sync::Arc::new(DecisionRuntime::new(std::sync::Arc::new(FixtureProvider {
                calls: AtomicUsize::new(0),
                response,
            })));
        runtime.enable();
        let (tx, rx) = std::sync::mpsc::channel();
        runtime
            .enqueue_shadow_observed(request, move |outcome| tx.send(outcome).unwrap())
            .expect("shadow admitted");
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("observer called")
    }

    #[test]
    fn shadow_observer_receives_answer_metadata_without_payload() {
        let outcome = observe_one(Ok(br#"{"model":"jev-1.13.0","answers":{"browser_relevant":{"type":"noul","noul":0.9}},"usage":{"input_tokens":10,"output_tokens":2}}"#.to_vec()));
        assert_eq!(outcome.request_id, "test-request");
        assert_eq!(outcome.outcome, "ok");
        assert_eq!(outcome.model.as_deref(), Some("jev-1.13.0"));
        assert_eq!(outcome.input_tokens, Some(10));
        assert_eq!(outcome.answers.len(), 1);
        assert_eq!(outcome.answers[0].question_id, "browser_relevant");
        assert!(outcome.answers[0].shadow_only);
        let wire = serde_json::to_string(&outcome).unwrap();
        assert!(!wire.contains("redacted"), "state leaked: {wire}");
    }

    #[test]
    fn shadow_observer_reports_failures_as_health_labels_only() {
        let outcome = observe_one(Err(DecisionError::RateLimited));
        assert_eq!(outcome.outcome, "RateLimited");
        assert!(outcome.answers.is_empty());
        let outcome = observe_one(Err(DecisionError::Unavailable(
            "secret-bearing detail".into(),
        )));
        assert_eq!(outcome.outcome, "Unavailable");
        assert!(!serde_json::to_string(&outcome)
            .unwrap()
            .contains("secret-bearing"));
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
