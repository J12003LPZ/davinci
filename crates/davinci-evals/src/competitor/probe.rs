//! Capability probing for external harness CLIs.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Output};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessCapabilities {
    pub version: Option<String>,
    pub supports_model_flag: bool,
    pub supports_permission_mode: bool,
    pub supports_structured_output: bool,
    pub supports_system_prompt_override: bool,
}

pub fn probe_harness(binary: &Path) -> Result<HarnessCapabilities, String> {
    let version = run_probe(binary, "--version")?;
    let help = run_probe(binary, "--help")?;
    Ok(parse_capabilities(
        &combined_output(&version),
        &combined_output(&help),
    ))
}

fn run_probe(binary: &Path, argument: &str) -> Result<Output, String> {
    Command::new(binary)
        .arg(argument)
        // Version/help probing must not expose evaluator credentials to an
        // untrusted or user-selected executable.
        .env_clear()
        .output()
        .map_err(|error| format!("failed to probe {} {argument}: {error}", binary.display()))
}

fn combined_output(output: &Output) -> String {
    let mut bytes = output.stdout.clone();
    bytes.extend_from_slice(&output.stderr);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn parse_capabilities(version_output: &str, help_output: &str) -> HarnessCapabilities {
    let version = version_output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned);
    let help = help_output.to_ascii_lowercase();
    HarnessCapabilities {
        version,
        supports_model_flag: help.contains("--model"),
        supports_permission_mode: help.contains("--permission-mode")
            || help.contains("--dangerously-skip-permissions"),
        supports_structured_output: help.contains("--output-format")
            || help.contains("--json")
            || help.contains("json output"),
        supports_system_prompt_override: help.contains("--system-prompt"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_version_and_help_are_parsed_without_guessing_unsupported_flags() {
        let capabilities = parse_capabilities(
            "Claude Code 1.2.3\n",
            "Usage: claude [options]\n  --model <name>\n  --permission-mode <mode>\n  --output-format json\n  --system-prompt <text>\n",
        );
        assert_eq!(capabilities.version.as_deref(), Some("Claude Code 1.2.3"));
        assert!(capabilities.supports_model_flag);
        assert!(capabilities.supports_permission_mode);
        assert!(capabilities.supports_structured_output);
        assert!(capabilities.supports_system_prompt_override);
    }

    #[test]
    fn missing_help_flags_are_false() {
        let capabilities = parse_capabilities("unknown", "Usage: harness [request]");
        assert_eq!(capabilities.version.as_deref(), Some("unknown"));
        assert!(!capabilities.supports_model_flag);
        assert!(!capabilities.supports_permission_mode);
        assert!(!capabilities.supports_structured_output);
        assert!(!capabilities.supports_system_prompt_override);
    }
}
