//! Context pruning: old tool output leaves the provider's view of the
//! transcript before compaction has to rewrite it.
//!
//! No TypeScript counterpart. A long investigation is mostly tool output
//! the model has already acted on: a grep it refined, a file it then
//! edited, a build log it read the last line of. Compaction summarizes the
//! whole conversation with another model call; pruning is cheaper and
//! lossless for the session: the JSONL keeps every byte, and only the
//! messages handed to the provider carry a placeholder in place of the
//! body. Once a result is pruned it stays pruned, so the prompt prefix is
//! rewritten once per prune pass rather than on every turn, and the
//! provider's prompt cache survives the turns in between.
//!
//! The rule: when the estimated context passes `start_fraction` of the
//! window, prune the oldest large tool results, never the most recent
//! `keep_recent` ones, until the estimate drops under `target_fraction` or
//! nothing prunable is left.

use std::collections::HashSet;

use davinci_ai::{ChatMessage, MessageContent};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PruneSettings {
    pub enabled: bool,
    /// Start pruning when the estimate exceeds this share of the window.
    pub start_fraction: f64,
    /// Prune until the estimate is under this share of the window.
    pub target_fraction: f64,
    /// The newest tool results are never pruned: the model is still using them.
    pub keep_recent: usize,
    /// Results shorter than this are not worth a placeholder.
    pub min_chars: usize,
}

impl Default for PruneSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            start_fraction: 0.5,
            target_fraction: 0.35,
            keep_recent: 8,
            min_chars: 1_500,
        }
    }
}

/// The text the provider sees in place of a pruned body.
pub fn placeholder(tool_name: &str, chars: usize) -> String {
    format!(
        "[output of {tool_name} pruned to save context ({chars} chars). Re-run the tool if you need it again.]"
    )
}

fn result_chars(message: &ChatMessage) -> usize {
    crate::compaction::estimate_content_chars(&message.content)
}

/// Which tool results to prune now, oldest first. `pruned` is what has been
/// pruned already; `tokens` is the current estimate for the projected
/// messages.
pub fn plan_prune(
    messages: &[ChatMessage],
    pruned: &HashSet<String>,
    tokens: u64,
    context_window: u64,
    settings: &PruneSettings,
) -> Vec<String> {
    if !settings.enabled || context_window == 0 {
        return Vec::new();
    }
    let start = (context_window as f64 * settings.start_fraction) as u64;
    if tokens <= start {
        return Vec::new();
    }
    let target = (context_window as f64 * settings.target_fraction) as u64;
    let candidates: Vec<(&String, usize)> = messages
        .iter()
        .filter(|message| message.role == "toolResult")
        .filter_map(|message| {
            let id = message.tool_call_id.as_ref()?;
            Some((id, result_chars(message)))
        })
        .collect();
    let protected = candidates.len().saturating_sub(settings.keep_recent);
    let mut remaining = tokens;
    let mut plan = Vec::new();
    for (id, chars) in candidates.into_iter().take(protected) {
        if remaining <= target {
            break;
        }
        if pruned.contains(id) || chars < settings.min_chars {
            continue;
        }
        let saved = (chars as u64).div_ceil(4);
        remaining = remaining.saturating_sub(saved);
        plan.push(id.clone());
    }
    plan
}

/// The messages as the provider should see them: pruned tool results carry
/// a placeholder instead of their body (and no images).
pub fn project(messages: &[ChatMessage], pruned: &HashSet<String>) -> Vec<ChatMessage> {
    if pruned.is_empty() {
        return messages.to_vec();
    }
    messages
        .iter()
        .map(|message| {
            if message.role != "toolResult" {
                return message.clone();
            }
            let Some(id) = &message.tool_call_id else {
                return message.clone();
            };
            if !pruned.contains(id) {
                return message.clone();
            }
            let chars = result_chars(message);
            let tool = message.tool_name.as_deref().unwrap_or("tool");
            ChatMessage {
                content: vec![MessageContent::Text {
                    text: placeholder(tool, chars),
                }],
                ..message.clone()
            }
        })
        .collect()
}

/// Token estimate for the projected view without building it.
pub fn estimate_projected_tokens(messages: &[ChatMessage], pruned: &HashSet<String>) -> u64 {
    messages
        .iter()
        .map(|message| {
            let is_pruned = message.role == "toolResult"
                && message
                    .tool_call_id
                    .as_ref()
                    .is_some_and(|id| pruned.contains(id));
            if is_pruned {
                let tool = message.tool_name.as_deref().unwrap_or("tool");
                (placeholder(tool, result_chars(message)).len() as u64).div_ceil(4)
            } else {
                crate::compaction::estimate_tokens(message)
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_result(id: &str, chars: usize) -> ChatMessage {
        ChatMessage::tool_result(id, "grep", "x".repeat(chars), false)
    }

    #[test]
    fn nothing_is_pruned_under_the_start_line() {
        let messages: Vec<_> = (0..12)
            .map(|i| tool_result(&format!("c{i}"), 4_000))
            .collect();
        let tokens = estimate_projected_tokens(&messages, &HashSet::new());
        // 12 * 1000 = 12k tokens; window 100k; start at 50k.
        let plan = plan_prune(
            &messages,
            &HashSet::new(),
            tokens,
            100_000,
            &PruneSettings::default(),
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn oldest_large_results_go_first_and_recent_ones_are_kept() {
        let messages: Vec<_> = (0..12)
            .map(|i| tool_result(&format!("c{i}"), 4_000))
            .collect();
        let settings = PruneSettings {
            keep_recent: 8,
            ..PruneSettings::default()
        };
        // 12k tokens against a 20k window: over the 10k start line.
        let plan = plan_prune(&messages, &HashSet::new(), 12_000, 20_000, &settings);
        // Only the first four are prunable; pruning all four brings 12k
        // down to 8k, above the 7k target, so all four are taken.
        assert_eq!(plan, vec!["c0", "c1", "c2", "c3"]);
        let pruned: HashSet<String> = plan.into_iter().collect();
        let projected = project(&messages, &pruned);
        assert!(projected[0].content.iter().any(|block| matches!(
            block,
            MessageContent::Text { text } if text.contains("pruned to save context")
        )));
        assert_eq!(projected[11], messages[11]);
        assert!(estimate_projected_tokens(&messages, &pruned) < 9_000);
    }

    #[test]
    fn small_and_already_pruned_results_are_skipped() {
        let mut messages: Vec<_> = (0..10)
            .map(|i| tool_result(&format!("c{i}"), 4_000))
            .collect();
        messages[0] = tool_result("c0", 100);
        let pruned: HashSet<String> = ["c1".to_string()].into_iter().collect();
        let settings = PruneSettings {
            keep_recent: 5,
            ..PruneSettings::default()
        };
        let plan = plan_prune(&messages, &pruned, 12_000, 20_000, &settings);
        assert_eq!(plan, vec!["c2", "c3", "c4"]);
    }

    #[test]
    fn pruning_stops_at_the_target() {
        let messages: Vec<_> = (0..20)
            .map(|i| tool_result(&format!("c{i}"), 4_000))
            .collect();
        let settings = PruneSettings {
            keep_recent: 2,
            ..PruneSettings::default()
        };
        // 20k tokens, 30k window: target is 10.5k, so ten prunings suffice.
        let plan = plan_prune(&messages, &HashSet::new(), 20_000, 30_000, &settings);
        assert_eq!(plan.len(), 10);
    }
}
