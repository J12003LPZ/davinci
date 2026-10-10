//! Prompt-cache miss accounting matching TS `core/cache-stats.ts`.

use davinci_session::SessionEntry;
use serde_json::Value;

/// Local estimator noise guard. This is not a provider eligibility threshold
/// and must not be presented as evidence about OpenAI cache lifetime/behavior.
const ESTIMATED_MISS_NOISE_TOKENS: i64 = 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct CacheMiss {
    pub missed_tokens: i64,
    pub missed_cost: f64,
    pub idle_ms: u64,
    pub model_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CacheWasteTotals {
    pub missed_tokens: i64,
    pub missed_cost: f64,
    pub miss_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssistantUsage {
    pub provider: String,
    pub model: String,
    pub timestamp: u64,
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_input: f64,
    pub cost_cache_read: f64,
    pub cost_cache_write: f64,
    /// The provider sent a usage block with a numeric `input`. When false,
    /// every count above is a placeholder 0 meaning "unknown", not "none".
    pub usage_reported: bool,
    /// The provider sent a numeric `cacheRead`. A missing field is unknown:
    /// reading it as 0 would claim a cache miss the provider never reported.
    pub cache_read_reported: bool,
    /// A numeric write count is known, including zero; the normalized
    /// placeholder carries `cacheWriteUnreported` when the provider omitted it.
    pub cache_write_reported: bool,
}

/// What the provider itself said about the prompt cache for one response.
/// A local prefix hash or cache key never produces `ReadReported`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCacheObservation {
    /// No usage block, or no cache-read field: nothing is known.
    Unknown,
    /// The provider reported cache reads of zero.
    NoReadReported,
    /// The provider reported cache reads above zero.
    ReadReported,
}

impl AssistantUsage {
    pub fn cache_observation(&self) -> ProviderCacheObservation {
        if !self.usage_reported || !self.cache_read_reported {
            ProviderCacheObservation::Unknown
        } else if self.cache_read > 0 {
            ProviderCacheObservation::ReadReported
        } else {
            ProviderCacheObservation::NoReadReported
        }
    }
}

pub trait ModelPriceSource {
    fn cache_read_per_million(&self, provider: &str, model: &str) -> f64;
}

impl ModelPriceSource for f64 {
    fn cache_read_per_million(&self, _provider: &str, _model: &str) -> f64 {
        *self
    }
}

#[derive(Debug, Clone)]
struct PreviousRequest {
    prompt_tokens: u64,
    model_key: String,
    timestamp: u64,
    reported_cache: bool,
}

fn detect_miss(
    prev: Option<&PreviousRequest>,
    message: &AssistantUsage,
    models: &dyn ModelPriceSource,
) -> Option<CacheMiss> {
    let prompt_tokens = message.input + message.cache_read + message.cache_write;
    let prev = prev?;
    if prompt_tokens == 0 {
        return None;
    }
    // Unknown usage is not evidence of a miss.
    if message.cache_observation() == ProviderCacheObservation::Unknown
        || !message.cache_write_reported
    {
        return None;
    }
    if message.cache_read + message.cache_write == 0 && !prev.reported_cache {
        return None;
    }

    let missed_tokens =
        (prev.prompt_tokens.min(prompt_tokens) as i64) - (message.cache_read as i64);
    if missed_tokens <= ESTIMATED_MISS_NOISE_TOKENS {
        return None;
    }

    let paid_tokens = message.input + message.cache_write;
    let paid_per_token = if paid_tokens > 0 {
        (message.cost_input + message.cost_cache_write) / paid_tokens as f64
    } else {
        0.0
    };
    let read_per_token = if message.cache_read > 0 {
        message.cost_cache_read / message.cache_read as f64
    } else {
        models.cache_read_per_million(&message.provider, &message.model) / 1_000_000.0
    };

    Some(CacheMiss {
        missed_tokens,
        missed_cost: missed_tokens as f64 * (paid_per_token - read_per_token).max(0.0),
        idle_ms: message.timestamp.saturating_sub(prev.timestamp),
        model_changed: format!("{}/{}", message.provider, message.model) != prev.model_key,
    })
}

fn as_previous_request(message: &AssistantUsage, reported_cache: bool) -> Option<PreviousRequest> {
    let prompt_tokens = message.input + message.cache_read + message.cache_write;
    if prompt_tokens == 0 || !message.usage_reported || !message.cache_write_reported {
        return None;
    }
    Some(PreviousRequest {
        prompt_tokens,
        model_key: format!("{}/{}", message.provider, message.model),
        timestamp: message.timestamp,
        reported_cache: reported_cache || message.cache_read + message.cache_write > 0,
    })
}

fn scan(
    entries: &[SessionEntry],
    models: &dyn ModelPriceSource,
) -> (
    Option<PreviousRequest>,
    CacheWasteTotals,
    Vec<(usize, CacheMiss)>,
) {
    let mut prev = None;
    let mut totals = CacheWasteTotals::default();
    let mut misses = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.entry_type == "compaction" || entry.entry_type == "branch_summary" {
            prev = None;
            continue;
        }
        if entry.entry_type == "message" {
            let Some(message) = assistant_usage_from_entry(entry) else {
                continue;
            };
            if let Some(miss) = detect_miss(prev.as_ref(), &message, models) {
                totals.missed_tokens += miss.missed_tokens;
                totals.missed_cost += miss.missed_cost;
                totals.miss_count += 1;
                misses.push((index, miss));
            }
            let reported = prev
                .as_ref()
                .map(|item| item.reported_cache)
                .unwrap_or(false);
            if let Some(next) = as_previous_request(&message, reported) {
                prev = Some(next);
            }
        }
    }
    (prev, totals, misses)
}

pub fn compute_cache_waste(
    entries: &[SessionEntry],
    models: &dyn ModelPriceSource,
) -> CacheWasteTotals {
    scan(entries, models).1
}

pub fn collect_cache_misses(
    entries: &[SessionEntry],
    models: &dyn ModelPriceSource,
) -> Vec<(usize, CacheMiss)> {
    scan(entries, models).2
}

pub fn detect_cache_miss(
    entries: &[SessionEntry],
    message: &AssistantUsage,
    models: &dyn ModelPriceSource,
) -> Option<CacheMiss> {
    detect_miss(scan(entries, models).0.as_ref(), message, models)
}

pub fn assistant_usage_from_entry(entry: &SessionEntry) -> Option<AssistantUsage> {
    let message = entry.message.as_ref()?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    Some(assistant_usage_from_value(message, entry.timestamp))
}

pub fn assistant_usage_from_value(message: &Value, fallback_timestamp: u64) -> AssistantUsage {
    let usage = message.get("usage");
    let cost = usage.and_then(|value| value.get("cost"));
    AssistantUsage {
        provider: message
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        model: message
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        timestamp: message
            .get("timestamp")
            .and_then(Value::as_u64)
            .unwrap_or(fallback_timestamp),
        usage_reported: usage
            .and_then(|value| value.get("input"))
            .and_then(Value::as_u64)
            .is_some(),
        cache_read_reported: usage
            .and_then(|value| value.get("cacheRead"))
            .and_then(Value::as_u64)
            .is_some(),
        cache_write_reported: usage
            .and_then(|value| value.get("cacheWrite"))
            .and_then(Value::as_u64)
            .is_some()
            && usage
                .and_then(|value| value.get("cacheWriteUnreported"))
                .and_then(Value::as_bool)
                != Some(true),
        input: usage_u64(usage, "input"),
        cache_read: usage_u64(usage, "cacheRead"),
        cache_write: usage_u64(usage, "cacheWrite"),
        cost_input: cost_f64(cost, "input"),
        cost_cache_read: cost_f64(cost, "cacheRead"),
        cost_cache_write: cost_f64(cost, "cacheWrite"),
    }
}

fn usage_u64(usage: Option<&Value>, key: &str) -> u64 {
    usage
        .and_then(|value| value.get(key))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn cost_f64(cost: Option<&Value>, key: &str) -> f64 {
    cost.and_then(|value| value.get(key))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

/// TS `formatTokens` from `footer.ts`.
pub fn format_tokens(count: i64) -> String {
    let count = count.max(0) as f64;
    if count < 1000.0 {
        return (count as i64).to_string();
    }
    if count < 10_000.0 {
        return format!("{:.1}k", count / 1000.0);
    }
    if count < 1_000_000.0 {
        return format!("{}k", (count / 1000.0).round() as i64);
    }
    if count < 10_000_000.0 {
        return format!("{:.1}M", count / 1_000_000.0);
    }
    format!("{}M", (count / 1_000_000.0).round() as i64)
}

/// TS `addCacheMissNotice` copy. Returns `None` below the display threshold.
pub fn format_cache_miss_notice(miss: &CacheMiss) -> Option<String> {
    if miss.missed_tokens < 20_000 && miss.missed_cost < 0.1 {
        return None;
    }
    let cost = if miss.missed_cost >= 0.01 {
        format!(" (~${:.2})", miss.missed_cost)
    } else {
        String::new()
    };
    let re_billed = format!(
        "{} tokens re-billed{cost}",
        format_tokens(miss.missed_tokens)
    );
    let label = if miss.model_changed {
        "Estimated cache miss after model switch".into()
    } else if miss.idle_ms > 0 {
        format!(
            "Estimated cache miss after {}m idle (expiry/routing/eligibility uncertain)",
            (miss.idle_ms as f64 / 60_000.0).round() as u64
        )
    } else {
        "Estimated cache miss".into()
    };
    Some(format!("{label}: {re_billed}"))
}

pub fn format_compaction_cost_notice(kind: &str, usage: &davinci_protocol::Usage) -> String {
    let tokens = usage.input + usage.output + usage.cache_read + usage.cache_write;
    let cost = if usage.cost.total >= 0.01 {
        format!(" (~${:.2})", usage.cost.total)
    } else {
        String::new()
    };
    let label = if kind == "branch_summary" {
        "Branch summary"
    } else {
        "Compaction"
    };
    format!(
        "{label}: {} tokens billed{cost}",
        format_tokens(tokens as i64)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS: f64 = 0.3;

    fn assistant(
        input: u64,
        cache_read: u64,
        cache_write: u64,
        cost_write: f64,
        cost_read: f64,
        model: &str,
        timestamp: u64,
    ) -> SessionEntry {
        SessionEntry {
            id: "x".into(),
            entry_type: "message".into(),
            parent_id: None,
            seq: 0,
            timestamp,
            message: Some(serde_json::json!({
                "role": "assistant",
                "provider": "test",
                "model": model,
                "timestamp": timestamp,
                "usage": {
                    "input": input,
                    "output": 10,
                    "cacheRead": cache_read,
                    "cacheWrite": cache_write,
                    "cost": {
                        "input": 0.0,
                        "output": 0.0,
                        "cacheRead": cost_read,
                        "cacheWrite": cost_write,
                        "total": 0.0
                    }
                }
            })),
            custom_type: None,
            extra: serde_json::Map::new(),
        }
    }

    fn turn1() -> SessionEntry {
        assistant(0, 0, 100_000, 0.375, 0.0, "test-model", 0)
    }

    fn turn2() -> SessionEntry {
        assistant(0, 100_000, 5_000, 0.019, 0.03, "test-model", 60_000)
    }

    #[test]
    fn compute_cache_waste_matches_ts_fixtures() {
        let turn3 = assistant(0, 0, 110_000, 0.4125, 0.0, "test-model", 120_000);
        let totals = compute_cache_waste(&[turn1(), turn2(), turn3], &MODELS);
        assert_eq!(totals.missed_tokens, 105_000);
        assert!((totals.missed_cost - 0.36225).abs() < 1e-5);

        let healthy = compute_cache_waste(&[turn1(), turn2()], &MODELS);
        assert_eq!(healthy.missed_tokens, 0);
        assert_eq!(healthy.missed_cost, 0.0);

        let reset = SessionEntry {
            id: "c".into(),
            entry_type: "compaction".into(),
            parent_id: None,
            seq: 0,
            timestamp: 0,
            message: None,
            custom_type: None,
            extra: serde_json::Map::new(),
        };
        let after = assistant(0, 0, 20_000, 0.075, 0.0, "test-model", 0);
        assert_eq!(
            compute_cache_waste(&[turn1(), reset, after], &MODELS).missed_tokens,
            0
        );

        let other = assistant(0, 0, 100_000, 0.375, 0.0, "other-model", 0);
        let switched = compute_cache_waste(&[turn1(), other], &MODELS);
        assert_eq!(switched.missed_tokens, 100_000);
        assert_eq!(switched.miss_count, 1);

        let a = assistant(100_000, 0, 0, 0.0, 0.0, "test-model", 0);
        let b = assistant(110_000, 0, 0, 0.0, 0.0, "test-model", 1);
        assert_eq!(compute_cache_waste(&[a, b], &MODELS).missed_tokens, 0);
    }

    #[test]
    fn collect_and_detect_match_ts() {
        let miss_turn = assistant(0, 0, 110_000, 0.4125, 0.0, "test-model", 120_000);
        let misses = collect_cache_misses(&[turn1(), turn2(), miss_turn.clone()], &MODELS);
        assert_eq!(misses.len(), 1);
        assert_eq!(misses[0].1.missed_tokens, 105_000);

        let live = assistant_usage_from_entry(&assistant(
            0,
            0,
            110_000,
            0.4125,
            0.0,
            "test-model",
            600_000,
        ))
        .unwrap();
        let miss = detect_cache_miss(&[turn1(), turn2()], &live, &MODELS).unwrap();
        assert_eq!(miss.missed_tokens, 105_000);
        assert!((miss.missed_cost - 0.36225).abs() < 1e-5);
        assert_eq!(miss.idle_ms, 540_000);
        assert!(!miss.model_changed);

        let other = assistant_usage_from_entry(&assistant(
            0,
            0,
            110_000,
            0.4125,
            0.0,
            "other-model",
            120_000,
        ))
        .unwrap();
        assert!(
            detect_cache_miss(&[turn1(), turn2()], &other, &MODELS)
                .unwrap()
                .model_changed
        );

        let healthy = assistant_usage_from_entry(&assistant(
            0,
            105_000,
            2_000,
            0.0075,
            0.0315,
            "test-model",
            120_000,
        ))
        .unwrap();
        assert!(detect_cache_miss(&[turn1(), turn2()], &healthy, &MODELS).is_none());
        let first = assistant_usage_from_entry(&turn1()).unwrap();
        assert!(detect_cache_miss(&[], &first, &MODELS).is_none());
    }

    #[test]
    fn notice_copy_matches_ts_threshold_and_labels() {
        assert_eq!(format_tokens(105_000), "105k");
        let miss = CacheMiss {
            missed_tokens: 105_000,
            missed_cost: 0.36225,
            idle_ms: 540_000,
            model_changed: false,
        };
        assert_eq!(
            format_cache_miss_notice(&miss).as_deref(),
            Some(
                "Estimated cache miss after 9m idle (expiry/routing/eligibility uncertain): 105k tokens re-billed (~$0.36)"
            )
        );
        let switched = CacheMiss {
            model_changed: true,
            idle_ms: 0,
            ..miss
        };
        assert!(format_cache_miss_notice(&switched)
            .unwrap()
            .starts_with("Estimated cache miss after model switch:"));
        let small = CacheMiss {
            missed_tokens: 5_000,
            missed_cost: 0.01,
            idle_ms: 0,
            model_changed: false,
        };
        assert!(format_cache_miss_notice(&small).is_none());
        let usage = davinci_protocol::Usage {
            input: 1_000,
            output: 2_000,
            cache_read: 0,
            cache_write: 0,
            cache_write_unreported: false,
            reasoning: None,
            total_tokens: 3_000,
            cost: davinci_protocol::UsageCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
                total: 0.02,
            },
        };
        assert_eq!(
            format_compaction_cost_notice("compaction", &usage),
            "Compaction: 3.0k tokens billed (~$0.02)"
        );
    }

    // ---- WOR-57: local cache identity vs what the provider reported -------
    //
    // Every case uses recorded/fake usage blocks. Nothing here calls a
    // provider, and none of it proves a billed cache hit: the local key only
    // says two requests were eligible to share a slot.

    use davinci_agent::CacheIdentity;

    fn identity(permission: &str, context: &[&str]) -> CacheIdentity {
        CacheIdentity {
            provider: "openai-codex".into(),
            model_id: "gpt-test".into(),
            system_prompt_hash: "sys".into(),
            tool_schema_hash: "tools".into(),
            permission_surface_hash: permission.into(),
            context_item_hashes: context.iter().map(|s| s.to_string()).collect(),
            agent_profile_hash: None,
            contract_hash: None,
            role: Some("root".into()),
        }
    }

    fn entry_with_usage(usage: Option<serde_json::Value>, timestamp: u64) -> SessionEntry {
        let mut message = serde_json::json!({
            "role": "assistant", "provider": "openai-codex",
            "model": "gpt-test", "timestamp": timestamp,
        });
        if let Some(usage) = usage {
            message["usage"] = usage;
        }
        SessionEntry {
            id: "u".into(),
            entry_type: "message".into(),
            parent_id: None,
            seq: 0,
            timestamp,
            message: Some(message),
            custom_type: None,
            extra: serde_json::Map::new(),
        }
    }

    fn usage(input: u64, read: u64, write: u64) -> serde_json::Value {
        serde_json::json!({"input": input, "output": 5, "cacheRead": read, "cacheWrite": write})
    }

    fn observe(entry: &SessionEntry) -> ProviderCacheObservation {
        assistant_usage_from_entry(entry)
            .unwrap()
            .cache_observation()
    }

    fn misses(entries: &[SessionEntry]) -> Vec<CacheMiss> {
        collect_cache_misses(entries, &MODELS)
            .into_iter()
            .map(|(_, miss)| miss)
            .collect()
    }

    #[test]
    fn warm_turn_same_key_and_provider_read_is_a_reported_hit_and_no_miss() {
        let a = identity("perm", &["ctx"]);
        assert_eq!(a.cache_key(), identity("perm", &["ctx"]).cache_key());
        let turns = [
            entry_with_usage(Some(usage(0, 0, 100_000)), 0),
            entry_with_usage(Some(usage(500, 100_000, 0)), 30_000),
        ];
        assert_eq!(observe(&turns[1]), ProviderCacheObservation::ReadReported);
        assert!(misses(&turns).is_empty());
    }

    #[test]
    fn a_mode_toggle_changes_the_local_key_and_a_zero_read_is_an_estimated_miss() {
        let before = identity("perm-a", &["ctx"]).cache_key();
        let after = identity("perm-b", &["ctx"]).cache_key();
        assert_ne!(before, after, "different tool surface, different namespace");
        let turns = [
            entry_with_usage(Some(usage(0, 0, 100_000)), 0),
            entry_with_usage(Some(usage(0, 100_000, 100)), 10_000),
            entry_with_usage(Some(usage(0, 0, 100_100)), 20_000),
        ];
        let found = misses(&turns);
        assert_eq!(found.len(), 1);
        let notice = format_cache_miss_notice(&found[0]).unwrap();
        assert!(notice.starts_with("Estimated cache miss"), "{notice}");
    }

    #[test]
    fn a_fold_rotates_the_namespace_and_the_compaction_resets_the_baseline() {
        let before = identity("perm", &["epoch-1"]).cache_key();
        let after = identity("perm", &["epoch-2"]).cache_key();
        assert_ne!(before, after);
        let fold = SessionEntry {
            id: "c".into(),
            entry_type: "compaction".into(),
            parent_id: None,
            seq: 0,
            timestamp: 0,
            message: None,
            custom_type: None,
            extra: serde_json::Map::new(),
        };
        let turns = [
            entry_with_usage(Some(usage(0, 0, 100_000)), 0),
            entry_with_usage(Some(usage(0, 100_000, 100)), 10_000),
            fold,
            entry_with_usage(Some(usage(0, 0, 30_000)), 20_000),
        ];
        assert!(
            misses(&turns).is_empty(),
            "the fold is not billed as a miss"
        );
    }

    #[test]
    fn a_restarted_worker_has_the_same_key_but_the_provider_decides_the_hit() {
        let first = identity("perm", &["ctx"]).cache_key();
        let restarted = identity("perm", &["ctx"]).cache_key();
        assert_eq!(first, restarted, "keys are stable across restarts");
        // Same key, yet the provider reports a cold prefix: key equality is
        // eligibility, never a hit.
        let turns = [
            entry_with_usage(Some(usage(0, 0, 100_000)), 0),
            entry_with_usage(Some(usage(0, 100_000, 100)), 10_000),
            entry_with_usage(Some(usage(0, 0, 100_100)), 20_000),
        ];
        assert_eq!(observe(&turns[2]), ProviderCacheObservation::NoReadReported);
        assert_eq!(misses(&turns).len(), 1);
    }

    #[test]
    fn provider_ttl_expiry_is_reported_as_an_uncertain_estimate_not_a_proven_cause() {
        let turns = [
            entry_with_usage(Some(usage(0, 0, 100_000)), 0),
            entry_with_usage(Some(usage(0, 100_000, 100)), 60_000),
            entry_with_usage(Some(usage(0, 0, 100_100)), 60_000 + 45 * 60_000),
        ];
        let found = misses(&turns);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].idle_ms, 45 * 60_000);
        let notice = format_cache_miss_notice(&found[0]).unwrap();
        assert!(
            notice.contains("expiry/routing/eligibility uncertain"),
            "{notice}"
        );
    }

    #[test]
    fn missing_usage_is_unknown_never_zero() {
        let silent = entry_with_usage(None, 20_000);
        assert_eq!(observe(&silent), ProviderCacheObservation::Unknown);
        let parsed = assistant_usage_from_entry(&silent).unwrap();
        assert!(!parsed.usage_reported);
        // Usage with input but no cacheRead field: still unknown.
        let partial = entry_with_usage(Some(serde_json::json!({"input": 120_000})), 30_000);
        assert_eq!(observe(&partial), ProviderCacheObservation::Unknown);
        // Neither can manufacture a miss after a healthy cached turn, and the
        // unreported turn does not become the new baseline.
        let turns = [
            entry_with_usage(Some(usage(0, 0, 100_000)), 0),
            entry_with_usage(Some(usage(0, 100_000, 100)), 10_000),
            silent,
            partial,
            entry_with_usage(Some(usage(0, 100_100, 50)), 40_000),
        ];
        assert!(misses(&turns).is_empty());
        // An explicit zero is a report, and is distinguishable from unknown.
        let zero = entry_with_usage(Some(usage(100, 0, 0)), 1);
        assert_eq!(observe(&zero), ProviderCacheObservation::NoReadReported);
    }

    #[test]
    fn wor37_missing_writes_cannot_produce_a_miss_estimate() {
        for raw in [
            serde_json::json!({"input":120000,"cacheRead":0}),
            serde_json::json!({"input":120000,"cacheRead":0,"cacheWrite":0,"cacheWriteUnreported":true}),
        ] {
            let unknown = entry_with_usage(Some(raw.clone()), 20000);
            assert!(
                misses(&[entry_with_usage(Some(usage(1000, 100000, 0)), 0), unknown]).is_empty()
            );
            // Unknown writes must not become the baseline for a later estimate either.
            assert!(misses(&[
                entry_with_usage(Some(raw), 0),
                entry_with_usage(Some(usage(120000, 0, 0)), 20000)
            ])
            .is_empty());
        }
    }
}
