//! Least privilege per node.
//!
//! Two enforcement layers:
//!  1. `role_tools` feeds the child pi's `--tools` allowlist (which applies to
//!     built-in AND native tools), so a researcher process never even has an
//!     edit or write tool.
//!  2. `role_bash_policy` is enforced inside the child by the graph worker hook
//!     in `worker_hooks.rs`, because "bash" is a single tool whose danger
//!     depends on the command text.
//!
//! Deny-then-allow: a command must match no destructive pattern AND at least
//! one allowed pattern.

use super::types::{BashPolicy, ResearchKind, Role};
use regex::RegexSet;
use std::sync::OnceLock;

const READ_TOOLS: &[&str] = &["read", "grep", "find", "ls"];

pub const GRAPH_SUBMIT_TOOL: &str = "graph_submit";

pub fn role_tools(role: Role) -> Vec<String> {
    let names: Vec<&str> = match role {
        Role::Classifier => vec![GRAPH_SUBMIT_TOOL],
        Role::Researcher | Role::TestAnalyzer | Role::Reviewer => {
            let mut tools = READ_TOOLS.to_vec();
            tools.push("bash");
            tools.push(GRAPH_SUBMIT_TOOL);
            tools
        }
        Role::Historian => vec!["read", "grep", "bash", GRAPH_SUBMIT_TOOL],
        Role::Planner => {
            let mut tools = READ_TOOLS.to_vec();
            tools.push(GRAPH_SUBMIT_TOOL);
            tools
        }
        Role::Writer => {
            let mut tools = READ_TOOLS.to_vec();
            tools.extend_from_slice(&["bash", "edit", "write", GRAPH_SUBMIT_TOOL]);
            tools
        }
    };
    names.into_iter().map(str::to_string).collect()
}

pub fn role_bash_policy(role: Role) -> BashPolicy {
    match role {
        Role::Classifier | Role::Planner => BashPolicy::None,
        Role::Researcher | Role::Historian => BashPolicy::ReadOnly,
        Role::TestAnalyzer | Role::Reviewer => BashPolicy::ReadAndTest,
        Role::Writer => BashPolicy::WriteNoGitMutation,
    }
}

pub fn role_for_research_kind(kind: ResearchKind) -> Role {
    match kind {
        ResearchKind::TestBaseline => Role::TestAnalyzer,
        ResearchKind::History => Role::Historian,
        ResearchKind::CodeSearch | ResearchKind::Docs => Role::Researcher,
    }
}

const DESTRUCTIVE_PATTERNS: &[&str] = &[
    r"(?i)\brm\b",
    r"(?i)\brmdir\b",
    r"(?i)\bmv\b",
    r"(?i)\bcp\b",
    r"(?i)\bmkdir\b",
    r"(?i)\btouch\b",
    r"(?i)\bchmod\b",
    r"(?i)\bchown\b",
    r"(?i)\bln\b",
    r"(?i)\btee\b",
    r"(?i)\btruncate\b",
    r"(?i)\bdd\b",
    // A single `>` redirect that is not part of `>>`, and `>>` itself.
    r"(^|[^<>])>[^>]",
    r"(^|[^<>])>$",
    r">>",
    r"(?i)\bnpm\s+(install|uninstall|update|ci|link|publish)\b",
    r"(?i)\byarn\s+(add|remove|install|publish)\b",
    r"(?i)\bpnpm\s+(add|remove|install|publish)\b",
    r"(?i)\bpip\s+(install|uninstall)\b",
    r"(?i)\bcargo\s+(install|publish|add|remove|clean)\b",
    r"(?i)\bgit\s+(add|commit|push|pull|merge|rebase|reset|checkout|restore|switch|stash|cherry-pick|revert|tag|init|clone|clean)\b",
    r"(?i)\bsudo\b",
    r"(?i)\bkill\b",
    r"(?i)\bpkill\b",
    r"(?i)\bshutdown\b",
    r"(?i)\bSet-Content\b",
    r"(?i)\bOut-File\b",
    r"(?i)\bRemove-Item\b",
    r"(?i)\bNew-Item\b",
];

const READ_PATTERNS: &[&str] = &[
    r"^\s*cat\b",
    r"^\s*head\b",
    r"^\s*tail\b",
    r"^\s*grep\b",
    r"^\s*rg\b",
    r"^\s*find\b",
    r"^\s*fd\b",
    r"^\s*ls\b",
    r"(?i)^\s*dir\b",
    r"^\s*pwd\b",
    r"^\s*echo\b",
    r"^\s*wc\b",
    r"^\s*sort\b",
    r"^\s*uniq\b",
    r"^\s*diff\b",
    r"^\s*stat\b",
    r"^\s*tree\b",
    r"^\s*which\b",
    r"(?i)^\s*where\b",
    r"^\s*jq\b",
    r"(?i)^\s*sed\s+-n\b",
    r"^\s*awk\b",
    r"(?i)^\s*node\s+--version\b",
    r"(?i)^\s*rustc\s+--version\b",
    r"(?i)^\s*cargo\s+(tree|metadata)\b",
    r"(?i)^\s*git\s+(status|log|diff|show|blame|branch|remote|ls-files|ls-tree|rev-parse|describe|shortlog)\b",
    r"(?i)^\s*npm\s+(ls|list|view|info|explain)\b",
    r"(?i)^\s*Get-(Content|ChildItem|Item|Location)\b",
    r"(?i)^\s*Select-String\b",
];

const TEST_PATTERNS: &[&str] = &[
    r"(?i)^\s*(npx\s+)?vitest\b",
    r"(?i)^\s*(npx\s+)?jest\b",
    r"(?i)^\s*(npx\s+)?mocha\b",
    r"(?i)^\s*(npx\s+)?playwright\s+test\b",
    r"(?i)^\s*(npx\s+)?tsc\b",
    r"(?i)^\s*(npx\s+)?tsgo\b",
    r"(?i)^\s*(npx\s+)?eslint\b",
    r"(?i)^\s*(npx\s+)?biome\s+(check|lint)\b",
    r"(?i)^\s*npm\s+(test|run\s+(test|tests|check|typecheck|lint|build))\b",
    r"(?i)^\s*yarn\s+(test|check|typecheck|lint)\b",
    r"(?i)^\s*pnpm\s+(test|check|typecheck|lint)\b",
    r"(?i)^\s*node\s+.*vitest[/\\]dist[/\\]cli\.js\b",
    r"(?i)^\s*node\s+--test\b",
    r"(?i)^\s*(python|pytest|cargo\s+(test|check|clippy|fmt|build|nextest)|go\s+(test|vet|build)|dotnet\s+(test|build))\b",
    r"^\s*make\s+(test|check|lint|fmt|clippy|build)\b",
    r"^\s*\.[/\\]test\.sh\b",
];

fn destructive_set() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| RegexSet::new(DESTRUCTIVE_PATTERNS).expect("destructive patterns compile"))
}

fn read_set() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| RegexSet::new(READ_PATTERNS).expect("read patterns compile"))
}

fn test_set() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| RegexSet::new(TEST_PATTERNS).expect("test patterns compile"))
}

fn git_mutation_set() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| {
        RegexSet::new([
            r"(?i)\bgit\s+(add|commit|push|pull|merge|rebase|reset|checkout|restore|switch|stash|cherry-pick|revert|tag|clean)\b",
        ])
        .expect("git mutation pattern compiles")
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BashDecision {
    Allowed,
    Blocked(String),
}

pub fn is_bash_command_allowed(policy: BashPolicy, command: &str) -> BashDecision {
    match policy {
        BashPolicy::None => BashDecision::Blocked("this role has no shell access".into()),
        BashPolicy::WriteNoGitMutation => {
            if git_mutation_set().is_match(command) {
                BashDecision::Blocked(
                    "git state changes are reserved for the human operator; the graph never commits"
                        .into(),
                )
            } else {
                BashDecision::Allowed
            }
        }
        BashPolicy::ReadOnly | BashPolicy::ReadAndTest => {
            if destructive_set().is_match(command) {
                return BashDecision::Blocked(
                    "command matches a destructive pattern; this role is read-only".into(),
                );
            }
            if read_set().is_match(command) {
                return BashDecision::Allowed;
            }
            if policy == BashPolicy::ReadAndTest && test_set().is_match(command) {
                return BashDecision::Allowed;
            }
            BashDecision::Blocked(
                if policy == BashPolicy::ReadAndTest {
                    "command is not on the read-only or test-runner allowlist"
                } else {
                    "command is not on the read-only allowlist"
                }
                .into(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(policy: BashPolicy, command: &str) -> bool {
        is_bash_command_allowed(policy, command) == BashDecision::Allowed
    }

    #[test]
    fn only_the_writer_may_mutate_files() {
        assert!(role_tools(Role::Writer).iter().any(|tool| tool == "write"));
        for role in Role::ALL.iter().filter(|role| **role != Role::Writer) {
            let tools = role_tools(*role);
            assert!(!tools.iter().any(|tool| tool == "write" || tool == "edit"));
        }
    }

    #[test]
    fn every_role_can_submit_its_artifact() {
        for role in Role::ALL {
            assert!(role_tools(*role)
                .iter()
                .any(|tool| tool == GRAPH_SUBMIT_TOOL));
        }
    }

    #[test]
    fn read_only_roles_reject_mutation_and_accept_inspection() {
        assert!(allowed(BashPolicy::ReadOnly, "git log -5"));
        assert!(allowed(BashPolicy::ReadOnly, "rg needle src"));
        assert!(!allowed(BashPolicy::ReadOnly, "rm -rf /"));
        assert!(!allowed(BashPolicy::ReadOnly, "echo hi > file"));
        assert!(!allowed(BashPolicy::ReadOnly, "cargo test"));
        assert!(allowed(BashPolicy::ReadAndTest, "cargo test"));
        assert!(allowed(BashPolicy::ReadAndTest, "cargo clippy"));
    }

    #[test]
    fn the_writer_may_build_but_never_change_git_state() {
        assert!(allowed(BashPolicy::WriteNoGitMutation, "cargo build"));
        assert!(allowed(
            BashPolicy::WriteNoGitMutation,
            "echo hello > out.txt"
        ));
        assert!(!allowed(
            BashPolicy::WriteNoGitMutation,
            "git commit -m done"
        ));
    }

    #[test]
    fn roles_without_a_shell_are_refused_outright() {
        assert!(!allowed(BashPolicy::None, "ls"));
        assert_eq!(role_bash_policy(Role::Classifier), BashPolicy::None);
        assert_eq!(role_bash_policy(Role::Planner), BashPolicy::None);
    }

    #[test]
    fn research_kinds_map_to_the_role_that_can_answer_them() {
        assert_eq!(
            role_for_research_kind(ResearchKind::TestBaseline),
            Role::TestAnalyzer
        );
        assert_eq!(
            role_for_research_kind(ResearchKind::History),
            Role::Historian
        );
        assert_eq!(
            role_for_research_kind(ResearchKind::CodeSearch),
            Role::Researcher
        );
    }
}
