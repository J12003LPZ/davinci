//! Content-free provider telemetry. No TypeScript counterpart.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use davinci_protocol::Usage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderAttemptObservation {
    pub schema_version: u32,
    pub kind: String,
    pub logical_request_id: String,
    pub attempt_id: Option<u64>,
    pub purpose: String,
    pub model: String,
    pub selected_effort: Option<String>,
    pub schema_hash: String,
    pub transport: Option<String>,
    pub elapsed_ms: f64,
    pub duration_ms: Option<f64>,
    pub status: String,
    pub http_status: Option<u16>,
    pub usage: Option<Usage>,
}

struct State {
    started: Instant,
    template: ProviderAttemptObservation,
    next_attempt: u64,
    events: Vec<ProviderAttemptObservation>,
}

impl State {
    fn record(&mut self, mut event: ProviderAttemptObservation) {
        event.elapsed_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        if self.events.len() < 4096 {
            self.events.push(event);
        } else if self.events.len() == 4096 {
            event.kind = "telemetry_overflow".into();
            event.status = "unknown".into();
            self.events.push(event);
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
        let state = Rc::new(RefCell::new(State {
            started: Instant::now(),
            template: ProviderAttemptObservation {
                schema_version: 1,
                kind: String::new(),
                logical_request_id: uuid::Uuid::new_v4().to_string(),
                attempt_id: None,
                purpose: String::new(),
                model: String::new(),
                selected_effort: None,
                schema_hash: String::new(),
                transport: None,
                elapsed_ms: 0.0,
                duration_ms: None,
                status: "started".into(),
                http_status: None,
                usage: None,
            },
            next_attempt: 0,
            events: Vec::new(),
        }));
        let previous = CURRENT.with(|current| current.replace(Some(Rc::clone(&state))));
        Self { state, previous }
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
            state.template.purpose = purpose.into();
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

/// Created immediately before the actual send, not for socket eligibility.
/// Early returns/panics remain unknown instead of manufacturing completion.
pub struct Attempt {
    state: Option<Rc<RefCell<State>>>,
    event: Option<ProviderAttemptObservation>,
    started: Instant,
}

impl Attempt {
    pub fn start(transport: &str) -> Self {
        let state = CURRENT.with(|current| current.borrow().clone());
        let event = state.as_ref().map(|state| {
            let mut state = state.borrow_mut();
            state.next_attempt += 1;
            let mut event = state.template.clone();
            event.kind = "attempt_start".into();
            event.attempt_id = Some(state.next_attempt);
            event.transport = Some(transport.into());
            state.record(event.clone());
            event
        });
        Self {
            state,
            event,
            started: Instant::now(),
        }
    }

    pub fn finish(mut self, status: &str, http_status: Option<u16>, usage: Option<Usage>) {
        self.record_end(status, http_status, usage);
    }

    fn record_end(&mut self, status: &str, http_status: Option<u16>, usage: Option<Usage>) {
        if let (Some(state), Some(mut event)) = (&self.state, self.event.take()) {
            event.kind = "attempt_end".into();
            event.status = status.into();
            event.http_status = http_status;
            event.usage = usage;
            event.duration_ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
            state.borrow_mut().record(event);
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
                    sha2::Sha256::digest(serde_json::to_vec(&schema.get("tools")).unwrap())
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
