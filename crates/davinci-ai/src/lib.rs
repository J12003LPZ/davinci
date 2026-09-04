//! Unified multi-provider LLM API matching `@earendil-works/pi-ai`.

mod attribution;
mod auth;
pub mod cache;
mod catalog;
mod codex;
pub mod codex_capabilities;
pub mod codex_flags;
pub mod codex_telemetry;
pub mod codex_transport;
mod codex_ws;
mod deferred;
pub mod request_shape;
pub mod responses_ledger;
mod http_proxy;
mod images;
mod model_config;
mod model_runtime;
mod models_store;
mod oauth;
mod oauth_callback;
mod oauth_providers;
mod provider_retry;
mod providers;
mod retry;
mod shell;
mod stream;
mod stream_decoder;
mod stream_decoder_anthropic;
mod stream_decoder_completions;
mod thinking;
pub mod trace;

pub use attribution::{is_install_telemetry_enabled, merge_provider_attribution_headers};
pub use auth::{
    bedrock_ambient_source, cloudflare_auth, copilot_available_model_ids,
    copilot_base_url_from_token, credential_expires_by, fetch_github_copilot_available_model_ids,
    parse_copilot_available_model_ids, resolve_provider_auth, vertex_ambient_auth, AuthStorage,
    AuthStorageError, Credential, CredentialKind, ResolvedAuth,
};
pub use catalog::{
    builtin_catalog_json, builtin_provider_ids, flatten_catalog, load_builtin_models,
    load_radius_models, models_from_provider_config, openrouter_image_models,
    radius_models_from_config, Model, ModelCost, KNOWN_PROVIDERS,
};
pub use codex_flags::CodexFeatureFlags;
pub use codex::{
    build_cached_websocket_request_body, build_sse_headers, build_websocket_headers,
    close_openai_codex_websocket_sessions, compress_request_body_zstd, connect_codex_websocket,
    encode_codex_sse_body, extract_account_id, get_openai_codex_websocket_debug_stats,
    is_previous_response_not_found, is_websocket_connection_limit_reached, jwt_expiry_ms,
    map_codex_event_type, normalize_codex_terminal_event, pi_user_agent, replay_codex_events,
    reset_openai_codex_websocket_debug_stats, resolve_codex_url, resolve_codex_websocket_url,
    resolve_websocket_connect_timeout_ms, should_fallback_to_sse,
    should_retry_missing_previous_response, should_retry_websocket_connection_limit,
    try_codex_websocket_transport, websocket_connect_timeout_error, websocket_idle_timeout_error,
    CachedWebSocketContinuation, CodexWebsocketOutcome, OpenAICodexWebSocketDebugStats,
    DEFAULT_CODEX_BASE_URL, DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS,
    OPENAI_BETA_RESPONSES_EXPERIMENTAL, OPENAI_BETA_RESPONSES_WEBSOCKETS,
    PREVIOUS_RESPONSE_NOT_FOUND, REQUEST_COMPRESSION_ZSTD_LEVEL, SESSION_WEBSOCKET_CACHE_TTL_MS,
    SESSION_WEBSOCKET_MAX_AGE_MS, WEBSOCKET_CLOSED_BEFORE_COMPLETED,
    WEBSOCKET_CONNECTION_LIMIT_REACHED, WEBSOCKET_MESSAGE_TOO_BIG_CLOSE_CODE,
};
pub use deferred::{
    cancel_deferred, fetch_deferred, DeferredFetchOptions, DeferredFetchResult, DeferredHandle,
};
pub use http_proxy::{
    resolve_http_proxy_url_for_target, tcp_connect_via_http_proxy,
    UNSUPPORTED_PROXY_PROTOCOL_MESSAGE,
};
pub use images::{
    generate_images, image_content, images_request_body, AssistantImages, GenerateImagesOptions,
    ImagesContext,
};
pub use model_config::{
    apply_config_auth, apply_config_auth_with_shell, apply_models_config,
    config_value_env_var_names, is_command_config_value, load_models_json, merge_headers,
    models_json_path, resolve_config_value, resolve_config_value_with_shell, ModelConfig,
    ModelsJsonProvider, NO_MODELS_AVAILABLE,
};
pub use model_runtime::{
    check_auth, empty_catalog_error, format_no_api_key_found_message,
    format_no_model_selected_message, format_no_models_available_message,
    format_oauth_auth_failed_message, get_available, snapshot_availability, AuthCheck,
    ModelRuntimeSnapshot,
};
pub use models_store::{
    catalog_url, load_models_store, merge_models, models_store_path, now_ms, parse_remote_catalog,
    save_models_store, ModelsStore, ModelsStoreEntry, DEFAULT_CATALOG_BASE_URL,
    REMOTE_CATALOG_REFRESH_INTERVAL_MS,
};
pub use oauth::{poll_oauth_device_code_flow, DeviceCodePoller, DevicePollStatus};
pub use oauth_callback::{
    callback_host, handle_callback_request, oauth_error_html, oauth_success_html, CallbackProvider,
    CallbackResponse, CallbackServer, ERR_CALLBACK_ROUTE_NOT_FOUND, ERR_INTERNAL_HTML,
    ERR_MISSING_CODE_OR_STATE, ERR_STATE_MISMATCH, TITLE_FAILED, TITLE_SUCCESS,
};
pub use oauth_providers::{
    authorize_request, device_status_from_error, exchange_authorization_code,
    fresh_authorize_request, generate_pkce, oauth_providers, parse_authorization_input,
    refresh_oauth_token, token_exchange_request, token_refresh_request, AuthorizeRequest,
    OauthTokens, Pkce, TokenExchangeRequest,
};
pub use provider_retry::{
    is_retryable_provider_error, provider_error_from_ureq, retry_delay_from_headers,
    retry_provider_request, ProviderError, ProviderRetryOptions,
};
pub use providers::{builtin_providers, Provider, ProviderSpec, KNOWN_APIS, PROVIDER_SPECS};
pub use retry::{is_retryable_assistant_error, is_retryable_error_text};
pub use shell::{
    command_timeout_from_env, execute_config_command, is_legacy_wsl_bash_path,
    resolve_shell_config, CommandTransport, ResolveCommandOptions, ShellConfig,
};
pub use stream::{
    assistant_to_chat, complete_from_events, complete_simple, events_from_complete,
    fixture_complete, live_complete, live_complete_streaming_with,
    live_complete_streaming_with_sink, live_complete_with, live_stream, parse_sse_block,
    replay_sse_events, request_body, request_body_with, request_url,
    resolve_json_schema_strict_sampling, AssistantMessage, AssistantMessageEvent, ContentBlock,
    StopReason, StreamEvent, StreamOptions,
};
pub use stream_decoder::{
    decoder_for, frames_of, new_message, supports_incremental_stream, ResponsesDecoder, SseFrame,
    SseFramer, StreamDecoder,
};
pub use stream_decoder_anthropic::AnthropicDecoder;
pub use stream_decoder_completions::CompletionsDecoder;
pub use thinking::{
    available_thinking_levels, clamp_reasoning, clamp_thinking_budget_to_answer_room,
    cycle_thinking_level, get_supported_thinking_levels, google_thinking_budget,
    thinking_budget_for_level, ThinkingBudgets, DEFAULT_THINKING_BUDGETS, MIN_ANSWER_TOKENS,
};

use davinci_protocol::Usage;
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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Vec<MessageContent>,
    #[serde(rename = "toolCallId", skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(rename = "toolName", skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// TS custom fields (`command`, `output`, `excludeFromContext`, `customType`, …).
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ChatMessage {
    pub fn text(role: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: vec![MessageContent::Text { text: text.into() }],
            ..Self::default()
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
            ..Self::default()
        }
    }

    pub fn extra_bool(&self, key: &str) -> bool {
        self.extra
            .get(key)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    pub fn extra_str(&self, key: &str) -> Option<&str> {
        self.extra.get(key).and_then(serde_json::Value::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    #[serde(
        rename = "constrainedSampling",
        skip_serializing_if = "Option::is_none"
    )]
    pub constrained_sampling: Option<serde_json::Value>,
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
        &davinci_protocol::ModelCost {
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
                headers: Default::default(),
                thinking_level_map: Default::default(),
            };
            let url = request_url(&model, &auth);
            assert!(url.starts_with("https://example.test"), "{api} -> {url}");
        }
    }
}
