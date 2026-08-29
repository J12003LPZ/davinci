//! Unified multi-provider LLM API matching `@earendil-works/pi-ai`.

pub mod auth;
pub mod catalog;
pub mod cost;
pub mod events;
pub mod http;
pub mod oauth;
pub mod providers;
pub mod request;
pub mod retry;
pub mod sigv4;
pub mod stream;
pub mod transport;
pub mod types;
pub mod vertex;

pub use auth::{
    AuthResult, AuthStorage, Credential, CredentialKind, FileAuthStorage, InMemoryCredentialStore,
};
pub use catalog::{
    builtin_providers, flatten_catalog, get_builtin_model, list_models, Model, ModelCost,
};
pub use cost::usage_cost;
pub use events::{AssistantMessage, AssistantMessageEvent, StopReason};
pub use oauth::{
    authorize_url, oauth_app, oauth_needs_refresh, parse_token_response, refresh_oauth_token,
};
pub use request::{build_request_body, resolve_api, RequestContext};
pub use retry::{is_retryable_assistant_error, RetryPolicy};
pub use sigv4::{sign_bedrock_post, AwsCredentials};
pub use stream::{complete, stream_complete, StreamEvent, StreamOptions};
pub use transport::{CodexWebsocketCache, Transport, TransportDecision};
pub use types::{
    ContentBlock, KnownApi, KnownProvider, Message, Role, ThinkingLevel, ToolCall, Usage,
};
pub use vertex::{resolve_vertex_auth, VertexAuth, GCP_VERTEX_CREDENTIALS_MARKER};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn env_api_key_vars(provider: &str) -> Option<&'static [&'static str]> {
    Some(match provider {
        "anthropic" => &[
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_OAUTH_TOKEN",
            "ANTHROPIC_API_KEY",
        ],
        "ant-ling" => &["ANT_LING_API_KEY"],
        "qwen-token-plan" | "qwen-token-plan-individual" => &["QWEN_TOKEN_PLAN_API_KEY"],
        "qwen-token-plan-cn" => &["QWEN_TOKEN_PLAN_CN_API_KEY"],
        "openai" => &["OPENAI_API_KEY"],
        "azure-openai-responses" => &["AZURE_OPENAI_API_KEY"],
        "nvidia" => &["NVIDIA_API_KEY"],
        "deepseek" => &["DEEPSEEK_API_KEY"],
        "google" => &["GEMINI_API_KEY"],
        "google-vertex" => &["GOOGLE_CLOUD_API_KEY"],
        "groq" => &["GROQ_API_KEY"],
        "cerebras" => &["CEREBRAS_API_KEY"],
        "xai" => &["XAI_API_KEY"],
        "radius" => &["RADIUS_API_KEY"],
        "openrouter" => &["OPENROUTER_API_KEY"],
        "vercel-ai-gateway" => &["AI_GATEWAY_API_KEY"],
        "zai" => &["ZAI_API_KEY"],
        "zai-coding-cn" => &["ZAI_CODING_CN_API_KEY"],
        "mistral" => &["MISTRAL_API_KEY"],
        "minimax" => &["MINIMAX_API_KEY"],
        "minimax-cn" => &["MINIMAX_CN_API_KEY"],
        "moonshotai" | "moonshotai-cn" => &["MOONSHOT_API_KEY"],
        "huggingface" => &["HF_TOKEN"],
        "fireworks" => &["FIREWORKS_API_KEY"],
        "together" => &["TOGETHER_API_KEY"],
        "baseten" => &["BASETEN_API_KEY"],
        "opencode" | "opencode-go" => &["OPENCODE_API_KEY"],
        "kimi-coding" => &["KIMI_API_KEY"],
        "cloudflare-workers-ai" | "cloudflare-ai-gateway" => &["CLOUDFLARE_API_KEY"],
        "xiaomi" => &["XIAOMI_API_KEY"],
        "xiaomi-token-plan-cn" => &["XIAOMI_TOKEN_PLAN_CN_API_KEY"],
        "xiaomi-token-plan-ams" => &["XIAOMI_TOKEN_PLAN_AMS_API_KEY"],
        "xiaomi-token-plan-sgp" => &["XIAOMI_TOKEN_PLAN_SGP_API_KEY"],
        "github-copilot" => &["COPILOT_GITHUB_TOKEN"],
        _ => return None,
    })
}

pub fn get_env_api_key(provider: &str) -> Option<String> {
    if let Some(vars) = env_api_key_vars(provider) {
        for var in vars {
            if provider == "anthropic" && *var == "ANTHROPIC_AUTH_TOKEN" {
                continue;
            }
            if let Ok(value) = std::env::var(var) {
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
    }
    if provider == "google-vertex" {
        let has_project = std::env::var("GOOGLE_CLOUD_PROJECT").is_ok()
            || std::env::var("GCLOUD_PROJECT").is_ok();
        let has_location = std::env::var("GOOGLE_CLOUD_LOCATION").is_ok();
        if has_project && has_location {
            return Some("<authenticated>".into());
        }
    }
    if provider == "amazon-bedrock"
        && (std::env::var("AWS_PROFILE").is_ok()
            || (std::env::var("AWS_ACCESS_KEY_ID").is_ok()
                && std::env::var("AWS_SECRET_ACCESS_KEY").is_ok())
            || std::env::var("AWS_BEARER_TOKEN_BEDROCK").is_ok())
    {
        return Some("<authenticated>".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_include_core_providers() {
        for provider in ["anthropic", "openai", "google"] {
            assert!(
                list_models(Some(provider))
                    .iter()
                    .any(|m| m.provider == provider),
                "missing {provider}"
            );
        }
        assert!(list_models(None).len() >= 50);
    }

    #[test]
    fn env_key_map_matches_typescript() {
        assert_eq!(env_api_key_vars("openai"), Some(&["OPENAI_API_KEY"][..]));
        assert_eq!(env_api_key_vars("google"), Some(&["GEMINI_API_KEY"][..]));
        assert!(env_api_key_vars("anthropic")
            .unwrap()
            .contains(&"ANTHROPIC_API_KEY"));
    }
}
