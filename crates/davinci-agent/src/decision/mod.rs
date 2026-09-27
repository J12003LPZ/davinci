pub mod advice;
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

/// Freshness material attached to a shadow decision. A result may be used only
/// by the submission that created the same key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionAdviceKey {
    pub request_id: String,
    pub generation: u64,
    pub evidence_revision: u64,
    pub mutation_revision: u64,
}

#[derive(Debug, Clone)]
pub struct ReadyDecision {
    pub key: DecisionAdviceKey,
    pub request: DecisionRequest,
    pub response: DecisionResponse,
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
    ready_shadow: Mutex<Option<ReadyDecision>>,
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
            ready_shadow: Mutex::new(None),
        }
    }

    fn clear_ready_shadow(&self) {
        *self
            .ready_shadow
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    pub fn enable(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.enabled.store(true, Ordering::SeqCst);
        self.clear_ready_shadow();
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
        self.clear_ready_shadow();
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
        self.clear_ready_shadow();
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
        self.enqueue_shadow_inner(move || (prepare(), None))
    }

    /// Enqueue a shadow decision and retain a successful response in a
    /// one-element mailbox. The caller never waits for this result.
    pub fn enqueue_shadow_with_result(
        self: &Arc<Self>,
        prepare: impl FnOnce() -> (DecisionRequest, DecisionAdviceKey) + Send + 'static,
    ) -> Result<(), DecisionError> {
        self.enqueue_shadow_inner(move || {
            let (request, key) = prepare();
            (request, Some(key))
        })
    }

    /// Consume the newest successful shadow result, if one is ready.
    pub fn take_ready_shadow(&self) -> Option<ReadyDecision> {
        self.ready_shadow
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn enqueue_shadow_inner(
        self: &Arc<Self>,
        prepare: impl FnOnce() -> (DecisionRequest, Option<DecisionAdviceKey>) + Send + 'static,
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
                let (request, key) = prepare();
                if let Ok(response) = release.0.evaluate_generation(&request, generation, true) {
                    if let Some(key) = key {
                        *release
                            .0
                            .ready_shadow
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                            Some(ReadyDecision {
                                key,
                                request,
                                response,
                            });
                    }
                }
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
        delay: std::time::Duration,
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
            std::thread::sleep(self.delay);
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
            delay: std::time::Duration::ZERO,
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
            delay: std::time::Duration::ZERO,
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
            delay: std::time::Duration::ZERO,
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

    #[test]
    fn disabled_shadow_admission_never_prepares_request() {
        let runtime = std::sync::Arc::new(DecisionRuntime::new(std::sync::Arc::new(FixtureProvider {
            calls: AtomicUsize::new(0), response: Err(DecisionError::CredentialInvalid),
            delay: std::time::Duration::ZERO,
        })));
        assert_eq!(runtime.enqueue_shadow_with_result(|| panic!("disabled preparation ran")), Err(DecisionError::Disabled));
    }

    #[test]
    fn blocked_request_preparation_is_off_submit_path_and_bounded_to_one_worker() {
        let runtime = std::sync::Arc::new(DecisionRuntime::new(std::sync::Arc::new(FixtureProvider {
            calls: AtomicUsize::new(0), response: Err(DecisionError::CredentialInvalid),
            delay: std::time::Duration::ZERO,
        })));
        runtime.enable();
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        let key = advice_key("preparing", runtime.generation());
        let started = std::time::Instant::now();
        runtime.enqueue_shadow_with_result(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
            (request(), key)
        }).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        entered_rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
        assert_eq!(runtime.enqueue_shadow_with_result(|| panic!("busy preparation ran")), Err(DecisionError::Busy));
        release_tx.send(()).unwrap();
    }

    fn wait_for_ready(runtime: &DecisionRuntime) -> Option<super::ReadyDecision> {
        for _ in 0..500 {
            if let Some(ready) = runtime.take_ready_shadow() {
                return Some(ready);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        None
    }

    fn advice_key(id: &str, generation: u64) -> super::DecisionAdviceKey {
        super::DecisionAdviceKey {
            request_id: id.to_owned(),
            generation,
            evidence_revision: 1,
            mutation_revision: 2,
        }
    }

    #[test]
    fn shadow_admission_is_nonblocking_bounded_and_one_slot() {
        let provider = std::sync::Arc::new(FixtureProvider {
            calls: AtomicUsize::new(0),
            response: Ok(
                br#"{"answers":{"browser_relevant":{"type":"noul","noul":1.0}}}"#.to_vec(),
            ),
            delay: std::time::Duration::from_millis(100),
        });
        let runtime = std::sync::Arc::new(DecisionRuntime::new(provider.clone()));
        let generation = runtime.enable();
        let started = std::time::Instant::now();
        runtime
            .enqueue_shadow_with_result({
                let key = advice_key("first", generation);
                move || (request(), key)
            })
            .expect("first shadow admission");
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
        assert_eq!(
            runtime.enqueue_shadow_with_result({
                let key = advice_key("busy", generation);
                move || (request(), key)
            }),
            Err(DecisionError::Busy)
        );

        let first = wait_for_ready(&runtime).expect("first result");
        assert_eq!(first.key.request_id, "first");
        assert!(runtime.take_ready_shadow().is_none());

        runtime
            .enqueue_shadow_with_result({
                let key = advice_key("second", generation);
                move || (request(), key)
            })
            .expect("second shadow admission");
        let second = wait_for_ready(&runtime).expect("second result");
        assert_eq!(second.key.request_id, "second");
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn disabling_or_replacing_generation_discards_late_shadow_results() {
        let provider = std::sync::Arc::new(FixtureProvider {
            calls: AtomicUsize::new(0),
            response: Ok(
                br#"{"answers":{"browser_relevant":{"type":"noul","noul":1.0}}}"#.to_vec(),
            ),
            delay: std::time::Duration::from_millis(50),
        });
        let runtime = std::sync::Arc::new(DecisionRuntime::new(provider));
        let generation = runtime.enable();
        runtime
            .enqueue_shadow_with_result({
                let key = advice_key("late", generation);
                move || (request(), key)
            })
            .expect("shadow admission");
        runtime.disable();
        std::thread::sleep(std::time::Duration::from_millis(80));
        assert!(runtime.take_ready_shadow().is_none());

        runtime.enable();
        runtime
            .enqueue_shadow_with_result({
                let key = advice_key("replaced", runtime.generation());
                move || (request(), key)
            })
            .expect("replacement shadow admission");
        runtime.replace_provider(std::sync::Arc::new(FixtureProvider {
            calls: AtomicUsize::new(0),
            response: Ok(
                br#"{"answers":{"browser_relevant":{"type":"noul","noul":1.0}}}"#.to_vec(),
            ),
            delay: std::time::Duration::ZERO,
        }));
        std::thread::sleep(std::time::Duration::from_millis(80));
        assert!(runtime.take_ready_shadow().is_none());
    }
}
