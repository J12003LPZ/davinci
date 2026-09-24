//! Capability-scoped OpenAI prompt-cache policy.
//!
//! This module deliberately consumes the existing `CodexCapabilities`
//! resolver instead of introducing another hostname/model detector.

use serde::{Deserialize, Serialize};

use crate::cache::CacheRetention;
use crate::catalog::Model;
use crate::codex_capabilities::CodexCapabilities;

pub const OPENAI_CACHE_CONTRACT_REVISION: &str = "openai-cache-2026-09-21-v1";
pub const OPENAI_CACHE_EXPLICIT_BOUNDARIES_ENV: &str = "PI_OPENAI_CACHE_EXPLICIT_BOUNDARIES";
pub const OPENAI_CACHE_NATIVE_REPLAY_ENV: &str = "PI_OPENAI_CACHE_NATIVE_REPLAY";
pub const OPENAI_CACHE_WORKER_AFFINITY_ENV: &str = "PI_OPENAI_CACHE_WORKER_AFFINITY";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCacheRuntimeFeatures {
    pub explicit_stable_boundary: bool,
    pub native_responses_replay: bool,
    pub worker_bootstrap_affinity: bool,
}

impl Default for OpenAiCacheRuntimeFeatures {
    fn default() -> Self {
        Self {
            explicit_stable_boundary: true,
            native_responses_replay: true,
            worker_bootstrap_affinity: true,
        }
    }
}

impl OpenAiCacheRuntimeFeatures {
    pub fn from_env() -> Self {
        Self {
            explicit_stable_boundary: feature_enabled_from_env(
                OPENAI_CACHE_EXPLICIT_BOUNDARIES_ENV,
                true,
            ),
            native_responses_replay: feature_enabled_from_env(OPENAI_CACHE_NATIVE_REPLAY_ENV, true),
            worker_bootstrap_affinity: feature_enabled_from_env(
                OPENAI_CACHE_WORKER_AFFINITY_ENV,
                true,
            ),
        }
    }
}

pub fn runtime_features() -> OpenAiCacheRuntimeFeatures {
    OpenAiCacheRuntimeFeatures::from_env()
}

fn feature_enabled_from_env(name: &str, default: bool) -> bool {
    feature_enabled_from_value(std::env::var(name).ok().as_deref(), default)
}

fn feature_enabled_from_value(value: Option<&str>, default: bool) -> bool {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return default;
    };
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

/// Roll back only the explicit-boundary optimization. A strict cache disable
/// still uses the verified explicit contract so the rollback cannot weaken a
/// privacy request.
pub fn apply_runtime_features(
    mut capabilities: OpenAiCacheCapabilities,
    retention: CacheRetention,
    features: OpenAiCacheRuntimeFeatures,
) -> OpenAiCacheCapabilities {
    if retention != CacheRetention::None
        && !features.explicit_stable_boundary
        && capabilities.cache_control_family == CacheControlFamily::ExplicitBoundaries
    {
        capabilities.cache_control_family = CacheControlFamily::LegacyImplicit;
        capabilities.cache_partition_semantics = CachePartitionSemantics::RoutingHint;
        capabilities.supports_breakpoint_content_types = false;
        capabilities.supports_ttl_30m = false;
    }
    capabilities
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheControlFamily {
    LegacyImplicit,
    ExplicitBoundaries,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachePartitionSemantics {
    RoutingHint,
    AccountingPartition,
    AdapterSpecific,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheCapabilitySource {
    AuthenticatedPublicOpenAi,
    AuthenticatedChatGptCodex,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCacheCapabilities {
    pub cache_control_family: CacheControlFamily,
    pub cache_partition_semantics: CachePartitionSemantics,
    pub supports_breakpoint_content_types: bool,
    pub supports_cache_diagnostics: bool,
    pub supports_ttl_30m: bool,
    pub supports_native_item_replay: bool,
    pub supports_incremental_continuation: bool,
    pub supports_paid_prewarm: bool,
    pub capability_source: CacheCapabilitySource,
    pub verified_at: &'static str,
    pub contract_revision: &'static str,
}

impl OpenAiCacheCapabilities {
    pub fn resolve(model: &Model, base_url: Option<&str>, is_oauth: bool) -> Self {
        let codex = CodexCapabilities::resolve(model, base_url, is_oauth);

        if !codex.responses_items {
            return Self::unknown();
        }

        let public_openai =
            model.provider == "openai" && model.api == "openai-responses" && !is_oauth;
        let chatgpt_codex =
            model.provider == "openai-codex" && model.api == "openai-codex-responses" && is_oauth;

        if public_openai {
            let explicit = codex.explicit_cache_breakpoints;
            return Self {
                cache_control_family: if explicit {
                    CacheControlFamily::ExplicitBoundaries
                } else {
                    CacheControlFamily::LegacyImplicit
                },
                cache_partition_semantics: if explicit {
                    CachePartitionSemantics::AccountingPartition
                } else {
                    CachePartitionSemantics::RoutingHint
                },
                supports_breakpoint_content_types: explicit,
                supports_cache_diagnostics: explicit,
                supports_ttl_30m: explicit,
                supports_native_item_replay: true,
                supports_incremental_continuation: codex.incremental_continuation,
                supports_paid_prewarm: false,
                capability_source: CacheCapabilitySource::AuthenticatedPublicOpenAi,
                verified_at: "2026-09-21",
                contract_revision: OPENAI_CACHE_CONTRACT_REVISION,
            };
        }

        if chatgpt_codex {
            return Self {
                cache_control_family: if codex.explicit_cache_breakpoints {
                    CacheControlFamily::ExplicitBoundaries
                } else {
                    CacheControlFamily::Unknown
                },
                cache_partition_semantics: CachePartitionSemantics::AdapterSpecific,
                supports_breakpoint_content_types: codex.explicit_cache_breakpoints,
                // Public Responses diagnostics are not assumed to exist on the
                // ChatGPT-backed adapter merely because its request shape is similar.
                supports_cache_diagnostics: false,
                supports_ttl_30m: model
                    .compat
                    .get("supportsPromptCacheTtl")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                supports_native_item_replay: true,
                supports_incremental_continuation: codex.incremental_continuation,
                supports_paid_prewarm: codex.generate_false_prewarm,
                capability_source: CacheCapabilitySource::AuthenticatedChatGptCodex,
                verified_at: "2026-09-21",
                contract_revision: OPENAI_CACHE_CONTRACT_REVISION,
            };
        }

        Self::unknown()
    }

    pub fn unknown() -> Self {
        Self {
            cache_control_family: CacheControlFamily::Unknown,
            cache_partition_semantics: CachePartitionSemantics::Unknown,
            supports_breakpoint_content_types: false,
            supports_cache_diagnostics: false,
            supports_ttl_30m: false,
            supports_native_item_replay: false,
            supports_incremental_continuation: false,
            supports_paid_prewarm: false,
            capability_source: CacheCapabilitySource::Unknown,
            verified_at: "2026-09-21",
            contract_revision: OPENAI_CACHE_CONTRACT_REVISION,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheIntent {
    ModelDefault,
    AppendOnly,
    StablePrefixOnly,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveCacheMode {
    ProviderDefault,
    Implicit,
    Explicit,
    OmitNewFields,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryStrategy {
    None,
    StableBootstrap,
    AppendOnlyHistory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveOpenAiCachePolicy {
    pub requested_intent: CacheIntent,
    pub mode: EffectiveCacheMode,
    pub boundary_strategy: BoundaryStrategy,
    pub emit_prompt_cache_key: bool,
    pub ttl_30m: bool,
    pub optimization_supported: bool,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CachePolicyError {
    StrictDisableUnsupported,
}

impl std::fmt::Display for CachePolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StrictDisableUnsupported => write!(
                f,
                "strict prompt-cache disable is unsupported for this backend/model contract"
            ),
        }
    }
}

impl std::error::Error for CachePolicyError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCacheWirePlan {
    pub emit_cache_key: bool,
    pub use_stable_bootstrap_breakpoint: bool,
    pub prompt_cache_mode: Option<&'static str>,
    pub prompt_cache_ttl: Option<&'static str>,
    pub legacy_retention: Option<&'static str>,
}

impl PromptCacheWirePlan {
    /// Pure request-shape planner for the existing retention setting.
    ///
    /// This never selects conversation state and never claims a provider hit.
    pub fn resolve(
        capabilities: &OpenAiCacheCapabilities,
        retention: CacheRetention,
        has_trusted_bootstrap: bool,
    ) -> Self {
        match capabilities.cache_control_family {
            CacheControlFamily::ExplicitBoundaries => match retention {
                CacheRetention::None => Self {
                    emit_cache_key: false,
                    use_stable_bootstrap_breakpoint: false,
                    prompt_cache_mode: Some("explicit"),
                    prompt_cache_ttl: capabilities.supports_ttl_30m.then_some("30m"),
                    legacy_retention: None,
                },
                CacheRetention::Short | CacheRetention::Long => Self {
                    emit_cache_key: true,
                    use_stable_bootstrap_breakpoint: capabilities.supports_breakpoint_content_types
                        && has_trusted_bootstrap,
                    prompt_cache_mode: has_trusted_bootstrap.then_some("implicit"),
                    prompt_cache_ttl: capabilities.supports_ttl_30m.then_some("30m"),
                    legacy_retention: None,
                },
            },
            CacheControlFamily::LegacyImplicit => Self {
                emit_cache_key: retention != CacheRetention::None,
                use_stable_bootstrap_breakpoint: false,
                prompt_cache_mode: None,
                prompt_cache_ttl: None,
                legacy_retention: (retention == CacheRetention::Long).then_some("24h"),
            },
            CacheControlFamily::Unknown => Self {
                emit_cache_key: retention != CacheRetention::None,
                use_stable_bootstrap_breakpoint: false,
                prompt_cache_mode: None,
                prompt_cache_ttl: None,
                legacy_retention: None,
            },
        }
    }
}

impl EffectiveOpenAiCachePolicy {
    pub fn resolve(
        capabilities: &OpenAiCacheCapabilities,
        intent: CacheIntent,
        strict_privacy: bool,
    ) -> Result<Self, CachePolicyError> {
        use BoundaryStrategy::*;
        use CacheControlFamily::*;
        use CacheIntent::*;
        use EffectiveCacheMode::*;

        if capabilities.cache_control_family == Unknown {
            if strict_privacy && intent == Disabled {
                return Err(CachePolicyError::StrictDisableUnsupported);
            }
            return Ok(Self {
                requested_intent: intent,
                mode: OmitNewFields,
                boundary_strategy: None,
                emit_prompt_cache_key: false,
                ttl_30m: false,
                optimization_supported: false,
                reason: "backend/model cache contract is unknown; using conservative schema",
            });
        }

        match (capabilities.cache_control_family, intent) {
            (ExplicitBoundaries, Disabled) => Ok(Self {
                requested_intent: intent,
                mode: Explicit,
                boundary_strategy: None,
                emit_prompt_cache_key: false,
                ttl_30m: capabilities.supports_ttl_30m,
                optimization_supported: true,
                reason: "explicit mode with no breakpoints disables implicit cache use",
            }),
            (ExplicitBoundaries, StablePrefixOnly) => Ok(Self {
                requested_intent: intent,
                mode: Explicit,
                boundary_strategy: StableBootstrap,
                emit_prompt_cache_key: true,
                ttl_30m: capabilities.supports_ttl_30m,
                optimization_supported: true,
                reason: "verified explicit-boundary contract",
            }),
            (ExplicitBoundaries, AppendOnly) => Ok(Self {
                requested_intent: intent,
                mode: Implicit,
                boundary_strategy: AppendOnlyHistory,
                emit_prompt_cache_key: true,
                ttl_30m: capabilities.supports_ttl_30m,
                optimization_supported: true,
                reason: "implicit provider breakpoint plus stable append-only boundaries",
            }),
            (ExplicitBoundaries, ModelDefault) => Ok(Self {
                requested_intent: intent,
                mode: ProviderDefault,
                boundary_strategy: None,
                emit_prompt_cache_key: true,
                ttl_30m: false,
                optimization_supported: true,
                reason: "provider-default prompt caching",
            }),
            (LegacyImplicit, Disabled) if strict_privacy => {
                Err(CachePolicyError::StrictDisableUnsupported)
            }
            (LegacyImplicit, Disabled) => Ok(Self {
                requested_intent: intent,
                mode: OmitNewFields,
                boundary_strategy: None,
                emit_prompt_cache_key: false,
                ttl_30m: false,
                optimization_supported: false,
                reason: "best-effort disable; legacy implicit provider behavior is not a privacy guarantee",
            }),
            (LegacyImplicit, StablePrefixOnly | AppendOnly | ModelDefault) => Ok(Self {
                requested_intent: intent,
                mode: ProviderDefault,
                boundary_strategy: None,
                emit_prompt_cache_key: true,
                ttl_30m: false,
                optimization_supported: true,
                reason: "legacy deterministic key/prefix behavior; GPT-5.6 fields omitted",
            }),
            (Unknown, _) => unreachable!("unknown handled above"),
        }
    }

    /// Compatibility bridge for the existing short/long/none setting.
    ///
    /// `long` remains a retention request and is deliberately not translated
    /// into the newer minimum-lifetime TTL control.
    pub fn intent_from_legacy_retention(retention: CacheRetention) -> CacheIntent {
        match retention {
            CacheRetention::None => CacheIntent::Disabled,
            CacheRetention::Short | CacheRetention::Long => CacheIntent::ModelDefault,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Model, ModelCost};
    use serde_json::json;

    fn model(provider: &str, api: &str, explicit: bool) -> Model {
        Model {
            id: if explicit { "gpt-5.6-sol" } else { "gpt-5" }.into(),
            name: "test".into(),
            api: api.into(),
            provider: provider.into(),
            base_url: None,
            reasoning: true,
            input: vec!["text".into()],
            cost: ModelCost {
                input: 1.0,
                output: 1.0,
                cache_read: 0.1,
                cache_write: 0.0,
            },
            context_window: 272_000,
            max_tokens: 128_000,
            compat: if explicit {
                json!({"supportsExplicitPromptCacheMode": true})
            } else {
                json!({})
            },
            headers: Default::default(),
            thinking_level_map: Default::default(),
        }
    }

    #[test]
    fn runtime_switch_parser_is_conservative_and_explicit() {
        assert!(!feature_enabled_from_value(Some("0"), true));
        assert!(!feature_enabled_from_value(Some("off"), true));
        assert!(feature_enabled_from_value(Some("1"), false));
        assert!(feature_enabled_from_value(Some("yes"), false));
        assert!(feature_enabled_from_value(Some("garbage"), true));
        assert!(!feature_enabled_from_value(Some("garbage"), false));
    }

    #[test]
    fn explicit_boundary_rollback_keeps_strict_disable_semantics() {
        let model = model("openai", "openai-responses", true);
        let caps =
            OpenAiCacheCapabilities::resolve(&model, Some("https://api.openai.com/v1"), false);
        let off = OpenAiCacheRuntimeFeatures {
            explicit_stable_boundary: false,
            ..OpenAiCacheRuntimeFeatures::default()
        };
        let rolled_back = apply_runtime_features(caps.clone(), CacheRetention::Short, off);
        assert_eq!(
            rolled_back.cache_control_family,
            CacheControlFamily::LegacyImplicit
        );
        assert!(!rolled_back.supports_breakpoint_content_types);
        assert!(!rolled_back.supports_ttl_30m);

        let strict_disable = apply_runtime_features(caps, CacheRetention::None, off);
        assert_eq!(
            strict_disable.cache_control_family,
            CacheControlFamily::ExplicitBoundaries
        );
    }

    #[test]
    fn wire_plan_places_one_stable_bootstrap_boundary_for_explicit_contract() {
        let model = model("openai", "openai-responses", true);
        let caps =
            OpenAiCacheCapabilities::resolve(&model, Some("https://api.openai.com/v1"), false);
        let plan = PromptCacheWirePlan::resolve(&caps, CacheRetention::Short, true);
        assert!(plan.emit_cache_key);
        assert!(plan.use_stable_bootstrap_breakpoint);
        assert_eq!(plan.prompt_cache_mode, Some("implicit"));
        assert_eq!(plan.prompt_cache_ttl, Some("30m"));
        assert_eq!(plan.legacy_retention, None);
    }

    #[test]
    fn wire_plan_explicit_disable_has_no_key_or_boundary() {
        let model = model("openai", "openai-responses", true);
        let caps =
            OpenAiCacheCapabilities::resolve(&model, Some("https://api.openai.com/v1"), false);
        let plan = PromptCacheWirePlan::resolve(&caps, CacheRetention::None, true);
        assert!(!plan.emit_cache_key);
        assert!(!plan.use_stable_bootstrap_breakpoint);
        assert_eq!(plan.prompt_cache_mode, Some("explicit"));
        assert_eq!(plan.prompt_cache_ttl, Some("30m"));
    }

    #[test]
    fn wire_plan_unknown_backend_never_emits_new_cache_fields() {
        let plan = PromptCacheWirePlan::resolve(
            &OpenAiCacheCapabilities::unknown(),
            CacheRetention::Long,
            true,
        );
        assert!(!plan.use_stable_bootstrap_breakpoint);
        assert_eq!(plan.prompt_cache_mode, None);
        assert_eq!(plan.prompt_cache_ttl, None);
        assert_eq!(plan.legacy_retention, None);
    }

    #[test]
    fn public_gpt56_gets_explicit_scoped_contract() {
        let model = model("openai", "openai-responses", true);
        let caps =
            OpenAiCacheCapabilities::resolve(&model, Some("https://api.openai.com/v1"), false);
        assert_eq!(
            caps.cache_control_family,
            CacheControlFamily::ExplicitBoundaries
        );
        assert_eq!(
            caps.cache_partition_semantics,
            CachePartitionSemantics::AccountingPartition
        );
        assert!(caps.supports_breakpoint_content_types);
        assert!(caps.supports_cache_diagnostics);
        assert!(caps.supports_ttl_30m);
    }

    #[test]
    fn older_public_responses_stays_legacy() {
        let model = model("openai", "openai-responses", false);
        let caps =
            OpenAiCacheCapabilities::resolve(&model, Some("https://api.openai.com/v1"), false);
        assert_eq!(
            caps.cache_control_family,
            CacheControlFamily::LegacyImplicit
        );
        assert_eq!(
            caps.cache_partition_semantics,
            CachePartitionSemantics::RoutingHint
        );
        assert!(!caps.supports_breakpoint_content_types);
    }

    #[test]
    fn deceptive_proxy_is_conservative_even_for_gpt56() {
        let model = model("openai", "openai-responses", true);
        let caps = OpenAiCacheCapabilities::resolve(
            &model,
            Some("https://api.openai.com.evil.test/v1"),
            false,
        );
        assert_eq!(caps, OpenAiCacheCapabilities::unknown());
    }

    #[test]
    fn codex_adapter_semantics_are_not_public_api_semantics() {
        let mut model = model("openai-codex", "openai-codex-responses", true);
        model.base_url = Some("https://chatgpt.com/backend-api".into());
        let caps =
            OpenAiCacheCapabilities::resolve(&model, Some("https://chatgpt.com/backend-api"), true);
        assert_eq!(
            caps.cache_partition_semantics,
            CachePartitionSemantics::AdapterSpecific
        );
        assert!(!caps.supports_cache_diagnostics);
    }

    #[test]
    fn explicit_disable_requires_no_breakpoints() {
        let model = model("openai", "openai-responses", true);
        let caps =
            OpenAiCacheCapabilities::resolve(&model, Some("https://api.openai.com/v1"), false);
        let policy =
            EffectiveOpenAiCachePolicy::resolve(&caps, CacheIntent::Disabled, true).unwrap();
        assert_eq!(policy.mode, EffectiveCacheMode::Explicit);
        assert_eq!(policy.boundary_strategy, BoundaryStrategy::None);
        assert!(!policy.emit_prompt_cache_key);
    }

    #[test]
    fn strict_disable_fails_closed_on_legacy_contract() {
        let model = model("openai", "openai-responses", false);
        let caps =
            OpenAiCacheCapabilities::resolve(&model, Some("https://api.openai.com/v1"), false);
        assert_eq!(
            EffectiveOpenAiCachePolicy::resolve(&caps, CacheIntent::Disabled, true),
            Err(CachePolicyError::StrictDisableUnsupported)
        );
    }

    #[test]
    fn long_legacy_retention_is_not_relabelled_as_new_ttl() {
        assert_eq!(
            EffectiveOpenAiCachePolicy::intent_from_legacy_retention(CacheRetention::Long),
            CacheIntent::ModelDefault
        );
    }
}
