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

mod argv;
pub use argv::{argv_subject, evaluate_argv};

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

/// Whether a verifier executes tests or only builds, lints, formats or type
/// checks. Each verifier is classified here, once.
///
/// Deliberate limitation: `make check` and `npm`/`yarn`/`pnpm check` are
/// `Check`, although a project may wire them to its test suite. They are
/// recognised as verifiers but never count as having run tests, so a pass does
/// not skip the requirement review. The extra review is the safe direction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum VerifierKind {
    Test,
    Check,
}

macro_rules! cargo_verifier {
    ($subcommands:literal) => {
        concat!(
            r"(?i)^\s*cargo(?:\.exe)?(?:\s+\+[a-z0-9_.-]+)?(?:\s+--(?:offline|locked|frozen))*\s+",
            $subcommands,
            r"\b"
        )
    };
}

const VERIFIER_PATTERNS: &[(VerifierKind, &str)] = &[
    (VerifierKind::Test, r"(?i)^\s*(?:npx\s+)?vitest\b"),
    (VerifierKind::Test, r"(?i)^\s*(?:npx\s+)?jest\b"),
    (VerifierKind::Test, r"(?i)^\s*(?:npx\s+)?mocha\b"),
    (
        VerifierKind::Test,
        r"(?i)^\s*(?:npx\s+)?playwright\s+test\b",
    ),
    (
        VerifierKind::Check,
        r"(?i)^\s*(?:npx\s+)?(?:tsc|tsgo|eslint)\b",
    ),
    (
        VerifierKind::Check,
        r"(?i)^\s*(?:npx\s+)?biome\s+(check|lint)\b",
    ),
    (
        VerifierKind::Test,
        r"(?i)^\s*npm\s+(test|run\s+(test|tests))\b",
    ),
    (
        VerifierKind::Check,
        r"(?i)^\s*npm\s+run\s+(check|typecheck|lint|build)\b",
    ),
    (VerifierKind::Test, r"(?i)^\s*(?:yarn|pnpm)\s+test\b"),
    (
        VerifierKind::Check,
        r"(?i)^\s*(?:yarn|pnpm)\s+(check|typecheck|lint)\b",
    ),
    (
        VerifierKind::Test,
        r"(?i)^\s*node\s+.*vitest[/\\]dist[/\\]cli\.js\b",
    ),
    (VerifierKind::Test, r"(?i)^\s*node\s+--test\b"),
    (VerifierKind::Test, r"(?i)^\s*ctest\b"),
    (VerifierKind::Test, r"(?i)^\s*mvn\s+test\b"),
    (VerifierKind::Test, r"(?i)^\s*gradle\s+test\b"),
    (
        VerifierKind::Test,
        r"(?i)^\s*(?:(?:python|python3)\s+-m\s+(?:pytest|unittest)|pytest)\b",
    ),
    (VerifierKind::Test, cargo_verifier!("(?:test|nextest)")),
    (VerifierKind::Check, cargo_verifier!("(?:check|clippy|fmt)")),
    (VerifierKind::Test, r"(?i)^\s*(?:go\s+test|dotnet\s+test)\b"),
    (VerifierKind::Check, r"(?i)^\s*go\s+vet\b"),
    (VerifierKind::Test, r"(?i)^\s*make\s+test\b"),
    (
        VerifierKind::Check,
        r"(?i)^\s*make\s+(check|lint|fmt|clippy|build)\b",
    ),
    (VerifierKind::Test, r"(?i)^\s*\.[/\\]test\.sh\b"),
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

/// Every verifier, test or check.
fn test_regex() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| {
        RegexSet::new(VERIFIER_PATTERNS.iter().map(|(_, pattern)| pattern))
            .expect("verifier patterns compile")
    })
}

/// The verifiers that build, lint, format or type-check without running tests.
fn non_test_check_regex() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| {
        RegexSet::new(
            VERIFIER_PATTERNS
                .iter()
                .filter(|(kind, _)| *kind == VerifierKind::Check)
                .map(|(_, pattern)| pattern),
        )
        .expect("check patterns compile")
    })
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
    let mut quote = None;
    let mut escaped = false;
    // String slices use byte offsets, not character positions.
    for (i, ch) in stripped.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if ch == '\\' && q == '"' {
                escaped = true;
            }
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' | '"' => quote = Some(ch),
            '>' => return true,
            '|' => {
                let rest = stripped[i + ch.len_utf8()..].trim_start();
                if rest.starts_with("tee ") || rest.starts_with("tee\t") || rest == "tee" {
                    return true;
                }
            }
            _ => {}
        }
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

/// Extract literal words for restricted-policy checks, without evaluating a
/// shell. Unknown expansions and shell structures fail closed. Dequoting matters:
/// `-de'lete'` is the same option as `-delete`, not harmless text.
pub(crate) fn literal_shell_words(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    for ch in command.chars() {
        if escaped {
            // Escaping operators differs between Bash and PowerShell. Do not
            // authorize a command whose structure depends on that difference.
            if matches!(
                ch,
                '\n' | '\r' | ';' | '|' | '&' | '<' | '>' | '(' | ')' | '{' | '}' | '$' | '`'
            ) {
                return None;
            }
            word.push(ch);
            escaped = false;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if q == '"' && ch == '\\' {
                escaped = true;
            } else if q == '"' && matches!(ch, '$' | '`') {
                return None;
            } else {
                word.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                started = true;
            }
            '\\' => {
                escaped = true;
                started = true;
            }
            '$' | '`' | '(' | ')' | '{' | '}' | '<' | '>' | ';' | '|' | '&' => return None,
            ch if ch.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            _ => {
                word.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() || escaped {
        return None;
    }
    if started {
        words.push(word);
    }
    (!words.is_empty()).then_some(words)
}

/// GNU-style long options may accept unambiguous abbreviations and `=value`.
fn selects_long_option(arg: &str, option: &str) -> bool {
    let flag = arg.split('=').next().unwrap_or(arg);
    flag.starts_with("--") && flag.len() > 2 && option.starts_with(flag)
}

fn git_is_read_only(words: &[String]) -> bool {
    let mut args = &words[1..];
    while let Some(flag) = args.first() {
        if matches!(flag.as_str(), "-C" | "--git-dir" | "--work-tree") {
            if args.len() < 2 {
                return false;
            }
            args = &args[2..];
        } else if matches!(
            flag.as_str(),
            "--no-pager" | "--no-optional-locks" | "--bare"
        ) || flag.starts_with("--git-dir=")
            || flag.starts_with("--work-tree=")
            || (flag.starts_with("-C") && flag.len() > 2)
        {
            args = &args[1..];
        } else {
            break;
        }
    }
    let Some((verb, args)) = args.split_first() else {
        return false;
    };
    if args.iter().any(|arg| {
        ["--output", "--ext-diff", "--textconv", "--show-signature"]
            .iter()
            .any(|option| selects_long_option(arg, option))
    }) {
        return false;
    }
    match verb.as_str() {
        "status" | "log" | "diff" | "show" | "blame" | "ls-files" | "ls-tree" | "rev-parse"
        | "describe" | "shortlog" => true,
        // A branch or remote name is a mutation operand, not a read operation.
        "branch" => args.iter().all(|arg| {
            matches!(
                arg.as_str(),
                "--list"
                    | "-a"
                    | "--all"
                    | "-r"
                    | "--remotes"
                    | "-v"
                    | "-vv"
                    | "--verbose"
                    | "--show-current"
                    | "--no-color"
            ) || (args.iter().any(|value| value == "--list") && !arg.starts_with('-'))
        }),
        "remote" => args
            .iter()
            .all(|arg| matches!(arg.as_str(), "-v" | "--verbose")),
        // In particular, reject aliases and global configuration overrides:
        // either can run commands even when the displayed verb looks harmless.
        _ => false,
    }
}

fn read_arguments_are_safe(segment: &str) -> bool {
    let Some(words) = literal_shell_words(&segment.replace("2>&1", "")) else {
        return false;
    };
    let args = &words[1..];
    match words[0].to_ascii_lowercase().as_str() {
        "git" | "git.exe" => git_is_read_only(&words),
        // These are interpreters (sed's `w`/`e`, awk's system()/output), not
        // inherently read-only commands. Use the native read/grep tools instead.
        "sed" | "awk" => false,
        "find" => args.iter().all(|arg| {
            !arg.starts_with('-')
                || matches!(
                    arg.as_str(),
                    "-H" | "-L"
                        | "-P"
                        | "-name"
                        | "-iname"
                        | "-path"
                        | "-ipath"
                        | "-type"
                        | "-maxdepth"
                        | "-mindepth"
                        | "-print"
                        | "-print0"
                        | "-size"
                        | "-mtime"
                        | "-mmin"
                        | "-empty"
                        | "-not"
                        | "-a"
                        | "-o"
                )
        }),
        "fd" => !args.iter().any(|arg| {
            selects_long_option(arg, "--exec")
                || selects_long_option(arg, "--exec-batch")
                || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains(['x', 'X']))
        }),
        "rg" => !args.iter().any(|arg| {
            selects_long_option(arg, "--pre") || selects_long_option(arg, "--hostname-bin")
        }),
        "sort" | "tree" => !args.iter().any(|arg| {
            selects_long_option(arg, "--output")
                || selects_long_option(arg, "--compress-program")
                || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('o'))
                || arg.to_ascii_lowercase().starts_with("/o")
        }),
        "uniq" => {
            let mut operands = 0;
            let mut options = true;
            args.iter().all(|arg| {
                if options && arg == "--" {
                    options = false;
                    return true;
                }
                if options && arg.starts_with('-') && arg != "-" {
                    matches!(
                        arg.as_str(),
                        "-c" | "-d"
                            | "-u"
                            | "-i"
                            | "-z"
                            | "--count"
                            | "--repeated"
                            | "--unique"
                            | "--ignore-case"
                            | "--zero-terminated"
                            | "--help"
                            | "--version"
                    )
                } else {
                    operands += 1;
                    operands <= 1
                }
            })
        }
        "node" | "rustc" => args.len() == 1 && args[0] == "--version",
        "cat" | "head" | "tail" | "grep" | "ls" | "dir" | "pwd" | "echo" | "wc" | "diff"
        | "stat" | "which" | "where" | "where.exe" | "type" | "jq" | "cargo" | "npm"
        | "get-content" | "get-childitem" | "get-item" | "get-location" | "get-process"
        | "get-command" | "select-string" => true,
        _ => false,
    }
}

fn test_arguments_are_safe(segment: &str) -> bool {
    let Some(words) = literal_shell_words(&segment.replace("2>&1", "")) else {
        return false;
    };
    match words[0].as_str() {
        "python" | "python3" => {
            words.get(1).map(String::as_str) == Some("-m")
                && matches!(
                    words.get(2).map(String::as_str),
                    Some("pytest" | "unittest")
                )
        }
        "node" => {
            let args = &words[1..];
            let Some(first) = args.first() else {
                return false;
            };
            if first == "--test" {
                // Node options are accepted only from the test-runner subset;
                // eval, print, preload, loaders and configuration injection are not tests.
                args[1..].iter().all(|arg| {
                    !arg.starts_with('-')
                        || matches!(
                            arg.as_str(),
                            "--test" | "--test-only" | "--test-todo" | "--test-force-exit"
                        )
                        || [
                            "--test-name-pattern=",
                            "--test-skip-pattern=",
                            "--test-concurrency=",
                            "--test-timeout=",
                        ]
                        .iter()
                        .any(|prefix| arg.starts_with(prefix))
                })
            } else {
                // The script operand itself must be the runner. A runner-like
                // trailing argument must never authorize `node -e ...`.
                let script = first.replace('\\', "/");
                !script.starts_with('-')
                    && (script == "vitest/dist/cli.js" || script.ends_with("/vitest/dist/cli.js"))
            }
        }
        "npx" | "vitest" | "jest" | "mocha" | "playwright" | "tsc" | "tsgo" | "eslint"
        | "biome" | "npm" | "yarn" | "pnpm" | "pytest" | "cargo" | "go" | "dotnet" | "make"
        | "./test.sh" => true,
        _ => false,
    }
}

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
        let unsafe_git = literal_shell_words(&without_stderr_join).is_some_and(|words| {
            matches!(
                words[0]
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str(),
                "git" | "git.exe"
            ) && !git_is_read_only(&words)
        });
        if git_mutation_regex().is_match(segment) || unsafe_git {
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
            read_regex().is_match(s)
                && read_arguments_are_safe(s)
                && !destructive_regex().is_match(&s.replace("2>&1", ""))
        });

    let is_test = !segments.is_empty()
        && !has_redirection
        && !has_substitution
        && !has_nested_shell
        && segments.iter().all(|s| {
            ((read_regex().is_match(s) && read_arguments_are_safe(s))
                || (test_regex().is_match(s) && test_arguments_are_safe(s)))
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

pub(crate) fn shell_env_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    matches!(chars.next(), Some('_') | Some('a'..='z') | Some('A'..='Z'))
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// The normalized program and arguments of a verifier segment, or `None` when
/// the segment is not a verifier. The single parser behind both questions
/// "is this a verifier" and "did it run tests".
fn verifier_command(segment: &str) -> Option<String> {
    verifier_program(segment).filter(|normalized| !normalized.contains("--no-run"))
}

/// As [`verifier_command`], but a `--no-run` build still counts: it compiles
/// the tests and can fail, though it never ran them.
fn verifier_program(segment: &str) -> Option<String> {
    let cleaned = segment.replace("2>&1", "");
    let words = literal_shell_words(cleaned.trim())?;
    let first_command = words.iter().position(|word| !shell_env_assignment(word))?;
    let normalized = words[first_command..].join(" ");
    test_regex().is_match(&normalized).then_some(normalized)
}

fn verification_segment(segment: &str) -> bool {
    verifier_command(segment).is_some()
}

/// Classifies whether a command runs verification programs and whether the
/// command's exit status can be trusted as their combined verification result.
pub fn verification_outcome(command: &str) -> Option<bool> {
    let report = analyze_command(command);
    let verification: Vec<bool> = report
        .segments
        .iter()
        .map(|segment| verification_segment(segment))
        .collect();
    if !verification.iter().any(|is_verification| *is_verification) {
        return None;
    }
    // An OR chain can turn a failed verifier into success. Likewise, mixing a
    // verifier with a non-verification segment means the final status can be
    // caused by something else. A chain of verifiers joined with AND is safe:
    // success means every verifier succeeded and any failure remains failure.
    let status_masked =
        command.contains("||") || verification.iter().any(|is_verification| !is_verification);
    Some(!status_masked)
}

/// Whether any segment of `command` is a verifier, test or check. Used to
/// decide that a nonzero exit invalidates earlier test evidence even when the
/// rest of the chain (`&& echo ok`) keeps the command from being classified.
pub fn invokes_verifier(command: &str) -> bool {
    analyze_command(command)
        .segments
        .iter()
        .any(|segment| verifier_program(segment).is_some())
}

/// As [`invokes_verifier`] or `turn::invokes_checker`, but tolerant of how the
/// program is spelled: a directory (`./node_modules/.bin/eslint`,
/// `.\node_modules\.bin\eslint.cmd`), case, an `.exe`, `.cmd`, `.bat` or `.ps1`
/// suffix and an `env`, `command`, `exec`, `time` or `sudo` prefix are ignored.
/// For revoking test coverage after a failure only: recognising more failures
/// can only cause more reviews, but this must never earn test credit.
pub(crate) fn invokes_verifier_or_checker_loosely(command: &str) -> bool {
    let (segments, malformed) = split_shell_segments_with_diagnostic(command);
    if malformed {
        return false;
    }
    segments.iter().any(|segment| {
        let words = split_words_keeping_backslashes(segment);
        let mut words = words.iter().map(String::as_str).peekable();
        // Skip a PowerShell call operator, then wrappers and their options.
        words.next_if_eq(&"&");
        let wrapper_name = |word: &str| {
            let base = word
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            base.strip_suffix(".exe").unwrap_or(&base).to_string()
        };
        let mut after_env = false;
        while let Some(word) = words.next_if(|word| {
            shell_env_assignment(word)
                || matches!(
                    wrapper_name(word).as_str(),
                    "env" | "time" | "sudo" | "command" | "exec"
                )
                || (after_env
                    && (matches!(
                        *word,
                        "--" | "-i" | "--ignore-environment" | "-u" | "--unset"
                    ) || word.starts_with("--unset=")))
        }) {
            after_env |= wrapper_name(word) == "env";
            if after_env && matches!(word, "-u" | "--unset") {
                words.next();
            }
        }
        let Some(program) = words.next() else {
            return false;
        };
        let program = program
            .trim_matches(|ch| matches!(ch, '"' | '\'' | '(' | ')' | '{' | '}'))
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let program = [".exe", ".cmd", ".bat", ".ps1"]
            .iter()
            .find_map(|suffix| program.strip_suffix(suffix))
            .unwrap_or(&program);
        let normalized = std::iter::once(program)
            .chain(words)
            .collect::<Vec<_>>()
            .join(" ");
        test_regex().is_match(&normalized) || crate::turn::is_checker_program(program)
    })
}

/// Splits on whitespace outside single or double quotes and drops the quote
/// characters. Unlike `literal_shell_words`, backslashes are kept: here they
/// are Windows path separators.
fn split_words_keeping_backslashes(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for ch in segment.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => word.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                in_word = true;
            }
            None if ch.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            None => {
                word.push(ch);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// Whether a verification command ran tests: some segment is a verifier that
/// is not a build, lint or type check. `cargo check && cargo test` ran tests;
/// `cargo clippy` alone did not.
pub fn verification_ran_tests(command: &str) -> bool {
    analyze_command(command).segments.iter().any(|segment| {
        verifier_command(segment)
            .is_some_and(|normalized| !non_test_check_regex().is_match(&normalized))
    })
}

impl ShellAnalysisReport {
    /// Classifies the declared effects of this shell command based on static analysis.
    ///
    /// Invariant: This is a conservative classification of apparent effects;
    /// it does NOT claim sandboxing or containment.
    pub fn classify_declared_effects(&self) -> Vec<crate::runtime::capabilities::DeclaredEffect> {
        use crate::runtime::capabilities::DeclaredEffect;
        let mut effects = vec![DeclaredEffect::ProcessExecution];

        if self.is_read_only && !self.has_redirection {
            effects.push(DeclaredEffect::FileSystemRead);
        } else {
            effects.push(DeclaredEffect::FileSystemWrite);
        }

        let cmd_lower = self.command.to_lowercase();
        // Detect network commands
        if cmd_lower.contains("curl")
            || cmd_lower.contains("wget")
            || cmd_lower.contains("fetch")
            || cmd_lower.contains("git clone")
            || cmd_lower.contains("git fetch")
            || cmd_lower.contains("git pull")
            || cmd_lower.contains("git push")
            || cmd_lower.contains("npm install")
            || cmd_lower.contains("npm i")
            || cmd_lower.contains("pip install")
            || cmd_lower.contains("cargo install")
        {
            effects.push(DeclaredEffect::NetworkAccess);
        }

        // Detect publish/deploy commands
        if cmd_lower.contains("git push")
            || cmd_lower.contains("npm publish")
            || cmd_lower.contains("cargo publish")
            || cmd_lower.contains("docker push")
            || cmd_lower.contains("deploy")
        {
            effects.push(DeclaredEffect::ExternalServicePublish);
        }

        effects.sort_by_key(|e| e.to_string());
        effects.dedup();
        effects
    }
}

/// Classifies declared effects for a shell command string without claiming sandboxing.
pub fn classify_shell_effects(command: &str) -> Vec<crate::runtime::capabilities::DeclaredEffect> {
    analyze_command(command).classify_declared_effects()
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

    // Verbatim copies of the lists the VERIFIER_PATTERNS table replaced
    // (`git show HEAD:crates/davinci-agent/src/shell_policy.rs`).
    const ORIGINAL_TEST_PATTERNS: &[&str] = &[
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
        r"(?i)^\s*ctest\b",
        r"(?i)^\s*mvn\s+test\b",
        r"(?i)^\s*gradle\s+test\b",
        r"(?i)^\s*(?:(?:python|python3)\s+-m\s+(?:pytest|unittest)|pytest|cargo(?:\.exe)?(?:\s+\+[a-z0-9_.-]+)?(?:\s+--(?:offline|locked|frozen))*\s+(?:test|check|clippy|fmt|nextest)|go\s+(?:test|vet)|dotnet\s+test)\b",
        r"(?i)^\s*make\s+(test|check|lint|fmt|clippy|build)\b",
        r"(?i)^\s*\.[/\\]test\.sh\b",
    ];

    const ORIGINAL_NON_TEST_CHECK_PATTERNS: &[&str] = &[
        r"(?i)^\s*(?:npx\s+)?(?:tsc|tsgo|eslint)\b",
        r"(?i)^\s*(?:npx\s+)?biome\s+(check|lint)\b",
        r"(?i)^\s*npm\s+run\s+(check|typecheck|lint|build)\b",
        r"(?i)^\s*(?:yarn|pnpm)\s+(check|typecheck|lint)\b",
        r"(?i)^\s*cargo(?:\.exe)?(?:\s+\+[a-z0-9_.-]+)?(?:\s+--(?:offline|locked|frozen))*\s+(check|clippy|fmt)\b",
        r"(?i)^\s*go\s+vet\b",
        r"(?i)^\s*make\s+(check|lint|fmt|clippy|build)\b",
    ];

    /// At least one command per original pattern, plus variants and near
    /// misses, so a regex edit in the table cannot change a classification.
    const PARITY_CORPUS: &[&str] = &[
        "vitest run",
        "npx vitest",
        "  NPX  Vitest run",
        "vitestx",
        "jest",
        "npx jest --ci",
        "mocha",
        "npx mocha test/",
        "playwright test",
        "npx playwright test",
        "playwright install",
        "tsc",
        "tsc --noEmit",
        "npx tsc -p .",
        "tsgo",
        "npx tsgo",
        "eslint .",
        "npx eslint src",
        "biome check .",
        "npx biome lint",
        "biome format",
        "npm test",
        "npm run test",
        "npm run tests",
        "npm run check",
        "npm run typecheck",
        "npm run lint",
        "npm run build",
        "npm run dev",
        "npm run testing",
        "npm install",
        "npm check",
        "yarn test",
        "yarn check",
        "yarn typecheck",
        "yarn lint",
        "yarn build",
        "pnpm test",
        "pnpm check",
        "pnpm typecheck",
        "pnpm lint",
        "pnpm build",
        "node node_modules/vitest/dist/cli.js run",
        "node C:\\x\\vitest\\dist\\cli.js",
        "node --test",
        "node --version",
        "node app.js",
        "ctest",
        "ctest --output-on-failure",
        "mvn test",
        "mvn package",
        "gradle test",
        "gradle build",
        "python -m pytest",
        "python3 -m pytest -q",
        "python -m unittest",
        "python3 -m unittest discover",
        "python -m pip install x",
        "pytest",
        "pytest -q tests/",
        "cargo test",
        "cargo nextest run",
        "cargo check",
        "cargo clippy",
        "cargo fmt",
        "cargo build",
        "cargo run",
        "cargo testing",
        "cargo +nightly test",
        "cargo +1.88 clippy",
        "cargo --offline --locked test",
        "cargo --frozen check",
        "cargo.exe test",
        "cargo.exe +stable --locked fmt",
        "CARGO test",
        "go test ./...",
        "go vet ./...",
        "go build ./...",
        "dotnet test",
        "dotnet build",
        "make test",
        "make tests",
        "make check",
        "make lint",
        "make fmt",
        "make clippy",
        "make build",
        "make install",
        "make",
        "./test.sh",
        ".\\test.sh",
        "./test.sh --fast",
        "./build.sh",
        "echo cargo test",
        "ls",
    ];

    fn compiled(patterns: &[&str]) -> RegexSet {
        RegexSet::new(patterns).unwrap()
    }

    #[test]
    fn verifier_table_classifies_every_original_pattern_as_before() {
        let was_verifier = compiled(ORIGINAL_TEST_PATTERNS);
        let was_check = compiled(ORIGINAL_NON_TEST_CHECK_PATTERNS);
        for command in PARITY_CORPUS {
            assert_eq!(
                test_regex().is_match(command),
                was_verifier.is_match(command),
                "verifier changed for {command:?}"
            );
            assert_eq!(
                non_test_check_regex().is_match(command),
                was_check.is_match(command),
                "check changed for {command:?}"
            );
        }
        // Every original pattern is exercised by at least one corpus command,
        // so the comparison above cannot pass vacuously.
        for (list, name) in [
            (ORIGINAL_TEST_PATTERNS, "test"),
            (ORIGINAL_NON_TEST_CHECK_PATTERNS, "check"),
        ] {
            for pattern in list {
                let one = RegexSet::new([pattern]).unwrap();
                assert!(
                    PARITY_CORPUS.iter().any(|c| one.is_match(c)),
                    "no corpus command for {name} pattern {pattern}"
                );
            }
        }
    }

    #[test]
    fn make_check_and_js_check_scripts_stay_verifiers_but_not_test_runs() {
        for command in [
            "make check",
            "npm run check",
            "yarn check",
            "pnpm check",
            "npm run typecheck",
        ] {
            assert!(test_regex().is_match(command), "{command}");
            assert!(non_test_check_regex().is_match(command), "{command}");
            assert!(!verification_ran_tests(command), "{command}");
        }
        assert!(verification_ran_tests("make test"));
        assert!(verification_ran_tests("make check && make test"));
    }

    #[test]
    fn verification_commands_are_recognised_by_their_program() {
        for (command, expected) in [
            ("cargo test", Some(true)),
            ("cargo test --workspace -- --nocapture", Some(true)),
            ("RUST_BACKTRACE=1 cargo test", Some(true)),
            ("cargo test && cargo clippy", Some(true)),
            ("cargo nextest run", Some(true)),
            ("npx vitest run", Some(true)),
            ("npx jest", Some(true)),
            ("tsc --noEmit", Some(true)),
            ("pytest -q", Some(true)),
            ("ctest --output-on-failure", Some(true)),
            ("mvn test", Some(true)),
            ("gradle test", Some(true)),
            ("cargo test 2>&1 | tail-20", Some(false)),
            ("cargo test || true", Some(false)),
            ("echo cargo test", None),
            ("cargo test --no-run", None),
            ("python script.py", None),
            ("cargo build", None),
            ("go build ./...", None),
            ("dotnet build", None),
            ("ls", None),
        ] {
            assert_eq!(verification_outcome(command), expected, "{command}");
        }
    }

    #[test]
    fn only_verifiers_that_execute_tests_count_as_ran_tests() {
        for (command, expected) in [
            ("cargo test", true),
            ("cargo --offline --locked test -p x", true),
            ("RUST_BACKTRACE=1 cargo test", true),
            ("cd crates/x && cargo test", true),
            ("cargo check && cargo test", true),
            ("pytest -q", true),
            ("npm test", true),
            ("go test ./...", true),
            ("cargo check", false),
            (
                "cargo --offline --locked clippy --all-targets -- -D warnings",
                false,
            ),
            ("cargo fmt --all -- --check", false),
            ("cargo +nightly fmt --check", false),
            ("RUST_BACKTRACE=1 cargo clippy", false),
            ("cargo check && cargo clippy", false),
            ("tsc --noEmit", false),
            ("npx eslint .", false),
            ("npm run typecheck", false),
            ("go vet ./...", false),
            // Deliberate limitation: these may run tests in a project, but are
            // classified as checks, so the extra review stays (the safe side).
            ("make check", false),
            ("npm run check", false),
            ("yarn check", false),
            ("pnpm check", false),
            ("make test", true),
            ("yarn test", true),
            ("pnpm test", true),
            ("cargo test --no-run", false),
            ("echo cargo test", false),
            ("ls", false),
        ] {
            assert_eq!(verification_ran_tests(command), expected, "{command}");
        }
    }

    #[test]
    fn cargo_global_flags_are_test_policy_compatible_without_granting_other_commands() {
        for command in [
            "cargo --offline test",
            "cargo --locked --offline fmt --check",
            "cargo +1.83.0 --frozen check",
        ] {
            assert_eq!(
                evaluate(ShellPolicyProfile::ReadAndTest, command),
                ShellCommandDecision::Allowed,
                "{command}"
            );
        }
        for command in [
            "cargo --offline publish",
            "cargo --offline install evil",
            "cargo --config bad test",
            "cargo --offline test && git push",
            "cargo +unsafe/../../x test",
        ] {
            assert_ne!(
                evaluate(ShellPolicyProfile::ReadAndTest, command),
                ShellCommandDecision::Allowed,
                "{command}"
            );
        }
    }

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
    fn security_unicode_pipeline_does_not_panic() {
        for text in ["🙂", "é", "界", "🙂🙂", "résumé 日本語"] {
            assert!(!has_file_redirection(&format!("echo {text} | cat")));
            assert!(has_file_redirection(&format!("echo {text} | tee out.txt")));
        }
    }

    #[test]
    fn security_redirection_after_literal_backslash_is_detected() {
        for command in [r"echo '\' > out.txt", r#"echo "\\" > out.txt"#] {
            assert!(
                has_file_redirection(command),
                "missed redirection: {command}"
            );
            assert!(matches!(
                evaluate(ShellPolicyProfile::ReadOnly, command),
                ShellCommandDecision::Denied { .. }
            ));
        }
        assert!(!has_file_redirection(r#"echo "literal > text""#));
        assert!(!has_file_redirection(r"echo \>"));
    }

    #[test]
    fn security_read_only_rejects_side_effects_in_inspection_commands() {
        for command in [
            "find . -delete",
            "find . -de'lete'",
            "fd -x python helper.py",
            "rg --pre helper.py needle",
            "sort --output=out.txt input.txt",
            "uniq input.txt out.txt",
            r#"awk 'BEGIN {system("python helper.py")}'"#,
            "sed -n 'w out.txt' input.txt",
            "git branch audit-branch",
            "git remote remove origin",
        ] {
            for profile in [
                ShellPolicyProfile::ReadOnly,
                ShellPolicyProfile::ReadAndTest,
            ] {
                assert!(
                    matches!(
                        evaluate(profile, command),
                        ShellCommandDecision::Denied { .. }
                    ),
                    "{profile:?} unexpectedly allowed {command}"
                );
            }
        }
    }

    #[test]
    fn security_test_profile_rejects_arbitrary_python() {
        for command in [
            r#"python -c "open('audit.txt', 'w').close()""#,
            "python helper.py",
            "python3 -m http.server",
        ] {
            assert!(
                matches!(
                    evaluate(ShellPolicyProfile::ReadAndTest, command),
                    ShellCommandDecision::Denied { .. }
                ),
                "unexpectedly allowed {command}"
            );
        }
        for command in ["python -m pytest tests/", "python3 -m unittest discover"] {
            assert_eq!(
                evaluate(ShellPolicyProfile::ReadAndTest, command),
                ShellCommandDecision::Allowed
            );
        }
    }

    #[test]
    fn security_writer_rejects_git_reference_and_config_mutations() {
        for command in [
            "git branch audit-branch",
            "git branch -D audit-branch",
            "git remote remove origin",
            "git update-ref refs/heads/audit HEAD",
            "git config core.hooksPath hooks",
        ] {
            assert!(
                matches!(
                    evaluate(ShellPolicyProfile::WriteNoGitMutation, command),
                    ShellCommandDecision::Denied { .. }
                ),
                "unexpectedly allowed {command}"
            );
        }
    }

    #[test]
    fn security_node_runner_arguments_are_not_eval_permission() {
        for command in [
            r#"node -e "require('fs').writeFileSync('audit.txt','x')" vitest/dist/cli.js"#,
            "node --eval=anything vitest/dist/cli.js",
            "node --print=anything vitest/dist/cli.js",
            "node --require helper.js --test",
        ] {
            assert!(
                matches!(
                    evaluate(ShellPolicyProfile::ReadAndTest, command),
                    ShellCommandDecision::Denied { .. }
                ),
                "unexpectedly allowed {command}"
            );
        }
        for command in [
            "node --test",
            "node --test tests/example.test.js",
            "node node_modules/vitest/dist/cli.js run",
        ] {
            assert_eq!(
                evaluate(ShellPolicyProfile::ReadAndTest, command),
                ShellCommandDecision::Allowed
            );
        }
    }

    #[test]
    fn security_writer_recognizes_absolute_git_executable() {
        assert!(matches!(
            evaluate(
                ShellPolicyProfile::WriteNoGitMutation,
                "/usr/bin/git branch audit-branch"
            ),
            ShellCommandDecision::Denied { .. }
        ));
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
