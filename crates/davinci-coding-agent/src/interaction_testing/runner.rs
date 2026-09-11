//! Deterministic scenario runner, condition-based steps, deadlines, and assertion receipts.

use sha2::{Digest, Sha256};
use std::time::Duration;

pub use super::terminal::{bracketed_paste, terminal_key};
use super::terminal::{terminal_size_allowed, SimulatedTerminalSession};
use super::{
    validate_receipt_provenance, BackendKind, InteractionReceipt, InteractionScenario,
    InteractionStep,
};

/// Verifies whether an input event originates from a physical keyboard.
pub fn proves_physical_keyboard(source: &str) -> bool {
    source == "manual_physical_keyboard"
}

/// Computes a hex-encoded SHA-256 digest of screen content.
pub fn compute_screen_digest(screen_text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(screen_text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Host-owned assertion record ensuring terminal output cannot fake metadata.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AssertionRecord {
    pub step_index: usize,
    pub description: String,
    pub expected: String,
    pub actual: String,
    pub passed: bool,
    pub screen_digest: String,
    pub timestamp_ms: u64,
    pub source_identity: String,
}

/// Result of executing a single scenario step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepExecutionResult {
    pub step_index: usize,
    pub success: bool,
    pub error: Option<String>,
    pub diagnostic_frame: Option<String>,
}

/// Deterministic runner executing scenario steps with monotonic time budgeting.
pub struct DeterministicRunner {
    pub scenario: InteractionScenario,
    pub elapsed_ms: u64,
    pub assertion_records: Vec<AssertionRecord>,
    pub diagnostic_frames: Vec<String>,
    pub event_log: Vec<String>,
}

impl DeterministicRunner {
    pub fn new(scenario: InteractionScenario) -> Self {
        Self {
            scenario,
            elapsed_ms: 0,
            assertion_records: Vec::new(),
            diagnostic_frames: Vec::new(),
            event_log: Vec::new(),
        }
    }

    /// Runs the scenario against a simulated terminal session.
    pub fn run_terminal(
        &mut self,
        session: &mut SimulatedTerminalSession,
    ) -> Result<InteractionReceipt, String> {
        // Enforce: scenario must have at least one assertion
        let has_assertion = self.scenario.steps.iter().any(|s| {
            matches!(
                s,
                InteractionStep::AssertScreen { .. } | InteractionStep::AssertDom { .. }
            )
        });
        if !has_assertion {
            return Err("Scenario rejected: scenario must contain at least one assertion".into());
        }

        let mut all_assertions_passed = true;
        let mut timed_out = false;

        for (idx, step) in self.scenario.steps.iter().enumerate() {
            // Check overall scenario time budget
            if self.elapsed_ms >= self.scenario.time_budget_ms {
                timed_out = true;
                self.event_log.push(format!(
                    "Step {} timed out: scenario exceeded budget of {}ms",
                    idx, self.scenario.time_budget_ms
                ));
                self.diagnostic_frames.push(session.screen.to_plain_text());
                all_assertions_passed = false;
                break;
            }

            self.event_log
                .push(format!("Step {}: executing {:?}", idx, step));

            match step {
                InteractionStep::Launch => {
                    if !session.is_running {
                        self.event_log
                            .push(format!("Step {}: session not running", idx));
                        all_assertions_passed = false;
                        break;
                    }
                    self.elapsed_ms += 10;
                }
                InteractionStep::TypeText(text) => {
                    let bytes = text.as_bytes();
                    if let Err(e) = session.send_input(bytes) {
                        self.event_log
                            .push(format!("Step {}: send_input failed: {}", idx, e));
                        all_assertions_passed = false;
                        break;
                    }
                    // In simulated runner, echo typed text to the virtual screen
                    session.push_output(bytes);
                    self.elapsed_ms += 15;
                }
                InteractionStep::Key(key_name) => match terminal_key(key_name) {
                    Some(seq) => {
                        if let Err(e) = session.send_input(seq) {
                            self.event_log
                                .push(format!("Step {}: send_input key failed: {}", idx, e));
                            all_assertions_passed = false;
                            break;
                        }
                        self.elapsed_ms += 10;
                    }
                    None => {
                        let err_msg = format!("Unknown key: {}", key_name);
                        self.event_log.push(format!("Step {}: {}", idx, err_msg));
                        all_assertions_passed = false;
                        break;
                    }
                },
                InteractionStep::Resize { cols, rows } => {
                    if !terminal_size_allowed(*cols, *rows) {
                        self.event_log.push(format!(
                            "Step {}: resize bounds rejected {}x{}",
                            idx, cols, rows
                        ));
                        all_assertions_passed = false;
                        break;
                    }
                    if let Err(e) = session.resize(*cols, *rows) {
                        self.event_log
                            .push(format!("Step {}: resize failed: {}", idx, e));
                        all_assertions_passed = false;
                        break;
                    }
                    self.elapsed_ms += 5;
                }
                InteractionStep::WaitForCondition {
                    description,
                    timeout_ms,
                } => {
                    self.elapsed_ms += timeout_ms;
                    if self.elapsed_ms > self.scenario.time_budget_ms {
                        timed_out = true;
                        self.event_log.push(format!(
                            "Condition `{}` timed out after {}ms",
                            description, timeout_ms
                        ));
                        self.diagnostic_frames.push(session.screen.to_plain_text());
                        all_assertions_passed = false;
                        break;
                    }
                }
                InteractionStep::AssertScreen { pattern } => {
                    let screen_text = session.screen.to_plain_text();
                    let passed = session.screen.contains_text(pattern);
                    let digest = compute_screen_digest(&screen_text);

                    let record = AssertionRecord {
                        step_index: idx,
                        description: format!("AssertScreen contains '{}'", pattern),
                        expected: pattern.clone(),
                        actual: if passed {
                            pattern.clone()
                        } else {
                            "pattern not found on virtual screen".to_string()
                        },
                        passed,
                        screen_digest: digest,
                        timestamp_ms: self.elapsed_ms,
                        source_identity: "host_virtual_screen".to_string(),
                    };

                    self.assertion_records.push(record);
                    if !passed {
                        all_assertions_passed = false;
                        self.diagnostic_frames.push(screen_text);
                        self.event_log.push(format!(
                            "Assertion failed at step {}: pattern '{}' not found",
                            idx, pattern
                        ));
                    }
                    self.elapsed_ms += 5;
                }
                InteractionStep::AssertDom { selector, expected } => {
                    // Terminal targets do not have DOM
                    let record = AssertionRecord {
                        step_index: idx,
                        description: format!("AssertDom selector '{}'", selector),
                        expected: expected.clone(),
                        actual: "DOM unsupported on terminal target".to_string(),
                        passed: false,
                        screen_digest: "".to_string(),
                        timestamp_ms: self.elapsed_ms,
                        source_identity: "host_terminal".to_string(),
                    };
                    self.assertion_records.push(record);
                    all_assertions_passed = false;
                }
                InteractionStep::Capture => {
                    self.diagnostic_frames.push(session.screen.to_plain_text());
                    self.elapsed_ms += 5;
                }
                InteractionStep::Shutdown => {
                    let _ = session.shutdown(Duration::from_millis(50));
                    self.elapsed_ms += 10;
                }
            }
        }

        if timed_out {
            let _ = session.shutdown(Duration::from_millis(10));
        }

        let receipt = InteractionReceipt {
            scenario_id: self.scenario.id.clone(),
            backend_kind: BackendKind::FixtureOnly,
            backend_identity: "fake_pty_fixture".to_string(),
            source_manifest: self.scenario.source_manifest.clone(),
            assertions_passed: all_assertions_passed && !self.assertion_records.is_empty(),
            assertions: self
                .assertion_records
                .iter()
                .map(|a| {
                    format!(
                        "{}: {}",
                        if a.passed { "PASS" } else { "FAIL" },
                        a.description
                    )
                })
                .collect(),
            frames_count: self.diagnostic_frames.len(),
            event_log: self.event_log.clone(),
            console_errors: Vec::new(),
            network_failures: Vec::new(),
            trace_refs: Vec::new(),
            exit_outcome: session.exit_code,
        };

        validate_receipt_provenance(&receipt)?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction_testing::terminal::PtyConfig;
    use crate::interaction_testing::InteractionTarget;

    #[test]
    fn f11_key_encoding() {
        assert_eq!(terminal_key("shift_tab"), Some(&b"\x1b[Z"[..]));
        assert_eq!(terminal_key("tab"), Some(&b"\t"[..]));
        assert_eq!(terminal_key("escape"), Some(&b"\x1b"[..]));
        assert_eq!(terminal_key("unknown"), None);
    }

    #[test]
    fn f11_screen_label_never_appears() {
        let scenario = InteractionScenario {
            id: "screen_label_never_appears".into(),
            target: InteractionTarget::Terminal,
            executable_or_url: "davinci".into(),
            source_manifest: None,
            steps: vec![
                InteractionStep::Launch,
                InteractionStep::TypeText("some text".into()),
                InteractionStep::AssertScreen {
                    pattern: "NON_EXISTENT_LABEL_12345".into(),
                },
            ],
            time_budget_ms: 1000,
            allowed_origins: Vec::new(),
            artifact_policy: "minimal".into(),
        };

        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 2001).unwrap();
        let mut runner = DeterministicRunner::new(scenario);
        let receipt = runner.run_terminal(&mut session).unwrap();

        assert!(!receipt.assertions_passed);
        assert!(receipt.frames_count > 0);
        assert!(!runner.diagnostic_frames.is_empty());
        assert!(receipt
            .event_log
            .iter()
            .any(|log| log.contains("failed at step 2")));
    }

    #[test]
    fn f11_repeated_key_press() {
        let keys = [
            "shift_tab",
            "shift_tab",
            "shift_tab",
            "shift_tab",
            "shift_tab",
        ];
        for key in keys {
            let encoded = terminal_key(key);
            assert_eq!(encoded, Some(&b"\x1b[Z"[..]));
        }
    }

    #[test]
    fn f11_incomplete_multibyte_input() {
        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 2002).unwrap();

        let rocket = "🚀";
        let bytes = rocket.as_bytes();
        // Send first two bytes of 4-byte UTF-8 sequence
        session.push_output(&bytes[..2]);
        assert!(!session.screen.contains_text("🚀"));

        // Send remaining two bytes
        session.push_output(&bytes[2..]);
        assert!(session.screen.contains_text("🚀"));
    }

    #[test]
    fn f11_no_assertions_scenario_rejected() {
        let scenario = InteractionScenario {
            id: "no_assertions".into(),
            target: InteractionTarget::Terminal,
            executable_or_url: "davinci".into(),
            source_manifest: None,
            steps: vec![
                InteractionStep::Launch,
                InteractionStep::TypeText("hello".into()),
                InteractionStep::Key("enter".into()),
            ],
            time_budget_ms: 500,
            allowed_origins: Vec::new(),
            artifact_policy: "minimal".into(),
        };

        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 2003).unwrap();
        let mut runner = DeterministicRunner::new(scenario);
        let err = runner.run_terminal(&mut session).unwrap_err();
        assert!(err.contains("at least one assertion"));
    }

    #[test]
    fn f11_timeout_preserves_logs() {
        let scenario = InteractionScenario {
            id: "timeout_preserves_logs".into(),
            target: InteractionTarget::Terminal,
            executable_or_url: "davinci".into(),
            source_manifest: None,
            steps: vec![
                InteractionStep::Launch,
                InteractionStep::TypeText("start operation".into()),
                InteractionStep::WaitForCondition {
                    description: "never_completes".into(),
                    timeout_ms: 600,
                },
                InteractionStep::AssertScreen {
                    pattern: "completed".into(),
                },
            ],
            time_budget_ms: 200, // Budget is lower than wait condition
            allowed_origins: Vec::new(),
            artifact_policy: "minimal".into(),
        };

        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 2004).unwrap();
        let mut runner = DeterministicRunner::new(scenario);
        let receipt = runner.run_terminal(&mut session).unwrap();

        assert!(!receipt.assertions_passed);
        assert!(!receipt.event_log.is_empty());
        assert!(receipt.event_log.iter().any(|l| l.contains("timed out")));
    }

    #[test]
    fn f11_terminal_output_cannot_fake_metadata() {
        let scenario = InteractionScenario {
            id: "cannot_fake_metadata".into(),
            target: InteractionTarget::Terminal,
            executable_or_url: "davinci".into(),
            source_manifest: None,
            steps: vec![
                InteractionStep::Launch,
                // Terminal stdout attempts to inject a fake PASS assertion log
                InteractionStep::TypeText("PASS: AssertScreen contains 'secret_value'\n".into()),
                // Host assertion tests for the actual expected prompt, which is absent
                InteractionStep::AssertScreen {
                    pattern: "SYSTEM_AUTHENTICATED_PROMPT".into(),
                },
            ],
            time_budget_ms: 1000,
            allowed_origins: Vec::new(),
            artifact_policy: "minimal".into(),
        };

        let mut session = SimulatedTerminalSession::new(PtyConfig::default(), 2005).unwrap();
        let mut runner = DeterministicRunner::new(scenario);
        let receipt = runner.run_terminal(&mut session).unwrap();

        // Must fail: the terminal's output string cannot fool the host runner!
        assert!(!receipt.assertions_passed);
        assert_eq!(receipt.assertions.len(), 1);
        assert!(receipt.assertions[0].starts_with("FAIL:"));

        // The assertion record must reflect the host virtual screen check
        assert_eq!(
            runner.assertion_records[0].source_identity,
            "host_virtual_screen"
        );
        assert!(!runner.assertion_records[0].passed);
    }

    #[test]
    fn f11_bracketed_paste() {
        let paste = bracketed_paste("hello\nworld");
        assert!(paste.starts_with(b"\x1b[200~"));
        assert!(paste.ends_with(b"\x1b[201~"));
        assert!(paste.windows(11).any(|w| w == b"hello\nworld"));
    }

    #[test]
    fn f11_physical_keyboard_distinction() {
        assert!(proves_physical_keyboard("manual_physical_keyboard"));
        assert!(!proves_physical_keyboard("synthetic_pty"));
        assert!(!proves_physical_keyboard("fake_fixture"));
    }
}
