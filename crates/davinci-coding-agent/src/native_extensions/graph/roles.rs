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

const READ_TOOLS: &[&str] = &[
    "read",
    "grep",
    "find",
    "ls",
    "repo_map",
    "symbol_search",
    "file_symbols",
    "file_dependencies",
    "symbol_relationships",
    "related_files",
    "code_query",
];

pub const GRAPH_SUBMIT_TOOL: &str = "graph_submit";

/// Sessionless graph subprocesses have no parent task-coordinator transport.
/// Reads must use that authority too, rather than an unrelated local registry.
pub(super) fn requires_task_coordinator(tool: &str) -> bool {
    matches!(
        tool,
        "task_create" | "task_update" | "task_list" | "task_get"
    ) || crate::native_extensions::language_intelligence::TOOL_NAMES.contains(&tool)
        || davinci_agent::tools::is_managed_process_tool(tool)
}

pub use crate::native_extensions::token_governor::ensure_governor_recovery_tool;

pub fn role_tools(role: Role) -> Vec<String> {
    let names: Vec<&str> = match role {
        Role::Classifier => vec![GRAPH_SUBMIT_TOOL, "repo_map", "code_query"],
        Role::Researcher | Role::TestAnalyzer | Role::Reviewer => {
            let mut tools = READ_TOOLS.to_vec();
            tools.push("bash");
            tools.push(GRAPH_SUBMIT_TOOL);
            tools.push("tool_search");
            tools
        }
        Role::Historian => vec!["read", "grep", "bash", GRAPH_SUBMIT_TOOL, "tool_search"],
        Role::Planner => {
            let mut tools = READ_TOOLS.to_vec();
            tools.push(GRAPH_SUBMIT_TOOL);
            tools.push("tool_search");
            tools
        }
        Role::Writer => {
            let mut tools = READ_TOOLS.to_vec();
            tools.extend_from_slice(&["bash", "edit", "write", GRAPH_SUBMIT_TOOL, "tool_search"]);
            tools.extend_from_slice(&[
                "patch_preview",
                "patch_apply",
                "patch_status",
                "patch_rollback",
            ]);
            tools
        }
    };
    let mut tools: Vec<String> = names.into_iter().map(str::to_string).collect();
    let semantic: &[&str] = match role {
        Role::Researcher | Role::Planner | Role::Writer => {
            crate::native_extensions::language_intelligence::TOOL_NAMES
        }
        Role::Reviewer => &["lsp_references", "lsp_implementations", "lsp_diagnostics"],
        Role::TestAnalyzer => &["lsp_diagnostics"],
        Role::Classifier | Role::Historian => &[],
    };
    tools.extend(semantic.iter().map(|name| (*name).to_string()));
    if matches!(
        role,
        Role::Planner | Role::TestAnalyzer | Role::Writer | Role::Reviewer
    ) {
        tools.extend(
            crate::native_extensions::test_impact::TOOL_NAMES
                .iter()
                .map(|name| (*name).to_string()),
        );
    }
    if matches!(role, Role::Writer | Role::TestAnalyzer) {
        tools.extend(
            [
                "process_start",
                "process_status",
                "process_output",
                "process_stop",
                "process_list",
            ]
            .map(str::to_string),
        );
    }
    ensure_governor_recovery_tool(&mut tools);
    tools
}

/// Return the bounded hot schema set for a worker while preserving the full
/// authorization surface in the parent-owned allowlist.
pub fn initial_worker_tools(role: Role, authorized: &[String]) -> Vec<String> {
    let preferred: &[&str] = match role {
        Role::Classifier => &[
            GRAPH_SUBMIT_TOOL,
            "repo_map",
            "code_query",
            "retrieve_output",
        ],
        Role::Researcher | Role::TestAnalyzer | Role::Reviewer => &[
            "read",
            "grep",
            "find",
            "bash",
            GRAPH_SUBMIT_TOOL,
            "tool_search",
            "retrieve_output",
        ],
        Role::Historian => &["read", "grep", "bash", GRAPH_SUBMIT_TOOL, "tool_search"],
        Role::Planner => &[
            "read",
            "grep",
            "find",
            "ls",
            GRAPH_SUBMIT_TOOL,
            "tool_search",
        ],
        Role::Writer => &[
            "read",
            "grep",
            "find",
            "ls",
            "bash",
            "edit",
            "write",
            GRAPH_SUBMIT_TOOL,
            "tool_search",
            "retrieve_output",
        ],
    };
    preferred
        .iter()
        .filter(|tool| authorized.iter().any(|candidate| candidate == **tool))
        .map(|tool| (*tool).to_string())
        .collect()
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

#[allow(dead_code)]
pub fn shell_segments(command: &str) -> Vec<String> {
    davinci_agent::shell_policy::split_shell_segments(command)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BashDecision {
    Allowed,
    Blocked(String),
}

pub fn is_bash_command_allowed(policy: BashPolicy, command: &str) -> BashDecision {
    if command.trim().is_empty() {
        return BashDecision::Blocked("empty command".into());
    }
    match davinci_agent::shell_policy::evaluate(shell_profile(policy), command) {
        davinci_agent::shell_policy::ShellCommandDecision::Allowed => BashDecision::Allowed,
        davinci_agent::shell_policy::ShellCommandDecision::Denied { reason }
        | davinci_agent::shell_policy::ShellCommandDecision::NeedsApproval { reason } => {
            BashDecision::Blocked(reason)
        }
    }
}

pub(super) fn shell_profile(policy: BashPolicy) -> davinci_agent::shell_policy::ShellPolicyProfile {
    match policy {
        BashPolicy::None => davinci_agent::shell_policy::ShellPolicyProfile::None,
        BashPolicy::ReadOnly => davinci_agent::shell_policy::ShellPolicyProfile::ReadOnly,
        BashPolicy::ReadAndTest => davinci_agent::shell_policy::ShellPolicyProfile::ReadAndTest,
        BashPolicy::WriteNoGitMutation => {
            davinci_agent::shell_policy::ShellPolicyProfile::WriteNoGitMutation
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_tools_require_parent_authority_and_follow_role_selection() {
        for name in crate::native_extensions::language_intelligence::TOOL_NAMES {
            assert!(requires_task_coordinator(name));
            assert!(role_tools(Role::Researcher).contains(&name.to_string()));
            assert!(!role_tools(Role::Classifier).contains(&name.to_string()));
        }
        assert!(!role_tools(Role::Reviewer).contains(&"lsp_hover".into()));
        assert!(role_tools(Role::TestAnalyzer).contains(&"lsp_diagnostics".into()));
        assert!(!requires_task_coordinator("read"));
    }

    fn allowed(policy: BashPolicy, command: &str) -> bool {
        is_bash_command_allowed(policy, command) == BashDecision::Allowed
    }

    #[test]
    fn only_the_writer_may_mutate_files() {
        assert!(role_tools(Role::Writer).iter().any(|tool| tool == "write"));
        for name in [
            "patch_preview",
            "patch_apply",
            "patch_status",
            "patch_rollback",
        ] {
            assert!(role_tools(Role::Writer).contains(&name.to_string()));
            for role in Role::ALL.iter().filter(|role| **role != Role::Writer) {
                assert!(!role_tools(*role).contains(&name.to_string()));
            }
        }
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
    fn graph_worker_defers_authorized_schemas_without_changing_permissions() {
        for role in [
            Role::Researcher,
            Role::TestAnalyzer,
            Role::Reviewer,
            Role::Historian,
            Role::Planner,
            Role::Writer,
        ] {
            let mut authorized = role_tools(role);
            authorized.extend([
                "mcp.catalog.search".to_string(),
                "mcp.catalog.write".to_string(),
            ]);
            let initial = initial_worker_tools(role, &authorized);

            assert!(authorized.contains(&"mcp.catalog.search".to_string()));
            assert!(!initial.contains(&"mcp.catalog.search".to_string()));
            assert!(initial.contains(&"tool_search".to_string()));
            assert!(initial.contains(&GRAPH_SUBMIT_TOOL.to_string()));
        }

        assert!(!role_tools(Role::Classifier).contains(&"tool_search".to_string()));
    }

    #[test]
    fn test_impact_role_projection_keeps_discovery_and_output_recovery() {
        for role in [
            Role::Planner,
            Role::TestAnalyzer,
            Role::Writer,
            Role::Reviewer,
        ] {
            let authorized = role_tools(role);
            let initial = initial_worker_tools(role, &authorized);
            for name in ["test_related", "test_impacted", "test_plan"] {
                assert!(authorized.contains(&name.into()));
                assert!(!initial.contains(&name.into()));
            }
            assert!(authorized.contains(&"retrieve_output".into()));
            assert!(initial.contains(&"tool_search".into()));
            assert!(!authorized.iter().any(|name| name.starts_with("graph_test")));
            let restricted = initial_worker_tools(role, &["tool_search".into()]);
            assert_eq!(restricted, ["tool_search"]);
        }
        assert!(!role_tools(Role::Classifier).contains(&"test_plan".into()));
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

        // Classifier structural queries can be compacted and require recovery.
        let classifier_tools = role_tools(Role::Classifier);
        assert!(classifier_tools.contains(&"repo_map".to_string()));
        assert!(classifier_tools.contains(&"code_query".to_string()));
        assert!(classifier_tools.contains(&"retrieve_output".to_string()));

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
