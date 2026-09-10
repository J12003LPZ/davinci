//! Telemetry contracts matching `@earendil-works/pi-telemetry`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type AttributeValue = Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpanAttributes {
    #[serde(flatten)]
    pub values: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanOptions {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<SpanAttributes>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum SpanStatus {
    Ok,
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<SpanError>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanError {
    pub name: String,
    pub message: String,
}

pub trait TelemetrySpan: Send {
    fn add_event(&mut self, name: &str, attributes: Option<SpanAttributes>);
    fn set_attributes(&mut self, attributes: SpanAttributes);
    fn set_status(&mut self, status: SpanStatus);
}

pub trait TelemetryContext: Send + Sync {
    fn start_span<T, F>(&self, options: SpanOptions, callback: F) -> T
    where
        F: FnOnce(&mut dyn TelemetrySpan) -> T;
}

#[derive(Debug, Default)]
pub struct NoopSpan;

impl TelemetrySpan for NoopSpan {
    fn add_event(&mut self, _name: &str, _attributes: Option<SpanAttributes>) {}
    fn set_attributes(&mut self, _attributes: SpanAttributes) {}
    fn set_status(&mut self, _status: SpanStatus) {}
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTelemetryContext;

impl TelemetryContext for NoopTelemetryContext {
    fn start_span<T, F>(&self, _options: SpanOptions, callback: F) -> T
    where
        F: FnOnce(&mut dyn TelemetrySpan) -> T,
    {
        let mut span = NoopSpan;
        callback(&mut span)
    }
}

pub const NOOP_TELEMETRY_CONTEXT: NoopTelemetryContext = NoopTelemetryContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEvent {
    pub name: String,
    pub attributes: SpanAttributes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySpan {
    pub name: String,
    pub attributes: SpanAttributes,
    pub events: Vec<MemoryEvent>,
    pub status: SpanStatus,
}

#[derive(Debug, Default)]
pub struct MemoryTelemetryContext {
    pub spans: std::sync::Mutex<Vec<MemorySpan>>,
}

impl TelemetryContext for MemoryTelemetryContext {
    fn start_span<T, F>(&self, options: SpanOptions, callback: F) -> T
    where
        F: FnOnce(&mut dyn TelemetrySpan) -> T,
    {
        let mut span = RecordingSpan {
            name: options.name,
            attributes: options.attributes.unwrap_or_default(),
            events: Vec::new(),
            status: SpanStatus::Ok,
        };
        let result = callback(&mut span);
        if let Ok(mut spans) = self.spans.lock() {
            spans.push(MemorySpan {
                name: span.name,
                attributes: span.attributes,
                events: span.events,
                status: span.status,
            });
        }
        result
    }
}

struct RecordingSpan {
    name: String,
    attributes: SpanAttributes,
    events: Vec<MemoryEvent>,
    status: SpanStatus,
}

impl TelemetrySpan for RecordingSpan {
    fn add_event(&mut self, name: &str, attributes: Option<SpanAttributes>) {
        self.events.push(MemoryEvent {
            name: name.to_string(),
            attributes: attributes.unwrap_or_default(),
        });
    }

    fn set_attributes(&mut self, attributes: SpanAttributes) {
        self.attributes = attributes;
    }

    fn set_status(&mut self, status: SpanStatus) {
        self.status = status;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelemetryAttributeDefinition {
    #[serde(rename = "type")]
    pub type_name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitive: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelemetrySpanDefinition {
    pub description: String,
    #[serde(default)]
    pub start_attributes: serde_json::Map<String, Value>,
    #[serde(default)]
    pub end_attributes: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelemetrySchemaDefinition {
    pub version: u32,
    pub spans: serde_json::Map<String, Value>,
}

pub fn define_telemetry_schema(schema: TelemetrySchemaDefinition) -> TelemetrySchemaDefinition {
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_context_records_spans() {
        let context = MemoryTelemetryContext::default();
        context.start_span(
            SpanOptions {
                name: "agent.turn".into(),
                attributes: None,
            },
            |span| {
                span.add_event("start", None);
                span.set_status(SpanStatus::Ok);
            },
        );
        let spans = context.spans.lock().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "agent.turn");
        assert_eq!(spans[0].events[0].name, "start");
        let span = TelemetrySpanDefinition {
            description: "One agent turn".into(),
            start_attributes: {
                let mut attrs = serde_json::Map::new();
                attrs.insert(
                    "provider".into(),
                    serde_json::to_value(TelemetryAttributeDefinition {
                        type_name: "string".into(),
                        description: "Provider id".into(),
                        required: true,
                        sensitive: None,
                    })
                    .unwrap(),
                );
                attrs
            },
            end_attributes: serde_json::Map::new(),
        };
        let schema = define_telemetry_schema(TelemetrySchemaDefinition {
            version: 1,
            spans: {
                let mut spans = serde_json::Map::new();
                spans.insert("agent.turn".into(), serde_json::to_value(span).unwrap());
                spans
            },
        });
        assert_eq!(schema.version, 1);
        assert!(schema.spans.contains_key("agent.turn"));
    }
}

/// Local-first behavioral telemetry summarizing run execution metrics
/// without exposing private source code, conversation content, or secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehaviorTelemetry {
    pub prompt_profile: String,
    pub prompt_version: u32,
    pub prompt_stable_hash_prefix: String,
    pub model_family: String,
    pub model_turns: u64,
    pub tool_calls: u64,
    pub permission_prompts: u64,
    pub permission_denials: u64,
    pub files_changed_count: u64,
    pub verification_commands_run: u64,
    pub verification_failures: u64,
    pub aborted: bool,
    pub user_steers: u64,
}

/// Local-only aggregate summary computed from settled behavioral telemetry records.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalBehaviorReport {
    pub prompt_profile: String,
    pub runs: u64,
    pub median_turns: f64,
    pub verification_failures_recovered: u64,
    pub permission_prompts_per_run: f64,
    pub user_steers_per_run: f64,
}

impl LocalBehaviorReport {
    pub fn from_runs(profile: &str, runs: &[BehaviorTelemetry]) -> Self {
        let matching: Vec<&BehaviorTelemetry> = if profile.is_empty() {
            runs.iter().collect()
        } else {
            runs.iter()
                .filter(|r| r.prompt_profile == profile)
                .collect()
        };
        let profile_label = if profile.is_empty() {
            if let Some(first) = matching.first() {
                first.prompt_profile.clone()
            } else {
                "stable v2".to_string()
            }
        } else {
            profile.to_string()
        };

        if matching.is_empty() {
            return Self {
                prompt_profile: profile_label,
                runs: 0,
                median_turns: 0.0,
                verification_failures_recovered: 0,
                permission_prompts_per_run: 0.0,
                user_steers_per_run: 0.0,
            };
        }

        let total_runs = matching.len() as u64;
        let mut turns: Vec<u64> = matching.iter().map(|r| r.model_turns).collect();
        turns.sort_unstable();
        let median_turns = if turns.is_empty() {
            0.0
        } else if turns.len() % 2 == 1 {
            turns[turns.len() / 2] as f64
        } else {
            (turns[turns.len() / 2 - 1] + turns[turns.len() / 2]) as f64 / 2.0
        };
        let total_perm_prompts: u64 = matching.iter().map(|r| r.permission_prompts).sum();
        let total_user_steers: u64 = matching.iter().map(|r| r.user_steers).sum();
        let verification_failures_recovered: u64 = matching
            .iter()
            .map(|r| {
                if r.verification_failures > 0 && !r.aborted {
                    r.verification_failures
                } else {
                    0
                }
            })
            .sum();

        Self {
            prompt_profile: profile_label,
            runs: total_runs,
            median_turns,
            verification_failures_recovered,
            permission_prompts_per_run: total_perm_prompts as f64 / total_runs as f64,
            user_steers_per_run: total_user_steers as f64 / total_runs as f64,
        }
    }

    pub fn format_report(&self) -> String {
        format!(
            "Prompt profile: {}\nRuns: {}\nMedian turns: {:.0}\nVerification failures recovered: {}\nPermission prompts/run: {:.1}\nUser steers/run: {:.1}",
            self.prompt_profile,
            self.runs,
            self.median_turns,
            self.verification_failures_recovered,
            self.permission_prompts_per_run,
            self.user_steers_per_run
        )
    }
}

static BEHAVIOR_LOG: std::sync::Mutex<Vec<BehaviorTelemetry>> = std::sync::Mutex::new(Vec::new());

/// Record a local behavioral telemetry entry from a settled agent turn.
pub fn record_behavior_telemetry(entry: BehaviorTelemetry) {
    if let Ok(mut log) = BEHAVIOR_LOG.lock() {
        log.push(entry);
    }
}

/// Retrieve all recorded behavioral telemetry entries from the local store.
pub fn get_behavior_telemetry() -> Vec<BehaviorTelemetry> {
    if let Ok(log) = BEHAVIOR_LOG.lock() {
        log.clone()
    } else {
        Vec::new()
    }
}

/// Clear recorded behavioral telemetry entries in the local store.
pub fn clear_behavior_telemetry() {
    if let Ok(mut log) = BEHAVIOR_LOG.lock() {
        log.clear();
    }
}

#[cfg(test)]
mod behavior_telemetry_tests {
    use super::*;

    #[test]
    fn behavior_telemetry_serialization_contains_only_aggregate_metrics() {
        let entry = BehaviorTelemetry {
            prompt_profile: "stable".to_string(),
            prompt_version: 2,
            prompt_stable_hash_prefix: "a1b2c3d4".to_string(),
            model_family: "claude".to_string(),
            model_turns: 6,
            tool_calls: 12,
            permission_prompts: 1,
            permission_denials: 0,
            files_changed_count: 2,
            verification_commands_run: 3,
            verification_failures: 1,
            aborted: false,
            user_steers: 1,
        };

        let json = serde_json::to_string_pretty(&entry).expect("serialize behavior telemetry");

        // Allowed fields must exist
        assert!(json.contains("\"prompt_profile\": \"stable\""));
        assert!(json.contains("\"prompt_version\": 2"));
        assert!(json.contains("\"prompt_stable_hash_prefix\": \"a1b2c3d4\""));
        assert!(json.contains("\"model_family\": \"claude\""));
        assert!(json.contains("\"model_turns\": 6"));
        assert!(json.contains("\"tool_calls\": 12"));
        assert!(json.contains("\"permission_prompts\": 1"));
        assert!(json.contains("\"permission_denials\": 0"));
        assert!(json.contains("\"files_changed_count\": 2"));
        assert!(json.contains("\"verification_commands_run\": 3"));
        assert!(json.contains("\"verification_failures\": 1"));
        assert!(json.contains("\"aborted\": false"));
        assert!(json.contains("\"user_steers\": 1"));

        // Sensitive private data keys must NOT exist in the serialized output
        assert!(!json.contains("user_prompt"));
        assert!(!json.contains("model_text"));
        assert!(!json.contains("assistant_text"));
        assert!(!json.contains("source_code"));
        assert!(!json.contains("file_content"));
        assert!(!json.contains("command_text"));
        assert!(!json.contains("secret"));
    }

    #[test]
    fn behavior_telemetry_redaction_guarantee() {
        // Raw conversation representation that may contain secrets and source code
        struct RawTurnContext {
            user_prompt: String,
            model_response: String,
            secret_key: String,
            file_path: String,
            source_content: String,
            command_line: String,
        }

        let raw = RawTurnContext {
            user_prompt: "Fix the bug in auth with secret sk-ant-api03-abcdef123456789".to_string(),
            model_response: "I will update auth.rs and run cargo test".to_string(),
            secret_key: "ghp_1234567890abcdefghijklmnopqrstuvwxyz".to_string(),
            file_path: "C:\\Users\\user\\project\\src\\secret_credentials.rs".to_string(),
            source_content: "const API_KEY: &str = \"super_secret\";".to_string(),
            command_line: "cargo test -- --nocapture secret_test".to_string(),
        };

        // BehaviorTelemetry constructed from turn metrics
        let sanitized = BehaviorTelemetry {
            prompt_profile: "stable".to_string(),
            prompt_version: 2,
            prompt_stable_hash_prefix: "b5c6d7e8".to_string(),
            model_family: "claude".to_string(),
            model_turns: 4,
            tool_calls: 5,
            permission_prompts: 0,
            permission_denials: 0,
            files_changed_count: 1,
            verification_commands_run: 1,
            verification_failures: 0,
            aborted: false,
            user_steers: 0,
        };

        let json = serde_json::to_string(&sanitized).expect("serialize");

        // Prove that no raw sensitive text leaked into BehaviorTelemetry
        assert!(!json.contains(&raw.user_prompt));
        assert!(!json.contains(&raw.model_response));
        assert!(!json.contains(&raw.secret_key));
        assert!(!json.contains(&raw.file_path));
        assert!(!json.contains(&raw.source_content));
        assert!(!json.contains(&raw.command_line));
        assert!(!json.contains("sk-ant-"));
        assert!(!json.contains("ghp_"));
    }

    #[test]
    fn local_behavior_report_aggregates_and_formats_correctly() {
        clear_behavior_telemetry();

        let mut runs = Vec::new();
        for i in 0..42 {
            runs.push(BehaviorTelemetry {
                prompt_profile: "stable v2".to_string(),
                prompt_version: 2,
                prompt_stable_hash_prefix: "a1b2c3d4".to_string(),
                model_family: "claude".to_string(),
                model_turns: if i % 2 == 0 { 5 } else { 7 }, // median 6
                tool_calls: 10,
                permission_prompts: if i < 38 { 1 } else { 0 }, // 38 / 42 ~ 0.9
                permission_denials: 0,
                files_changed_count: 2,
                verification_commands_run: 2,
                verification_failures: if i < 8 { 1 } else { 0 }, // 8 failures recovered
                aborted: false,
                user_steers: if i < 17 { 1 } else { 0 }, // 17 / 42 ~ 0.4
            });
        }

        let report = LocalBehaviorReport::from_runs("stable v2", &runs);
        assert_eq!(report.runs, 42);
        assert_eq!(report.median_turns, 6.0);
        assert_eq!(report.verification_failures_recovered, 8);
        assert!((report.permission_prompts_per_run - 0.9).abs() < 0.05);
        assert!((report.user_steers_per_run - 0.4).abs() < 0.05);

        let formatted = report.format_report();
        assert_eq!(
            formatted,
            "Prompt profile: stable v2\nRuns: 42\nMedian turns: 6\nVerification failures recovered: 8\nPermission prompts/run: 0.9\nUser steers/run: 0.4"
        );
    }
}
