//! Native Responses item storage and lineage tracking matching §6.2.
//! Preserves lossless provider items beside Pi's generic ChatMessage transcript.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{content_text, ChatMessage, MessageContent};

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
        match item_type {
            "message" => serde_json::from_value(value.clone()).unwrap_or_else(|_| Self::Raw {
                item_type: item_type.to_string(),
                payload: value.clone(),
            }),
            "function_call" => {
                serde_json::from_value(value.clone()).unwrap_or_else(|_| Self::Raw {
                    item_type: item_type.to_string(),
                    payload: value.clone(),
                })
            }
            "custom_tool_call" => {
                serde_json::from_value(value.clone()).unwrap_or_else(|_| Self::Raw {
                    item_type: item_type.to_string(),
                    payload: value.clone(),
                })
            }
            "function_call_output" => {
                serde_json::from_value(value.clone()).unwrap_or_else(|_| Self::Raw {
                    item_type: item_type.to_string(),
                    payload: value.clone(),
                })
            }
            "custom_tool_call_output" => {
                serde_json::from_value(value.clone()).unwrap_or_else(|_| Self::Raw {
                    item_type: item_type.to_string(),
                    payload: value.clone(),
                })
            }
            "reasoning_summary" => {
                serde_json::from_value(value.clone()).unwrap_or_else(|_| Self::Raw {
                    item_type: item_type.to_string(),
                    payload: value.clone(),
                })
            }
            "encrypted_reasoning" => {
                serde_json::from_value(value.clone()).unwrap_or_else(|_| Self::Raw {
                    item_type: item_type.to_string(),
                    payload: value.clone(),
                })
            }
            _ => Self::Raw {
                item_type: item_type.to_string(),
                payload: value.clone(),
            },
        }
    }
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
