//! LLM types and stream contract.
//!
//! Live provider HTTP is intentionally out of the Phase 4 CI surface.
//! Tests use `MockProvider` fixtures so TypeScript remains the catalog owner.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub api: String,
    pub provider: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    pub reasoning: bool,
    pub input: Vec<String>,
    #[serde(rename = "contextWindow")]
    pub context_window: u32,
    #[serde(rename = "maxTokens")]
    pub max_tokens: u32,
    pub cost: ModelCost,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(rename = "cacheRead")]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum Message {
    User {
        content: Value,
        timestamp: i64,
    },
    Assistant {
        content: Vec<AssistantContent>,
        api: String,
        provider: String,
        model: String,
        usage: Usage,
        #[serde(rename = "stopReason")]
        stop_reason: StopReason,
        timestamp: i64,
        #[serde(
            rename = "errorMessage",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        error_message: Option<String>,
    },
    #[serde(rename = "toolResult")]
    ToolResult {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        content: Vec<Value>,
        #[serde(rename = "isError")]
        is_error: bool,
        timestamp: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AssistantContent {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        redacted: Option<bool>,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Pending,
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input: i64,
    pub output: i64,
    #[serde(rename = "cacheRead")]
    pub cache_read: i64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: i64,
    #[serde(rename = "totalTokens")]
    pub total_tokens: i64,
    pub cost: UsageCost,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UsageCost {
    pub input: f64,
    pub output: f64,
    #[serde(rename = "cacheRead")]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: f64,
    pub total: f64,
}

impl Usage {
    pub fn with_tokens(input: i64, output: i64) -> Self {
        Self {
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            total_tokens: input + output,
            cost: UsageCost {
                input: input as f64 * 0.001,
                output: output as f64 * 0.002,
                cache_read: 0.0,
                cache_write: 0.0,
                total: input as f64 * 0.001 + output as f64 * 0.002,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Context {
    #[serde(
        rename = "systemPrompt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub system_prompt: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantMessageEvent {
    Start {
        partial: Message,
    },
    TextDelta {
        delta: String,
        partial: Message,
    },
    ToolcallStart {
        partial: Message,
    },
    Done {
        reason: StopReason,
        message: Message,
    },
    Error {
        reason: StopReason,
        error: Message,
    },
}

#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    pub thinking_level: Option<String>,
}

/// Stream contract: never throw for request/model failures; encode them as events.
pub fn stream(
    model: &Model,
    context: &Context,
    options: Option<StreamOptions>,
) -> Vec<AssistantMessageEvent> {
    MockProvider::default().stream(model, context, options)
}

pub fn complete(model: &Model, context: &Context, options: Option<StreamOptions>) -> Message {
    match stream(model, context, options).last() {
        Some(
            AssistantMessageEvent::Done { message, .. }
            | AssistantMessageEvent::Error { error: message, .. },
        ) => message.clone(),
        _ => panic!("stream produced no terminal event"),
    }
}

#[derive(Debug, Clone, Default)]
pub struct MockProvider {
    pub forced_text: Option<String>,
    pub tool_calls: Vec<(String, String, Value)>,
    pub fail: bool,
    pub stop_reason: Option<StopReason>,
}

impl MockProvider {
    pub fn stream(
        &self,
        model: &Model,
        context: &Context,
        _options: Option<StreamOptions>,
    ) -> Vec<AssistantMessageEvent> {
        if self.fail {
            let error = assistant_message(
                model,
                vec![AssistantContent::Text {
                    text: String::new(),
                }],
                Usage::default(),
                StopReason::Error,
                Some("mock provider error".into()),
            );
            return vec![
                AssistantMessageEvent::Start {
                    partial: error.clone(),
                },
                AssistantMessageEvent::Error {
                    reason: StopReason::Error,
                    error,
                },
            ];
        }
        let prompt = last_user_text(context).unwrap_or_default();
        let text = self
            .forced_text
            .clone()
            .unwrap_or_else(|| format!("echo:{prompt}"));
        let mut content = vec![AssistantContent::Text { text: text.clone() }];
        for (id, name, arguments) in &self.tool_calls {
            content.push(AssistantContent::ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            });
        }
        let stop = self.stop_reason.unwrap_or(if self.tool_calls.is_empty() {
            StopReason::Stop
        } else {
            StopReason::ToolUse
        });
        let message = assistant_message(
            model,
            content,
            Usage::with_tokens(8, text.len() as i64),
            stop,
            None,
        );
        vec![
            AssistantMessageEvent::Start {
                partial: message.clone(),
            },
            AssistantMessageEvent::TextDelta {
                delta: text,
                partial: message.clone(),
            },
            AssistantMessageEvent::Done {
                reason: stop,
                message,
            },
        ]
    }
}

pub fn assistant_message(
    model: &Model,
    content: Vec<AssistantContent>,
    usage: Usage,
    stop_reason: StopReason,
    error_message: Option<String>,
) -> Message {
    Message::Assistant {
        content,
        api: model.api.clone(),
        provider: model.provider.clone(),
        model: model.id.clone(),
        usage,
        stop_reason,
        timestamp: pi_core::now_ms(),
        error_message,
    }
}

pub fn last_user_text(context: &Context) -> Option<String> {
    context
        .messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::User { content, .. } => content.as_str().map(str::to_string).or_else(|| {
                content
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|item| item.get("text"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }),
            _ => None,
        })
}

pub fn test_model() -> Model {
    Model {
        id: "mock-1".into(),
        name: "Mock".into(),
        api: "openai-completions".into(),
        provider: "mock".into(),
        base_url: "https://example.invalid".into(),
        reasoning: false,
        input: vec!["text".into()],
        context_window: 8192,
        max_tokens: 1024,
        cost: ModelCost::default(),
    }
}

pub fn validate_tool_arguments(tool: &Tool, arguments: &Value) -> Result<Value, String> {
    if !arguments.is_object() {
        return Err("tool arguments must be an object".into());
    }
    let Some(required) = tool.parameters.get("required").and_then(Value::as_array) else {
        return Ok(arguments.clone());
    };
    for key in required {
        let Some(name) = key.as_str() else { continue };
        if arguments.get(name).is_none() {
            return Err(format!("missing required argument {name}"));
        }
    }
    Ok(arguments.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_lifecycle_is_start_delta_done() {
        let model = test_model();
        let context = Context {
            system_prompt: None,
            messages: vec![Message::User {
                content: Value::String("hi".into()),
                timestamp: 0,
            }],
            tools: None,
        };
        let events = stream(&model, &context, None);
        assert!(matches!(
            events.first(),
            Some(AssistantMessageEvent::Start { .. })
        ));
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Done { .. })
        ));
        let Message::Assistant { usage, .. } = complete(&model, &context, None) else {
            panic!("expected assistant");
        };
        assert_eq!(usage.total_tokens, usage.input + usage.output);
    }

    #[test]
    fn failures_are_encoded_not_thrown() {
        let provider = MockProvider {
            fail: true,
            ..MockProvider::default()
        };
        let events = provider.stream(
            &test_model(),
            &Context {
                system_prompt: None,
                messages: vec![],
                tools: None,
            },
            None,
        );
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Error { .. })
        ));
    }

    #[test]
    fn tool_argument_validation() {
        let tool = Tool {
            name: "read".into(),
            description: "read a file".into(),
            parameters: serde_json::json!({"required":["path"]}),
        };
        assert!(validate_tool_arguments(&tool, &serde_json::json!({"path":"a"})).is_ok());
        assert!(validate_tool_arguments(&tool, &serde_json::json!({})).is_err());
    }
}
