//! Offline evaluation of the natural-language design studio's deterministic
//! parts: what a typed brief becomes, which concept plans are accepted, and
//! what a failure report may carry. Labels are an independent fixture, not
//! derived from the code under test, so a regression in the classifier or the
//! parser moves a number here.
//!
//! This measures routing and validation. It does not measure how good a
//! generated design looks, whether a real browser renders it, or how a live
//! model behaves; those need a real run and are reported as unverified.

use davinci_coding_agent::design::{
    commands::{parse_design_command, DesignCommand},
    generation::{parse_directions, sanitize_snippet, validate_directions},
    intent::{infer_kind, KindSource},
    types::DesignKind,
};
use serde::Deserialize;
use serde_json::json;

const CORPUS: &str = include_str!("../../fixtures/behavior/design-studio.json");

#[derive(Deserialize)]
struct Labelled {
    brief: String,
    kind: String,
    source: String,
}

/// One measured fixture and whether it held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignStudioEvalResult {
    pub fixture: String,
    pub passed: bool,
    pub detail: String,
}

/// Counts and the bar each group must clear. Every threshold is 1.0: the
/// labelled set is small and deterministic, so one miss is a regression.
#[derive(Debug, Clone, PartialEq)]
pub struct DesignStudioReport {
    pub results: Vec<DesignStudioEvalResult>,
    pub brief_fidelity: f64,
    pub kind_accuracy: f64,
    pub plan_decisions: f64,
    pub redaction: f64,
}

pub const THRESHOLD: f64 = 1.0;

impl DesignStudioReport {
    pub fn passed(&self) -> bool {
        [
            self.brief_fidelity,
            self.kind_accuracy,
            self.plan_decisions,
            self.redaction,
        ]
        .iter()
        .all(|score| *score >= THRESHOLD)
    }
}

fn labelled() -> Vec<Labelled> {
    serde_json::from_str(CORPUS).expect("design studio corpus")
}

pub fn design_studio_fixture_names() -> Vec<String> {
    labelled().into_iter().map(|row| row.brief).collect()
}

fn kind_name(kind: DesignKind) -> &'static str {
    match kind {
        DesignKind::Landing => "landing",
        DesignKind::Product => "product",
        DesignKind::Document => "document",
    }
}

fn ratio(results: &[&DesignStudioEvalResult]) -> f64 {
    if results.is_empty() {
        return 0.0;
    }
    results.iter().filter(|result| result.passed).count() as f64 / results.len() as f64
}

fn result(fixture: String, passed: bool, detail: String) -> DesignStudioEvalResult {
    DesignStudioEvalResult {
        fixture,
        passed,
        detail,
    }
}

fn direction(layout: &str, typography: &str, strategy: &str) -> serde_json::Value {
    json!({"title": format!("{layout} {typography}"), "layout": layout, "typography": typography, "content_strategy": strategy})
}

/// Plans a model might return, and whether the host should accept them for
/// the stated concept count.
fn plans() -> Vec<(&'static str, String, u32, bool)> {
    let three = json!([
        direction(
            "editorial",
            "serif",
            "lead with the story and one calm action"
        ),
        direction(
            "split",
            "sans",
            "pair a live metric with the primary control"
        ),
        direction("modular", "mono", "dense grid of independent cards")
    ]);
    let repeated = json!([
        direction("editorial", "serif", "same idea"),
        direction("split", "sans", "same idea"),
        direction("modular", "mono", "same idea")
    ]);
    let same_style = json!([
        direction("split", "sans", "first idea"),
        direction("split", "sans", "second idea")
    ]);
    vec![
        ("three distinct concepts", three.to_string(), 3, true),
        ("a fenced plan", format!("```json\n{three}\n```"), 3, true),
        (
            "a plan wrapped in an object",
            json!({"directions": three}).to_string(),
            3,
            true,
        ),
        (
            "one concept as a bare object",
            direction("split", "sans", "one clear idea").to_string(),
            1,
            true,
        ),
        (
            "a bare object when three were asked",
            direction("split", "sans", "one clear idea").to_string(),
            3,
            false,
        ),
        (
            "two concepts when three were asked",
            three
                .as_array()
                .map(|list| json!(&list[..2]).to_string())
                .unwrap(),
            3,
            false,
        ),
        ("the same idea restyled", repeated.to_string(), 3, false),
        (
            "the same layout and typography",
            same_style.to_string(),
            2,
            false,
        ),
        (
            "prose around the plan",
            format!("Sure, here you go: {three}"),
            3,
            false,
        ),
        ("an empty plan", "[]".into(), 3, false),
        ("not a plan at all", "42".into(), 3, false),
    ]
}

/// Text a failure report must never carry whole.
fn leaks() -> Vec<(&'static str, String, &'static str)> {
    vec![
        (
            "an API key",
            "reply was sk-live0123456789abcdef0123456789abcdef end".into(),
            "sk-live0123456789abcdef0123456789abcdef",
        ),
        (
            "a bearer token",
            "Authorization: Bearer abc.def.ghi trailing".into(),
            "abc.def.ghi",
        ),
        (
            "a long opaque token",
            "token=Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZg== done".into(),
            "Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZg==",
        ),
        (
            "a JSON apiKey",
            r#"{"apiKey":"sk-proj-AbCdEf0123456789xyzWVUT"}"#.into(),
            r#"sk-proj-AbCdEf0123456789xyzWVUT"#,
        ),
        (
            "a JSON APIKEY in capitals",
            r#"{"APIKEY":"sk-ABCDEFGH12345678"}"#.into(),
            r#"sk-ABCDEFGH12345678"#,
        ),
        (
            "a JSON token",
            r#"{"token":"opaque-session-value"}"#.into(),
            r#"opaque-session-value"#,
        ),
        (
            "a JSON secret",
            r#"{"Secret": "s3cr3t-value-here"}"#.into(),
            r#"s3cr3t-value-here"#,
        ),
        (
            "a JSON password",
            r#"{"password":"hunter2"}"#.into(),
            r#"hunter2"#,
        ),
        (
            "a JSON authorization",
            r#"{"authorization":"Bearer abc.def.ghi"}"#.into(),
            r#"abc.def.ghi"#,
        ),
        (
            "a JSON cookie",
            r#"{"cookie":"sid=AbCdEf0123456789"}"#.into(),
            r#"AbCdEf0123456789"#,
        ),
        (
            "an escaped JSON key",
            r#"args "{\"apiKey\":\"sk-proj-ZZZZ1234567890\"}""#.into(),
            r#"sk-proj-ZZZZ1234567890"#,
        ),
        (
            "a Cookie header",
            r#"Cookie: session=abc123; theme=dark"#.into(),
            r#"abc123"#,
        ),
        (
            "a Set-Cookie header",
            r#"Set-Cookie: sid=AbCdEf0123456789XyZ; Path=/"#.into(),
            r#"AbCdEf0123456789XyZ"#,
        ),
        (
            "a SET-COOKIE header in capitals",
            r#"SET-COOKIE: sid=zzzzzzzz1; Secure"#.into(),
            r#"zzzzzzzz1"#,
        ),
        (
            "a scheme glued to its header",
            r#"Authorization:Bearer abc.def.ghi"#.into(),
            r#"abc.def.ghi"#,
        ),
        (
            "a basic credential",
            r#"Authorization: Basic dXNlcjpwYXNzd29yZA=="#.into(),
            r#"dXNlcjpwYXNzd29yZA=="#,
        ),
        (
            "a bare JWT",
            r#"jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r end"#.into(),
            r#"eyJhbGciOiJIUzI1NiJ9"#,
        ),
        (
            "a quoted GitHub token",
            r#"token "ghp_abcdefghijklmnopqrstuvwxyzABCDEFGH""#.into(),
            r#"ghp_abcdefghijklmnopqrstuvwxyzABCDEFGH"#,
        ),
        (
            "a gho_ token",
            r#"gho_abcdefghijklmnopqrstuvwxyzABCDEFGH"#.into(),
            r#"gho_abcdefghijklmnopqrstuvwxyzABCDEFGH"#,
        ),
        (
            "a fine-grained GitHub token",
            r#"github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz"#.into(),
            r#"github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz"#,
        ),
        (
            "an AWS key id",
            r#"key AKIAIOSFODNN7EXAMPLE end"#.into(),
            r#"AKIAIOSFODNN7EXAMPLE"#,
        ),
    ]
}

pub fn run_design_studio_evals() -> DesignStudioReport {
    let mut fidelity = Vec::new();
    let mut accuracy = Vec::new();
    for row in labelled() {
        // A brief that opens with a verb word goes after `new`; any other
        // brief is typed bare, as a person would.
        let first = row.brief.split_whitespace().next().unwrap_or_default();
        let verb = [
            "new", "list", "open", "status", "revise", "resume", "select", "verify", "fork",
            "restore", "export", "apply", "sync",
        ]
        .contains(&first);
        let line = if verb {
            format!("/design new {}", row.brief)
        } else {
            format!("/design {}", row.brief)
        };
        match parse_design_command(&line) {
            Ok(DesignCommand::Create { input }) => {
                fidelity.push(result(
                    format!("brief kept: {}", row.brief),
                    input.brief == row.brief && input.variants == 3,
                    format!("got {:?} with {} concepts", input.brief, input.variants),
                ));
                let (kind, source) = infer_kind(&row.brief);
                let source = match source {
                    KindSource::Inferred => "inferred",
                    KindSource::Default => "default",
                    KindSource::Explicit => "explicit",
                };
                accuracy.push(result(
                    format!("kind: {}", row.brief),
                    kind_name(kind) == row.kind
                        && source == row.source
                        && kind_name(input.kind) == row.kind,
                    format!(
                        "expected {}/{}, got {}/{source}",
                        row.kind,
                        row.source,
                        kind_name(kind)
                    ),
                ));
            }
            other => {
                fidelity.push(result(
                    format!("brief kept: {}", row.brief),
                    false,
                    format!("parsed as {other:?}"),
                ));
                accuracy.push(result(
                    format!("kind: {}", row.brief),
                    false,
                    "not a create".into(),
                ));
            }
        }
    }

    let mut decisions = Vec::new();
    for (name, text, count, accept) in plans() {
        let accepted = parse_directions(&text, count)
            .and_then(|plan| validate_directions(&plan, count))
            .is_ok();
        decisions.push(result(
            format!("plan: {name}"),
            accepted == accept,
            format!("expected accepted={accept}, got {accepted}"),
        ));
    }

    let mut redaction = Vec::new();
    for (name, text, secret) in leaks() {
        let snippet = sanitize_snippet(&text, 120);
        redaction.push(result(
            format!("redaction: {name}"),
            !snippet.contains(secret) && snippet.chars().count() <= 130,
            format!("snippet {snippet:?}"),
        ));
    }
    // Redaction must not eat ordinary prose about design.
    for prose in [
        "the hero section needs a bluer call to action",
        "design tokens: colors and spacing",
        "cookie banner copy for the footer",
    ] {
        let kept = sanitize_snippet(prose, 120);
        redaction.push(result(
            format!("redaction keeps prose: {prose}"),
            kept == prose,
            format!("snippet {kept:?}"),
        ));
    }
    let long = sanitize_snippet(&"word ".repeat(10_000), 400);
    redaction.push(result(
        "redaction: a huge reply is cut".into(),
        long.chars().count() <= 410,
        format!("{} characters", long.chars().count()),
    ));

    let report = DesignStudioReport {
        brief_fidelity: ratio(&fidelity.iter().collect::<Vec<_>>()),
        kind_accuracy: ratio(&accuracy.iter().collect::<Vec<_>>()),
        plan_decisions: ratio(&decisions.iter().collect::<Vec<_>>()),
        redaction: ratio(&redaction.iter().collect::<Vec<_>>()),
        results: fidelity
            .into_iter()
            .chain(accuracy)
            .chain(decisions)
            .chain(redaction)
            .collect(),
    };
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_studio_clears_every_bar_offline() {
        let report = run_design_studio_evals();
        let failed: Vec<_> = report
            .results
            .iter()
            .filter(|result| !result.passed)
            .map(|result| format!("{}: {}", result.fixture, result.detail))
            .collect();
        assert!(failed.is_empty(), "{failed:#?}");
        assert!(report.passed());
        assert!(design_studio_fixture_names().len() >= 30);
    }

    #[test]
    fn a_wrong_label_moves_the_score() {
        // The bar is only worth something if a regression can fail it.
        let (kind, _) = infer_kind("a dashboard");
        assert_ne!(kind_name(kind), "landing");
        let report = DesignStudioReport {
            results: vec![],
            brief_fidelity: 1.0,
            kind_accuracy: 29.0 / 30.0,
            plan_decisions: 1.0,
            redaction: 1.0,
        };
        assert!(!report.passed());
    }
}
