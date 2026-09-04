//! Independent feature flags and kill switches matching §17.
//! Enables phased rollout, A/B evaluation, and instantaneous fallback.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexFeatureFlags {
    pub transport_pool: bool,
    pub responses_ledger: bool,
    pub tool_call_ledger: bool,
    pub apply_patch: bool,
    pub prewarming: bool,
    pub hot_tools: bool,
    pub telemetry: bool,
    pub caching_prefix_optimization: bool,
}

impl CodexFeatureFlags {
    fn env_bool(var_name: &str, default: bool) -> bool {
        match std::env::var(var_name) {
            Ok(v) => match v.trim().to_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => true,
                "0" | "false" | "no" | "off" => false,
                _ => default,
            },
            Err(_) => default,
        }
    }

    /// Default production flags, honoring environment variables.
    pub fn from_env() -> Self {
        Self {
            transport_pool: Self::env_bool("PI_CODEX_TRANSPORT_POOL", true),
            responses_ledger: Self::env_bool("PI_CODEX_RESPONSES_LEDGER", true),
            tool_call_ledger: Self::env_bool("PI_CODEX_TOOL_CALL_LEDGER", true),
            apply_patch: Self::env_bool("PI_CODEX_APPLY_PATCH", true),
            prewarming: Self::env_bool("PI_CODEX_PREWARMING", true),
            hot_tools: Self::env_bool("PI_CODEX_HOT_TOOLS", true),
            telemetry: Self::env_bool("PI_CODEX_TELEMETRY", true),
            caching_prefix_optimization: Self::env_bool(
                "PI_CODEX_CACHING_PREFIX_OPTIMIZATION",
                true,
            ),
        }
    }

    /// All features disabled for baseline comparison against unoptimized profile.
    pub fn all_disabled() -> Self {
        Self {
            transport_pool: false,
            responses_ledger: false,
            tool_call_ledger: false,
            apply_patch: false,
            prewarming: false,
            hot_tools: false,
            telemetry: false,
            caching_prefix_optimization: false,
        }
    }

    /// All features enabled for fully optimized harness.
    pub fn all_enabled() -> Self {
        Self {
            transport_pool: true,
            responses_ledger: true,
            tool_call_ledger: true,
            apply_patch: true,
            prewarming: true,
            hot_tools: true,
            telemetry: true,
            caching_prefix_optimization: true,
        }
    }
}

impl Default for CodexFeatureFlags {
    fn default() -> Self {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honors_defaults_and_all_disabled() {
        let disabled = CodexFeatureFlags::all_disabled();
        assert!(!disabled.transport_pool);
        assert!(!disabled.apply_patch);
        assert!(!disabled.responses_ledger);

        let enabled = CodexFeatureFlags::all_enabled();
        assert!(enabled.transport_pool);
        assert!(enabled.apply_patch);
        assert!(enabled.responses_ledger);
    }
}
