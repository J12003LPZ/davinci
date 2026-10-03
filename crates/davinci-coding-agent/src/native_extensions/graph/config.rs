//! Defaults, project-local overrides, and verification command detection.
//!
//! Every budget that bounds time or money is OFF by default (`0` = unlimited);
//! a project opts back into a backstop through `<cwd>/.pi/graph.json`.

use super::store::CONFIG_DIR;
use super::types::{GraphBudgets, Role, VerifyCommandSpec};
use super::validate::validate_config_shape;
use crate::native_extensions::ecosystem::verification::SecurityPolicyMode;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphConfig {
    pub budgets: GraphBudgets,
    /// "provider/modelId" per role; absent = inherit the session model.
    pub models: BTreeMap<Role, String>,
    /// Overrides auto-detection when non-empty.
    pub verify_commands: Vec<VerifyCommandSpec>,
    /// Extra extension paths loaded into every worker via `-e`.
    pub worker_extensions: Vec<String>,
    /// Extra tool names those extensions register, added to every allowlist.
    pub worker_extra_tools: Vec<String>,
    /// Security verification policy mode; defaults to Risk.
    pub security_verification: SecurityPolicyMode,
}

#[derive(Debug, Clone, Default)]
pub struct LoadedConfig {
    pub config: GraphConfig,
    pub errors: Vec<String>,
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_u64_budget(value: &Value, name: &str) -> Result<u64, String> {
    value
        .as_u64()
        .ok_or_else(|| format!("{name} must be a non-negative whole integer"))
}

#[allow(dead_code)]
pub fn parse_usize_budget(value: &Value, name: &str) -> Result<usize, String> {
    let n = parse_u64_budget(value, name)?;
    usize::try_from(n).map_err(|_| format!("{name} exceeds maximum usize"))
}

pub fn load_config(cwd: &Path) -> LoadedConfig {
    let mut loaded = LoadedConfig::default();
    let davinci_path = cwd.join(CONFIG_DIR).join("graph.json");
    let config_path = if davinci_path.exists() {
        davinci_path
    } else {
        cwd.join(super::store::LEGACY_CONFIG_DIR).join("graph.json")
    };
    let Ok(raw) = fs::read_to_string(&config_path) else {
        return loaded;
    };
    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(parsed) => parsed,
        Err(error) => {
            loaded.errors.push(format!(
                "{}: invalid JSON: {error}",
                config_path.to_string_lossy()
            ));
            return loaded;
        }
    };
    loaded.errors = validate_config_shape(&parsed);
    if !loaded.errors.is_empty() {
        return loaded;
    }
    let Some(object) = parsed.as_object() else {
        return loaded;
    };

    if let Some(budgets) = object.get("budgets").and_then(Value::as_object) {
        let target = &mut loaded.config.budgets;
        if let Some(val) = budgets.get("maxResearchers") {
            if let Ok(value) = parse_u64_budget(val, "budgets.maxResearchers") {
                target.max_researchers = value as u32;
            }
        }
        if let Some(val) = budgets.get("maxParallelWorkers") {
            if let Ok(value) = parse_u64_budget(val, "budgets.maxParallelWorkers") {
                target.max_parallel_workers = value.max(1) as u32;
            }
        }
        if let Some(val) = budgets.get("maxWorkers") {
            if let Ok(value) = parse_u64_budget(val, "budgets.maxWorkers") {
                target.max_workers = value as u32;
            }
        }
        if let Some(val) = budgets.get("maxRevisionCycles") {
            if let Ok(value) = parse_u64_budget(val, "budgets.maxRevisionCycles") {
                target.max_revision_cycles = value as u32;
            }
        }
        if let Some(val) = budgets.get("maxReplans") {
            if let Ok(value) = parse_u64_budget(val, "budgets.maxReplans") {
                target.max_replans = value as u32;
            }
        }
        if let Some(val) = budgets.get("maxCostUsd") {
            if let Some(value) = val.as_f64().filter(|n| *n >= 0.0) {
                target.max_cost_usd = value;
            }
        }
        if let Some(val) = budgets.get("runDeadlineMs") {
            if let Ok(value) = parse_u64_budget(val, "budgets.runDeadlineMs") {
                target.run_deadline_ms = value;
            }
        }
        if let Some(val) = budgets.get("verifyCommandTimeoutMs") {
            if let Ok(value) = parse_u64_budget(val, "budgets.verifyCommandTimeoutMs") {
                target.verify_command_timeout_ms = value;
            }
        }
        if let Some(timeouts) = budgets.get("workerTimeoutMs").and_then(Value::as_object) {
            for (role, timeout) in timeouts {
                if let (Some(role), Ok(t)) = (
                    Role::parse(role),
                    parse_u64_budget(timeout, &format!("budgets.workerTimeoutMs.{role}")),
                ) {
                    target.worker_timeout_ms.set(role, t);
                }
            }
        }
    }

    if let Some(models) = object.get("models").and_then(Value::as_object) {
        for (role, model) in models {
            if let (Some(role), Some(model)) = (Role::parse(role), model.as_str()) {
                loaded.config.models.insert(role, model.to_string());
            }
        }
    }

    if let Some(commands) = object.get("verifyCommands").and_then(Value::as_array) {
        for command in commands {
            let (Some(name), Some(text)) = (
                command.get("name").and_then(Value::as_str),
                command.get("command").and_then(Value::as_str),
            ) else {
                continue;
            };
            loaded.config.verify_commands.push(VerifyCommandSpec {
                name: name.to_string(),
                command: text.to_string(),
                from_plan: false,
            });
        }
    }
    loaded.config.worker_extensions = string_array(object.get("workerExtensions"));
    loaded.config.worker_extra_tools = string_array(object.get("workerExtraTools"));
    if let Some(val) = object.get("securityVerification").and_then(Value::as_str) {
        match val {
            "off" => loaded.config.security_verification = SecurityPolicyMode::Off,
            "risk" => loaded.config.security_verification = SecurityPolicyMode::Risk,
            "always" => loaded.config.security_verification = SecurityPolicyMode::Always,
            _ => {}
        }
    }
    loaded
}

fn package_scripts(cwd: &Path) -> Option<BTreeMap<String, String>> {
    let raw = fs::read_to_string(cwd.join("package.json")).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    let scripts = parsed.get("scripts")?.as_object()?;
    Some(
        scripts
            .iter()
            .filter_map(|(name, command)| {
                command
                    .as_str()
                    .map(|command| (name.clone(), command.to_string()))
            })
            .collect(),
    )
}

pub fn read_package_scripts(cwd: &Path) -> Vec<String> {
    package_scripts(cwd)
        .map(|scripts| scripts.keys().cloned().collect())
        .unwrap_or_default()
}

fn spec(name: &str, command: &str) -> VerifyCommandSpec {
    VerifyCommandSpec {
        name: name.to_string(),
        command: command.to_string(),
        from_plan: false,
    }
}

/// Deterministic project-shape detection. A Cargo workspace verifies through
/// cargo; an npm project through its scripts. No model is consulted.
pub fn detect_verify_commands(cwd: &Path) -> Vec<VerifyCommandSpec> {
    let mut commands = Vec::new();
    if cwd.join("Cargo.toml").is_file() {
        commands.push(spec("fmt", "cargo fmt --check"));
        commands.push(spec(
            "clippy",
            "cargo clippy --workspace --all-targets -- -D warnings",
        ));
        commands.push(spec("test", "cargo test --workspace"));
    }
    if let Some(scripts) = package_scripts(cwd) {
        if scripts.contains_key("check") {
            commands.push(spec("check", "npm run check"));
        } else {
            if scripts.contains_key("typecheck") {
                commands.push(spec("typecheck", "npm run typecheck"));
            }
            if scripts.contains_key("lint") {
                commands.push(spec("lint", "npm run lint"));
            }
        }
        if scripts.contains_key("test") {
            commands.push(spec("npm-test", "npm test"));
        }
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(dir: &Path, name: &str, content: &str) {
        if let Some(parent) = dir.join(name).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn a_missing_config_yields_unlimited_defaults_without_errors() {
        let dir = tempdir().unwrap();
        let loaded = load_config(dir.path());
        assert!(loaded.errors.is_empty());
        assert_eq!(loaded.config.budgets, GraphBudgets::default());
        assert_eq!(loaded.config.budgets.max_cost_usd, 0.0);
        assert_eq!(loaded.config.budgets.run_deadline_ms, 0);
    }

    #[test]
    fn a_project_can_opt_back_into_backstops() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            ".pi/graph.json",
            r#"{"budgets":{"maxCostUsd":15,"runDeadlineMs":5400000,"maxWorkers":12,
                "workerTimeoutMs":{"writer":2700000}},
                "models":{"reviewer":"openai/gpt"},
                "verifyCommands":[{"name":"test","command":"cargo test"}],
                "workerExtraTools":["retrieve_output"]}"#,
        );
        let loaded = load_config(dir.path());
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        assert_eq!(loaded.config.budgets.max_cost_usd, 15.0);
        assert_eq!(loaded.config.budgets.run_deadline_ms, 5_400_000);
        assert_eq!(loaded.config.budgets.max_workers, 12);
        assert_eq!(
            loaded.config.budgets.worker_timeout_ms.get(Role::Writer),
            2_700_000
        );
        assert_eq!(
            loaded.config.budgets.worker_timeout_ms.get(Role::Reviewer),
            0
        );
        assert_eq!(
            loaded
                .config
                .models
                .get(&Role::Reviewer)
                .map(String::as_str),
            Some("openai/gpt")
        );
        assert_eq!(loaded.config.verify_commands.len(), 1);
        assert_eq!(loaded.config.worker_extra_tools, vec!["retrieve_output"]);
    }

    #[test]
    fn an_invalid_config_is_reported_and_ignored() {
        let dir = tempdir().unwrap();
        write(dir.path(), ".pi/graph.json", "{not json");
        let loaded = load_config(dir.path());
        assert_eq!(loaded.errors.len(), 1);
        assert_eq!(loaded.config.budgets, GraphBudgets::default());
    }

    #[test]
    fn cargo_projects_verify_through_cargo() {
        let dir = tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[workspace]\n");
        let commands = detect_verify_commands(dir.path());
        let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["fmt", "clippy", "test"]);
    }

    #[test]
    fn npm_projects_prefer_a_single_check_script() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "package.json",
            r#"{"scripts":{"check":"biome","typecheck":"tsc","test":"vitest"}}"#,
        );
        let names: Vec<String> = detect_verify_commands(dir.path())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, vec!["check", "npm-test"]);
    }

    #[test]
    fn integer_budget_rejects_fractional_value() {
        assert!(parse_u64_budget(&serde_json::json!(3.7), "maxWorkers").is_err());
    }

    #[test]
    fn integer_budget_rejects_negative_value() {
        assert!(parse_u64_budget(&serde_json::json!(-1), "maxWorkers").is_err());
    }

    #[test]
    fn graph_security_verification_defaults_to_risk() {
        assert_eq!(
            GraphConfig::default().security_verification,
            SecurityPolicyMode::Risk
        );
    }

    #[test]
    fn graph_security_verification_loads_valid_values() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            ".pi/graph.json",
            r#"{"securityVerification": "always"}"#,
        );
        let loaded = load_config(dir.path());
        assert!(loaded.errors.is_empty());
        assert_eq!(
            loaded.config.security_verification,
            SecurityPolicyMode::Always
        );
    }

    #[test]
    fn graph_security_verification_malformed_reports_error_and_preserves_default() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            ".pi/graph.json",
            r#"{"securityVerification": "invalid_mode"}"#,
        );
        let loaded = load_config(dir.path());
        assert!(!loaded.errors.is_empty());
        assert_eq!(
            loaded.config.security_verification,
            SecurityPolicyMode::Risk
        );
    }
}
