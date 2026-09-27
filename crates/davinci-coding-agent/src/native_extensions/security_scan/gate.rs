//! Deterministic changed-surface rules for the graph security gate; native-only.
//!
//! The rules are token-bounded shapes rather than substring needles, and the
//! gate judges only the lines a change adds when it is given the change's
//! unified diff, so text that predates the change never blocks it.

use super::FindingSeverity;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Hit {
    pub rule_id: &'static str,
    pub severity: FindingSeverity,
    pub line: usize,
    pub message: &'static str,
    pub evidence: String,
}

fn private_key() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----").unwrap())
}

/// Group 1 is the credential-shaped token. The leading class stands in for a
/// look-behind: `task-`, `risk-` or `disk-` never start a token.
fn api_key() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:^|[^A-Za-z0-9_\-])(sk-(?:proj-|ant-(?:api[0-9]{2}-)?)?[A-Za-z0-9_\-]{20,}|gh[pousr]_[A-Za-z0-9]{36,}|AKIA[0-9A-Z]{16}|xox[abprs]-[A-Za-z0-9\-]{10,}|AIza[0-9A-Za-z_\-]{35})(?:$|[^A-Za-z0-9_\-])",
        )
        .unwrap()
    })
}

/// `eval(` as a free call: never `retrieval(`, `model.eval()` or `$eval(`.
fn eval_call() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|[^A-Za-z0-9_$.])eval\s*\(").unwrap())
}

fn shell_true() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:^|[^A-Za-z0-9_])shell\s*=\s*True(?:$|[^A-Za-z0-9_])").unwrap()
    })
}

/// A real key has letters and digits and is not a run of one placeholder.
fn plausible_secret(token: &str) -> bool {
    let body = token
        .trim_start_matches("sk-proj-")
        .trim_start_matches("sk-ant-")
        .trim_start_matches("sk-");
    let distinct = body.chars().collect::<BTreeSet<_>>().len();
    body.chars().any(|c| c.is_ascii_digit())
        && body.chars().any(|c| c.is_ascii_alphabetic())
        && distinct >= 8
        && !body.to_ascii_lowercase().contains("xxxx")
}

fn declares_eval(prefix: &str) -> bool {
    let prefix = prefix.trim_end();
    ["fn", "def", "function", "async function"]
        .iter()
        .any(|keyword| prefix.ends_with(keyword))
}

fn is_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//")
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
        || trimmed.starts_with("--")
        || (trimmed.starts_with('#') && !trimmed.starts_with("#["))
}

fn redact_tokens(line: &str) -> String {
    let mut out = super::redact_evidence(line);
    for capture in api_key().captures_iter(line) {
        if let Some(token) = capture.get(1) {
            out = out.replace(token.as_str(), "[REDACTED]");
        }
    }
    out
}

/// Every rule hit in `text`, with 1-based line numbers.
pub(super) fn detect(text: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        if private_key().is_match(line) {
            hits.push(Hit {
                rule_id: "secret.private-key",
                severity: FindingSeverity::Critical,
                line: line_no,
                message: "Private key material appears in repository content",
                evidence: "[REDACTED PRIVATE KEY MATERIAL]".into(),
            });
            continue;
        }
        if api_key()
            .captures_iter(line)
            .filter_map(|capture| capture.get(1))
            .any(|token| plausible_secret(token.as_str()))
        {
            hits.push(Hit {
                rule_id: "secret.api-key",
                severity: FindingSeverity::High,
                line: line_no,
                message: "API-key-shaped credential appears in repository content",
                evidence: redact_tokens(line),
            });
        }
        if is_comment(line) {
            continue;
        }
        if eval_call()
            .find_iter(line)
            .any(|found| !declares_eval(&line[..found.start() + 1]))
        {
            hits.push(Hit {
                rule_id: "command.eval",
                severity: FindingSeverity::Medium,
                line: line_no,
                message: "Dynamic evaluation can execute untrusted input",
                evidence: redact_tokens(line),
            });
        }
        if shell_true().is_match(line) {
            hits.push(Hit {
                rule_id: "command.shell",
                severity: FindingSeverity::Medium,
                line: line_no,
                message: "Shell execution with interpolation requires validation",
                evidence: redact_tokens(line),
            });
        }
    }
    hits
}

/// What one file's unified diff says the change did to it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct FileChange {
    pub deleted: bool,
    /// New-side line numbers the change added; `None` when the diff does not
    /// say (binary or oversized patches), so the whole file is judged.
    pub added: Option<BTreeSet<usize>>,
}

fn hunk_start() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^@@ -[0-9]+(?:,[0-9]+)? \+([0-9]+)(?:,[0-9]+)? @@").unwrap())
}

fn strip_side(path: &str, side: &str) -> String {
    let path = path.trim_end_matches('\t').trim();
    path.strip_prefix(side)
        .unwrap_or(path)
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

/// Parse a multi-file unified diff (Git or the graph's mutation delta).
pub(super) fn parse_diff(diff: &str) -> BTreeMap<String, FileChange> {
    struct Current {
        old: Option<String>,
        new: Option<String>,
        change: FileChange,
        in_hunk: bool,
        next_line: usize,
    }
    fn finish(current: Option<Current>, out: &mut BTreeMap<String, FileChange>) {
        if let Some(current) = current {
            if let Some(path) = current.new.or(current.old) {
                out.insert(path, current.change);
            }
        }
    }
    let mut out = BTreeMap::new();
    let mut current: Option<Current> = None;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            finish(current.take(), &mut out);
            current = Some(Current {
                old: None,
                new: None,
                change: FileChange {
                    deleted: false,
                    added: Some(BTreeSet::new()),
                },
                in_hunk: false,
                next_line: 0,
            });
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            match hunk_start()
                .captures(line)
                .and_then(|capture| capture[1].parse::<usize>().ok())
            {
                Some(start) => {
                    file.in_hunk = true;
                    file.next_line = start;
                }
                None => {
                    file.in_hunk = false;
                    file.change.added = None;
                }
            }
            continue;
        }
        if !file.in_hunk {
            if let Some(path) = line.strip_prefix("--- ") {
                if path.trim() != "/dev/null" {
                    file.old = Some(strip_side(path, "a/"));
                }
            } else if let Some(path) = line.strip_prefix("+++ ") {
                if path.trim() == "/dev/null" {
                    file.change.deleted = true;
                } else {
                    file.new = Some(strip_side(path, "b/"));
                }
            } else if line.starts_with("Binary files ") {
                file.change.added = None;
            }
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => {
                if let Some(added) = file.change.added.as_mut() {
                    added.insert(file.next_line);
                }
                file.next_line += 1;
            }
            Some(b' ') | None => file.next_line += 1,
            _ => {}
        }
    }
    finish(current, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(text: &str) -> Vec<&'static str> {
        detect(text).into_iter().map(|hit| hit.rule_id).collect()
    }

    #[test]
    fn security_gate_rules_ignore_identifier_suffixes() {
        for benign in [
            "let task-name = 1;",
            "fn retrieval(query: &str) {}",
            "let x = retrieval(q);",
            "model.eval()",
            "risk-assessment-for-the-whole-system-2024",
            "mask-aaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "disk-usage-report-12345678901234567890",
            "pub fn eval(&self) -> bool { true }",
            "// never call eval(input)",
            "let key = \"sk-xxxxxxxxxxxxxxxxxxxxxxxxxxxx\";",
            "subprocess.run(cmd, shell=False)",
            "noshell=True",
        ] {
            assert!(rules(benign).is_empty(), "{benign}: {:?}", rules(benign));
        }
    }

    #[test]
    fn security_gate_rules_find_real_shapes() {
        assert_eq!(
            rules("OPENAI_API_KEY=sk-proj-4f9Qa2Lk8Zt3Vb7Nc1Xd6Rm0Hs"),
            ["secret.api-key"]
        );
        assert_eq!(
            rules("token = \"ghp_1234567890abcdefghijABCDEFGHIJklmnop\""),
            ["secret.api-key"]
        );
        assert_eq!(rules("aws = AKIAIOSFODNN7EXAMPLE"), ["secret.api-key"]);
        assert_eq!(
            rules("-----BEGIN RSA PRIVATE KEY-----"),
            ["secret.private-key"]
        );
        assert_eq!(rules("result = eval(user_input)"), ["command.eval"]);
        assert_eq!(
            rules("subprocess.run(cmd, shell = True)"),
            ["command.shell"]
        );
        let hit = &detect("k=sk-proj-4f9Qa2Lk8Zt3Vb7Nc1Xd6Rm0Hs")[0];
        assert!(!hit.evidence.contains("4f9Qa2Lk8Zt3"), "{}", hit.evidence);
    }

    #[test]
    fn security_gate_diff_parser_reports_added_lines_and_deletions() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n keep\n-old\n+new one\n+new two\n keep\ndiff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n--- a/gone.rs\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-bye\ndiff --git a/big.rs b/big.rs\n--- a/big.rs\n+++ b/big.rs\n@@ -1,3000 +1,3001 @@\n@@ large file modified @@\n";
        let parsed = parse_diff(diff);
        assert_eq!(parsed["src/a.rs"].added, Some(BTreeSet::from([2usize, 3])));
        assert!(!parsed["src/a.rs"].deleted);
        assert!(parsed["gone.rs"].deleted);
        assert_eq!(parsed["big.rs"].added, None);
    }
}
