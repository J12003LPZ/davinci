//! Declarative terminal and UI interaction scenarios.

use super::runner::DeterministicRunner;
use super::terminal::{PtyConfig, SimulatedTerminalSession};
use super::{InteractionReceipt, InteractionScenario, InteractionStep, InteractionTarget};

/// Returns the standard 5-mode permission cycling scenario.
pub fn scenario_five_mode_cycle() -> InteractionScenario {
    InteractionScenario {
        id: "terminal_five_mode_cycle".into(),
        target: InteractionTarget::Terminal,
        executable_or_url: "davinci".into(),
        source_manifest: Some("tests/manifests/permission_cycle.json".into()),
        steps: vec![
            InteractionStep::Launch,
            InteractionStep::TypeText("Fix #42: add 🚀 unicode draft".into()),
            InteractionStep::Key("shift_tab".into()), // -> Accept Edits
            InteractionStep::Key("shift_tab".into()), // -> Plan Mode
            InteractionStep::Key("shift_tab".into()), // -> Auto Mode
            InteractionStep::Key("shift_tab".into()), // -> Always Approve
            InteractionStep::Key("shift_tab".into()), // -> Manual
            InteractionStep::AssertScreen {
                pattern: "Fix #42: add 🚀 unicode draft".into(),
            },
            InteractionStep::Key("tab".into()), // Normal tab preserved
            InteractionStep::Capture,
            InteractionStep::Shutdown,
        ],
        time_budget_ms: 5000,
        allowed_origins: Vec::new(),
        artifact_policy: "capture_frames".into(),
    }
}

/// Returns a narrow terminal (40x12) interaction scenario.
pub fn scenario_narrow_terminal() -> InteractionScenario {
    InteractionScenario {
        id: "terminal_narrow_40x12".into(),
        target: InteractionTarget::Terminal,
        executable_or_url: "davinci".into(),
        source_manifest: None,
        steps: vec![
            InteractionStep::Launch,
            InteractionStep::Resize { cols: 40, rows: 12 },
            InteractionStep::TypeText("git status".into()),
            InteractionStep::Key("shift_tab".into()),
            InteractionStep::AssertScreen {
                pattern: "git status".into(),
            },
            InteractionStep::Shutdown,
        ],
        time_budget_ms: 3000,
        allowed_origins: Vec::new(),
        artifact_policy: "minimal".into(),
    }
}

/// Returns a draft preservation scenario with slash command and newline.
pub fn scenario_draft_preservation() -> InteractionScenario {
    InteractionScenario {
        id: "terminal_draft_preservation".into(),
        target: InteractionTarget::Terminal,
        executable_or_url: "davinci".into(),
        source_manifest: None,
        steps: vec![
            InteractionStep::Launch,
            InteractionStep::TypeText("/model sonnet\ncontinuation prompt 🎯".into()),
            InteractionStep::Key("shift_tab".into()),
            InteractionStep::AssertScreen {
                pattern: "continuation prompt 🎯".into(),
            },
            InteractionStep::Key("shift_tab".into()),
            InteractionStep::AssertScreen {
                pattern: "/model sonnet".into(),
            },
            InteractionStep::Shutdown,
        ],
        time_budget_ms: 4000,
        allowed_origins: Vec::new(),
        artifact_policy: "capture_frames".into(),
    }
}

/// Runs a scenario in a simulated terminal environment and returns the receipt.
pub fn execute_simulated_scenario(
    scenario: InteractionScenario,
    cols: u16,
    rows: u16,
) -> Result<InteractionReceipt, String> {
    let cfg = PtyConfig {
        cols,
        rows,
        ..Default::default()
    };
    let mut session = SimulatedTerminalSession::new(cfg, 3001)?;
    let mut runner = DeterministicRunner::new(scenario);
    runner.run_terminal(&mut session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f11_run_five_mode_cycle_scenario() {
        let scenario = scenario_five_mode_cycle();
        let receipt = execute_simulated_scenario(scenario, 80, 24).unwrap();

        assert!(receipt.assertions_passed);
        assert_eq!(receipt.scenario_id, "terminal_five_mode_cycle");
        assert!(receipt.frames_count > 0);
        assert!(receipt.assertions.iter().any(|a| a.contains("PASS")));
    }

    #[test]
    fn f11_run_narrow_terminal_scenario() {
        let scenario = scenario_narrow_terminal();
        let receipt = execute_simulated_scenario(scenario, 40, 12).unwrap();

        assert!(receipt.assertions_passed);
        assert_eq!(receipt.scenario_id, "terminal_narrow_40x12");
    }

    #[test]
    fn f11_run_draft_preservation_scenario() {
        let scenario = scenario_draft_preservation();
        let receipt = execute_simulated_scenario(scenario, 80, 24).unwrap();

        assert!(receipt.assertions_passed);
        assert_eq!(receipt.scenario_id, "terminal_draft_preservation");
    }
}
