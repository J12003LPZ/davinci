//! Privacy-safe OpenAI prompt-cache diagnostics.
//!
//! Provider diagnostics are evidence about a provider comparison only. They
//! are kept separate from local prefix fingerprints, websocket continuation,
//! and local cache hits.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::cache::{cache_retention_from_options, CacheRetention};
use crate::catalog::Model;
use crate::openai_cache_policy::OpenAiCacheCapabilities;
use crate::responses_ledger::{NativeResponsesOutput, NativeResponsesResumeRecord};
use crate::stream::StreamOptions;
use davinci_protocol::Usage;

pub const OPENAI_CACHE_DIAGNOSTICS_EVERY_N_ENV: &str = "PI_OPENAI_CACHE_DIAGNOSTICS_EVERY_N";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPromptCacheDiagnostics {
    #[serde(rename = "type")]
    pub diagnostic_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison_reusable_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_missed_tokens: Option<u64>,
}

impl ProviderPromptCacheDiagnostics {
    pub fn from_response(response: &Value) -> Option<Self> {
        let value = response.get("prompt_cache_diagnostics")?;
        let diagnostic_type = value.get("type")?.as_str()?.to_string();
        Some(Self {
            diagnostic_type,
            reason: value
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string),
            comparison_reusable_tokens: value
                .get("comparison_reusable_tokens")
                .and_then(Value::as_u64),
            cache_missed_tokens: value.get("cache_missed_tokens").and_then(Value::as_u64),
        })
    }

    pub fn from_native_output(output: &NativeResponsesOutput) -> Option<Self> {
        output.final_response.as_ref().and_then(Self::from_response)
    }

    /// Provider-reported hit/miss only. Unavailable/expired/unknown results
    /// are not coerced into either outcome.
    pub fn reported_hit(&self) -> Option<bool> {
        match self.diagnostic_type.as_str() {
            "cache_hit" => Some(true),
            "cache_miss" => Some(false),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppliedPromptCachePolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
    pub comparison_requested: bool,
}

impl AppliedPromptCachePolicy {
    pub fn from_response(response: &Value) -> Option<Self> {
        let options = response.get("prompt_cache_options")?;
        Some(Self {
            mode: options
                .get("mode")
                .and_then(Value::as_str)
                .map(str::to_string),
            ttl: options
                .get("ttl")
                .and_then(Value::as_str)
                .map(str::to_string),
            comparison_requested: options.get("comparison_response_id").is_some(),
        })
    }

    pub fn from_native_output(output: &NativeResponsesOutput) -> Option<Self> {
        output.final_response.as_ref().and_then(Self::from_response)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCacheUsageBreakdown {
    pub ordinary_input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub raw_input_tokens: u64,
    pub output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_ratio: Option<f64>,
}

impl ProviderCacheUsageBreakdown {
    pub fn from_usage(usage: &Usage) -> Self {
        let raw_input_tokens = usage
            .input
            .saturating_add(usage.cache_read)
            .saturating_add(usage.cache_write);
        Self {
            ordinary_input_tokens: usage.input,
            cache_read_tokens: usage.cache_read,
            cache_write_tokens: usage.cache_write,
            raw_input_tokens,
            output_tokens: usage.output,
            cache_read_ratio: (raw_input_tokens > 0)
                .then(|| usage.cache_read as f64 / raw_input_tokens as f64),
        }
    }
}

pub fn diagnostics_every_n_from_env() -> u64 {
    std::env::var(OPENAI_CACHE_DIAGNOSTICS_EVERY_N_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(10_000))
        .unwrap_or(0)
}

pub fn should_sample_comparison(response_id: &str, every_n: u64) -> bool {
    if every_n == 0 || response_id.is_empty() {
        return false;
    }
    if every_n == 1 {
        return true;
    }
    let digest = Sha256::digest(response_id.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(bytes) % every_n == 0
}

pub fn sampled_comparison_response_id(
    model: &Model,
    options: &StreamOptions,
    every_n: u64,
) -> Option<String> {
    if cache_retention_from_options(options) == CacheRetention::None {
        return None;
    }
    let capabilities = OpenAiCacheCapabilities::resolve(model, model.base_url.as_deref(), false);
    if !capabilities.supports_cache_diagnostics {
        return None;
    }
    let record = options.native_responses_resume.as_ref()?;
    let response_id = record.turn.output.response_id.as_deref()?;
    should_sample_comparison(response_id, every_n).then(|| response_id.to_string())
}

pub fn configured_comparison_response_id(model: &Model, options: &StreamOptions) -> Option<String> {
    sampled_comparison_response_id(model, options, diagnostics_every_n_from_env())
}

pub fn latest_diagnostic(
    record: Option<&NativeResponsesResumeRecord>,
) -> Option<ProviderPromptCacheDiagnostics> {
    record
        .and_then(|record| ProviderPromptCacheDiagnostics::from_native_output(&record.turn.output))
}

pub fn latest_applied_policy(
    record: Option<&NativeResponsesResumeRecord>,
) -> Option<AppliedPromptCachePolicy> {
    record.and_then(|record| AppliedPromptCachePolicy::from_native_output(&record.turn.output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_is_not_mislabeled_as_a_hit_or_miss() {
        let diag = ProviderPromptCacheDiagnostics {
            diagnostic_type: "unavailable".into(),
            reason: None,
            comparison_reusable_tokens: None,
            cache_missed_tokens: None,
        };
        assert_eq!(diag.reported_hit(), None);
    }

    #[test]
    fn sampling_is_deterministic_and_zero_is_disabled() {
        assert!(!should_sample_comparison("resp_1", 0));
        assert!(should_sample_comparison("resp_1", 1));
        assert_eq!(
            should_sample_comparison("resp_stable", 7),
            should_sample_comparison("resp_stable", 7)
        );
    }
}
