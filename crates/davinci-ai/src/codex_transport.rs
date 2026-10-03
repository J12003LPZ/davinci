//! Production Codex WebSocket transport pool and lane multiplexer matching §6.3.
//! Manages connection lifecycle, stream lanes, prewarming, cancellation, and rotation.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codex::{SESSION_WEBSOCKET_CACHE_TTL_MS, SESSION_WEBSOCKET_MAX_AGE_MS};
use crate::responses_ledger::ResponsesLedger;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportHealthMetrics {
    pub account_id: String,
    pub active_lanes: usize,
    pub total_requests: u64,
    pub prewarm_requests: u64,
    pub delta_continuations: u64,
    pub full_replays: u64,
    pub cancellations: u64,
    pub connection_age_secs: u64,
    pub is_healthy: bool,
}

#[derive(Debug)]
pub struct StreamLane {
    pub stream_id: String,
    pub response_id: Option<String>,
    pub created_at: Instant,
    pub last_active_at: Instant,
}

#[derive(Debug)]
pub struct TransportConnection {
    pub key: String,
    pub account_id: String,
    pub base_url: String,
    pub created_at: Instant,
    pub last_active_at: Instant,
    pub lanes: HashMap<String, StreamLane>,
    pub prewarmed_shape_hash: Option<String>,
    pub is_healthy: bool,
    pub total_requests: u64,
    pub prewarm_requests: u64,
    pub delta_continuations: u64,
    pub full_replays: u64,
    pub cancellations: u64,
}

impl TransportConnection {
    pub fn new(account_id: impl Into<String>, base_url: impl Into<String>) -> Self {
        let now = Instant::now();
        let acct = account_id.into();
        let url = base_url.into();
        let key = format!("{}:{}", acct, url);
        Self {
            key,
            account_id: acct,
            base_url: url,
            created_at: now,
            last_active_at: now,
            lanes: HashMap::new(),
            prewarmed_shape_hash: None,
            is_healthy: true,
            total_requests: 0,
            prewarm_requests: 0,
            delta_continuations: 0,
            full_replays: 0,
            cancellations: 0,
        }
    }

    pub fn is_expired(&self, max_age: Duration, idle_timeout: Duration) -> bool {
        let now = Instant::now();
        now.duration_since(self.created_at) >= max_age
            || now.duration_since(self.last_active_at) >= idle_timeout
    }

    pub fn open_lane(&mut self, stream_id: impl Into<String>) -> &mut StreamLane {
        let sid = stream_id.into();
        let now = Instant::now();
        self.last_active_at = now;
        self.lanes.entry(sid.clone()).or_insert_with(|| StreamLane {
            stream_id: sid,
            response_id: None,
            created_at: now,
            last_active_at: now,
        })
    }

    pub fn close_lane(&mut self, stream_id: &str) -> Option<StreamLane> {
        self.last_active_at = Instant::now();
        self.lanes.remove(stream_id)
    }

    pub fn metrics(&self) -> TransportHealthMetrics {
        TransportHealthMetrics {
            account_id: self.account_id.clone(),
            active_lanes: self.lanes.len(),
            total_requests: self.total_requests,
            prewarm_requests: self.prewarm_requests,
            delta_continuations: self.delta_continuations,
            full_replays: self.full_replays,
            cancellations: self.cancellations,
            connection_age_secs: self.created_at.elapsed().as_secs(),
            is_healthy: self.is_healthy,
        }
    }
}

pub struct CodexTransportPool {
    connections: HashMap<String, TransportConnection>,
    max_age: Duration,
    idle_timeout: Duration,
    refreshing_accounts: HashMap<String, Instant>,
}

impl Default for CodexTransportPool {
    fn default() -> Self {
        Self {
            connections: HashMap::new(),
            max_age: Duration::from_millis(SESSION_WEBSOCKET_MAX_AGE_MS),
            idle_timeout: Duration::from_millis(SESSION_WEBSOCKET_CACHE_TTL_MS),
            refreshing_accounts: HashMap::new(),
        }
    }
}

impl CodexTransportPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_create_connection(
        &mut self,
        account_id: &str,
        base_url: &str,
    ) -> &mut TransportConnection {
        let key = format!("{}:{}", account_id, base_url);
        self.rotate_expired();
        self.connections
            .entry(key)
            .or_insert_with(|| TransportConnection::new(account_id, base_url))
    }

    /// Prewarming support (§6.3, §7):
    /// Sends `generate: false` request only for new or invalidated request shape
    /// while user-visible work (permissions/tool calls) is already underway.
    /// Never delays user input.
    pub fn should_send_prewarm(
        &mut self,
        account_id: &str,
        base_url: &str,
        shape_hash: &str,
    ) -> bool {
        let conn = self.get_or_create_connection(account_id, base_url);
        if conn.prewarmed_shape_hash.as_deref() == Some(shape_hash) {
            return false;
        }
        conn.prewarmed_shape_hash = Some(shape_hash.to_string());
        conn.prewarm_requests += 1;
        true
    }

    /// Build continuation or replay payload using `ResponsesLedger`.
    pub fn build_continuation_request(
        &mut self,
        account_id: &str,
        base_url: &str,
        base_body: &Value,
        ledger: &ResponsesLedger,
        previous_response_id: Option<&str>,
    ) -> (Value, bool) {
        let conn = self.get_or_create_connection(account_id, base_url);
        conn.total_requests += 1;

        if let Some(prev_id) = previous_response_id {
            if let Some(delta) = ledger.delta_since_response_id(prev_id) {
                let mut req = base_body.clone();
                if let Value::Object(map) = &mut req {
                    map.insert(
                        "previous_response_id".into(),
                        Value::String(prev_id.to_string()),
                    );
                    map.insert("input".into(), Value::Array(delta));
                }
                conn.delta_continuations += 1;
                return (req, true);
            }
        }

        // Full lossless replay when previous response cannot continue
        let mut req = base_body.clone();
        if let Value::Object(map) = &mut req {
            map.remove("previous_response_id");
            map.insert("input".into(), Value::Array(ledger.full_replay()));
        }
        conn.full_replays += 1;
        (req, false)
    }

    /// Explicit cancellation (§6.3, §9):
    /// Builds `response.cancel` message and marks cancellation in lane.
    pub fn cancel_lane(
        &mut self,
        account_id: &str,
        base_url: &str,
        stream_id: &str,
        response_id: Option<&str>,
    ) -> Option<Value> {
        let conn = self.get_or_create_connection(account_id, base_url);
        conn.cancellations += 1;
        let _ = conn.close_lane(stream_id);
        response_id.map(|resp_id| {
            serde_json::json!({
                "type": "response.cancel",
                "response_id": resp_id,
            })
        })
    }

    /// Coordinated single-flight OAuth refresh (§6.3, §9).
    pub fn should_refresh_oauth(&mut self, account_id: &str) -> bool {
        let now = Instant::now();
        if let Some(started) = self.refreshing_accounts.get(account_id) {
            if now.duration_since(*started) < Duration::from_secs(15) {
                return false;
            }
        }
        self.refreshing_accounts.insert(account_id.to_string(), now);
        true
    }

    pub fn complete_oauth_refresh(&mut self, account_id: &str) {
        self.refreshing_accounts.remove(account_id);
    }

    /// Drains idle and expired connections before replacement.
    pub fn rotate_expired(&mut self) {
        let max_age = self.max_age;
        let idle = self.idle_timeout;
        self.connections
            .retain(|_, conn| !conn.is_expired(max_age, idle) || !conn.lanes.is_empty());
    }

    pub fn health_metrics(&self) -> Vec<TransportHealthMetrics> {
        self.connections.values().map(|c| c.metrics()).collect()
    }
}

static GLOBAL_POOL: OnceLock<Mutex<CodexTransportPool>> = OnceLock::new();

pub fn global_codex_transport_pool() -> &'static Mutex<CodexTransportPool> {
    GLOBAL_POOL.get_or_init(|| Mutex::new(CodexTransportPool::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::responses_ledger::{ResponsesContentPart, ResponsesItem};
    use serde_json::json;

    #[test]
    fn prewarming_deduplication() {
        let mut pool = CodexTransportPool::new();
        let shape1 = "hash_abc";
        assert!(pool.should_send_prewarm("user_1", "https://chatgpt.com", shape1));
        assert!(!pool.should_send_prewarm("user_1", "https://chatgpt.com", shape1));

        let shape2 = "hash_xyz";
        assert!(pool.should_send_prewarm("user_1", "https://chatgpt.com", shape2));
    }

    #[test]
    fn continuation_delta_versus_full_replay() {
        let mut pool = CodexTransportPool::new();
        let mut ledger = ResponsesLedger::new("lin_1");
        ledger.append_item(ResponsesItem::Message {
            id: None,
            role: "user".into(),
            content: vec![ResponsesContentPart::InputText {
                text: "test".into(),
            }],
            phase: None,
        });
        ledger.mark_response_boundary("resp_first");

        ledger.append_item(ResponsesItem::FunctionCallOutput {
            call_id: "call_1".into(),
            output: "done".into(),
        });

        let base_body = json!({"model": "gpt-5-codex"});

        // Continuation with valid previous_response_id
        let (req_delta, is_cont) = pool.build_continuation_request(
            "u1",
            "https://chatgpt.com",
            &base_body,
            &ledger,
            Some("resp_first"),
        );
        assert!(is_cont);
        assert_eq!(req_delta["previous_response_id"], "resp_first");
        assert_eq!(req_delta["input"].as_array().unwrap().len(), 1);

        // Fallback to full replay when previous_response_id is invalid
        let (req_replay, is_cont2) = pool.build_continuation_request(
            "u1",
            "https://chatgpt.com",
            &base_body,
            &ledger,
            Some("non_existent"),
        );
        assert!(!is_cont2);
        assert!(req_replay.get("previous_response_id").is_none());
        assert_eq!(req_replay["input"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn explicit_cancellation_frame() {
        let mut pool = CodexTransportPool::new();
        let cancel_frame = pool
            .cancel_lane("u1", "https://chatgpt.com", "stream_1", Some("resp_456"))
            .unwrap();
        assert_eq!(cancel_frame["type"], "response.cancel");
        assert_eq!(cancel_frame["response_id"], "resp_456");
    }
}
