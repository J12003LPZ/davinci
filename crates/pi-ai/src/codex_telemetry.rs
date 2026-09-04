//! Codex observability telemetry matching §15 of the Codex Efficiency Spec.
//! Emits sanitized JSONL execution events for analysis and regression detection.

use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexTelemetryTimestamps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_build_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_send_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_byte_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_useful_tool_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_queue_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_start_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_finish_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_complete_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexTelemetryTokens {
    pub cached_read: u64,
    pub cache_write: u64,
    pub uncached_input: u64,
    pub output: u64,
    pub reasoning: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTelemetryEvent {
    pub timestamp: u64,
    pub session_id: String,
    pub turn_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    pub stable_prefix_hash: String,
    pub request_shape_hash: String,

    pub timestamps: CodexTelemetryTimestamps,
    pub tokens: CodexTelemetryTokens,

    pub tool_definition_count: usize,
    pub tool_definition_bytes: usize,

    pub is_continuation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_reason: Option<String>,

    pub cache_mode: String,
    pub observed_cache_reuse: f64,

    #[serde(default)]
    pub retry_count: u32,
    pub fallback_to_sse: bool,
    pub cancelled: bool,
    pub compaction_boundary: bool,

    pub provider_latency_ms: u64,
    pub pi_overhead_ms: u64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_memory_bytes: Option<u64>,
}

impl Default for CodexTelemetryEvent {
    fn default() -> Self {
        Self {
            timestamp: now_millis(),
            session_id: String::new(),
            turn_id: 0,
            response_id: None,
            stream_id: None,
            call_id: None,
            agent_id: None,
            stable_prefix_hash: String::new(),
            request_shape_hash: String::new(),
            timestamps: CodexTelemetryTimestamps::default(),
            tokens: CodexTelemetryTokens::default(),
            tool_definition_count: 0,
            tool_definition_bytes: 0,
            is_continuation: false,
            replay_reason: None,
            cache_mode: "auto".into(),
            observed_cache_reuse: 0.0,
            retry_count: 0,
            fallback_to_sse: false,
            cancelled: false,
            compaction_boundary: false,
            provider_latency_ms: 0,
            pi_overhead_ms: 0,
            peak_memory_bytes: None,
        }
    }
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Sanitizer ensuring no raw prompt texts, secret keys, or unbounded outputs leak into logs.
pub fn sanitize_telemetry_payload(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                let lower = key.to_lowercase();
                if lower.contains("key")
                    || lower.contains("token")
                    || lower.contains("secret")
                    || lower.contains("auth")
                    || lower.contains("password")
                {
                    *val = Value::String("[REDACTED]".into());
                } else if lower.contains("prompt")
                    || lower.contains("instruction")
                    || lower.contains("input_text")
                    || lower.contains("reasoning.encrypted_content")
                {
                    *val = Value::String("[REDACTED_PROMPT]".into());
                } else {
                    sanitize_telemetry_payload(val);
                }
            }
        }
        Value::Array(list) => {
            for item in list {
                sanitize_telemetry_payload(item);
            }
        }
        _ => {}
    }
}

static TELEMETRY_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn telemetry_dir_lock() -> &'static Mutex<Option<PathBuf>> {
    TELEMETRY_DIR.get_or_init(|| Mutex::new(None))
}

pub fn set_codex_telemetry_dir(path: PathBuf) {
    if let Ok(mut guard) = telemetry_dir_lock().lock() {
        *guard = Some(path);
    }
}

pub fn default_codex_telemetry_path() -> PathBuf {
    if let Ok(guard) = telemetry_dir_lock().lock() {
        if let Some(path) = guard.as_ref() {
            return path.join("codex_events.jsonl");
        }
    }
    if let Ok(env_path) = std::env::var("PI_CODEX_TELEMETRY_PATH") {
        return PathBuf::from(env_path);
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".pi")
        .join("telemetry")
        .join("codex_events.jsonl")
}

pub fn record_codex_telemetry_event(event: &CodexTelemetryEvent) {
    let line = match serde_json::to_string(event) {
        Ok(s) => s,
        Err(_) => return,
    };
    let path = default_codex_telemetry_path();
    if let Some(parent) = path.parent() {
        let _ = create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{}", line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_sanitized_telemetry_event() {
        let event = CodexTelemetryEvent {
            session_id: "test_sess".into(),
            stable_prefix_hash: "abc".into(),
            request_shape_hash: "def".into(),
            tokens: CodexTelemetryTokens {
                cached_read: 500,
                uncached_input: 100,
                ..Default::default()
            },
            observed_cache_reuse: 500.0 / 600.0,
            ..Default::default()
        };

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["session_id"], "test_sess");
        assert_eq!(json["tokens"]["cached_read"], 500);
        assert!(!json.to_string().contains("REDACTED"));
    }

    #[test]
    fn sanitizes_sensitive_keys() {
        let mut raw = serde_json::json!({
            "api_key": "sk-secret123",
            "prompt": "my super secret code",
            "normal_field": 42
        });
        sanitize_telemetry_payload(&mut raw);
        assert_eq!(raw["api_key"], "[REDACTED]");
        assert_eq!(raw["prompt"], "[REDACTED_PROMPT]");
        assert_eq!(raw["normal_field"], 42);
    }
}
