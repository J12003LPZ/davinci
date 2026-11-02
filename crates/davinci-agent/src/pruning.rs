//! Context pruning: old tool output leaves the provider's view of the
//! transcript before compaction has to rewrite it.
//!
//! No TypeScript counterpart. A long investigation is mostly tool output
//! the model has already acted on: a grep it refined, a file it then
//! edited, a build log it read the last line of. Compaction summarizes the
//! whole conversation with another model call; pruning is cheaper and
//! lossless for the session: the JSONL keeps every byte, and only the
//! messages handed to the provider carry a placeholder in place of the
//! body. The agent retains readable evidence before replacing text and
//! restores the original body if that evidence becomes unavailable.
//!
//! The rule: when the estimated context passes `start_fraction` of the
//! window, prune the oldest large tool results, never the most recent
//! `keep_recent` ones, until the estimate drops under `target_fraction` or
//! nothing prunable is left.

use std::collections::HashSet;

use davinci_ai::ChatMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PruneOrder {
    /// Current behavior: prune oldest first, regardless of outcome.
    #[default]
    OldestFirst,
    /// Experimental profile: prune routine results before error results.
    ErrorsLast,
}

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
    /// Candidate order within the eligible window.
    pub order: PruneOrder,
}

impl Default for PruneSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            start_fraction: 0.5,
            target_fraction: 0.35,
            keep_recent: 8,
            min_chars: 1_500,
            order: PruneOrder::OldestFirst,
        }
    }
}

impl PruneSettings {
    /// Cache-sensitive routes prune in fewer, larger passes because every
    /// rewrite invalidates the suffix of the cached conversation.
    pub fn cached_route() -> Self {
        Self {
            start_fraction: 0.65,
            target_fraction: 0.35,
            ..Self::default()
        }
    }

    /// Experimental profile: errors survive until routine results cannot meet
    /// the target. The default is unchanged.
    pub fn evidence_route() -> Self {
        Self {
            order: PruneOrder::ErrorsLast,
            ..Self::default()
        }
    }

    /// `DAVINCI_PRUNE_PROFILE=default|cached|off|evidence` overrides the route.
    /// Explicit caller settings are never replaced.
    pub fn for_route(base: &PruneSettings, cache_sensitive: bool) -> PruneSettings {
        if *base != PruneSettings::default() {
            return *base;
        }
        match std::env::var("DAVINCI_PRUNE_PROFILE").ok().as_deref() {
            Some("off") => PruneSettings {
                enabled: false,
                ..*base
            },
            Some("default") => *base,
            Some("cached") => PruneSettings::cached_route(),
            Some("evidence") => PruneSettings::evidence_route(),
            _ if cache_sensitive => PruneSettings::cached_route(),
            _ => *base,
        }
    }
}

/// The text the provider sees in place of a pruned body.
pub fn placeholder(tool_name: &str, chars: usize) -> String {
    format!(
        "[output of {tool_name} pruned to save context ({chars} chars); class: tool_output. Do not repeat the operation to recover output; consult retained evidence or session history.]"
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
    let mut candidates: Vec<(&String, usize, bool)> = messages
        .iter()
        .filter(|message| message.role == "toolResult")
        .filter_map(|message| {
            let id = message.tool_call_id.as_ref()?;
            Some((id, result_chars(message), message.is_error == Some(true)))
        })
        .collect();
    if settings.order == PruneOrder::ErrorsLast {
        // Stable sort: routine results keep their order, then error results.
        candidates.sort_by_key(|(_, _, is_error)| *is_error);
    }
    let protected = candidates.len().saturating_sub(settings.keep_recent);
    let mut remaining = tokens;
    let mut plan = Vec::new();
    for (id, chars, _) in candidates.into_iter().take(protected) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_routes_prune_later_and_deeper() {
        let settings = PruneSettings::cached_route();
        assert!(settings.enabled);
        assert_eq!(settings.start_fraction, 0.65);
        assert_eq!(settings.target_fraction, 0.35);
    }

    #[test]
    fn custom_pruning_is_not_overridden_by_route() {
        let custom = PruneSettings {
            keep_recent: 2,
            ..PruneSettings::default()
        };
        assert_eq!(PruneSettings::for_route(&custom, true), custom);
    }

    #[test]
    fn pruned_mutation_does_not_request_reexecution() {
        let body = placeholder("bash", 7_000);
        assert!(!body.contains("Re-run"));
        assert!(body.contains("Do not repeat"));
        assert!(body.contains("class: tool_output"));
    }

    fn tool_result(id: &str, chars: usize) -> ChatMessage {
        ChatMessage::tool_result(id, "grep", "x".repeat(chars), false)
    }

    fn error_result(id: &str, chars: usize) -> ChatMessage {
        ChatMessage::tool_result(id, "bash", "x".repeat(chars), true)
    }

    #[test]
    fn evidence_order_prunes_routine_results_before_errors() {
        let messages = vec![
            error_result("e0", 4_000),
            tool_result("r0", 4_000),
            error_result("e1", 4_000),
            tool_result("r1", 4_000),
            error_result("e2", 4_000),
            tool_result("r2", 4_000),
        ];
        let settings = PruneSettings {
            keep_recent: 0,
            order: PruneOrder::ErrorsLast,
            ..Default::default()
        };
        let plan = plan_prune(&messages, &HashSet::new(), 12_000, 20_000, &settings);
        assert_eq!(plan, vec!["r0", "r1", "r2", "e0", "e1"]);
    }

    #[test]
    fn default_order_stays_oldest_first() {
        let messages = vec![
            error_result("e0", 4_000),
            tool_result("r0", 4_000),
            error_result("e1", 4_000),
            tool_result("r1", 4_000),
            error_result("e2", 4_000),
            tool_result("r2", 4_000),
        ];
        let settings = PruneSettings {
            keep_recent: 0,
            ..Default::default()
        };
        let plan = plan_prune(&messages, &HashSet::new(), 12_000, 20_000, &settings);
        assert_eq!(plan, vec!["e0", "r0", "e1", "r1", "e2"]);
    }

    #[test]
    fn evidence_route_is_errors_last_and_default_is_unchanged() {
        assert_eq!(PruneSettings::default().order, PruneOrder::OldestFirst);
        assert_eq!(
            PruneSettings::evidence_route().order,
            PruneOrder::ErrorsLast
        );
    }

    #[test]
    fn nothing_is_pruned_under_the_start_line() {
        let messages: Vec<_> = (0..12)
            .map(|i| tool_result(&format!("c{i}"), 4_000))
            .collect();
        let tokens = messages
            .iter()
            .map(crate::compaction::estimate_tokens)
            .sum();
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
