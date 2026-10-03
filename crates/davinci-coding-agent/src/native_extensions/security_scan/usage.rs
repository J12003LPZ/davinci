//! Per-request accounting for the isolated native review loop.
//! Uses normalized usage from davinci-ai, including cached input tokens.
use davinci_protocol::Usage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestUsage {
    pub measured: Option<MeasuredTokens>,
    pub estimated_cost_usd: Option<f64>,
    pub accounted_tokens: u64,
    pub reserved_tokens: u64,
    pub wall_time_ms: u64,
    pub failed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasuredTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}

fn component_total(usage: &Usage) -> u64 {
    usage
        .input
        .saturating_add(usage.output)
        .saturating_add(usage.cache_read)
        .saturating_add(usage.cache_write)
}

fn measured(usage: &Usage) -> bool {
    usage.total_tokens > 0 && usage.total_tokens >= component_total(usage)
}

pub fn account(usage: Option<&Usage>, reservation: u64) -> u64 {
    usage.map_or(reservation, |usage| {
        if measured(usage) {
            usage.total_tokens
        } else {
            reservation
                .max(component_total(usage))
                .max(usage.total_tokens)
        }
    })
}

impl RequestUsage {
    pub fn new(usage: Option<&Usage>, reservation: u64, wall_time_ms: u64, failed: bool) -> Self {
        let known = usage.filter(|usage| measured(usage));
        Self {
            measured: known.map(|usage| MeasuredTokens {
                input: usage.input,
                output: usage.output,
                cache_read: usage.cache_read,
                cache_write: usage.cache_write,
                total: usage.total_tokens,
            }),
            // Provider adapters calculate costs from catalog prices. Default
            // zero costs do not establish either known pricing or a free call.
            estimated_cost_usd: known
                .map(|usage| usage.cost.total)
                .filter(|cost| cost.is_finite() && *cost > 0.0),
            accounted_tokens: account(usage, reservation),
            reserved_tokens: reservation,
            wall_time_ms,
            failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_usage_missing_or_zero_totals_do_not_erase_reservations() {
        assert_eq!(account(None, 1000), 1000);
        assert_eq!(account(Some(&Usage::default()), 1000), 1000);
        let usage = Usage {
            input: 1500,
            output: 200,
            cache_read: 300,
            ..Usage::default()
        };
        assert_eq!(account(Some(&usage), 1000), 2000);
    }

    #[test]
    fn security_usage_includes_cache_but_does_not_double_count_reasoning() {
        let usage = Usage {
            input: 100,
            output: 200,
            cache_read: 300,
            cache_write: 50,
            reasoning: Some(100),
            total_tokens: 650,
            cost: Default::default(),
        };
        assert_eq!(account(Some(&usage), 1000), 650);
    }

    #[test]
    fn security_usage_unknown_measurements_and_prices_stay_null() {
        let absent = serde_json::to_value(RequestUsage::new(None, 1000, 12, true)).unwrap();
        assert!(absent["measured"].is_null());
        assert!(absent["estimatedCostUsd"].is_null());
        assert_eq!(absent["accountedTokens"], 1000);
        assert_eq!(absent["failed"], true);
        let usage = Usage {
            input: 100,
            output: 20,
            total_tokens: 120,
            ..Usage::default()
        };
        let known = RequestUsage::new(Some(&usage), 1000, 8, false);
        assert_eq!(known.measured.unwrap().total, 120);
        assert!(known.estimated_cost_usd.is_none());
        let inconsistent = Usage {
            total_tokens: 10,
            ..usage
        };
        let unknown = RequestUsage::new(Some(&inconsistent), 1000, 8, false);
        assert!(unknown.measured.is_none());
        assert_eq!(unknown.accounted_tokens, 1000);
    }
}
