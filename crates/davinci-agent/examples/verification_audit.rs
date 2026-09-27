//! Classify recorded commands without executing them, for checkpoint measurements.
use davinci_agent::verification::{classify, CheckKind};
use serde::Deserialize;
use serde_json::json;
use std::io::{self, BufRead};
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    id: String,
    tool: String,
    command: String,
    cwd: PathBuf,
    paths: Vec<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The classifier parses source with its bounded AST inspector; it never
    // invokes the recorded command or imports the recorded project's modules.
    for line in io::stdin().lock().lines() {
        let input: Input = serde_json::from_str(&line?)?;
        let result = classify(&input.tool, &input.command, &input.cwd, &input.paths);
        let kind = match result.kind {
            CheckKind::Suite => "suite",
            CheckKind::TargetedScript => "targeted_script",
            CheckKind::SyntaxOnly => "syntax_only",
            CheckKind::Unknown => "unknown",
        };
        println!(
            "{}",
            json!({"id": input.id, "kind": kind,
            "complete": result.complete, "reason": result.reason,
            "covered": result.covered})
        );
    }
    Ok(())
}
