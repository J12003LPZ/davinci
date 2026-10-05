//! Content-free provider telemetry. No TypeScript counterpart.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use davinci_protocol::Usage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Provider counters before inclusive input is split into disjoint buckets.
/// Missing counters stay absent; neither an omitted write count nor an
/// impossible sum is evidence of zero usage. Contains no response content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawOpenAiUsage {
    pub input_total: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub output: Option<u64>,
    pub reasoning: Option<u64>,
    pub total: Option<u64>,
    pub anomalous: bool,
}

impl RawOpenAiUsage {
    pub fn complete(&self) -> bool {
        !self.anomalous
            && self.input_total.is_some()
            && self.cache_read.is_some()
            && self.cache_write.is_some()
            && self.output.is_some()
    }

    fn agrees_with(&self, usage: &Usage) -> bool {
        self.complete()
            && usage
                .input
                .checked_add(usage.cache_read)
                .and_then(|n| n.checked_add(usage.cache_write))
                == self.input_total
            && Some(usage.cache_read) == self.cache_read
            && Some(usage.cache_write) == self.cache_write
            && Some(usage.output) == self.output
            && self.input_total.and_then(|n| n.checked_add(usage.output))
                == Some(usage.total_tokens)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderAttemptObservation {
    pub schema_version: u32,
    pub kind: String,
    pub logical_request_id: String,
    #[serde(default)]
    pub root_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub parent_actor_id: Option<String>,
    #[serde(default)]
    pub parent_logical_request_id: Option<String>,
    pub attempt_id: Option<u64>,
    pub purpose: String,
    pub model: String,
    #[serde(default)]
    pub returned_model: Option<String>,
    #[serde(default)]
    pub returned_service_tier: Option<String>,
    #[serde(default)]
    pub requested_service_tier: Option<String>,
    pub selected_effort: Option<String>,
    pub schema_hash: String,
    pub transport: Option<String>,
    pub elapsed_ms: f64,
    pub duration_ms: Option<f64>,
    pub status: String,
    pub http_status: Option<u16>,
    pub usage: Option<Usage>,
    #[serde(default)]
    pub raw_usage: Option<RawOpenAiUsage>,
    /// None means the producer has no completeness evidence (including old records).
    #[serde(default)]
    pub usage_complete: Option<bool>,
}

impl ProviderAttemptObservation {
    /// Missing response metadata is unknown, including older recorded observations.
    pub fn service_tier_honored(&self) -> Option<bool> {
        let requested = self.requested_service_tier.as_deref()?;
        let returned = self.returned_service_tier.as_deref()?;
        Some(requested == returned)
    }
}

struct State {
    started: Instant,
    template: ProviderAttemptObservation,
    next_attempt: u64,
    events: Vec<ProviderAttemptObservation>,
    budget: Option<Arc<dyn AttemptBudget>>,
    output_limit: Option<u64>,
    purpose_override: Option<String>,
    reconciliation_error: Option<String>,
}

/// Host-owned admission at the actual transport send, including retries.
/// No implementation can grant authority by changing model-visible text.
pub trait AttemptBudget: Send + Sync {
    fn validate_request(
        &self,
        _model: &crate::Model,
        _auth: &crate::ResolvedAuth,
        _options: &crate::StreamOptions,
        _body: &Value,
        _url: &str,
    ) -> Result<(), String> {
        Ok(())
    }
    /// Opaque host binding for durable session admission; absent is unverified.
    fn binding_identity(&self) -> Option<String> {
        None
    }
    fn reserve(
        &self,
        event: &ProviderAttemptObservation,
        output_limit: Option<u64>,
    ) -> Result<(), String>;
    fn reconcile(&self, event: &ProviderAttemptObservation) -> Result<(), String>;
}

struct ProcessBudget {
    root: String,
    actor: String,
    parent_actor: Option<String>,
    output_limit: Option<u64>,
    budget: Arc<dyn AttemptBudget>,
}

static PROCESS_BUDGET: OnceLock<ProcessBudget> = OnceLock::new();

pub fn process_budget_binding() -> Option<(String, Option<u64>)> {
    let budget = PROCESS_BUDGET.get()?;
    Some((budget.budget.binding_identity()?, budget.output_limit))
}

/// Install once at host startup, before workers start. Background threads and
/// auxiliary scopes inherit the same authority; prompts cannot replace it.
pub fn install_process_budget(
    root: String,
    actor: String,
    parent_actor: Option<String>,
    output_limit: Option<u64>,
    budget: Arc<dyn AttemptBudget>,
) -> Result<(), String> {
    if root.is_empty() || actor.is_empty() || output_limit == Some(0) {
        return Err("process budget requires root, actor and positive output limit".into());
    }
    PROCESS_BUDGET
        .set(ProcessBudget {
            root,
            actor,
            parent_actor,
            output_limit,
            budget,
        })
        .map_err(|_| "process budget is already bound".into())
}

pub(crate) fn bounded_output_limit(requested: Option<u64>) -> Option<u64> {
    match PROCESS_BUDGET.get().and_then(|budget| budget.output_limit) {
        Some(limit) => Some(requested.filter(|n| *n > 0).unwrap_or(limit).min(limit)),
        None => requested,
    }
}

pub(crate) fn active_budget() -> Option<Arc<dyn AttemptBudget>> {
    PROCESS_BUDGET.get().map(|p| p.budget.clone()).or_else(|| {
        CURRENT.with(|current| {
            current
                .borrow()
                .as_ref()
                .and_then(|s| s.borrow().budget.clone())
        })
    })
}

pub(crate) fn validate_request(
    model: &crate::Model,
    auth: &crate::ResolvedAuth,
    options: &crate::StreamOptions,
    body: &Value,
    url: &str,
) -> Result<(), String> {
    if let Some(budget) = active_budget() {
        budget.validate_request(model, auth, options, body, url)?;
    }
    Ok(())
}

/// A rejected terminal receipt must not become a successful one-request task.
pub(crate) fn validate_completion() -> Result<(), String> {
    CURRENT.with(|current| {
        match current
            .borrow()
            .as_ref()
            .and_then(|s| s.borrow().reconciliation_error.clone())
        {
            Some(error) => Err(error),
            None => Ok(()),
        }
    })
}

impl State {
    fn record(&mut self, mut event: ProviderAttemptObservation) {
        event.elapsed_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        match self.events.len().cmp(&4096) {
            std::cmp::Ordering::Less => self.events.push(event),
            std::cmp::Ordering::Equal => {
                event.kind = "telemetry_overflow".into();
                event.status = "unknown".into();
                self.events.push(event);
            }
            std::cmp::Ordering::Greater => {}
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
}

/// Scoped to a synchronous coding request, including all harness retries.
/// Rc keeps this guard on the executing thread; nested scopes restore lineage.
pub struct ObservationScope {
    state: Rc<RefCell<State>>,
    previous: Option<Rc<RefCell<State>>>,
}

impl ObservationScope {
    pub fn capture() -> Self {
        let process = PROCESS_BUDGET.get();
        let budget = process.map(|p| p.budget.clone()).or_else(|| {
            CURRENT.with(|current| {
                current
                    .borrow()
                    .as_ref()
                    .and_then(|state| state.borrow().budget.clone())
            })
        });
        let parent = CURRENT.with(|current| {
            current
                .borrow()
                .as_ref()
                .map(|state| state.borrow().template.clone())
        });
        let logical_request_id = uuid::Uuid::new_v4().to_string();
        let state = Rc::new(RefCell::new(State {
            started: Instant::now(),
            template: ProviderAttemptObservation {
                schema_version: 1,
                kind: String::new(),
                root_id: process
                    .map(|p| p.root.clone())
                    .or_else(|| parent.as_ref().and_then(|p| p.root_id.clone()))
                    .or_else(|| Some(logical_request_id.clone())),
                actor_id: parent
                    .as_ref()
                    .and_then(|p| p.actor_id.clone())
                    .or_else(|| process.map(|p| p.actor.clone())),
                parent_actor_id: parent
                    .as_ref()
                    .and_then(|p| p.parent_actor_id.clone())
                    .or_else(|| process.and_then(|p| p.parent_actor.clone())),
                parent_logical_request_id: parent.as_ref().map(|p| p.logical_request_id.clone()),
                logical_request_id,
                attempt_id: None,
                purpose: String::new(),
                model: String::new(),
                returned_model: None,
                returned_service_tier: None,
                requested_service_tier: None,
                selected_effort: None,
                schema_hash: String::new(),
                transport: None,
                elapsed_ms: 0.0,
                duration_ms: None,
                status: "started".into(),
                http_status: None,
                usage: None,
                raw_usage: None,
                usage_complete: None,
            },
            next_attempt: 0,
            events: Vec::new(),
            budget,
            output_limit: None,
            purpose_override: None,
            reconciliation_error: None,
        }));
        let previous = CURRENT.with(|current| current.replace(Some(Rc::clone(&state))));
        Self { state, previous }
    }

    /// Host-only lineage propagated across worker threads; never prompt text.
    pub fn capture_for(root: &str, actor: &str, parent_actor: Option<&str>) -> Self {
        let scope = Self::capture();
        {
            let mut state = scope.state.borrow_mut();
            state.template.root_id =
                Some(PROCESS_BUDGET.get().map_or(root, |p| &p.root).to_owned());
            state.template.actor_id = Some(actor.to_owned());
            state.template.parent_actor_id = parent_actor.map(str::to_owned);
        }
        scope
    }

    pub fn with_budget(self, budget: Arc<dyn AttemptBudget>) -> Self {
        self.state.borrow_mut().budget =
            Some(PROCESS_BUDGET.get().map_or(budget, |p| p.budget.clone()));
        self
    }

    pub fn with_purpose(self, purpose: &str) -> Self {
        self.state.borrow_mut().purpose_override = Some(purpose.into());
        self
    }

    pub fn finish(self, status: &str) -> Vec<ProviderAttemptObservation> {
        let mut state = self.state.borrow_mut();
        if !state.events.is_empty() {
            let mut event = state.template.clone();
            event.kind = "logical_end".into();
            event.status = status.into();
            event.duration_ms = Some(state.started.elapsed().as_secs_f64() * 1000.0);
            state.record(event);
        }
        std::mem::take(&mut state.events)
    }
}

impl Drop for ObservationScope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.previous.take()));
    }
}

pub fn begin_request(purpose: &str, model: &str, effort: Option<&str>, schema: &str) {
    CURRENT.with(|current| {
        if let Some(state) = current.borrow().as_ref() {
            let mut state = state.borrow_mut();
            state.template.purpose = state
                .purpose_override
                .clone()
                .unwrap_or_else(|| purpose.into());
            state.template.model = model.into();
            state.template.selected_effort = effort.map(str::to_owned);
            state.template.schema_hash = schema.into();
            if state.events.is_empty() {
                let mut event = state.template.clone();
                event.kind = "logical_start".into();
                state.record(event);
            }
        }
    });
}

pub(crate) fn dispatch_options(body: &Value) {
    CURRENT.with(|current| {
        if let Some(state) = current.borrow().as_ref() {
            let mut state = state.borrow_mut();
            state.output_limit = ["max_output_tokens", "max_completion_tokens", "max_tokens"]
                .iter()
                .find_map(|key| body.get(key).and_then(Value::as_u64));
            state.template.requested_service_tier = body
                .get("service_tier")
                .and_then(Value::as_str)
                .filter(|value| value.len() <= 256)
                .map(str::to_owned);
        }
    });
}

pub(crate) fn record_returned_identity(response: &Value) {
    CURRENT.with(|current| {
        if let Some(state) = current.borrow().as_ref() {
            let mut state = state.borrow_mut();
            let bounded = |key| {
                response
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|value| value.len() <= 256)
                    .map(str::to_owned)
            };
            state.template.returned_model = bounded("model");
            state.template.returned_service_tier = bounded("service_tier");
        }
    });
}

/// Record raw counters from an OpenAI response in the current host-owned scope.
pub fn record_openai_usage(usage: &Value) {
    CURRENT.with(|current| {
        let current = current.borrow();
        let Some(state) = current.as_ref() else {
            return;
        };
        let fields = [
            usage
                .get("input_tokens")
                .or_else(|| usage.get("prompt_tokens")),
            usage
                .pointer("/input_tokens_details/cached_tokens")
                .or_else(|| usage.pointer("/prompt_tokens_details/cached_tokens")),
            usage
                .pointer("/input_tokens_details/cache_write_tokens")
                .or_else(|| usage.pointer("/prompt_tokens_details/cache_write_tokens")),
            usage
                .get("output_tokens")
                .or_else(|| usage.get("completion_tokens")),
            usage
                .pointer("/output_tokens_details/reasoning_tokens")
                .or_else(|| usage.pointer("/completion_tokens_details/reasoning_tokens")),
            usage.get("total_tokens"),
        ];
        let [input_total, cache_read, cache_write, output, reasoning, total] =
            fields.map(|value| value.and_then(Value::as_u64));
        let mut anomalous = fields
            .iter()
            .any(|value| value.is_some_and(|value| value.as_u64().is_none()));
        if let Some(input) = input_total {
            anomalous |= cache_read.is_some_and(|read| read > input)
                || cache_write.is_some_and(|write| write > input);
        }
        if let (Some(input), Some(read), Some(write)) = (input_total, cache_read, cache_write) {
            anomalous |= read.checked_add(write).is_none_or(|cached| cached > input);
        }
        if let (Some(input), Some(output), Some(total)) = (input_total, output, total) {
            anomalous |= input.checked_add(output) != Some(total);
        }
        if let (Some(reasoning), Some(output)) = (reasoning, output) {
            anomalous |= reasoning > output;
        }
        let raw = RawOpenAiUsage {
            input_total,
            cache_read,
            cache_write,
            output,
            reasoning,
            total,
            anomalous,
        };
        let mut state = state.borrow_mut();
        state.template.usage_complete = Some(raw.complete());
        state.template.raw_usage = Some(raw);
    });
}

/// Created immediately before the actual send, not for socket eligibility.
/// Early returns/panics remain unknown instead of manufacturing completion.
pub struct Attempt {
    state: Option<Rc<RefCell<State>>>,
    event: Option<ProviderAttemptObservation>,
    started: Instant,
}

impl Attempt {
    pub fn start(transport: &str) -> Self {
        Self::try_start(transport).expect("provider admission denied; adapters must use try_start")
    }

    pub fn try_start(transport: &str) -> Result<Self, String> {
        let state = CURRENT.with(|current| current.borrow().clone());
        if state.is_none() && PROCESS_BUDGET.get().is_some() {
            return Err("root budget denied: provider request has no observation scope".into());
        }
        let event = state
            .as_ref()
            .map(|state| -> Result<_, String> {
                let mut state = state.borrow_mut();
                state.next_attempt += 1;
                state.template.raw_usage = None;
                state.template.usage_complete = None;
                state.template.returned_model = None;
                state.template.returned_service_tier = None;
                let mut event = state.template.clone();
                event.kind = "attempt_start".into();
                event.attempt_id = Some(state.next_attempt);
                event.transport = Some(transport.into());
                if let Some(budget) = &state.budget {
                    budget.reserve(&event, state.output_limit)?;
                }
                state.record(event.clone());
                Ok(event)
            })
            .transpose()?;
        Ok(Self {
            state,
            event,
            started: Instant::now(),
        })
    }

    pub fn finish(mut self, status: &str, http_status: Option<u16>, usage: Option<Usage>) {
        self.record_end(status, http_status, usage);
    }

    fn record_end(&mut self, status: &str, http_status: Option<u16>, usage: Option<Usage>) {
        if let (Some(state), Some(mut event)) = (&self.state, self.event.take()) {
            let mut state = state.borrow_mut();
            event.kind = "attempt_end".into();
            event.status = status.into();
            event.http_status = http_status;
            event.usage = usage;
            event.raw_usage = state.template.raw_usage.clone();
            event.usage_complete = event.raw_usage.as_ref().map(|raw| {
                event
                    .usage
                    .as_ref()
                    .is_some_and(|usage| raw.agrees_with(usage))
            });
            event.returned_model = state.template.returned_model.clone();
            event.returned_service_tier = state.template.returned_service_tier.clone();
            event.duration_ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
            if let Some(budget) = &state.budget {
                if let Err(error) = budget.reconcile(&event) {
                    state.reconciliation_error = Some(error);
                    event.status = "unknown".into();
                }
            }
            state.record(event);
        }
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.record_end("unknown", None, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_tier_observation_missing_is_unknown_and_mismatch_is_explicit() {
        for (returned, expected) in [
            (Some("priority"), Some(true)),
            (Some("default"), Some(false)),
            (None, None),
        ] {
            let scope = ObservationScope::capture();
            begin_request("coding", "model", None, "schema");
            dispatch_options(&serde_json::json!({"service_tier":"priority"}));
            if let Some(tier) = returned {
                record_returned_identity(&serde_json::json!({"service_tier":tier}));
            }
            let observations = scope.finish("completed");
            assert_eq!(
                observations.last().unwrap().service_tier_honored(),
                expected
            );
        }
    }

    #[test]
    fn rejected_terminal_receipt_cannot_return_success() {
        struct Reject;
        impl AttemptBudget for Reject {
            fn reserve(
                &self,
                _: &ProviderAttemptObservation,
                _: Option<u64>,
            ) -> Result<(), String> {
                Ok(())
            }
            fn reconcile(&self, _: &ProviderAttemptObservation) -> Result<(), String> {
                Err("subscription-only receipt rejected".into())
            }
        }
        let scope = ObservationScope::capture().with_budget(Arc::new(Reject));
        begin_request("coding", "fixture", None, "fixture");
        Attempt::try_start("http")
            .unwrap()
            .finish("completed", Some(200), None);
        assert!(validate_completion()
            .unwrap_err()
            .contains("subscription-only"));
        assert_eq!(scope.finish("failed")[2].status, "unknown");
    }

    #[test]
    fn harness_attempts_preserve_root_and_nested_lineage() {
        let outer = ObservationScope::capture_for("root", "lead", None);
        begin_request("coding", "model", None, "schema");
        let inner = ObservationScope::capture();
        begin_request("compaction", "model", None, "schema");
        Attempt::start("http").finish("failed", Some(500), None);
        let child = inner.finish("failed");
        Attempt::start("http").finish("completed", Some(200), None);
        let parent = outer.finish("completed");
        assert!(child
            .iter()
            .all(|event| event.root_id.as_deref() == Some("root")));
        assert_eq!(
            child[0].parent_logical_request_id.as_deref(),
            Some(parent[0].logical_request_id.as_str())
        );
        assert!(parent
            .iter()
            .all(|event| event.actor_id.as_deref() == Some("lead")));
    }

    #[test]
    fn harness_unknown_usage_never_becomes_measured_zero() {
        let scope = ObservationScope::capture();
        begin_request("coding", "model", None, "schema");
        let attempt = Attempt::start("http");
        record_openai_usage(&serde_json::json!({
            "input_tokens": 100, "output_tokens": 10,
            "input_tokens_details": {"cached_tokens": 50}
        }));
        attempt.finish("completed", Some(200), Some(Usage::default()));
        let second = Attempt::start("http");
        record_openai_usage(&serde_json::json!({
            "input_tokens": 100, "output_tokens": 10, "total_tokens": 110,
            "input_tokens_details": {"cached_tokens": 50, "cache_write_tokens": 20}
        }));
        second.finish(
            "completed",
            Some(200),
            Some(Usage {
                input: 30,
                output: 10,
                cache_read: 50,
                cache_write: 20,
                total_tokens: 110,
                ..Usage::default()
            }),
        );
        drop(Attempt::start("http"));
        let events = scope.finish("completed");
        let ends: Vec<_> = events
            .iter()
            .filter(|event| event.kind == "attempt_end")
            .collect();
        assert_eq!(ends[0].usage_complete, Some(false));
        assert_eq!(ends[0].raw_usage.as_ref().unwrap().cache_write, None);
        assert_eq!(ends[1].usage_complete, Some(true));
        assert_eq!(ends[2].usage_complete, None);
        assert_eq!(ends[2].raw_usage, None);
    }

    #[test]
    fn harness_normalized_usage_must_match_raw_counters() {
        for normalized in [None, Some(Usage::default())] {
            let scope = ObservationScope::capture();
            begin_request("coding", "model", None, "schema");
            let attempt = Attempt::start("http");
            record_openai_usage(&serde_json::json!({
                "input_tokens": 100, "output_tokens": 10,
                "input_tokens_details": {"cached_tokens": 50, "cache_write_tokens": 20}
            }));
            attempt.finish("completed", Some(200), normalized);
            let events = scope.finish("completed");
            assert_eq!(
                events
                    .iter()
                    .find(|event| event.kind == "attempt_end")
                    .unwrap()
                    .usage_complete,
                Some(false)
            );
        }
    }

    #[test]
    fn retries_share_logical_identity_and_distinct_attempts() {
        let scope = ObservationScope::capture();
        begin_request("coding", "test/model", Some("medium"), "schema");
        Attempt::start("http").finish("failed", Some(429), None);
        begin_request("coding", "test/model", Some("medium"), "schema");
        Attempt::start("websocket").finish("completed", None, None);
        let events = scope.finish("completed");
        assert_eq!(
            events.iter().filter(|e| e.kind == "logical_start").count(),
            1
        );
        let starts: Vec<_> = events
            .iter()
            .filter(|e| e.kind == "attempt_start")
            .collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[0].logical_request_id, starts[1].logical_request_id);
        assert_ne!(starts[0].attempt_id, starts[1].attempt_id);
        assert_eq!(events.last().unwrap().kind, "logical_end");
    }

    #[test]
    fn unfinished_attempt_is_unknown_and_nested_scope_restores_parent() {
        let outer = ObservationScope::capture();
        begin_request("coding", "outer", None, "schema");
        {
            let inner = ObservationScope::capture();
            begin_request("prewarm", "inner", None, "schema");
            drop(Attempt::start("websocket"));
            let events = inner.finish("unknown");
            assert!(events.iter().all(|e| e.purpose == "prewarm"));
            assert!(events
                .iter()
                .any(|e| e.kind == "attempt_end" && e.status == "unknown"));
        }
        Attempt::start("http").finish("completed", Some(200), None);
        assert!(outer.finish("completed").iter().all(|e| e.model == "outer"));
    }

    #[test]
    fn no_dispatch_is_not_a_model_request() {
        assert!(ObservationScope::capture().finish("failed").is_empty());
    }

    /// Offline diagnostic only: metadata generation and serialization, no provider latency.
    #[test]
    #[ignore = "explicit release-mode telemetry overhead measurement"]
    fn telemetry_metadata_overhead() {
        use sha2::Digest;
        let schema = serde_json::json!({"tools": "x".repeat(65_536)});
        let mut samples = Vec::new();
        for _ in 0..21 {
            let started = Instant::now();
            for _ in 0..1000 {
                let scope = ObservationScope::capture();
                let hash = format!(
                    "{:x}",
                    davinci_sys::hex::Lower(&sha2::Sha256::digest(serde_json::to_vec(&schema.get("tools")).unwrap()))
                );
                begin_request("coding", "fixture", Some("medium"), &hash);
                Attempt::start("http").finish("completed", Some(200), Some(Usage::default()));
                std::hint::black_box(serde_json::to_vec(&scope.finish("completed")).unwrap());
            }
            samples.push(started.elapsed().as_secs_f64() / 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        println!("telemetry_overhead schema_bytes=65536 iterations=21000 median_seconds_per_request={} p95_seconds_per_request={}", samples[10], samples[19]);
    }
}
