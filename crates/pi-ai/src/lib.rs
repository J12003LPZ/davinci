//! Unified multi-provider LLM API matching `@earendil-works/pi-ai`.

mod auth;
mod catalog;
mod oauth;
mod providers;
mod stream;

pub use auth::{
    resolve_provider_auth, AuthStorage, AuthStorageError, Credential, CredentialKind, ResolvedAuth,
};
pub use catalog::{
    builtin_provider_ids, flatten_catalog, load_builtin_models, Model, ModelCost, KNOWN_PROVIDERS,
};
pub use oauth::{poll_oauth_device_code_flow, DeviceCodePoller, DevicePollStatus};
pub use providers::{
    builtin_providers, load_models_json, Provider, ProviderSpec, KNOWN_APIS, PROVIDER_SPECS,
};
pub use stream::{
    assistant_to_chat, complete_from_events, fixture_complete, live_complete, parse_sse_block,
    replay_sse_events, request_url, AssistantMessage, AssistantMessageEvent, ContentBlock,
    StopReason, StreamEvent,
};

use pi_protocol::Usage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MessageContent {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        redacted: Option<bool>,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    #[serde(rename = "toolCall")]
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Vec<MessageContent>,
    #[serde(rename = "toolCallId", skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(rename = "toolName", skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl ChatMessage {
    pub fn text(role: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: vec![MessageContent::Text { text: text.into() }],
            tool_call_id: None,
            tool_name: None,
            is_error: None,
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            role: "toolResult".into(),
            content: vec![MessageContent::Text {
                text: content.into(),
            }],
            tool_call_id: Some(tool_call_id.into()),
            tool_name: Some(tool_name.into()),
            is_error: Some(is_error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub fn content_text(content: &[MessageContent]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            MessageContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

pub fn calculate_usage(
    model: &Model,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
) -> Usage {
    Usage::from_tokens(
        input,
        output,
        cache_read,
        cache_write,
        &pi_protocol::ModelCost {
            input: model.cost.input,
            output: model.cost.output,
            cache_read: model.cost.cache_read,
            cache_write: model.cost.cache_write,
        },
    )
}

pub fn find_model<'a>(models: &'a [Model], provider: &str, id: &str) -> Option<&'a Model> {
    models
        .iter()
        .find(|model| model.provider == provider && model.id == id)
}

pub fn rustls_root_count() -> usize {
    let _ = rustls::version::TLS13;
    let _ = rustls_pki_types::CertificateDer::from(Vec::<u8>::new());
    webpki_roots::TLS_SERVER_ROOTS.len()
}

pub fn fuzzy_models<'a>(models: &'a [Model], query: &str) -> Vec<&'a Model> {
    let query = query.to_ascii_lowercase();
    models
        .iter()
        .filter(|model| {
            model.id.to_ascii_lowercase().contains(&query)
                || model.name.to_ascii_lowercase().contains(&query)
                || format!("{}/{}", model.provider, model.id)
                    .to_ascii_lowercase()
                    .contains(&query)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_openai_anthropic_google_catalogs() {
        let models = load_builtin_models();
        assert!(models
            .iter()
            .any(|m| m.provider == "openai" && m.id == "gpt-4"));
        assert!(models.iter().any(|m| m.provider == "anthropic"));
        assert!(models.iter().any(|m| m.provider == "google"));
        assert!(rustls_root_count() > 0);
        for spec in PROVIDER_SPECS {
            if spec.id == "radius" {
                continue;
            }
            assert!(
                builtin_provider_ids().contains(&spec.id),
                "missing catalog for {}",
                spec.id
            );
        }
        let auth = ResolvedAuth {
            api_key: Some("k".into()),
            headers: Default::default(),
            source: "test".into(),
        };
        for api in KNOWN_APIS {
            let model = Model {
                id: "m".into(),
                name: "m".into(),
                api: (*api).into(),
                provider: "openai".into(),
                base_url: Some("https://example.test".into()),
                reasoning: false,
                input: vec!["text".into()],
                cost: ModelCost {
                    input: 0.0,
                    output: 0.0,
                    cache_read: 0.0,
                    cache_write: 0.0,
                },
                context_window: 1,
                max_tokens: 1,
                compat: serde_json::Value::Null,
            };
            let url = request_url(&model, &auth);
            assert!(url.starts_with("https://example.test"), "{api} -> {url}");
        }
    }
}
