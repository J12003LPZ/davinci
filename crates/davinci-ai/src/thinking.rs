use crate::catalog::Model;
use davinci_protocol::ThinkingLevel;
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

/// TS `getSupportedThinkingLevels`.
pub fn get_supported_thinking_levels(model: &Model) -> Vec<ThinkingLevel> {
    if !model.reasoning {
        return vec![ThinkingLevel::Off];
    }
    ThinkingLevel::all()
        .iter()
        .copied()
        .filter(|level| match model.thinking_level_map.get(level.as_str()) {
            Some(None) => false,
            Some(Some(_)) => true,
            None => !matches!(*level, ThinkingLevel::Xhigh | ThinkingLevel::Max),
        })
        .collect()
}

/// TS `AgentSession.getAvailableThinkingLevels`.
pub fn available_thinking_levels(model: Option<&Model>) -> Vec<ThinkingLevel> {
    match model {
        Some(model) => get_supported_thinking_levels(model),
        None => ThinkingLevel::all().to_vec(),
    }
}

/// TS `AgentSession.cycleThinkingLevel` (`undefined` when the model does not reason).
pub fn cycle_thinking_level(
    model: Option<&Model>,
    current: ThinkingLevel,
) -> Option<ThinkingLevel> {
    if model.is_some_and(|model| !model.reasoning) {
        return None;
    }
    let levels = available_thinking_levels(model);
    if levels.is_empty() {
        return None;
    }
    let index = levels
        .iter()
        .position(|level| *level == current)
        .unwrap_or(0);
    Some(levels[(index + 1) % levels.len()])
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

    #[test]
    fn supported_thinking_levels_lock_ts() {
        let mut map = std::collections::BTreeMap::new();
        map.insert("off".into(), None);
        map.insert("xhigh".into(), Some("xhigh".into()));
        map.insert("max".into(), Some("max".into()));
        let reasoning = Model {
            id: "claude-fable-5".into(),
            name: "Fable".into(),
            api: "anthropic-messages".into(),
            provider: "anthropic".into(),
            base_url: None,
            reasoning: true,
            input: vec!["text".into()],
            cost: crate::catalog::ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: 1,
            max_tokens: 1,
            compat: serde_json::Value::Null,
            headers: Default::default(),
            thinking_level_map: map,
        };
        assert_eq!(
            get_supported_thinking_levels(&reasoning),
            vec![
                ThinkingLevel::Minimal,
                ThinkingLevel::Low,
                ThinkingLevel::Medium,
                ThinkingLevel::High,
                ThinkingLevel::Xhigh,
                ThinkingLevel::Max,
            ]
        );
        let mute = Model {
            reasoning: false,
            thinking_level_map: Default::default(),
            ..reasoning.clone()
        };
        assert_eq!(
            get_supported_thinking_levels(&mute),
            vec![ThinkingLevel::Off]
        );
        assert_eq!(cycle_thinking_level(Some(&mute), ThinkingLevel::Off), None);
        assert_eq!(
            cycle_thinking_level(Some(&reasoning), ThinkingLevel::High),
            Some(ThinkingLevel::Xhigh)
        );
    }

    #[test]
    fn catalog_fable_5_thinking_map_matches_ts() {
        let models = crate::load_builtin_models();
        let fable = models
            .iter()
            .find(|model| model.provider == "anthropic" && model.id == "claude-fable-5")
            .expect("claude-fable-5");
        assert_eq!(fable.thinking_level_map.get("off"), Some(&None));
        assert_eq!(
            get_supported_thinking_levels(fable),
            vec![
                ThinkingLevel::Minimal,
                ThinkingLevel::Low,
                ThinkingLevel::Medium,
                ThinkingLevel::High,
                ThinkingLevel::Xhigh,
                ThinkingLevel::Max,
            ]
        );
        let mute = models
            .iter()
            .find(|model| !model.reasoning)
            .expect("non-reasoning model");
        assert_eq!(
            get_supported_thinking_levels(mute),
            vec![ThinkingLevel::Off]
        );
        assert_eq!(cycle_thinking_level(Some(mute), ThinkingLevel::Off), None);
    }
}
