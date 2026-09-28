//! The user's say over delegation: "don't use subagents" holds until they
//! say otherwise.
//!
//! No TypeScript counterpart. Claude Code relies on the model following such
//! an instruction; this is the deterministic backstop for the same promise.
//! The prompt still carries the rule (`TOOL_USE_STRATEGY`); this module makes
//! a forgotten or compacted instruction unable to spawn workers anyway.
//!
//! The detector is deliberately narrow. It fires only on an explicit
//! directive about delegation ("don't use subagents", "no agents please",
//! "without subagents", "do it yourself, no delegation") and the explicit
//! reversal ("you can use subagents again"). Text that merely mentions
//! agents ("don't modify the agents directory", "read AGENTS.md") is not a
//! directive.

use regex::Regex;
use std::sync::OnceLock;

/// What a user message says about delegation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationDirective {
    /// Do not start subagents, teammates or workflows.
    Forbid,
    /// Delegation is allowed again.
    Allow,
}

/// Nouns that mean "a delegated worker".
const WORKER: &str = r"(?:sub[- ]?agents?|(?:parallel |background |other )?agents?|workers?|teammates?|agent teams?|(?:dynamic )?workflows?|delegation)";
/// Verbs that mean "start one".
const START: &str = r"(?:use|using|spawn|spawning|launch|launching|start|starting|run|running|call|calling|create|creating|dispatch|dispatching|fan out to|fanning out to|delegate to|delegating to)";
/// What may follow the worker noun in a directive: the end of the clause or
/// a clause word. "don't run the agent loop" and "no agent profile found"
/// are about something else and must not match.
const TAIL: &str = r"(?:\s*(?:[.,!;:)]|$)|\s+(?:please|for|to|in|on|here|this|at all|and|or|again|now|instead|anymore|any more|if|when|so|just|unless)\b)";

fn forbid_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            // "don't use subagents", "do not spawn any agents", "never launch workers"
            format!(r"\b(?:don'?t|do not|dont|never|stop|avoid|no need to)\s+{START}\s+(?:any\s+|a\s+|the\s+|more\s+)?{WORKER}{TAIL}"),
            // "no subagents", "no agents please", "no delegation"
            format!(r"\bno\s+(?:more\s+)?{WORKER}{TAIL}"),
            // "without subagents", "without using agents"
            format!(r"\bwithout\s+(?:using\s+|any\s+)*{WORKER}{TAIL}"),
            // "don't delegate", "do not delegate this"
            r"\b(?:don'?t|do not|dont|never)\s+delegate\b".to_string(),
        ]
        .iter()
        .map(|pattern| Regex::new(&format!("(?i){pattern}")).expect("valid forbid pattern"))
        .collect()
    })
}

fn allow_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            // "you can use subagents (again)", "feel free to spawn agents"
            format!(r"\b(?:you can|you may|feel free to|go ahead and|it'?s ok(?:ay)? to|ok(?:ay)? to|allowed to|now)\s+{START}\s+(?:some\s+|a few\s+|the\s+)?{WORKER}{TAIL}"),
            // "use subagents", "spawn agents for this" as an instruction at the start of a sentence
            format!(r"(?:^|[.!?\n]\s*)(?:please\s+)?(?:use|spawn|launch|start)\s+(?:some\s+|a few\s+|parallel\s+)?{WORKER}{TAIL}"),
            // "subagents are fine/ok/allowed again"
            format!(r"\b{WORKER}\s+(?:are|is)\s+(?:fine|ok(?:ay)?|allowed)\b"),
        ]
        .iter()
        .map(|pattern| Regex::new(&format!("(?i){pattern}")).expect("valid allow pattern"))
        .collect()
    })
}

/// A mention like "AGENTS.md", "workflows.rs" or "agents/" is a path, not a
/// worker. The match may already include the `.` (the clause tail accepts
/// punctuation), so both shapes are checked.
fn is_path_mention(text: &str, end: usize) -> bool {
    let matched = &text[..end];
    let rest = &text[end..];
    let alnum_next = |s: &str| s.starts_with(|c: char| c.is_ascii_alphanumeric());
    (matched.ends_with('.') && alnum_next(rest))
        || (rest.starts_with('.') && alnum_next(&rest[1..]))
        || rest.starts_with('/')
        || rest.starts_with('\\')
}

/// The last delegation directive in `text`, if it states one.
pub fn delegation_directive(text: &str) -> Option<DelegationDirective> {
    let mut last: Option<(usize, DelegationDirective)> = None;
    let mut consider = |patterns: &[Regex], directive: DelegationDirective| {
        for pattern in patterns {
            for found in pattern.find_iter(text) {
                if is_path_mention(text, found.end()) {
                    continue;
                }
                if last.is_none_or(|(at, _)| found.start() >= at) {
                    last = Some((found.start(), directive));
                }
            }
        }
    };
    consider(forbid_patterns(), DelegationDirective::Forbid);
    consider(allow_patterns(), DelegationDirective::Allow);
    last.map(|(_, directive)| directive)
}

/// Apply the user's messages in order: the latest directive wins, and a
/// message without one leaves the policy as it was.
pub fn delegation_forbidden_after<'a>(
    initial: bool,
    user_texts: impl IntoIterator<Item = &'a str>,
) -> bool {
    user_texts.into_iter().fold(initial, |forbidden, text| {
        match delegation_directive(text) {
            Some(DelegationDirective::Forbid) => true,
            Some(DelegationDirective::Allow) => false,
            None => forbidden,
        }
    })
}

/// The tool result a delegation call gets while the user has forbidden it.
pub const DELEGATION_FORBIDDEN_MESSAGE: &str = "The user asked not to use subagents in this conversation, so this call was not run. Do the work directly with your own tools. If delegation seems necessary, say why and ask the user first.";

/// Tools that start delegated work.
pub fn is_delegation_tool(name: &str) -> bool {
    matches!(name, "agent" | "workflow_run")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_refusals_forbid() {
        for text in [
            "do not use subagents",
            "Don't use sub-agents for this, just read the files",
            "review this and do not write code. do not use subagents",
            "never spawn agents in this repo",
            "No subagents please",
            "fix it without subagents",
            "do it yourself without using any agents",
            "please don't delegate this",
            "stop using workers",
            "dont launch any teammates",
            "no delegation, I want you to do it",
            "don't run a workflow for this",
        ] {
            assert_eq!(
                delegation_directive(text),
                Some(DelegationDirective::Forbid),
                "{text}"
            );
        }
    }

    #[test]
    fn explicit_permission_allows() {
        for text in [
            "you can use subagents again",
            "Feel free to spawn agents for the research",
            "go ahead and launch parallel agents",
            "Use subagents to map the crate.",
            "ok. spawn agents for each module",
            "subagents are fine now",
        ] {
            assert_eq!(
                delegation_directive(text),
                Some(DelegationDirective::Allow),
                "{text}"
            );
        }
    }

    #[test]
    fn mentions_are_not_directives() {
        for text in [
            "don't modify the agents directory",
            "read AGENTS.md and summarize it",
            "update crates/davinci-agent/src/subagent.rs",
            "how good is the agent orchestration?",
            "the workers crashed yesterday, find out why",
            "don't use unwrap in the agent loop",
            "no, the agent profile is wrong",
            "list files in agents/",
            "don't run the agent loop twice",
            "no agent profile found for reviewer",
            "you can use the agent profile named scoped",
            "use workflows.rs as the reference",
        ] {
            assert_eq!(delegation_directive(text), None, "{text}");
        }
    }

    #[test]
    fn the_later_directive_in_a_message_wins() {
        assert_eq!(
            delegation_directive("earlier I said no subagents, but now you can use subagents"),
            Some(DelegationDirective::Allow)
        );
        assert_eq!(
            delegation_directive("use subagents for search. Actually, don't use subagents."),
            Some(DelegationDirective::Forbid)
        );
    }

    #[test]
    fn a_conversation_keeps_the_policy_until_reversed() {
        assert!(delegation_forbidden_after(
            false,
            ["do not use subagents", "now fix the bug", "run the tests"]
        ));
        assert!(!delegation_forbidden_after(
            false,
            ["do not use subagents", "ok you can use subagents again"]
        ));
        assert!(!delegation_forbidden_after(false, ["refactor this"]));
    }

    #[test]
    fn only_delegation_tools_are_gated() {
        assert!(is_delegation_tool("agent"));
        assert!(is_delegation_tool("workflow_run"));
        assert!(!is_delegation_tool("agent_message"));
        assert!(!is_delegation_tool("read"));
    }
}
