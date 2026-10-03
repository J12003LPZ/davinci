//! Prompt-cache helpers matching vendor/pi/packages/ai/src/api/openai-prompt-cache.ts
//! and the per-API cache retention helpers in anthropic-messages.ts,
//! openai-responses.ts, openai-completions.ts, and bedrock-converse-stream.ts.

use serde_json::{json, Value};

use crate::catalog::Model;
use crate::stream::StreamOptions;

/// TS `CacheRetention`: `"short" | "long" | "none"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRetention {
    Short,
    Long,
    None,
}

/// TS `resolveCacheRetention` (identical in each API file): explicit value
/// first, then `PI_CACHE_RETENTION=long`, defaulting to short.
pub fn resolve_cache_retention(explicit: Option<&str>, env: Option<&str>) -> CacheRetention {
    match explicit {
        Some("short") => return CacheRetention::Short,
        Some("long") => return CacheRetention::Long,
        Some("none") => return CacheRetention::None,
        _ => {}
    }
    if env == Some("long") {
        return CacheRetention::Long;
    }
    CacheRetention::Short
}

/// Resolve retention from stream options plus the real process environment.
pub fn cache_retention_from_options(options: &StreamOptions) -> CacheRetention {
    resolve_cache_retention(
        options.cache_retention.as_deref(),
        std::env::var("PI_CACHE_RETENTION").ok().as_deref(),
    )
}

/// Resolve the effective prompt cache key: explicit `cache_key` first, then fallback to `session_id`.
pub fn effective_prompt_cache_key(options: &StreamOptions) -> Option<&str> {
    options
        .cache_key
        .as_deref()
        .or(options.session_id.as_deref())
}

/// TS `OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH` (openai-prompt-cache.ts).
pub const OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH: usize = 64;

/// TS `clampOpenAIPromptCacheKey`: `Array.from` semantics — count Unicode
/// scalar values, not bytes.
pub fn clamp_openai_prompt_cache_key(key: &str) -> String {
    key.chars()
        .take(OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH)
        .collect()
}

fn compat_bool(compat: &Value, key: &str, default: bool) -> bool {
    compat.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// TS anthropic-messages.ts `getCacheControl`: no marker for retention none;
/// `ttl: "1h"` only for long retention on models that support it (default true).
pub fn anthropic_cache_control(compat: &Value, retention: CacheRetention) -> Option<Value> {
    match retention {
        CacheRetention::None => None,
        CacheRetention::Short => Some(json!({"type": "ephemeral"})),
        CacheRetention::Long => {
            if compat_bool(compat, "supportsLongCacheRetention", true) {
                Some(json!({"type": "ephemeral", "ttl": "1h"}))
            } else {
                Some(json!({"type": "ephemeral"}))
            }
        }
    }
}

/// TS `AnthropicMessagesCompat.supportsCacheControlOnTools`. Default: true.
pub fn supports_cache_control_on_tools(compat: &Value) -> bool {
    compat_bool(compat, "supportsCacheControlOnTools", true)
}

/// TS `supportsLongCacheRetention` (all compat interfaces). Default: true.
pub fn supports_long_cache_retention(compat: &Value) -> bool {
    compat_bool(compat, "supportsLongCacheRetention", true)
}

/// TS openai-completions.ts:1621 — explicit compat, else auto-detected for
/// OpenRouter Anthropic models.
pub fn completions_cache_control_format(model: &Model) -> Option<String> {
    if let Some(format) = model
        .compat
        .get("cacheControlFormat")
        .and_then(Value::as_str)
    {
        return Some(format.to_string());
    }
    if model.provider == "openrouter" && model.id.starts_with("anthropic/") {
        return Some("anthropic".to_string());
    }
    None
}

/// TS openai-completions.ts detected `supportsLongCacheRetention`: false for
/// Together, Cloudflare Workers AI / AI Gateway, NVIDIA NIM, and Ant Ling.
pub fn completions_supports_long_cache_retention(model: &Model) -> bool {
    if let Some(explicit) = model
        .compat
        .get("supportsLongCacheRetention")
        .and_then(Value::as_bool)
    {
        return explicit;
    }
    let base = model.base_url.as_deref().unwrap_or("");
    let blocked = base.contains("api.together.xyz")
        || base.contains("api.cloudflare.com")
        || base.contains("gateway.ai.cloudflare.com")
        || base.contains("integrate.api.nvidia.com")
        || base.contains("api.ant-ling.com")
        || model.provider == "together"
        || model.provider == "cloudflare-workers-ai"
        || model.provider == "cloudflare-ai-gateway"
        || model.provider == "nvidia"
        || model.provider == "ant-ling";
    !blocked
}

/// TS bedrock-converse-stream.ts `supportsPromptCaching`: Claude 5 / 4.x /
/// 3.7 Sonnet / 3.5 Haiku by id or name; `AWS_BEDROCK_FORCE_CACHE=1` forces
/// caching for inference profiles whose ARN hides the model name.
/// `force_env` is the value of `AWS_BEDROCK_FORCE_CACHE` (injected for tests).
pub fn bedrock_supports_prompt_caching(model: &Model, force_env: Option<&str>) -> bool {
    let candidates = [model.id.to_lowercase(), model.name.to_lowercase()];
    let has_claude = candidates.iter().any(|s| s.contains("claude"));
    if !has_claude {
        return force_env == Some("1");
    }
    candidates.iter().any(|s| {
        s.contains("fable-5")
            || s.contains("opus-5")
            || s.contains("sonnet-5")
            || s.contains("-4-")
            || s.contains("claude-3-7-sonnet")
            || s.contains("claude-3-5-haiku")
    })
}

/// TS bedrock-converse-stream.ts cachePoint blocks (lines 879, 1091).
pub fn bedrock_cache_point(retention: CacheRetention) -> Option<Value> {
    match retention {
        CacheRetention::None => None,
        CacheRetention::Short => Some(json!({"cachePoint": {"type": "default"}})),
        CacheRetention::Long => Some(json!({"cachePoint": {"type": "default", "ttl": "1h"}})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Model, ModelCost};
    use serde_json::{json, Value};

    fn model(provider: &str, id: &str, base_url: &str, compat: Value) -> Model {
        Model {
            id: id.into(),
            name: id.into(),
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
            compat,
            headers: Default::default(),
            thinking_level_map: Default::default(),
        }
    }

    #[test]
    fn resolves_retention_with_ts_precedence() {
        assert_eq!(resolve_cache_retention(None, None), CacheRetention::Short);
        assert_eq!(
            resolve_cache_retention(None, Some("long")),
            CacheRetention::Long
        );
        // Only the exact value "long" upgrades; anything else is ignored.
        assert_eq!(
            resolve_cache_retention(None, Some("none")),
            CacheRetention::Short
        );
        assert_eq!(
            resolve_cache_retention(Some("none"), Some("long")),
            CacheRetention::None
        );
        assert_eq!(
            resolve_cache_retention(Some("long"), None),
            CacheRetention::Long
        );
        assert_eq!(
            resolve_cache_retention(Some("short"), Some("long")),
            CacheRetention::Short
        );
        // Unknown explicit values fall back to the env/default chain.
        assert_eq!(
            resolve_cache_retention(Some("bogus"), None),
            CacheRetention::Short
        );
    }

    #[test]
    fn clamps_prompt_cache_key_at_64_chars() {
        let short = "s".repeat(64);
        assert_eq!(clamp_openai_prompt_cache_key(&short), short);
        let long = "x".repeat(70);
        assert_eq!(clamp_openai_prompt_cache_key(&long).chars().count(), 64);
        // Char-count semantics (TS Array.from), not bytes.
        let emoji = "🦀".repeat(70);
        assert_eq!(clamp_openai_prompt_cache_key(&emoji).chars().count(), 64);
    }

    #[test]
    fn builds_anthropic_cache_control_per_retention_and_compat() {
        assert_eq!(
            anthropic_cache_control(&Value::Null, CacheRetention::None),
            None
        );
        assert_eq!(
            anthropic_cache_control(&Value::Null, CacheRetention::Short),
            Some(json!({"type": "ephemeral"}))
        );
        assert_eq!(
            anthropic_cache_control(&Value::Null, CacheRetention::Long),
            Some(json!({"type": "ephemeral", "ttl": "1h"}))
        );
        assert_eq!(
            anthropic_cache_control(
                &json!({"supportsLongCacheRetention": false}),
                CacheRetention::Long
            ),
            Some(json!({"type": "ephemeral"}))
        );
    }

    #[test]
    fn compat_flags_default_true() {
        assert!(supports_cache_control_on_tools(&Value::Null));
        assert!(!supports_cache_control_on_tools(
            &json!({"supportsCacheControlOnTools": false})
        ));
        assert!(supports_long_cache_retention(&Value::Null));
        assert!(!supports_long_cache_retention(
            &json!({"supportsLongCacheRetention": false})
        ));
    }

    #[test]
    fn detects_completions_cache_control_format() {
        // Explicit compat wins.
        let explicit = model(
            "custom",
            "anything",
            "https://example.com/v1",
            json!({"cacheControlFormat": "anthropic"}),
        );
        assert_eq!(
            completions_cache_control_format(&explicit).as_deref(),
            Some("anthropic")
        );
        // OpenRouter + anthropic/ model id is auto-detected (openai-completions.ts:1621).
        let openrouter = model(
            "openrouter",
            "anthropic/claude-sonnet-5",
            "https://openrouter.ai/api/v1",
            Value::Null,
        );
        assert_eq!(
            completions_cache_control_format(&openrouter).as_deref(),
            Some("anthropic")
        );
        let openrouter_gpt = model(
            "openrouter",
            "openai/gpt-5.5",
            "https://openrouter.ai/api/v1",
            Value::Null,
        );
        assert_eq!(completions_cache_control_format(&openrouter_gpt), None);
        let plain = model(
            "openai",
            "gpt-5.5",
            "https://api.openai.com/v1",
            Value::Null,
        );
        assert_eq!(completions_cache_control_format(&plain), None);
    }

    #[test]
    fn completions_long_retention_excludes_known_hosts() {
        let openai = model(
            "openai",
            "gpt-5.5",
            "https://api.openai.com/v1",
            Value::Null,
        );
        assert!(completions_supports_long_cache_retention(&openai));
        let together = model(
            "together",
            "meta-llama",
            "https://api.together.xyz/v1",
            Value::Null,
        );
        assert!(!completions_supports_long_cache_retention(&together));
        let nvidia = model(
            "nvidia",
            "nim",
            "https://integrate.api.nvidia.com/v1",
            Value::Null,
        );
        assert!(!completions_supports_long_cache_retention(&nvidia));
        let cloudflare = model(
            "cloudflare-workers-ai",
            "wf",
            "https://api.cloudflare.com/client/v4",
            Value::Null,
        );
        assert!(!completions_supports_long_cache_retention(&cloudflare));
        let ant_ling = model("ant-ling", "ling", "https://example.com/v1", Value::Null);
        assert!(!completions_supports_long_cache_retention(&ant_ling));
        // Explicit compat overrides detection.
        let forced = model(
            "together",
            "meta-llama",
            "https://api.together.xyz/v1",
            json!({"supportsLongCacheRetention": true}),
        );
        assert!(completions_supports_long_cache_retention(&forced));
    }

    #[test]
    fn bedrock_caching_gated_to_claude_models() {
        let fable = model(
            "amazon-bedrock",
            "anthropic.claude-fable-5",
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            Value::Null,
        );
        assert!(bedrock_supports_prompt_caching(&fable, None));
        let sonnet4 = model(
            "amazon-bedrock",
            "anthropic.claude-sonnet-4-20250514-v1:0",
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            Value::Null,
        );
        assert!(bedrock_supports_prompt_caching(&sonnet4, None));
        let sonnet37 = model(
            "amazon-bedrock",
            "anthropic.claude-3-7-sonnet-20250219-v1:0",
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            Value::Null,
        );
        assert!(bedrock_supports_prompt_caching(&sonnet37, None));
        let haiku35 = model(
            "amazon-bedrock",
            "anthropic.claude-3-5-haiku-20241022-v1:0",
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            Value::Null,
        );
        assert!(bedrock_supports_prompt_caching(&haiku35, None));
        let nova = model(
            "amazon-bedrock",
            "amazon.nova-pro-v1:0",
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            Value::Null,
        );
        assert!(!bedrock_supports_prompt_caching(&nova, None));
        // Inference profiles without "claude" in the ARN can be forced via env.
        let profile = model(
            "amazon-bedrock",
            "arn:aws:bedrock:us-east-1:123:application-inference-profile/abc",
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            Value::Null,
        );
        assert!(!bedrock_supports_prompt_caching(&profile, None));
        assert!(bedrock_supports_prompt_caching(&profile, Some("1")));
        // Claude 3.5 Sonnet is NOT in the allow list.
        let sonnet35 = model(
            "amazon-bedrock",
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            Value::Null,
        );
        assert!(!bedrock_supports_prompt_caching(&sonnet35, None));
    }

    #[test]
    fn builds_bedrock_cache_point() {
        assert_eq!(bedrock_cache_point(CacheRetention::None), None);
        assert_eq!(
            bedrock_cache_point(CacheRetention::Short),
            Some(json!({"cachePoint": {"type": "default"}}))
        );
        assert_eq!(
            bedrock_cache_point(CacheRetention::Long),
            Some(json!({"cachePoint": {"type": "default", "ttl": "1h"}}))
        );
    }

    #[test]
    fn explicit_cache_key_wins_over_session_id() {
        let options = StreamOptions {
            session_id: Some("session-a".into()),
            cache_key: Some("graph-role-a".into()),
            ..StreamOptions::default()
        };
        assert_eq!(effective_prompt_cache_key(&options), Some("graph-role-a"));
    }

    #[test]
    fn session_id_remains_fallback_cache_key() {
        let options = StreamOptions {
            session_id: Some("session-a".into()),
            cache_key: None,
            ..StreamOptions::default()
        };
        assert_eq!(effective_prompt_cache_key(&options), Some("session-a"));
    }
}
