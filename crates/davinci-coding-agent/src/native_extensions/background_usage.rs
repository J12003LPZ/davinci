//! Process-lifetime provider receipts for existing session background reviews.
//! No upstream TypeScript counterpart. Reservations never become measurements.

use super::security_scan::usage::{MeasuredTokens, RequestUsage};
use super::NativeExtensionHost;
use davinci_agent::Agent;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Debug, Default)]
struct Totals {
    requests: u64,
    pending: u64,
    unknown_tokens: u64,
    unknown_cost: u64,
    failed: u64,
    measured: Option<MeasuredTokens>,
    cost: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Counter(Arc<Mutex<Totals>>);

/// Existing provider telemetry identifies actual sends across retries and
/// transport fallback. A terminal assistant repeats the last attempt's usage.
#[derive(Default)]
pub(crate) struct ProviderReceipts {
    attempts: Vec<Option<RequestUsage>>,
    indices: HashMap<(String, u64), usize>,
    retries: usize,
    overflow: bool,
}

impl ProviderReceipts {
    pub(crate) fn retry(&mut self) {
        self.retries = self.retries.saturating_add(1);
    }

    pub(crate) fn observe(&mut self, observation: &Value) {
        let kind = observation["kind"].as_str();
        if kind == Some("telemetry_overflow") {
            self.overflow = true;
            return;
        }
        if !matches!(kind, Some("attempt_start" | "attempt_end")) {
            return;
        }
        let Some((logical, attempt)) = observation["logical_request_id"]
            .as_str()
            .filter(|logical| !logical.is_empty())
            .zip(observation["attempt_id"].as_u64())
        else {
            return;
        };
        let key = (logical.to_string(), attempt);
        let index = if let Some(index) = self.indices.get(&key) {
            *index
        } else {
            let index = self.attempts.len();
            self.indices.insert(key, index);
            self.attempts.push(None);
            index
        };
        if kind == Some("attempt_end") && self.attempts[index].is_none() {
            let usage =
                serde_json::from_value::<davinci_protocol::Usage>(observation["usage"].clone())
                    .ok();
            self.attempts[index] = Some(RequestUsage::new(
                usage.as_ref(),
                0,
                0,
                observation["status"] != "completed",
            ));
        }
    }

    pub(crate) fn has_activity(&self) -> bool {
        !self.attempts.is_empty() || self.retries > 0 || self.overflow
    }

    pub(crate) fn take(&mut self, terminal: Option<&RequestUsage>) -> Vec<RequestUsage> {
        let terminal_failed = terminal.is_some_and(|usage| usage.failed);
        let unknown = RequestUsage::new(None, 0, 0, true);
        let terminal = terminal.unwrap_or(&unknown);
        let mut receipts = Vec::new();
        if self.attempts.is_empty() {
            receipts.extend((0..self.retries).map(|_| RequestUsage::new(None, 0, 0, true)));
            receipts.push(terminal.clone());
        } else {
            // Retry markers schedule work; a cancellation may prevent its
            // dispatch. Actual send observations determine the request count.
            receipts.extend(
                self.attempts
                    .drain(..)
                    .map(|usage| usage.unwrap_or_else(|| RequestUsage::new(None, 0, 0, true))),
            );
            if let Some(last) = receipts.last_mut() {
                last.failed |= terminal_failed;
            }
        }
        if self.overflow {
            receipts.push(RequestUsage::new(None, 0, 0, true));
        }
        *self = Self::default();
        receipts
    }
}

impl Counter {
    pub fn start(&self) {
        let mut totals = self.0.lock().unwrap_or_else(|e| e.into_inner());
        totals.requests += 1;
        totals.pending += 1;
    }

    pub fn record(&self, usage: &RequestUsage) {
        let mut totals = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if totals.pending == 0 {
            totals.requests += 1;
        } else {
            totals.pending -= 1;
        }
        totals.failed += u64::from(usage.failed);
        if let Some(tokens) = &usage.measured {
            let measured = totals.measured.get_or_insert(MeasuredTokens {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                total: 0,
            });
            measured.input = measured.input.saturating_add(tokens.input);
            measured.output = measured.output.saturating_add(tokens.output);
            measured.cache_read = measured.cache_read.saturating_add(tokens.cache_read);
            measured.cache_write = measured.cache_write.saturating_add(tokens.cache_write);
            measured.total = measured.total.saturating_add(tokens.total);
        } else {
            totals.unknown_tokens += 1;
        }
        if let Some(cost) = usage.estimated_cost_usd {
            totals.cost += cost;
        } else {
            totals.unknown_cost += 1;
        }
    }

    pub fn snapshot(&self) -> Value {
        let totals = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let zero = json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0});
        let measured = totals.measured.as_ref().map(|tokens| json!(tokens));
        json!({
            "requests":totals.requests, "pendingRequests":totals.pending,
            "failedRequests":totals.failed,
            "unknownTokenRequests":totals.unknown_tokens,
            "unknownCostRequests":totals.unknown_cost,
            "tokens": if totals.unknown_tokens + totals.pending == 0 {
                measured.clone().unwrap_or(zero)
            } else { Value::Null },
            "measuredTokens":measured,
            "estimatedCostUsd":if totals.unknown_cost + totals.pending == 0 {
                json!(totals.cost)
            } else { Value::Null },
            "measuredEstimatedCostUsd":if totals.requests > totals.unknown_cost + totals.pending {
                json!(totals.cost)
            } else { Value::Null },
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct BackgroundUsage {
    pub learning: Counter,
    pub security_watch: Counter,
}

impl BackgroundUsage {
    pub fn snapshot(&self) -> Value {
        json!({
            "scope":"current process; not persisted in session history",
            "learning":self.learning.snapshot(),
            "securityWatch":self.security_watch.snapshot(),
        })
    }
}

#[derive(Default)]
struct Registration {
    usage: BackgroundUsage,
    host: Weak<Mutex<NativeExtensionHost>>,
    owner: Weak<Mutex<davinci_agent::jobs::JobBook>>,
}

struct LiveHost {
    host: Weak<Mutex<NativeExtensionHost>>,
    owner: Weak<Mutex<davinci_agent::jobs::JobBook>>,
}

fn registry() -> &'static Mutex<HashMap<String, Registration>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Registration>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn live_hosts() -> &'static Mutex<HashMap<usize, LiveHost>> {
    static HOSTS: OnceLock<Mutex<HashMap<usize, LiveHost>>> = OnceLock::new();
    HOSTS.get_or_init(Default::default)
}

fn ensure_bound(agent: &Agent) {
    let host = live_hosts()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(Arc::as_ptr(&agent.tool_context.jobs) as usize))
        .filter(|entry| {
            entry
                .owner
                .upgrade()
                .is_some_and(|owner| Arc::ptr_eq(&owner, &agent.tool_context.jobs))
        })
        .and_then(|entry| entry.host.upgrade());
    if let Some(host) = host {
        bind(agent, &host);
    }
}

fn key(agent: &Agent) -> String {
    agent.session.as_ref().map_or_else(
        || format!("ephemeral:{:p}", Arc::as_ptr(&agent.tool_context.jobs)),
        |session| format!("session:{}:{}", session.path.display(), session.header.id),
    )
}

/// Bind before every turn and after a session switch. Workers clone their
/// counter at dispatch, keeping in-flight usage attributed to its origin.
pub fn bind(agent: &Agent, host: &Arc<Mutex<NativeExtensionHost>>) {
    let key = key(agent);
    let mut native = host.lock().unwrap_or_else(|e| e.into_inner());
    {
        let mut hosts = live_hosts().lock().unwrap_or_else(|e| e.into_inner());
        hosts.retain(|_, entry| entry.host.strong_count() > 0 && entry.owner.strong_count() > 0);
        hosts.insert(
            Arc::as_ptr(&agent.tool_context.jobs) as usize,
            LiveHost {
                host: Arc::downgrade(host),
                owner: Arc::downgrade(&agent.tool_context.jobs),
            },
        );
    }
    if native.accounting_key.as_deref() == Some(key.as_str()) {
        return;
    }
    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    // Reuse in-memory receipts across reloads without inheriting measurements
    // when an allocator reuses a departed agent's address.
    if agent.session.is_none()
        && registry.get(&key).is_some_and(|entry| {
            entry
                .owner
                .upgrade()
                .is_none_or(|owner| !Arc::ptr_eq(&owner, &agent.tool_context.jobs))
        })
    {
        registry.remove(&key);
    }
    let entry = registry.entry(key.clone()).or_default();
    entry.host = Arc::downgrade(host);
    entry.owner = Arc::downgrade(&agent.tool_context.jobs);
    native.accounting_key = Some(key);
    native.background = entry.usage.clone();
    native
        .learning
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .background_usage = native.background.learning.clone();
    native
        .security
        .set_watch_usage(native.background.security_watch.clone());
}

pub fn for_agent(agent: &Agent) -> Value {
    ensure_bound(agent);
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key(agent))
        .map(|entry| entry.usage.snapshot())
        .unwrap_or_else(|| {
            json!({
                "scope":"unavailable; background host not bound in this process",
                "learning":{"tokens":null,"estimatedCostUsd":null,"requests":null},
                "securityWatch":{"tokens":null,"estimatedCostUsd":null,"requests":null},
            })
        })
}

pub fn native_status(agent: &Agent) -> Option<Value> {
    ensure_bound(agent);
    let host = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key(agent))?
        .host
        .upgrade()?;
    let mut host = host.lock().unwrap_or_else(|e| e.into_inner());
    Some(host.status_sections())
}

pub fn lsp_status(agent: &Agent) -> Option<Value> {
    ensure_bound(agent);
    let host = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key(agent))?
        .host
        .upgrade()?;
    let host = host.lock().unwrap_or_else(|e| e.into_inner());
    Some(host.language_intelligence.status())
}

pub fn render(background: &Value) -> String {
    [
        ("learning", "learning review"),
        ("securityWatch", "security watch"),
    ]
    .into_iter()
    .map(|(key, label)| {
        let usage = &background[key];
        let tokens = if let Some(total) = usage["tokens"]["total"].as_u64() {
            format!(
                "input {} · output {} · cache read {} · cache write {} · total {total}",
                usage["tokens"]["input"],
                usage["tokens"]["output"],
                usage["tokens"]["cacheRead"],
                usage["tokens"]["cacheWrite"]
            )
        } else {
            let partial = usage["measuredTokens"]["total"]
                .as_u64()
                .map(|total| format!(" ({total} measured so far)"))
                .unwrap_or_default();
            format!("tokens unknown{partial}")
        };
        let cost = usage["estimatedCostUsd"]
            .as_f64()
            .map(|cost| format!("estimated ${cost:.4}"))
            .unwrap_or_else(|| "estimated cost unknown".into());
        let count = |key: &str| {
            usage[key]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "unknown".into())
        };
        format!(
            "background {label}: {tokens} · {cost} · {} requests · {} pending · {} failed",
            usage["requests"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "unknown".into()),
            count("pendingRequests"),
            count("failedRequests")
        )
    })
    .collect::<Vec<_>>()
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_protocol::Usage;

    #[test]
    fn session_switch_and_host_reload_preserve_origin_of_inflight_receipts() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("fixture");
        let first =
            davinci_session::JsonlSession::create_in_directory(dir.path(), "/fixture", None)
                .unwrap();
        agent.session = Some(first);
        let host = Arc::new(Mutex::new(NativeExtensionHost::default()));
        bind(&agent, &host);
        let old = host.lock().unwrap().background.learning.clone();
        old.start();
        let first_session = agent.session.take();
        agent.session = Some(
            davinci_session::JsonlSession::create_in_directory(dir.path(), "/fixture", None)
                .unwrap(),
        );
        bind(&agent, &host);
        old.record(&RequestUsage::new(
            Some(&Usage {
                input: 10,
                output: 5,
                total_tokens: 15,
                ..Default::default()
            }),
            1000,
            1,
            false,
        ));
        assert_eq!(for_agent(&agent)["learning"]["requests"], 0);
        agent.session = first_session;
        bind(&agent, &host);
        assert_eq!(for_agent(&agent)["learning"]["tokens"]["total"], 15);
        let reloaded = Arc::new(Mutex::new(NativeExtensionHost::default()));
        bind(&agent, &reloaded);
        assert_eq!(for_agent(&agent)["learning"]["tokens"]["total"], 15);
    }

    #[test]
    fn in_memory_agents_keep_separate_receipts_across_host_reload() {
        let first = Agent::new("fixture");
        let second = Agent::new("fixture");
        let host = Arc::new(Mutex::new(NativeExtensionHost::default()));
        bind(&first, &host);
        host.lock()
            .unwrap()
            .background
            .learning
            .record(&RequestUsage::new(None, 8000, 1, true));
        bind(&second, &host);
        assert_eq!(for_agent(&second)["learning"]["requests"], 0);
        assert_eq!(for_agent(&first)["learning"]["unknownTokenRequests"], 1);
        let reloaded = Arc::new(Mutex::new(NativeExtensionHost::default()));
        bind(&first, &reloaded);
        assert_eq!(for_agent(&first)["learning"]["unknownTokenRequests"], 1);
    }

    #[test]
    fn mixed_receipts_keep_measured_cache_and_unknown_reservations_separate() {
        let counters = BackgroundUsage::default();
        let usage = Usage {
            input: 100,
            output: 20,
            cache_read: 30,
            cache_write: 10,
            total_tokens: 160,
            reasoning: Some(10),
            ..Usage::default()
        };
        counters.learning.start();
        assert!(counters.snapshot()["learning"]["tokens"].is_null());
        counters
            .learning
            .record(&RequestUsage::new(Some(&usage), 1000, 10, false));
        let first = counters.snapshot();
        assert_eq!(first["learning"]["tokens"]["total"], 160);
        assert_eq!(first["learning"]["tokens"]["cacheRead"], 30);
        assert!(first["learning"]["estimatedCostUsd"].is_null());
        counters.learning.start();
        counters
            .learning
            .record(&RequestUsage::new(None, 8000, 2, true));
        let mixed = counters.snapshot();
        assert!(mixed["learning"]["tokens"].is_null());
        assert_eq!(mixed["learning"]["measuredTokens"]["total"], 160);
        assert_eq!(mixed["learning"]["unknownTokenRequests"], 1);
        assert_eq!(mixed["learning"]["failedRequests"], 1);
        assert_eq!(mixed["securityWatch"]["tokens"]["total"], 0);
        assert!(render(&mixed).contains("tokens unknown (160 measured so far)"));
    }
}
