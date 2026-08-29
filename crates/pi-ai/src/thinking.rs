use pi_protocol::ThinkingLevel;
use serde::{Deserialize, Serialize};

pub const MIN_ANSWER_TOKENS: u32 = 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingBudgets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimal: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high: Option<u32>,
}

pub const DEFAULT_THINKING_BUDGETS: ThinkingBudgets = ThinkingBudgets {
    minimal: Some(1024),
    low: Some(2048),
    medium: Some(8192),
    high: Some(16384),
};

pub fn clamp_reasoning(level: ThinkingLevel) -> Option<ThinkingLevel> {
    match level {
        ThinkingLevel::Off => None,
        ThinkingLevel::Xhigh | ThinkingLevel::Max => Some(ThinkingLevel::High),
        other => Some(other),
    }
}

pub fn thinking_budget_for_level(level: ThinkingLevel, custom: Option<&ThinkingBudgets>) -> u32 {
    let budgets = merge_budgets(custom);
    let resolved = clamp_reasoning(level).unwrap_or(ThinkingLevel::High);
    match resolved {
        ThinkingLevel::Minimal => budgets.minimal.unwrap_or(1024),
        ThinkingLevel::Low => budgets.low.unwrap_or(2048),
        ThinkingLevel::Medium => budgets.medium.unwrap_or(8192),
        ThinkingLevel::High | ThinkingLevel::Xhigh | ThinkingLevel::Max => {
            budgets.high.unwrap_or(16384)
        }
        ThinkingLevel::Off => 0,
    }
}

pub fn clamp_thinking_budget_to_answer_room(thinking_budget: u32, ceiling: u32) -> u32 {
    thinking_budget.min(ceiling.saturating_sub(MIN_ANSWER_TOKENS))
}

pub fn google_thinking_budget(
    model_id: &str,
    level: ThinkingLevel,
    custom: Option<&ThinkingBudgets>,
) -> i32 {
    if let Some(custom) = custom {
        let resolved = clamp_reasoning(level).unwrap_or(ThinkingLevel::High);
        let override_budget = match resolved {
            ThinkingLevel::Minimal => custom.minimal,
            ThinkingLevel::Low => custom.low,
            ThinkingLevel::Medium => custom.medium,
            ThinkingLevel::High | ThinkingLevel::Xhigh | ThinkingLevel::Max => custom.high,
            ThinkingLevel::Off => None,
        };
        if let Some(value) = override_budget {
            return value as i32;
        }
    }

    let resolved = clamp_reasoning(level).unwrap_or(ThinkingLevel::High);
    if model_id.contains("2.5-pro") {
        return match resolved {
            ThinkingLevel::Minimal => 128,
            ThinkingLevel::Low => 2048,
            ThinkingLevel::Medium => 8192,
            _ => 32768,
        };
    }
    if model_id.contains("2.5-flash-lite") {
        return match resolved {
            ThinkingLevel::Minimal => 512,
            ThinkingLevel::Low => 2048,
            ThinkingLevel::Medium => 8192,
            _ => 24576,
        };
    }
    if model_id.contains("2.5-flash") {
        return match resolved {
            ThinkingLevel::Minimal => 128,
            ThinkingLevel::Low => 2048,
            ThinkingLevel::Medium => 8192,
            _ => 24576,
        };
    }
    -1
}

fn merge_budgets(custom: Option<&ThinkingBudgets>) -> ThinkingBudgets {
    let mut budgets = DEFAULT_THINKING_BUDGETS.clone();
    if let Some(custom) = custom {
        if custom.minimal.is_some() {
            budgets.minimal = custom.minimal;
        }
        if custom.low.is_some() {
            budgets.low = custom.low;
        }
        if custom.medium.is_some() {
            budgets.medium = custom.medium;
        }
        if custom.high.is_some() {
            budgets.high = custom.high;
        }
    }
    budgets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_budget_defaults_and_custom_match_ts() {
        assert_eq!(thinking_budget_for_level(ThinkingLevel::Medium, None), 8192);
        assert_eq!(thinking_budget_for_level(ThinkingLevel::Xhigh, None), 16384);
        let custom = ThinkingBudgets {
            medium: Some(4096),
            ..ThinkingBudgets::default()
        };
        assert_eq!(
            thinking_budget_for_level(ThinkingLevel::Medium, Some(&custom)),
            4096
        );
        assert_eq!(
            clamp_thinking_budget_to_answer_room(16384, 4096),
            4096 - MIN_ANSWER_TOKENS
        );
        assert_eq!(
            google_thinking_budget("gemini-2.5-pro", ThinkingLevel::High, None),
            32768
        );
        assert_eq!(
            google_thinking_budget(
                "gemini-2.5-flash",
                ThinkingLevel::Medium,
                Some(&ThinkingBudgets {
                    medium: Some(1111),
                    ..ThinkingBudgets::default()
                })
            ),
            1111
        );
    }
}
