//! Native Responses item storage and lineage tracking matching §6.2.
//! Preserves lossless provider items beside Pi's generic ChatMessage transcript.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::responses_request::{PreparedProviderRequest, WireManifest};
use crate::{content_text, ChatMessage, MessageContent};

pub const NATIVE_RESPONSES_TURN_ENTRY_TYPE: &str = "openai_responses_native_turn_v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesContentPart {
    InputText { text: String },
    OutputText { text: String },
    Refusal { refusal: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesItem {
    Message {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        role: String,
        content: Vec<ResponsesContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
    },
    FunctionCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        name: String,
        arguments: String,
    },
    CustomToolCall {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        call_id: String,
        name: String,
        input: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
    CustomToolCallOutput {
        call_id: String,
        output: String,
    },
    ReasoningSummary {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        summary: String,
    },
    EncryptedReasoning {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        encrypted_content: String,
    },
    Raw {
        item_type: String,
        payload: Value,
    },
}

impl ResponsesItem {
    pub fn to_json_value(&self) -> Value {
        match self {
            Self::Raw { payload, .. } => payload.clone(),
            other => serde_json::to_value(other).unwrap_or(Value::Null),
        }
    }

    pub fn from_json_value(value: &Value) -> Self {
        let item_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        let parsed = match item_type {
            "message"
            | "function_call"
            | "custom_tool_call"
            | "function_call_output"
            | "custom_tool_call_output"
            | "reasoning_summary"
            | "encrypted_reasoning" => serde_json::from_value::<Self>(value.clone()).ok(),
            _ => None,
        };

        // Serde normally ignores unknown object fields. Native Responses replay
        // must not silently discard provider fields, so retain the original
        // object whenever the typed representation is not byte-structure
        // equivalent after a JSON round-trip.
        if let Some(item) = parsed {
            if item.to_json_value() == *value {
                return item;
            }
        }

        Self::Raw {
            item_type: item_type.to_string(),
            payload: value.clone(),
        }
    }
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeResponsesOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    pub output_items: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_response: Option<Value>,
    pub terminal_event_type: String,
}

impl NativeResponsesOutput {
    /// Extract a lossless terminal Responses output from raw SSE/WebSocket
    /// event payloads. Failed/errored/unterminated streams are intentionally
    /// not resumable.
    pub fn from_events(events: &[Value]) -> Option<Self> {
        let terminal = events.iter().rev().find(|event| {
            matches!(
                event.get("type").and_then(Value::as_str),
                Some("response.completed" | "response.done" | "response.incomplete")
            )
        })?;
        let terminal_event_type = terminal.get("type")?.as_str()?.to_string();
        let final_response = terminal.get("response").cloned();
        let response_id = terminal
            .pointer("/response/id")
            .and_then(Value::as_str)
            .map(str::to_string);

        let output_items = final_response
            .as_ref()
            .and_then(|response| response.get("output"))
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty())
            .cloned()
            .unwrap_or_else(|| completed_output_items(events));

        Some(Self {
            response_id,
            output_items,
            final_response,
            terminal_event_type,
        })
    }

    /// Extract native output from a non-streaming Responses response object.
    pub fn from_response_value(value: &Value) -> Option<Self> {
        let response = value.get("response").unwrap_or(value);
        let status = response.get("status").and_then(Value::as_str);
        if matches!(status, Some("failed" | "cancelled")) || response.get("error").is_some() {
            return None;
        }
        let output_items = response
            .get("output")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if output_items.is_empty() && response.get("id").is_none() {
            return None;
        }
        Some(Self {
            response_id: response
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string),
            output_items,
            final_response: Some(response.clone()),
            terminal_event_type: match status {
                Some("incomplete") => "response.incomplete",
                _ => "response.completed",
            }
            .into(),
        })
    }
}

fn completed_output_items(events: &[Value]) -> Vec<Value> {
    let mut completed = events
        .iter()
        .filter(|event| {
            event.get("type").and_then(Value::as_str) == Some("response.output_item.done")
        })
        .filter_map(|event| {
            let item = event.get("item")?.clone();
            let index = event
                .get("output_index")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX);
            Some((index, item))
        })
        .collect::<Vec<_>>();
    completed.sort_by_key(|(index, _)| *index);
    completed.into_iter().map(|(_, item)| item).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeResponsesTurn {
    pub request_input_items: Vec<Value>,
    pub output: NativeResponsesOutput,
    pub wire_manifest: WireManifest,
}

impl NativeResponsesTurn {
    pub fn from_prepared(
        prepared: &PreparedProviderRequest,
        output: NativeResponsesOutput,
    ) -> Option<Self> {
        let request_input_items = prepared
            .body()
            .get("input")
            .and_then(Value::as_array)?
            .clone();
        Some(Self {
            request_input_items,
            output,
            wire_manifest: prepared.manifest().clone(),
        })
    }

    pub fn full_native_replay_prefix(&self) -> Vec<Value> {
        let mut items = self.request_input_items.clone();
        items.extend(self.output.output_items.clone());
        items
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeResponsesResumeRecord {
    pub turn: NativeResponsesTurn,
    pub resume_provider_message_count: usize,
    pub resume_provider_messages_fingerprint: String,
}

impl NativeResponsesResumeRecord {
    pub fn matches_provider_prefix(&self, messages: &[ChatMessage]) -> bool {
        if messages.len() < self.resume_provider_message_count {
            return false;
        }
        provider_messages_fingerprint(&messages[..self.resume_provider_message_count])
            == self.resume_provider_messages_fingerprint
    }
}

pub fn provider_messages_fingerprint(messages: &[ChatMessage]) -> String {
    let bytes = serde_json::to_vec(messages).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(b"davinci.responses-provider-projection.v1\0");
    hasher.update((messages.len() as u64).to_le_bytes());
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseBoundary {
    pub response_id: String,
    pub item_index_end: usize,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponsesLedger {
    pub lineage_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_lineage_id: Option<String>,
    pub items: Vec<ResponsesItem>,
    pub boundaries: Vec<ResponseBoundary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_response_id: Option<String>,
    pub is_cancelled: bool,
    pub has_compaction: bool,
}

impl Default for ResponsesLedger {
    fn default() -> Self {
        Self::new(uuid::Uuid::new_v4().to_string())
    }
}

impl ResponsesLedger {
    pub fn new(lineage_id: impl Into<String>) -> Self {
        Self {
            lineage_id: lineage_id.into(),
            parent_lineage_id: None,
            items: Vec::new(),
            boundaries: Vec::new(),
            last_response_id: None,
            is_cancelled: false,
            has_compaction: false,
        }
    }

    /// Legacy session migration: replay generic ChatMessage list into a clean ResponsesLedger.
    pub fn from_messages(lineage_id: impl Into<String>, messages: &[ChatMessage]) -> Self {
        let mut ledger = Self::new(lineage_id);
        for message in messages {
            if message.role == "toolResult" {
                let call_id = message
                    .tool_call_id
                    .as_deref()
                    .unwrap_or_default()
                    .split_once('|')
                    .map(|(c, _)| c)
                    .unwrap_or(message.tool_call_id.as_deref().unwrap_or_default());
                let output = content_text(&message.content);
                // Check if this tool result was for a custom tool or function call
                if message.tool_name.as_deref() == Some("apply_patch") {
                    ledger.append_item(ResponsesItem::CustomToolCallOutput {
                        call_id: call_id.to_string(),
                        output,
                    });
                } else {
                    ledger.append_item(ResponsesItem::FunctionCallOutput {
                        call_id: call_id.to_string(),
                        output,
                    });
                }
                continue;
            }
            if message.role == "assistant" {
                let text = content_text(&message.content);
                if !text.is_empty() {
                    ledger.append_item(ResponsesItem::Message {
                        id: None,
                        role: "assistant".into(),
                        content: vec![ResponsesContentPart::OutputText { text }],
                        phase: None,
                    });
                }
                for block in &message.content {
                    if let MessageContent::ToolCall {
                        id,
                        name,
                        arguments,
                    } = block
                    {
                        let call_id = id.split_once('|').map(|(c, _)| c).unwrap_or(id);
                        if name == "apply_patch" {
                            let input = arguments
                                .get("input")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            ledger.append_item(ResponsesItem::CustomToolCall {
                                id: Some(id.clone()),
                                call_id: call_id.to_string(),
                                name: name.clone(),
                                input,
                            });
                        } else {
                            ledger.append_item(ResponsesItem::FunctionCall {
                                id: Some(id.clone()),
                                call_id: call_id.to_string(),
                                name: name.clone(),
                                arguments: arguments.to_string(),
                            });
                        }
                    }
                }
                continue;
            }
            let text = content_text(&message.content);
            if !text.is_empty() {
                ledger.append_item(ResponsesItem::Message {
                    id: None,
                    role: "user".into(),
                    content: vec![ResponsesContentPart::InputText { text }],
                    phase: None,
                });
            }
        }
        ledger
    }

    pub fn append_item(&mut self, item: ResponsesItem) {
        self.items.push(item);
    }

    pub fn mark_response_boundary(&mut self, response_id: impl Into<String>) {
        let resp = response_id.into();
        self.last_response_id = Some(resp.clone());
        let boundary = ResponseBoundary {
            response_id: resp,
            item_index_end: self.items.len(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        self.boundaries.push(boundary);
    }

    /// Computes delta input for continuation using `previous_response_id`.
    /// Returns None if previous response ID is not found in boundaries or continuation cannot be established.
    pub fn delta_since_response_id(&self, previous_response_id: &str) -> Option<Vec<Value>> {
        if previous_response_id.is_empty() {
            return None;
        }
        let boundary = self
            .boundaries
            .iter()
            .rfind(|b| b.response_id == previous_response_id)?;
        if boundary.item_index_end > self.items.len() {
            return None;
        }
        let delta_items = &self.items[boundary.item_index_end..];
        Some(
            delta_items
                .iter()
                .map(ResponsesItem::to_json_value)
                .collect(),
        )
    }

    /// Serializes all items as native Responses API input array for full replay.
    pub fn full_replay(&self) -> Vec<Value> {
        self.items
            .iter()
            .map(ResponsesItem::to_json_value)
            .collect()
    }

    pub fn start_new_lineage(&mut self, new_lineage_id: impl Into<String>) {
        self.parent_lineage_id = Some(self.lineage_id.clone());
        self.lineage_id = new_lineage_id.into();
        self.boundaries.clear();
        self.last_response_id = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_output_prefers_terminal_envelope_and_preserves_unknown_fields() {
        let events = vec![
            serde_json::json!({
                "type":"response.output_item.done",
                "output_index":0,
                "item":{"type":"reasoning","id":"rs_1","future":1}
            }),
            serde_json::json!({
                "type":"response.completed",
                "response":{
                    "id":"resp_1",
                    "status":"completed",
                    "output":[{"type":"reasoning","id":"rs_final","opaque":"x"}],
                    "future_response_field":true
                }
            }),
        ];
        let output = NativeResponsesOutput::from_events(&events).unwrap();
        assert_eq!(output.response_id.as_deref(), Some("resp_1"));
        assert_eq!(output.output_items[0]["id"], "rs_final");
        assert_eq!(output.output_items[0]["opaque"], "x");
        assert_eq!(
            output.final_response.as_ref().unwrap()["future_response_field"],
            true
        );
    }

    #[test]
    fn failed_or_unterminated_event_stream_is_not_resumable() {
        assert!(NativeResponsesOutput::from_events(&[
            serde_json::json!({"type":"response.output_item.done","item":{"type":"message"}}),
            serde_json::json!({"type":"response.failed","response":{"id":"resp_bad"}}),
        ])
        .is_none());
    }

    #[test]
    fn resume_record_requires_exact_provider_projection_prefix() {
        let prefix = vec![ChatMessage::text("user", "first")];
        let record = NativeResponsesResumeRecord {
            turn: NativeResponsesTurn {
                request_input_items: vec![],
                output: NativeResponsesOutput {
                    response_id: Some("resp".into()),
                    output_items: vec![],
                    final_response: None,
                    terminal_event_type: "response.completed".into(),
                },
                wire_manifest: WireManifest {
                    schema_version: 1,
                    ordered_prefix_fingerprint: "x".into(),
                    request_bytes_before_compression: 0,
                    segments: vec![],
                },
            },
            resume_provider_message_count: 1,
            resume_provider_messages_fingerprint: provider_messages_fingerprint(&prefix),
        };
        assert!(record.matches_provider_prefix(&prefix));
        assert!(record.matches_provider_prefix(&[
            ChatMessage::text("user", "first"),
            ChatMessage::text("toolResult", "new tail"),
        ]));
        assert!(!record.matches_provider_prefix(&[ChatMessage::text("user", "edited")]));
    }

    #[test]
    fn lossless_delta_and_full_replay() {
        let mut ledger = ResponsesLedger::new("lin_1");
        ledger.append_item(ResponsesItem::Message {
            id: None,
            role: "user".into(),
            content: vec![ResponsesContentPart::InputText {
                text: "Hello".into(),
            }],
            phase: None,
        });
        ledger.mark_response_boundary("resp_1");

        assert_eq!(ledger.full_replay().len(), 1);
        let delta_empty = ledger.delta_since_response_id("resp_1").unwrap();
        assert!(delta_empty.is_empty());

        ledger.append_item(ResponsesItem::FunctionCallOutput {
            call_id: "call_abc".into(),
            output: "success".into(),
        });

        let delta_1 = ledger.delta_since_response_id("resp_1").unwrap();
        assert_eq!(delta_1.len(), 1);
        assert_eq!(delta_1[0]["type"], "function_call_output");
        assert_eq!(delta_1[0]["call_id"], "call_abc");

        assert!(ledger.delta_since_response_id("resp_missing").is_none());
    }

    #[test]
    fn preserves_unknown_fields_on_known_native_items() {
        let original = serde_json::json!({
            "type": "function_call",
            "id": "fc_1",
            "call_id": "call_1",
            "name": "read",
            "arguments": "{\"path\":\"Cargo.toml\"}",
            "provider_future_field": {"revision": 2}
        });
        let item = ResponsesItem::from_json_value(&original);
        assert!(matches!(item, ResponsesItem::Raw { .. }));
        assert_eq!(item.to_json_value(), original);
    }

    #[test]
    fn keeps_exact_known_native_items_typed() {
        let original = serde_json::json!({
            "type": "function_call",
            "id": "fc_1",
            "call_id": "call_1",
            "name": "read",
            "arguments": "{}"
        });
        let item = ResponsesItem::from_json_value(&original);
        assert!(matches!(item, ResponsesItem::FunctionCall { .. }));
        assert_eq!(item.to_json_value(), original);
    }

    #[test]
    fn migrates_generic_messages_to_responses_items() {
        let messages = vec![
            ChatMessage::text("user", "What is 2+2?"),
            ChatMessage {
                role: "assistant".into(),
                content: vec![MessageContent::ToolCall {
                    id: "call_1|fc_1".into(),
                    name: "apply_patch".into(),
                    arguments: serde_json::json!({"input": "*** Begin Patch\n*** End Patch"}),
                }],
                ..Default::default()
            },
            ChatMessage::tool_result("call_1|fc_1", "apply_patch", "Applied patch", false),
        ];

        let ledger = ResponsesLedger::from_messages("lin_migrated", &messages);
        assert_eq!(ledger.items.len(), 3);
        assert!(matches!(&ledger.items[0], ResponsesItem::Message { .. }));
        assert!(matches!(
            &ledger.items[1],
            ResponsesItem::CustomToolCall { name, .. } if name == "apply_patch"
        ));
        assert!(matches!(
            &ledger.items[2],
            ResponsesItem::CustomToolCallOutput { call_id, .. } if call_id == "call_1"
        ));
    }
}
