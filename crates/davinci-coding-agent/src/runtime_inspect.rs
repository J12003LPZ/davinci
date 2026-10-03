//! CLI grammar and dispatch for the read-only runtime inspector.

use davinci_agent::runtime::operations::{inspect, InspectionTarget, InspectorOutput};
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaintenanceCommand {
    Inspect { target: InspectTarget, json: bool },
    DoctorRuntime { json: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectTarget {
    Run(String),
    Operation(String),
    Session(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceParseError(pub String);

impl fmt::Display for MaintenanceParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Parse only the maintenance grammar.  A literal `--` before the first
/// command token means the user is sending a prompt, so this returns `None`
/// and lets the normal argument parser handle it.
pub fn parse_command(raw: &[String]) -> Result<Option<MaintenanceCommand>, MaintenanceParseError> {
    let Some(command_index) = first_non_offline(raw) else {
        return Ok(None);
    };
    if raw[..command_index].iter().any(|argument| argument == "--") {
        return Ok(None);
    }
    let command = raw[command_index].as_str();
    match command {
        "inspect" => parse_inspect(&raw[command_index + 1..]),
        "doctor" => parse_doctor(&raw[command_index + 1..]),
        _ => Ok(None),
    }
}

pub fn try_run(raw: &[String]) -> Option<Result<i32, String>> {
    let command = match parse_command(raw) {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(error) => {
            let output = parse_error_output(error.to_string());
            print_output(&output, true);
            return Some(Ok(output.exit_code as i32));
        }
    };
    let json = match &command {
        MaintenanceCommand::Inspect { json, .. } | MaintenanceCommand::DoctorRuntime { json } => {
            *json
        }
    };
    let target = match command {
        MaintenanceCommand::Inspect { target, .. } => match target {
            InspectTarget::Run(id) => InspectionTarget::Run(id),
            InspectTarget::Operation(id) => InspectionTarget::Operation(id),
            InspectTarget::Session(id) => InspectionTarget::Session(id),
        },
        MaintenanceCommand::DoctorRuntime { .. } => InspectionTarget::DoctorRuntime,
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let output = run_at(&cwd, target);
    print_output(&output, json);
    Some(Ok(output.exit_code as i32))
}

pub fn run_at(cwd: &Path, target: InspectionTarget) -> InspectorOutput {
    inspect(cwd, target)
}

fn first_non_offline(raw: &[String]) -> Option<usize> {
    raw.iter().position(|argument| argument != "--offline")
}

fn parse_inspect(
    arguments: &[String],
) -> Result<Option<MaintenanceCommand>, MaintenanceParseError> {
    let mut positional = Vec::new();
    let mut json = false;
    for argument in arguments {
        if argument == "--json" {
            json = true;
        } else if argument == "--offline" {
            // `--offline` is a global flag and can be repeated by wrappers.
        } else if argument == "--" {
            return Ok(None);
        } else if argument.starts_with('-') {
            return Err(MaintenanceParseError(format!(
                "unknown inspector flag {argument}"
            )));
        } else {
            positional.push(argument.as_str());
        }
    }
    if positional.len() != 2 {
        return Err(MaintenanceParseError(
            "usage: davinci inspect <run|operation|session> <id> [--json]".into(),
        ));
    }
    let id = positional[1].to_owned();
    if id.trim().is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
        return Err(MaintenanceParseError(
            "inspection id is empty or invalid".into(),
        ));
    }
    let target = match positional[0] {
        "run" => {
            uuid::Uuid::parse_str(&id)
                .map_err(|_| MaintenanceParseError("run id must be a UUID".into()))?;
            InspectTarget::Run(id)
        }
        "operation" => {
            uuid::Uuid::parse_str(&id)
                .map_err(|_| MaintenanceParseError("operation id must be a UUID".into()))?;
            InspectTarget::Operation(id)
        }
        "session" => InspectTarget::Session(id),
        other => {
            return Err(MaintenanceParseError(format!(
                "unknown inspection target {other}; expected run, operation, or session"
            )))
        }
    };
    Ok(Some(MaintenanceCommand::Inspect { target, json }))
}

fn parse_doctor(arguments: &[String]) -> Result<Option<MaintenanceCommand>, MaintenanceParseError> {
    let mut json = false;
    for argument in arguments {
        match argument.as_str() {
            "runtime" => {}
            "--json" => json = true,
            "--offline" => {}
            other => {
                return Err(MaintenanceParseError(format!(
                    "usage: davinci doctor runtime [--json] (unexpected {other})"
                )))
            }
        }
    }
    if !arguments.iter().any(|argument| argument == "runtime") {
        return Err(MaintenanceParseError(
            "usage: davinci doctor runtime [--json]".into(),
        ));
    }
    Ok(Some(MaintenanceCommand::DoctorRuntime { json }))
}

fn parse_error_output(message: String) -> InspectorOutput {
    InspectorOutput {
        schema_version: davinci_agent::runtime::operations::INSPECTOR_SCHEMA_VERSION,
        command: "maintenance_parse".into(),
        status: davinci_agent::runtime::operations::InspectorStatus::Unavailable,
        exit_code: 3,
        findings: vec![message],
        report: serde_json::json!({"read_only": true}),
        truncated: false,
    }
}

fn print_output(output: &InspectorOutput, json: bool) {
    if json {
        println!("{}", output.json_string());
    } else {
        println!("{}", output.text());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_all_maintenance_forms() {
        assert!(matches!(
            parse_command(&args(&[
                "inspect",
                "operation",
                "00000000-0000-0000-0000-000000000001",
                "--json"
            ])),
            Ok(Some(MaintenanceCommand::Inspect { .. }))
        ));
        assert!(matches!(
            parse_command(&args(&["doctor", "runtime"])),
            Ok(Some(MaintenanceCommand::DoctorRuntime { .. }))
        ));
    }

    #[test]
    fn literal_prompt_separator_is_not_a_maintenance_command() {
        assert_eq!(
            parse_command(&args(&["--", "inspect", "operation", "not-an-id"])),
            Ok(None)
        );
    }

    #[test]
    fn invalid_ids_and_grammar_are_rejected() {
        assert!(parse_command(&args(&["inspect", "operation", "not-an-id"])).is_err());
        assert!(parse_command(&args(&["inspect", "operation"])).is_err());
        assert!(parse_command(&args(&["doctor"])).is_err());
    }
}
