//! Centralized shell command normalization and risk analyzer.
//!
//! Provides conservative cross-platform parsing, risk classification, and
//! policy enforcement for Bash and PowerShell commands.
//! Used by:
//! - `permission.rs` (turn permission checks)
//! - Graph `worker_hooks.rs` and `roles.rs` (least-privilege worker sandboxes)
//! - Runtime agent and task isolation policies

use regex::RegexSet;
use std::sync::OnceLock;

/// Policy profile governing allowed shell capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellPolicyProfile {
    /// No shell access allowed.
    None,
    /// Only safe read-only commands allowed; no filesystem mutations, package installs, or redirections.
    ReadOnly,
    /// Safe read-only commands plus authorized test and lint runners.
    ReadAndTest,
    /// Allows general workspace mutations, but strictly blocks git state mutations (commit, push, checkout, etc.).
    WriteNoGitMutation,
    /// Permissive policy (mode auto/ask without graph restrictions).
    Permissive,
}

/// Category of security or execution risk identified in a shell command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellRiskCategory {
    DestructiveFs,
    PrivilegeEscalation,
    PackageInstall,
    GitMutation,
    CommandSubstitution,
    NestedShell,
    Redirection,
    UnknownSyntax,
}

impl std::fmt::Display for ShellRiskCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DestructiveFs => write!(f, "destructive filesystem operation"),
            Self::PrivilegeEscalation => write!(f, "privilege escalation"),
            Self::PackageInstall => write!(f, "package manager / environment modification"),
            Self::GitMutation => write!(f, "git state mutation"),
            Self::CommandSubstitution => write!(f, "command substitution"),
            Self::NestedShell => write!(f, "nested shell execution"),
            Self::Redirection => write!(f, "file output redirection"),
            Self::UnknownSyntax => write!(f, "unknown or unparseable shell syntax"),
        }
    }
}

/// Comprehensive analysis report for a shell command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellAnalysisReport {
    pub command: String,
    pub segments: Vec<String>,
    pub has_substitution: bool,
    pub has_nested_shell: bool,
    pub has_redirection: bool,
    pub has_unknown_syntax: bool,
    pub risks: Vec<(ShellRiskCategory, String)>,
    pub is_read_only: bool,
    pub is_test: bool,
}

/// Policy evaluation outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellCommandDecision {
    Allowed,
    NeedsApproval { reason: String },
    Denied { reason: String },
}

// ----------------------------------------------------------------------------
// Regex sets for patterns
// ----------------------------------------------------------------------------

const DESTRUCTIVE_PATTERNS: &[&str] = &[
    r"(?i)\brm\b",
    r"(?i)\brmdir\b",
    r"(?i)\bmv\b",
    r"(?i)\bcp\b",
    r"(?i)\bmkdir\b",
    r"(?i)\btouch\b",
    r"(?i)\bchmod\b",
    r"(?i)\bchown\b",
    r"(?i)\bchgrp\b",
    r"(?i)\bln\b",
    r"(?i)\btee\b",
    r"(?i)\btruncate\b",
    r"(?i)\bdd\b",
    r"(?i)\bshred\b",
    r"(?i)\bwipe\b",
    // Windows / PowerShell destructive operations
    r"(?i)\bdel\b",
    r"(?i)\berase\b",
    r"(?i)\brd\b",
    r"(?i)\bren\b",
    r"(?i)\brename\b",
    r"(?i)\bmove\b",
    r"(?i)\bcopy\b",
    r"(?i)\bRemove-Item\b",
    r"(?i)\bNew-Item\b",
    r"(?i)\bCopy-Item\b",
    r"(?i)\bMove-Item\b",
    r"(?i)\bClear-Content\b",
    r"(?i)\bSet-Content\b",
    r"(?i)\bOut-File\b",
    r"(?i)\bAdd-Content\b",
    // Process termination
    r"(?i)\bkill\b",
    r"(?i)\bpkill\b",
    r"(?i)\bkillall\b",
    r"(?i)\bStop-Process\b",
    r"(?i)\btaskkill\b",
    r"(?i)\bshutdown\b",
];

const PRIVILEGE_PATTERNS: &[&str] = &[
    r"(?i)\bsudo\b",
    r"(?i)\bdoas\b",
    r"(?i)\bsu\b",
    r"(?i)\brunas\b",
    r"(?i)\bgsudo\b",
    r"(?i)\bpkexec\b",
    r"(?i)\bpbrun\b",
];

const PACKAGE_PATTERNS: &[&str] = &[
    r"(?i)\bnpm\s+(install|i|uninstall|update|ci|link|publish)\b",
    r"(?i)\byarn\s+(add|remove|install|publish)\b",
    r"(?i)\bpnpm\s+(add|remove|install|publish)\b",
    r"(?i)\bpip\s+(install|uninstall)\b",
    r"(?i)\bcargo\s+(install|publish|add|rm|remove|clean)\b",
    r"(?i)\bgem\s+(install|uninstall)\b",
    r"(?i)\bbrew\s+(install|uninstall|upgrade)\b",
    r"(?i)\bchoco\s+(install|uninstall)\b",
    r"(?i)\bwinget\s+(install|uninstall)\b",
    r"(?i)\bapt(?:-get)?\s+(install|remove|purge|upgrade)\b",
];

const GIT_MUTATION_PATTERNS: &[&str] = &[
    r"(?i)\bgit(?:\.exe)?(?:\s+-{1,2}\S+(?:\s+[^-\s]\S*)?)*\s+(add|commit|push|pull|merge|rebase|reset|checkout|restore|switch|stash|cherry-pick|revert|tag|clean|init|clone)\b",
];

const NESTED_SHELL_PATTERNS: &[&str] = &[
    r"(?i)\b(?:bash|sh|zsh|dash|ksh)(?:\.exe)?\s+-c\b",
    r"(?i)\b(?:powershell|pwsh)(?:\.exe)?\s+(?:-[c|C]|-[c|C]ommand|-[e|E]|-[e|E]ncodedCommand|-[f|F]ile)\b",
    r"(?i)\bcmd(?:\.exe)?\s+/[cC]\b",
    r"(?i)\b(?:eval|exec)\b",
];

const READ_PATTERNS: &[&str] = &[
    r"(?i)^\s*cat\b",
    r"(?i)^\s*head\b",
    r"(?i)^\s*tail\b",
    r"(?i)^\s*grep\b",
    r"(?i)^\s*rg\b",
    r"(?i)^\s*find\b",
    r"(?i)^\s*fd\b",
    r"(?i)^\s*ls\b",
    r"(?i)^\s*dir\b",
    r"(?i)^\s*pwd\b",
    r"(?i)^\s*echo\b",
    r"(?i)^\s*wc\b",
    r"(?i)^\s*sort\b",
    r"(?i)^\s*uniq\b",
    r"(?i)^\s*diff\b",
    r"(?i)^\s*stat\b",
    r"(?i)^\s*tree\b",
    r"(?i)^\s*which\b",
    r"(?i)^\s*where(?:\.exe)?\b",
    r"(?i)^\s*type\b",
    r"(?i)^\s*jq\b",
    r"(?i)^\s*sed\s+-n\b",
    r"(?i)^\s*awk\b",
    r"(?i)^\s*node\s+--version\b",
    r"(?i)^\s*rustc\s+--version\b",
    r"(?i)^\s*cargo\s+(tree|metadata)\b",
    r"(?i)^\s*git(?:\.exe)?(?:\s+-{1,2}\S+(?:\s+[^-\s]\S*)?)*\s+(status|log|diff|show|blame|branch|remote|ls-files|ls-tree|rev-parse|describe|shortlog)\b",
    r"(?i)^\s*npm\s+(ls|list|view|info|explain)\b",
    r"(?i)^\s*Get-(Content|ChildItem|Item|Location|Process|Command)\b",
    r"(?i)^\s*Select-String\b",
];

const TEST_PATTERNS: &[&str] = &[
    r"(?i)^\s*(?:npx\s+)?vitest\b",
    r"(?i)^\s*(?:npx\s+)?jest\b",
    r"(?i)^\s*(?:npx\s+)?mocha\b",
    r"(?i)^\s*(?:npx\s+)?playwright\s+test\b",
    r"(?i)^\s*(?:npx\s+)?tsc\b",
    r"(?i)^\s*(?:npx\s+)?tsgo\b",
    r"(?i)^\s*(?:npx\s+)?eslint\b",
    r"(?i)^\s*(?:npx\s+)?biome\s+(check|lint)\b",
    r"(?i)^\s*npm\s+(test|run\s+(test|tests|check|typecheck|lint|build))\b",
    r"(?i)^\s*yarn\s+(test|check|typecheck|lint)\b",
    r"(?i)^\s*pnpm\s+(test|check|typecheck|lint)\b",
    r"(?i)^\s*node\s+.*vitest[/\\]dist[/\\]cli\.js\b",
    r"(?i)^\s*node\s+--test\b",
    r"(?i)^\s*(?:python|python3|pytest|cargo\s+(?:test|check|clippy|fmt|build|nextest)|go\s+(?:test|vet|build)|dotnet\s+(?:test|build))\b",
    r"(?i)^\s*make\s+(test|check|lint|fmt|clippy|build)\b",
    r"(?i)^\s*\.[/\\]test\.sh\b",
];

fn destructive_regex() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| RegexSet::new(DESTRUCTIVE_PATTERNS).expect("destructive patterns compile"))
}

fn privilege_regex() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| RegexSet::new(PRIVILEGE_PATTERNS).expect("privilege patterns compile"))
}

fn package_regex() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| RegexSet::new(PACKAGE_PATTERNS).expect("package patterns compile"))
}

fn git_mutation_regex() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| RegexSet::new(GIT_MUTATION_PATTERNS).expect("git mutation patterns compile"))
}

fn nested_shell_regex() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| RegexSet::new(NESTED_SHELL_PATTERNS).expect("nested shell patterns compile"))
}

fn read_regex() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| RegexSet::new(READ_PATTERNS).expect("read patterns compile"))
}

fn test_regex() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| RegexSet::new(TEST_PATTERNS).expect("test patterns compile"))
}

// ----------------------------------------------------------------------------
// Core Parser and Segment Splitter
// ----------------------------------------------------------------------------

/// Checks whether a command string contains command substitutions or process substitutions.
pub fn has_command_substitution(command: &str) -> bool {
    command.contains("$(")
        || command.contains('`')
        || command.contains("<(")
        || command.contains(">(")
        || command.contains("@(")
}

/// Checks whether a command string contains output redirections to a file or pipe.
/// Does not count `2>&1` (stderr to stdout redirection).
pub fn has_file_redirection(command: &str) -> bool {
    let stripped = command.replace("2>&1", "");
    let mut in_single = false;
    let mut in_double = false;
    let mut prev = '\0';
    let chars: Vec<char> = stripped.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if in_single {
            if ch == '\'' && prev != '\\' {
                in_single = false;
            }
        } else if in_double {
            if ch == '"' && prev != '\\' {
                in_double = false;
            }
        } else {
            match ch {
                '\'' => in_single = true,
                '"' => in_double = true,
                '>' => {
                    return true;
                }
                '|' => {
                    let rest = stripped[i + 1..].trim_start();
                    if rest.starts_with("tee ") || rest.starts_with("tee\t") || rest == "tee" {
                        return true;
                    }
                }
                _ => {}
            }
        }
        prev = ch;
    }
    false
}

/// Splits a shell command line conservatively into its execution segments
/// along `&&`, `||`, `;`, `|`, `&` (when not redirect), and newlines outside quotes.
/// Also returns whether unclosed quotes or brackets were detected.
pub fn split_shell_segments_with_diagnostic(command: &str) -> (Vec<String>, bool) {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut subshell_depth = 0usize;
    let mut prev = '\0';
    let chars: Vec<char> = command.chars().collect();
    let len = chars.len();

    for i in 0..len {
        let ch = chars[i];
        if escaped {
            current.push(ch);
            escaped = false;
            prev = ch;
            continue;
        }

        if let Some(q) = quote {
            current.push(ch);
            if ch == '\\' && q == '"' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            prev = ch;
            continue;
        }

        match ch {
            '\\' => {
                escaped = true;
                current.push(ch);
            }
            '"' | '\'' => {
                quote = Some(ch);
                current.push(ch);
            }
            '(' => {
                subshell_depth += 1;
                current.push(ch);
            }
            ')' => {
                subshell_depth = subshell_depth.saturating_sub(1);
                current.push(ch);
            }
            ';' | '\n' if subshell_depth == 0 => {
                let seg = current.trim().to_string();
                if !seg.is_empty() {
                    segments.push(seg);
                }
                current.clear();
            }
            '|' if subshell_depth == 0 => {
                let next = chars.get(i + 1).copied().unwrap_or('\0');
                if next != '|' {
                    let seg = current.trim().to_string();
                    if !seg.is_empty() {
                        segments.push(seg);
                    }
                    current.clear();
                }
            }
            '&' if subshell_depth == 0 => {
                let next = chars.get(i + 1).copied().unwrap_or('\0');
                if next == '&' {
                    // Handled in second `&`
                } else if prev == '&'
                    || (prev != '>' && prev != '<' && next != '>' && next != '<' && next != '1')
                {
                    let seg = current.trim().to_string();
                    if !seg.is_empty() {
                        segments.push(seg);
                    }
                    current.clear();
                } else {
                    current.push(ch);
                }
            }
            _ => {
                current.push(ch);
            }
        }
        prev = ch;
    }

    let seg = current.trim().to_string();
    if !seg.is_empty() {
        segments.push(seg);
    }

    let has_unclosed = quote.is_some() || subshell_depth > 0 || escaped;
    (segments, has_unclosed)
}

/// Splits a shell command line conservatively into its execution segments.
pub fn split_shell_segments(command: &str) -> Vec<String> {
    split_shell_segments_with_diagnostic(command).0
}

// ----------------------------------------------------------------------------
// Command Analysis and Risk Inspection
// ----------------------------------------------------------------------------

/// Performs comprehensive inspection of a shell command string.
pub fn analyze_command(command: &str) -> ShellAnalysisReport {
    let trimmed = command.trim();
    let (segments, has_unclosed) = split_shell_segments_with_diagnostic(trimmed);

    let has_substitution = has_command_substitution(trimmed);
    let has_nested_shell = nested_shell_regex().is_match(trimmed);
    let has_redirection = has_file_redirection(trimmed);

    let mut risks = Vec::new();

    if has_unclosed {
        risks.push((
            ShellRiskCategory::UnknownSyntax,
            "unclosed quotes, escapes, or subshells in command line".to_string(),
        ));
    }

    if has_substitution {
        risks.push((
            ShellRiskCategory::CommandSubstitution,
            "contains command or process substitution".to_string(),
        ));
    }

    if has_nested_shell {
        risks.push((
            ShellRiskCategory::NestedShell,
            "invokes nested shell interpreter (sh -c, bash -c, powershell -Command, etc.)"
                .to_string(),
        ));
    }

    if has_redirection {
        risks.push((
            ShellRiskCategory::Redirection,
            "contains output redirection writing to filesystem".to_string(),
        ));
    }

    // Inspect command segments for risk patterns
    for segment in &segments {
        let without_stderr_join = segment.replace("2>&1", "");
        if destructive_regex().is_match(&without_stderr_join) {
            risks.push((
                ShellRiskCategory::DestructiveFs,
                format!("matches destructive pattern in segment `{segment}`"),
            ));
        }
        if privilege_regex().is_match(segment) {
            risks.push((
                ShellRiskCategory::PrivilegeEscalation,
                format!("matches privilege escalation in segment `{segment}`"),
            ));
        }
        if package_regex().is_match(segment) {
            risks.push((
                ShellRiskCategory::PackageInstall,
                format!("matches package modification pattern in segment `{segment}`"),
            ));
        }
        if git_mutation_regex().is_match(segment) {
            risks.push((
                ShellRiskCategory::GitMutation,
                format!("matches git state mutation pattern in segment `{segment}`"),
            ));
        }
    }

    // Check read-only and test qualification
    let is_read_only = !segments.is_empty()
        && !has_redirection
        && !has_substitution
        && !has_nested_shell
        && segments.iter().all(|s| {
            read_regex().is_match(s) && !destructive_regex().is_match(&s.replace("2>&1", ""))
        });

    let is_test = !segments.is_empty()
        && !has_redirection
        && !has_substitution
        && !has_nested_shell
        && segments.iter().all(|s| {
            (read_regex().is_match(s) || test_regex().is_match(s))
                && !destructive_regex().is_match(&s.replace("2>&1", ""))
        });

    ShellAnalysisReport {
        command: trimmed.to_string(),
        segments,
        has_substitution,
        has_nested_shell,
        has_redirection,
        has_unknown_syntax: has_unclosed,
        risks,
        is_read_only,
        is_test,
    }
}

/// Evaluates a shell command against a policy profile.
pub fn evaluate(profile: ShellPolicyProfile, command: &str) -> ShellCommandDecision {
    let report = analyze_command(command);

    match profile {
        ShellPolicyProfile::None => ShellCommandDecision::Denied {
            reason: "this role has no shell access".to_string(),
        },
        ShellPolicyProfile::Permissive => {
            if report.has_unknown_syntax {
                ShellCommandDecision::NeedsApproval {
                    reason: "unrecognized shell syntax requires user confirmation".to_string(),
                }
            } else {
                ShellCommandDecision::Allowed
            }
        }
        ShellPolicyProfile::WriteNoGitMutation => {
            if report.has_unknown_syntax {
                return ShellCommandDecision::Denied {
                    reason: "unrecognized shell syntax is forbidden in restricted mode".to_string(),
                };
            }
            if let Some((_, reason)) = report
                .risks
                .iter()
                .find(|(cat, _)| *cat == ShellRiskCategory::GitMutation)
            {
                return ShellCommandDecision::Denied {
                    reason: format!(
                        "git state changes are reserved for the human operator; the graph never commits ({reason})"
                    ),
                };
            }
            ShellCommandDecision::Allowed
        }
        ShellPolicyProfile::ReadOnly => {
            if report.has_unknown_syntax {
                return ShellCommandDecision::Denied {
                    reason: "unrecognized shell syntax is forbidden in restricted mode".to_string(),
                };
            }
            if let Some((cat, reason)) = report.risks.first() {
                return ShellCommandDecision::Denied {
                    reason: format!("command violates read-only policy: {cat} ({reason})"),
                };
            }
            if report.is_read_only {
                ShellCommandDecision::Allowed
            } else {
                ShellCommandDecision::Denied {
                    reason: "command is not in the read-only allowlist".to_string(),
                }
            }
        }
        ShellPolicyProfile::ReadAndTest => {
            if report.has_unknown_syntax {
                return ShellCommandDecision::Denied {
                    reason: "unrecognized shell syntax is forbidden in restricted mode".to_string(),
                };
            }
            if let Some((cat, reason)) = report.risks.first() {
                return ShellCommandDecision::Denied {
                    reason: format!("command violates read-and-test policy: {cat} ({reason})"),
                };
            }
            if report.is_test || report.is_read_only {
                ShellCommandDecision::Allowed
            } else {
                ShellCommandDecision::Denied {
                    reason: "command is not in the read-and-test allowlist".to_string(),
                }
            }
        }
    }
}

// ----------------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_segments_handles_chaining_and_quotes() {
        let (segs, unclosed) = split_shell_segments_with_diagnostic(
            "git status && cargo test 2>&1 | tail -n 5; echo done\nls",
        );
        assert!(!unclosed);
        assert_eq!(
            segs,
            vec![
                "git status",
                "cargo test 2>&1",
                "tail -n 5",
                "echo done",
                "ls"
            ]
        );

        let (segs2, unclosed2) =
            split_shell_segments_with_diagnostic(r#"echo "a && b" 'c | d' e\;f"#);
        assert!(!unclosed2);
        assert_eq!(segs2, vec![r#"echo "a && b" 'c | d' e\;f"#]);

        let (_, unclosed3) = split_shell_segments_with_diagnostic("echo 'unclosed");
        assert!(unclosed3);
    }

    #[test]
    fn detects_command_substitution() {
        assert!(has_command_substitution("echo $(whoami)"));
        assert!(has_command_substitution("echo `id`"));
        assert!(has_command_substitution("diff <(cat a) <(cat b)"));
        assert!(has_command_substitution("echo @(1, 2)"));
        assert!(!has_command_substitution("echo simple text"));
    }

    #[test]
    fn detects_file_redirection_excluding_stderr_join() {
        assert!(has_file_redirection("echo data > out.txt"));
        assert!(has_file_redirection("cat a >> b.txt"));
        assert!(has_file_redirection("echo data | tee out.txt"));
        assert!(!has_file_redirection("cargo test 2>&1"));
        assert!(!has_file_redirection("echo hello"));
    }

    #[test]
    fn detects_nested_shells() {
        let r1 = analyze_command("bash -c 'rm -rf /'");
        assert!(r1.has_nested_shell);

        let r2 = analyze_command("sh -c \"echo safe\"");
        assert!(r2.has_nested_shell);

        let r3 = analyze_command("powershell -Command \"Get-Process\"");
        assert!(r3.has_nested_shell);

        let r4 = analyze_command("pwsh -EncodedCommand dGVzdA==");
        assert!(r4.has_nested_shell);

        let r5 = analyze_command("cmd.exe /c dir");
        assert!(r5.has_nested_shell);

        let r6 = analyze_command("eval $VAR");
        assert!(r6.has_nested_shell);
    }

    #[test]
    fn detects_destructive_and_privilege_operations() {
        let r1 = analyze_command("rm -rf /tmp/data");
        assert!(r1
            .risks
            .iter()
            .any(|(c, _)| *c == ShellRiskCategory::DestructiveFs));

        let r2 = analyze_command("sudo apt update");
        assert!(r2
            .risks
            .iter()
            .any(|(c, _)| *c == ShellRiskCategory::PrivilegeEscalation));

        let r3 = analyze_command("Remove-Item -Recurse C:\\temp");
        assert!(r3
            .risks
            .iter()
            .any(|(c, _)| *c == ShellRiskCategory::DestructiveFs));

        let r4 = analyze_command("npm install -g malicious");
        assert!(r4
            .risks
            .iter()
            .any(|(c, _)| *c == ShellRiskCategory::PackageInstall));
    }

    #[test]
    fn detects_git_mutations() {
        let r1 = analyze_command("git commit -m 'fix'");
        assert!(r1
            .risks
            .iter()
            .any(|(c, _)| *c == ShellRiskCategory::GitMutation));

        let r2 = analyze_command("git push origin main");
        assert!(r2
            .risks
            .iter()
            .any(|(c, _)| *c == ShellRiskCategory::GitMutation));

        let r3 = analyze_command("git.exe -C /repo checkout -b branch");
        assert!(r3
            .risks
            .iter()
            .any(|(c, _)| *c == ShellRiskCategory::GitMutation));

        let r4 = analyze_command("git status");
        assert!(!r4
            .risks
            .iter()
            .any(|(c, _)| *c == ShellRiskCategory::GitMutation));
    }

    #[test]
    fn policy_profile_evaluations() {
        // ReadOnly allows safe reads
        assert_eq!(
            evaluate(ShellPolicyProfile::ReadOnly, "cat README.md"),
            ShellCommandDecision::Allowed
        );
        assert_eq!(
            evaluate(ShellPolicyProfile::ReadOnly, "git status"),
            ShellCommandDecision::Allowed
        );
        assert_eq!(
            evaluate(ShellPolicyProfile::ReadOnly, "Get-Content src/lib.rs"),
            ShellCommandDecision::Allowed
        );

        // ReadOnly blocks writes, tests, destructive, and substitutions
        assert!(matches!(
            evaluate(ShellPolicyProfile::ReadOnly, "rm file.txt"),
            ShellCommandDecision::Denied { .. }
        ));
        assert!(matches!(
            evaluate(ShellPolicyProfile::ReadOnly, "cargo test"),
            ShellCommandDecision::Denied { .. }
        ));
        assert!(matches!(
            evaluate(ShellPolicyProfile::ReadOnly, "echo $(whoami)"),
            ShellCommandDecision::Denied { .. }
        ));

        // ReadAndTest allows test runners
        assert_eq!(
            evaluate(ShellPolicyProfile::ReadAndTest, "cargo test 2>&1"),
            ShellCommandDecision::Allowed
        );
        assert_eq!(
            evaluate(ShellPolicyProfile::ReadAndTest, "npm test"),
            ShellCommandDecision::Allowed
        );
        assert_eq!(
            evaluate(ShellPolicyProfile::ReadAndTest, "pytest tests/"),
            ShellCommandDecision::Allowed
        );

        // ReadAndTest blocks mutations and git mutations
        assert!(matches!(
            evaluate(ShellPolicyProfile::ReadAndTest, "git commit -m 'x'"),
            ShellCommandDecision::Denied { .. }
        ));

        // WriteNoGitMutation allows builds/edits, blocks git commit/push
        assert_eq!(
            evaluate(ShellPolicyProfile::WriteNoGitMutation, "cargo build"),
            ShellCommandDecision::Allowed
        );
        assert!(matches!(
            evaluate(
                ShellPolicyProfile::WriteNoGitMutation,
                "git commit -m 'done'"
            ),
            ShellCommandDecision::Denied { .. }
        ));
        assert!(matches!(
            evaluate(ShellPolicyProfile::WriteNoGitMutation, "git push"),
            ShellCommandDecision::Denied { .. }
        ));

        // None denies all
        assert!(matches!(
            evaluate(ShellPolicyProfile::None, "ls"),
            ShellCommandDecision::Denied { .. }
        ));
    }
}
