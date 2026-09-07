//! Native scan syntax; extends the discovery contract in
//! vendor/davinci/packages/coding-agent/src/core/slash-commands.ts.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanMode {
    Quick,
    Standard,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportFormat {
    Terminal,
    Json,
    Sarif,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReportCommand {
    pub scan_id: Option<String>,
    pub finding_id: Option<String>,
}

impl ReportCommand {
    pub fn parse(input: &str) -> Result<Self, String> {
        let tokens = tokenize(input)?;
        let mut result = Self {
            scan_id: None,
            finding_id: None,
        };
        let mut iter = tokens.iter();
        while let Some(token) = iter.next() {
            let target = if token == "--finding" {
                &mut result.finding_id
            } else if token.starts_with('-') {
                return Err("unknown security report flag".into());
            } else {
                if result.scan_id.is_some() {
                    return Err("repeated scan identity".into());
                }
                validate_report_id(token)?;
                result.scan_id = Some(token.clone());
                continue;
            };
            if target.is_some() {
                return Err("repeated finding selector".into());
            }
            let id = iter.next().ok_or("--finding requires an identity")?;
            validate_report_id(id)?;
            *target = Some(id.clone());
        }
        Ok(result)
    }
}

fn validate_report_id(id: &str) -> Result<(), String> {
    let parsed = uuid::Uuid::parse_str(id).map_err(|_| "invalid report identity")?;
    if parsed.to_string() != id {
        return Err("noncanonical report identity".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Selection {
    Worktree,
    Changed,
    Diff(Option<(String, String)>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanCommand {
    pub scopes: Vec<String>,
    pub mode: ScanMode,
    pub format: ReportFormat,
    pub focus: Option<String>,
    pub selection: Selection,
}

impl ScanCommand {
    pub fn parse(input: &str) -> Result<Self, String> {
        Self::parse_with_default(input, ScanMode::Standard)
    }

    pub fn parse_with_default(input: &str, default_mode: ScanMode) -> Result<Self, String> {
        let tokens = tokenize(input)?;
        let mut result = Self {
            scopes: Vec::new(),
            mode: default_mode,
            format: ReportFormat::Terminal,
            focus: None,
            selection: Selection::Worktree,
        };
        let mut seen = BTreeSet::new();
        let mut positional = false;
        let mut explicit = false;
        let mut literal = false;
        let mut iter = tokens.iter().peekable();
        while let Some(token) = iter.next() {
            if !literal && token == "--" {
                literal = true;
                continue;
            }
            if literal || !token.starts_with('-') {
                if positional || explicit {
                    return Err("use one positional path or repeated --scope".into());
                }
                positional = true;
                result.scopes.push(token.clone());
                continue;
            }
            if token != "--scope" && !seen.insert(token.as_str()) {
                return Err(format!("repeated flag: {token}"));
            }
            match token.as_str() {
                "--changed" | "--diff" => {
                    if result.selection != Selection::Worktree {
                        return Err("--changed and --diff are mutually exclusive".into());
                    }
                    result.selection = if token == "--changed" {
                        Selection::Changed
                    } else {
                        let range = if iter
                            .peek()
                            .is_some_and(|next| !next.starts_with('-') && next.contains(".."))
                        {
                            let range = iter.next().unwrap();
                            let (base, head) = range.split_once("..").unwrap();
                            if base.is_empty()
                                || head.is_empty()
                                || base.contains("..")
                                || head.contains("..")
                                || head.starts_with('.')
                            {
                                return Err("--diff requires exactly base..head".into());
                            }
                            Some((base.to_string(), head.to_string()))
                        } else {
                            None
                        };
                        Selection::Diff(range)
                    };
                }
                "--scope" | "--mode" | "--format" | "--focus" => {
                    let value = iter
                        .next()
                        .filter(|s| !s.is_empty() && !s.starts_with('-'))
                        .ok_or_else(|| format!("{token} requires a value"))?;
                    match token.as_str() {
                        "--scope" => {
                            if positional {
                                return Err("cannot mix a path with --scope".into());
                            }
                            explicit = true;
                            result.scopes.push(value.clone());
                        }
                        "--mode" => {
                            result.mode = match value.as_str() {
                                "quick" => ScanMode::Quick,
                                "standard" => ScanMode::Standard,
                                "deep" => ScanMode::Deep,
                                _ => return Err("mode must be quick, standard, or deep".into()),
                            }
                        }
                        "--format" => {
                            result.format = match value.as_str() {
                                "terminal" => ReportFormat::Terminal,
                                "json" => ReportFormat::Json,
                                "sarif" => ReportFormat::Sarif,
                                _ => return Err("format must be terminal, json, or sarif".into()),
                            }
                        }
                        _ => {
                            if ![
                                "auth",
                                "injection",
                                "filesystem",
                                "secrets",
                                "dependencies",
                                "crypto",
                                "logic",
                                "configuration",
                                "ai-tools",
                            ]
                            .contains(&value.as_str())
                            {
                                return Err("unknown security focus".into());
                            }
                            result.focus = Some(value.clone());
                        }
                    }
                }
                _ => return Err(format!("unknown security scan flag: {token}")),
            }
        }
        if result.scopes.iter().any(|s| s.is_empty()) {
            return Err("empty scope".into());
        }
        Ok(result)
    }
}

// Backslashes remain literal, including in quoted Windows paths. This is
// argument grouping only; no shell expansion or command execution occurs.
fn tokenize(input: &str) -> Result<Vec<String>, String> {
    if input.len() > 16 * 1024 || input.chars().any(|c| c.is_control() && !c.is_whitespace()) {
        return Err("invalid or oversized command".into());
    }
    let mut tokens = Vec::new();
    let mut value = String::new();
    let mut quote = None;
    let mut started = false;
    for ch in input.chars() {
        if quote == Some(ch) {
            quote = None;
        } else if quote.is_some() {
            value.push(ch);
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            started = true;
        } else if ch.is_whitespace() {
            if started {
                tokens.push(std::mem::take(&mut value));
                started = false;
            }
        } else {
            value.push(ch);
            started = true;
        }
    }
    if quote.is_some() {
        return Err("unterminated quoted argument".into());
    }
    if started {
        tokens.push(value);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_report_parser_accepts_bound_finding_and_rejects_ambiguous_selectors() {
        let scan = uuid::Uuid::new_v4().to_string();
        let finding = uuid::Uuid::new_v4().to_string();
        let parsed = ReportCommand::parse(&format!("{scan} --finding {finding}")).unwrap();
        assert_eq!(parsed.scan_id.as_deref(), Some(scan.as_str()));
        assert_eq!(parsed.finding_id.as_deref(), Some(finding.as_str()));
        assert!(ReportCommand::parse(&format!("--finding {finding}"))
            .unwrap()
            .scan_id
            .is_none());
        for invalid in [
            "--finding".into(),
            "--finding ../report".into(),
            format!("{scan} {scan}"),
            format!("--finding {finding} --finding {finding}"),
            "--format json".into(),
        ] {
            assert!(ReportCommand::parse(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn security_scan_command_is_discoverable_and_invocable() {
        assert!(crate::native_extensions::NATIVE_COMMANDS.contains(&"security-scan"));
        assert!(crate::native_extensions::command_specs()
            .iter()
            .any(|(name, _, _)| *name == "security-scan"));
        assert_eq!(ScanCommand::parse("").unwrap().mode, ScanMode::Standard);
    }

    #[test]
    fn security_scan_parser_accepts_quoted_scope() {
        let parsed = ScanCommand::parse(
            r#"--scope "src/my files" --scope '文档' --mode deep --format json"#,
        )
        .unwrap();
        assert_eq!(parsed.scopes, ["src/my files", "文档"]);
        assert_eq!(parsed.mode, ScanMode::Deep);
        assert_eq!(parsed.format, ReportFormat::Json);
        assert_eq!(ScanCommand::parse("").unwrap().mode, ScanMode::Standard);
        assert_eq!(ScanCommand::parse(".").unwrap().scopes, ["."]);
    }

    #[test]
    fn security_scan_rejects_unknown_or_conflicting_flags_before_side_effects() {
        for input in [
            "--oops",
            "src --scope lib",
            "--changed --diff",
            "--mode quick --mode deep",
            "--format xml",
            "--scope",
            "--focus other",
            "'unclosed",
            "src other",
            "--diff a...b",
            "--diff a..",
            "--changed --changed",
            "--scope --mode quick",
        ] {
            assert!(ScanCommand::parse(input).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn security_scan_parser_preserves_windows_paths_and_literal_hyphens() {
        assert_eq!(
            ScanCommand::parse(r#""C:\repo\my files""#).unwrap().scopes,
            [r"C:\repo\my files"]
        );
        assert_eq!(
            ScanCommand::parse("-- -source").unwrap().scopes,
            ["-source"]
        );
        assert_eq!(
            ScanCommand::parse("--diff base..head").unwrap().selection,
            Selection::Diff(Some(("base".into(), "head".into())))
        );
    }
}
