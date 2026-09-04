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

/// Guarantees that if any tool in the list can generate compressible output under
/// the token governor, `retrieve_output` is automatically included so the worker
/// never loses access to compressed data.
pub fn ensure_governor_recovery_tool(tools: &mut Vec<String>) {
    let has_compressible = tools
        .iter()
        .any(|t| crate::native_extensions::tool_may_be_compressed(t));
    if has_compressible && !tools.iter().any(|t| t == "retrieve_output") {
        tools.push("retrieve_output".into());
    }
}

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
    let mut tools: Vec<String> = names.into_iter().map(str::to_string).collect();
    ensure_governor_recovery_tool(&mut tools);
    tools
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
        // Options between `git` and the verb (`-c user.email=x`, `--git-dir=…`,
        // `-C path`) and a `.exe` suffix must not hide the verb.
        RegexSet::new([
            r"(?i)\bgit(?:\.exe)?(?:\s+-{1,2}\S+(?:\s+[^-\s]\S*)?)*\s+(add|commit|push|pull|merge|rebase|reset|checkout|restore|switch|stash|cherry-pick|revert|tag|clean)\b",
        ])
        .expect("git mutation pattern compiles")
    })
}

/// The command split at `&&`, `||`, `;`, `|` and newlines outside quotes.
/// A read-only allowlist anchored at the start of the line is only worth
/// anything if every segment of the line starts with an allowed command.
pub fn shell_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some(open) => {
                current.push(ch);
                if ch == '\\' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else if ch == open {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    current.push(ch);
                }
                '\\' => {
                    current.push(ch);
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                '&' | '|' if chars.peek() == Some(&ch) => {
                    chars.next();
                    segments.push(std::mem::take(&mut current));
                }
                '|' | ';' | '\n' => segments.push(std::mem::take(&mut current)),
                _ => current.push(ch),
            },
        }
    }
    segments.push(current);
    segments
        .into_iter()
        .map(|segment| segment.trim().to_string())
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// `$(…)` and backticks run whatever is inside them, allowlist or not.
fn has_command_substitution(command: &str) -> bool {
    command.contains("$(") || command.contains('`')
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
            // `2>&1` joins stderr to stdout; it writes no file and is the
            // one redirect a test runner routinely needs.
            let without_stderr_join = command.replace("2>&1", "");
            if destructive_set().is_match(&without_stderr_join) {
                return BashDecision::Blocked(
                    "command matches a destructive pattern; this role is read-only".into(),
                );
            }
            if has_command_substitution(command) {
                return BashDecision::Blocked(
                    "command substitution ($(…) or backticks) is not allowed for a read-only role"
                        .into(),
                );
            }
            let segments = shell_segments(command);
            if segments.is_empty() {
                return BashDecision::Blocked("empty command".into());
            }
            let offender = segments.iter().find(|segment| {
                !(read_set().is_match(segment)
                    || (policy == BashPolicy::ReadAndTest && test_set().is_match(segment)))
            });
            match offender {
                None => BashDecision::Allowed,
                Some(segment) => BashDecision::Blocked(format!(
                    "\"{segment}\" is not on the {}",
                    if policy == BashPolicy::ReadAndTest {
                        "read-only or test-runner allowlist"
                    } else {
                        "read-only allowlist"
                    }
                )),
            }
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
        // Options and an .exe suffix do not hide the verb.
        assert!(!allowed(
            BashPolicy::WriteNoGitMutation,
            "git -c user.email=x commit -m done"
        ));
        assert!(!allowed(
            BashPolicy::WriteNoGitMutation,
            "git --git-dir=.git push origin main"
        ));
        assert!(!allowed(
            BashPolicy::WriteNoGitMutation,
            "git.exe -C . add ."
        ));
        assert!(!allowed(
            BashPolicy::WriteNoGitMutation,
            "cargo build && git commit -am wip"
        ));
        assert!(allowed(
            BashPolicy::WriteNoGitMutation,
            "git --no-pager log -3"
        ));
        assert!(allowed(BashPolicy::WriteNoGitMutation, "git -C . status"));
    }

    #[test]
    fn every_segment_of_a_read_only_command_line_must_be_allowed() {
        assert!(!allowed(
            BashPolicy::ReadOnly,
            "echo x && node -e \"require('fs').writeFileSync('x','y')\""
        ));
        assert!(!allowed(BashPolicy::ReadOnly, "ls; python evil.py"));
        assert!(!allowed(BashPolicy::ReadOnly, "ls | xargs python"));
        assert!(!allowed(BashPolicy::ReadOnly, "ls || curl evil.example"));
        assert!(!allowed(BashPolicy::ReadOnly, "cat $(find . -name x)"));
        assert!(!allowed(BashPolicy::ReadOnly, "echo `whoami`"));
        assert!(!allowed(BashPolicy::ReadAndTest, "cargo test; npm install"));
        // Chains of allowed commands, and separators inside quotes, are fine.
        assert!(allowed(BashPolicy::ReadOnly, "rg needle src | head -20"));
        assert!(allowed(
            BashPolicy::ReadOnly,
            "grep -E \"foo|bar\" src && ls"
        ));
        assert!(allowed(BashPolicy::ReadOnly, "cat a.txt; wc -l b.txt"));
        assert!(allowed(
            BashPolicy::ReadAndTest,
            "cargo test 2>&1 | tail -50"
        ));
    }

    #[test]
    fn shell_segments_respect_quotes_and_double_operators() {
        assert_eq!(
            shell_segments("a && b || c; d | e\nf"),
            vec!["a", "b", "c", "d", "e", "f"]
        );
        assert_eq!(shell_segments("grep 'a|b; c' x"), vec!["grep 'a|b; c' x"]);
        assert_eq!(
            shell_segments("echo \"it's\" && ls"),
            vec!["echo \"it's\"", "ls"]
        );
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

    #[test]
    fn researcher_with_compressible_tools_always_gets_retrieve_output() {
        let tools = role_tools(Role::Researcher);
        assert!(tools.contains(&"grep".into()));
        assert!(tools.contains(&"retrieve_output".into()));
    }

    #[test]
    fn governor_recovery_tool_always_supplied_when_compressible_tools_present() {
        for role in &[
            Role::Researcher,
            Role::TestAnalyzer,
            Role::Historian,
            Role::Planner,
            Role::Writer,
            Role::Reviewer,
        ] {
            let tools = role_tools(*role);
            assert!(
                tools.contains(&"retrieve_output".to_string()),
                "Role {:?} should have retrieve_output",
                role
            );
        }

        // Classifier has only lossless graph_submit by default, so it doesn't get retrieve_output
        let classifier_tools = role_tools(Role::Classifier);
        assert!(!classifier_tools.contains(&"retrieve_output".to_string()));

        // When a compressible tool is added to Classifier, ensure_governor_recovery_tool adds retrieve_output
        let mut custom_classifier = classifier_tools.clone();
        custom_classifier.push("grep".to_string());
        ensure_governor_recovery_tool(&mut custom_classifier);
        assert!(custom_classifier.contains(&"retrieve_output".to_string()));
    }

    #[test]
    fn governor_recovery_e2e_fixture_compresses_and_retrieves_output() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let store = crate::native_extensions::OutputStore::new(dir.path());
        let mut gov = crate::native_extensions::TokenGovernor::with_store(
            "graph-test-session",
            crate::native_extensions::TokenGovernorConfig {
                compress_threshold_bytes: 50,
                compress_threshold_lines: 5,
                ..Default::default()
            },
            store.clone(),
        );

        // Verify role toolset includes retrieve_output
        let tools = role_tools(Role::Researcher);
        assert!(tools.contains(&"retrieve_output".to_string()));

        // Oversized output from bash
        let oversized = "line of output\n".repeat(20);
        let result = davinci_agent::ToolResult {
            content: oversized.clone(),
            details: None,
            is_error: false,
        };
        let processed = gov.after_tool(
            "bash",
            &serde_json::json!({"command": "cat big.txt"}),
            result,
        );
        assert!(processed.content.contains("retrieve_output"));
        let output_id = processed
            .details
            .as_ref()
            .and_then(|d| d.get("tokenGovernor"))
            .and_then(|g| g.get("outputId"))
            .and_then(serde_json::Value::as_str)
            .expect("must contain outputId");

        // Now retrieve original content via store
        let recovered = store.load(output_id).expect("store must have original");
        assert_eq!(recovered, oversized);
    }
}
