//! Provider attribution headers matching
//! `vendor/pi/packages/coding-agent/src/core/provider-attribution.ts`.

use crate::catalog::Model;

const OPENROUTER_HOST: &str = "openrouter.ai";
const NVIDIA_NIM_HOST: &str = "integrate.api.nvidia.com";
const CLOUDFLARE_API_HOST: &str = "api.cloudflare.com";
const CLOUDFLARE_AI_GATEWAY_HOST: &str = "gateway.ai.cloudflare.com";
const OPENCODE_HOST: &str = "opencode.ai";

fn matches_host(base_url: &str, expected: &str) -> bool {
    url::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| host == expected)
}

fn is_openrouter_model(model: &Model) -> bool {
    model.provider == "openrouter"
        || model
            .base_url
            .as_deref()
            .is_some_and(|url| url.contains(OPENROUTER_HOST))
}

fn is_nvidia_nim_model(model: &Model) -> bool {
    model.provider == "nvidia"
        || model
            .base_url
            .as_deref()
            .is_some_and(|url| matches_host(url, NVIDIA_NIM_HOST))
}

fn is_cloudflare_model(model: &Model) -> bool {
    model.provider == "cloudflare-workers-ai"
        || model.provider == "cloudflare-ai-gateway"
        || model.base_url.as_deref().is_some_and(|url| {
            matches_host(url, CLOUDFLARE_API_HOST) || matches_host(url, CLOUDFLARE_AI_GATEWAY_HOST)
        })
}

fn is_opencode_model(model: &Model) -> bool {
    model.provider == "opencode"
        || model.provider == "opencode-go"
        || model
            .base_url
            .as_deref()
            .is_some_and(|url| matches_host(url, OPENCODE_HOST))
}

pub fn is_install_telemetry_enabled(explicit: Option<bool>) -> bool {
    if let Ok(value) = std::env::var("PI_TELEMETRY") {
        let lower = value.to_ascii_lowercase();
        return value == "1" || lower == "true" || lower == "yes";
    }
    explicit.unwrap_or(true)
}

fn default_attribution_headers(model: &Model, install_telemetry: bool) -> Vec<(String, String)> {
    if !install_telemetry {
        return Vec::new();
    }
    if is_openrouter_model(model) {
        return vec![
            ("HTTP-Referer".into(), "https://pi.dev".into()),
            ("X-OpenRouter-Title".into(), "pi".into()),
            ("X-OpenRouter-Categories".into(), "cli-agent".into()),
        ];
    }
    if is_nvidia_nim_model(model) {
        return vec![("X-BILLING-INVOKE-ORIGIN".into(), "Pi".into())];
    }
    if is_cloudflare_model(model) {
        return vec![("User-Agent".into(), "pi-coding-agent".into())];
    }
    Vec::new()
}

fn session_headers(model: &Model, session_id: Option<&str>) -> Vec<(String, String)> {
    let Some(session_id) = session_id.filter(|id| !id.is_empty()) else {
        return Vec::new();
    };
    if !is_opencode_model(model) {
        return Vec::new();
    }
    vec![
        ("x-opencode-session".into(), session_id.to_string()),
        ("x-opencode-client".into(), "pi".into()),
    ]
}

/// TS `mergeProviderAttributionHeaders`: session + defaults, then caller headers win.
pub fn merge_provider_attribution_headers(
    model: &Model,
    session_id: Option<&str>,
    install_telemetry: Option<bool>,
    existing: &[(String, String)],
) -> Vec<(String, String)> {
    let mut merged = session_headers(model, session_id);
    merged.extend(default_attribution_headers(
        model,
        is_install_telemetry_enabled(install_telemetry),
    ));
    for (key, value) in existing {
        if let Some(slot) = merged
            .iter_mut()
            .find(|(existing_key, _)| existing_key.eq_ignore_ascii_case(key))
        {
            *slot = (key.clone(), value.clone());
        } else {
            merged.push((key.clone(), value.clone()));
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ModelCost;
    use serde_json::Value;

    fn model(provider: &str, base_url: &str) -> Model {
        Model {
            id: "test".into(),
            name: "test".into(),
            api: "openai-completions".into(),
            provider: provider.into(),
            base_url: Some(base_url.into()),
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
            compat: Value::Null,
            headers: Default::default(),
        }
    }

    #[test]
    fn openrouter_nim_cloudflare_and_opencode_headers_match_ts() {
        let previous = std::env::var("PI_TELEMETRY").ok();
        std::env::remove_var("PI_TELEMETRY");
        let openrouter = merge_provider_attribution_headers(
            &model("openrouter", "https://openrouter.ai/api/v1"),
            None,
            Some(true),
            &[("Authorization".into(), "Bearer x".into())],
        );
        assert_eq!(
            openrouter
                .iter()
                .find(|(key, _)| key == "HTTP-Referer")
                .map(|(_, value)| value.as_str()),
            Some("https://pi.dev")
        );
        assert_eq!(
            openrouter
                .iter()
                .find(|(key, _)| key == "X-OpenRouter-Title")
                .map(|(_, value)| value.as_str()),
            Some("pi")
        );
        assert_eq!(
            openrouter
                .iter()
                .find(|(key, _)| key == "X-OpenRouter-Categories")
                .map(|(_, value)| value.as_str()),
            Some("cli-agent")
        );
        let request_wins = merge_provider_attribution_headers(
            &model("openrouter", "https://openrouter.ai/api/v1"),
            None,
            Some(true),
            &[
                ("HTTP-Referer".into(), "https://provider.example".into()),
                ("X-OpenRouter-Title".into(), "request-title".into()),
            ],
        );
        assert_eq!(
            request_wins
                .iter()
                .find(|(key, _)| key == "HTTP-Referer")
                .map(|(_, value)| value.as_str()),
            Some("https://provider.example")
        );
        let nim = merge_provider_attribution_headers(
            &model("nvidia", "https://integrate.api.nvidia.com/v1"),
            None,
            Some(true),
            &[],
        );
        assert_eq!(
            nim.iter()
                .find(|(key, _)| key == "X-BILLING-INVOKE-ORIGIN")
                .map(|(_, value)| value.as_str()),
            Some("Pi")
        );
        let cloudflare = merge_provider_attribution_headers(
            &model(
                "cloudflare-workers-ai",
                "https://api.cloudflare.com/client/v4",
            ),
            None,
            Some(true),
            &[],
        );
        assert_eq!(
            cloudflare
                .iter()
                .find(|(key, _)| key == "User-Agent")
                .map(|(_, value)| value.as_str()),
            Some("pi-coding-agent")
        );
        let off = merge_provider_attribution_headers(
            &model("openrouter", "https://openrouter.ai/api/v1"),
            None,
            Some(false),
            &[],
        );
        assert!(off.iter().all(|(key, _)| key != "HTTP-Referer"));
        let session = merge_provider_attribution_headers(
            &model("opencode", "https://opencode.ai/zen/v1"),
            Some("sess-1"),
            Some(false),
            &[],
        );
        assert_eq!(
            session
                .iter()
                .find(|(key, _)| key == "x-opencode-session")
                .map(|(_, value)| value.as_str()),
            Some("sess-1")
        );
        assert_eq!(
            session
                .iter()
                .find(|(key, _)| key == "x-opencode-client")
                .map(|(_, value)| value.as_str()),
            Some("pi")
        );
        match previous {
            Some(value) => std::env::set_var("PI_TELEMETRY", value),
            None => std::env::remove_var("PI_TELEMETRY"),
        }
    }
}
