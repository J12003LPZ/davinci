//! Deterministic loop signals and progress watchdog for agent execution.
//! Matching spec: docs/superpowers/plans/2026-09-07-davinci-feature-specs/09-whole-task-budgets-loop-detection.md

use serde::{Deserialize, Serialize};

/// Maximum observations kept in the watchdog history per task.
pub const MAX_WATCHDOG_HISTORY: usize = 128;

/// Contract helper: detects repeated observations without progress.
pub fn repeat_without_progress(observations: &[(&str, &str, &str)], new_evidence: bool) -> bool {
    !new_evidence
        && observations.len() >= 3
        && observations[observations.len() - 3..]
            .windows(2)
            .all(|pair| pair[0] == pair[1])
}

/// Normalize non-semantic noise in command and test outputs (timestamps, durations, PIDs).
pub fn normalize_output_noise(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut cleaned = String::new();
        let mut words = trimmed.split_whitespace().peekable();
        while let Some(word) = words.next() {
            let leading_punct: String = word.chars().take_while(|c| !c.is_alphanumeric()).collect();
            let trailing_punct: String = word
                .chars()
                .rev()
                .take_while(|c| !c.is_alphanumeric())
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            let core = if word.len() > leading_punct.len() + trailing_punct.len() {
                &word[leading_punct.len()..word.len().saturating_sub(trailing_punct.len())]
            } else {
                word
            };

            // ISO 8601 timestamp or date e.g. 2026-09-09 or 2026-09-09T15:30:00Z
            if (core.len() == 10
                && core.chars().nth(4) == Some('-')
                && core.chars().nth(7) == Some('-'))
                || (core.len() >= 19 && core.contains('T') && core.contains(':'))
            {
                cleaned.push_str(&format!("{leading_punct}<TIMESTAMP>{trailing_punct} "));
            }
            // Durations like 0.45s, 12.3ms, 120ms
            else if (core.ends_with("ms") || core.ends_with('s'))
                && core[..core
                    .len()
                    .saturating_sub(if core.ends_with("ms") { 2 } else { 1 })]
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.')
            {
                cleaned.push_str(&format!("{leading_punct}<TIME>{trailing_punct} "));
            }
            // Process pid
            else if core.eq_ignore_ascii_case("pid") || core.eq_ignore_ascii_case("process") {
                cleaned.push_str(&format!("{leading_punct}{core}{trailing_punct} "));
                if let Some(next) = words.peek() {
                    let next_leading: String =
                        next.chars().take_while(|c| !c.is_alphanumeric()).collect();
                    let next_trailing: String = next
                        .chars()
                        .rev()
                        .take_while(|c| !c.is_alphanumeric())
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    let next_core = if next.len() > next_leading.len() + next_trailing.len() {
                        &next[next_leading.len()..next.len().saturating_sub(next_trailing.len())]
                    } else {
                        next
                    };
                    if !next_core.is_empty() && next_core.chars().all(|c| c.is_ascii_digit()) {
                        let _ = words.next();
                        cleaned.push_str(&format!("{next_leading}<PID>{next_trailing} "));
                    }
                }
            } else {
                cleaned.push_str(word);
                cleaned.push(' ');
            }
        }
        normalized.push_str(cleaned.trim_end());
        normalized.push('\n');
    }
    normalized.trim_end().to_string()
}

/// A structured observation recorded for watchdog loop detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressObservation {
    pub tool_name: String,
    pub input_fingerprint: String,
    pub output_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hypothesis_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypothesis_evidence_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_failure_signature: Option<String>,
    #[serde(default)]
    pub is_pruning_recovery: bool,
    #[serde(default)]
    pub is_job_poll: bool,
    #[serde(default)]
    pub has_new_evidence: bool,
}

/// Deterministic loop signals flagged by the watchdog state machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopSignal {
    RepeatedCommandWithoutNewOutput {
        tool: String,
        command_or_input: String,
        count: usize,
    },
    RepeatedUnchangedReads {
        path: String,
        count: usize,
    },
    OscillatingEdits {
        path: String,
        cycle_sequence: Vec<String>,
    },
    RepeatedTestFailureWithoutNewDiagnosis {
        signature: String,
        count: usize,
    },
}

impl LoopSignal {
    pub fn description(&self) -> String {
        match self {
            Self::RepeatedCommandWithoutNewOutput {
                tool,
                command_or_input,
                count,
            } => {
                format!("Repeated `{tool}` command with identical input and output across {count} attempts with no new diagnosis: {command_or_input}")
            }
            Self::RepeatedUnchangedReads { path, count } => {
                format!("Repeated read of unchanged file `{path}` {count} times without a new hypothesis.")
            }
            Self::OscillatingEdits { path, .. } => {
                format!("Oscillating edits detected on `{path}` reverting to previous state (A->B->A cycle).")
            }
            Self::RepeatedTestFailureWithoutNewDiagnosis { signature, count } => {
                format!("Same test failure observed across {count} attempts with no new diagnosis: {signature}")
            }
        }
    }
}

/// Lifecycle state of the progress watchdog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WatchdogState {
    #[default]
    Running,
    Paused(LoopSignal),
    PlanRequested,
    Stopped,
}

/// Progress watchdog state machine bounding task history and detecting loops.
#[derive(Debug, Clone, Default)]
pub struct ProgressWatchdog {
    max_history: usize,
    observations: Vec<ProgressObservation>,
    state: WatchdogState,
}

impl ProgressWatchdog {
    pub fn new() -> Self {
        Self {
            max_history: MAX_WATCHDOG_HISTORY,
            observations: Vec::with_capacity(MAX_WATCHDOG_HISTORY),
            state: WatchdogState::Running,
        }
    }

    pub fn observations(&self) -> &[ProgressObservation] {
        &self.observations
    }

    pub fn state(&self) -> &WatchdogState {
        &self.state
    }

    /// Check whether new tool/turn dispatch is permitted.
    pub fn can_dispatch(&self) -> bool {
        matches!(self.state, WatchdogState::Running)
    }

    /// Pause the watchdog with an identified loop signal.
    pub fn pause(&mut self, signal: LoopSignal) {
        self.state = WatchdogState::Paused(signal);
    }

    /// Reduce user/host choice (continue/plan/stop) against remaining budget.
    pub fn reduce_choice(&mut self, remaining_budget: u64, choice: &str) -> &'static str {
        let is_paused = matches!(self.state, WatchdogState::Paused(_));
        let decision = super::budget::budget_decision(remaining_budget, is_paused, choice);
        match decision {
            "continue" => {
                self.state = WatchdogState::Running;
            }
            "return_to_plan" => {
                self.state = WatchdogState::PlanRequested;
            }
            "stop_checkpoint" | "hard_stop" => {
                self.state = WatchdogState::Stopped;
            }
            _ => {}
        }
        decision
    }

    /// Record a finalized observation and evaluate deterministic loop signals.
    pub fn observe(&mut self, obs: ProgressObservation) -> Option<LoopSignal> {
        if self.observations.len() >= self.max_history {
            self.observations.remove(0);
        }
        self.observations.push(obs);
        let signal = self.evaluate();
        if let Some(ref s) = signal {
            self.state = WatchdogState::Paused(s.clone());
        }
        signal
    }

    /// Evaluate current history against loop detection rules.
    pub fn evaluate(&self) -> Option<LoopSignal> {
        if let Some(signal) = self.check_oscillating_edits() {
            return Some(signal);
        }
        if let Some(signal) = self.check_repeated_test_failures() {
            return Some(signal);
        }
        if let Some(signal) = self.check_repeated_reads() {
            return Some(signal);
        }
        if let Some(signal) = self.check_repeated_command() {
            return Some(signal);
        }
        None
    }

    fn check_oscillating_edits(&self) -> Option<LoopSignal> {
        let last = self.observations.last()?;
        let target_path = last.target_path.as_deref()?;
        let current_hash = last.edit_content_hash.as_deref()?;

        // Filter recent edits on the same target path
        let edits: Vec<&str> = self
            .observations
            .iter()
            .filter(|o| o.target_path.as_deref() == Some(target_path))
            .filter_map(|o| o.edit_content_hash.as_deref())
            .collect();

        if edits.len() >= 3 {
            let n = edits.len();
            let a1 = edits[n - 3];
            let b = edits[n - 2];
            let a2 = edits[n - 1];
            if a1 == a2 && a1 != b && a2 == current_hash {
                return Some(LoopSignal::OscillatingEdits {
                    path: target_path.to_string(),
                    cycle_sequence: vec![a1.to_string(), b.to_string(), a2.to_string()],
                });
            }
        }
        None
    }

    fn check_repeated_test_failures(&self) -> Option<LoopSignal> {
        let last = self.observations.last()?;
        let sig = last.test_failure_signature.as_deref()?;

        if self.observations.len() < 3 {
            return None;
        }

        let window = &self.observations[self.observations.len() - 3..];
        let all_same_sig = window
            .iter()
            .all(|o| o.test_failure_signature.as_deref() == Some(sig));
        if !all_same_sig {
            return None;
        }

        // If any observation has new evidence or a changed diagnosis/hypothesis, it's progress
        let has_new_progress = window.iter().any(|o| o.has_new_evidence);
        let hypotheses: Vec<Option<&str>> =
            window.iter().map(|o| o.hypothesis_id.as_deref()).collect();
        let changed_hypotheses = hypotheses[0] != hypotheses[1] || hypotheses[1] != hypotheses[2];

        if !has_new_progress && !changed_hypotheses {
            return Some(LoopSignal::RepeatedTestFailureWithoutNewDiagnosis {
                signature: sig.to_string(),
                count: 3,
            });
        }
        None
    }

    fn check_repeated_reads(&self) -> Option<LoopSignal> {
        let last = self.observations.last()?;
        if last.tool_name != "read" && last.tool_name != "read_file" {
            return None;
        }
        if last.is_pruning_recovery || last.has_new_evidence {
            return None;
        }

        if self.observations.len() < 3 {
            return None;
        }

        let window = &self.observations[self.observations.len() - 3..];
        let all_same_read = window.iter().all(|o| {
            (o.tool_name == "read" || o.tool_name == "read_file")
                && o.input_fingerprint == last.input_fingerprint
                && o.output_digest == last.output_digest
                && !o.is_pruning_recovery
                && !o.has_new_evidence
        });

        if all_same_read {
            let path = last
                .target_path
                .clone()
                .unwrap_or_else(|| last.input_fingerprint.clone());
            return Some(LoopSignal::RepeatedUnchangedReads { path, count: 3 });
        }
        None
    }

    fn check_repeated_command(&self) -> Option<LoopSignal> {
        if self.observations.len() < 3 {
            return None;
        }

        let last = self.observations.last()?;
        if last.is_job_poll || last.is_pruning_recovery || last.has_new_evidence {
            return None;
        }

        let window = &self.observations[self.observations.len() - 3..];
        let has_new_progress = window.iter().any(|o| o.has_new_evidence)
            || window.windows(2).any(|w| {
                w[0].hypothesis_id != w[1].hypothesis_id
                    && (!w[0].hypothesis_evidence_refs.is_empty()
                        || !w[1].hypothesis_evidence_refs.is_empty())
            });

        if has_new_progress {
            return None;
        }

        let all_same_command = window.iter().all(|o| {
            o.tool_name == last.tool_name
                && o.input_fingerprint == last.input_fingerprint
                && o.output_digest == last.output_digest
                && !o.is_job_poll
                && !o.is_pruning_recovery
                && !o.has_new_evidence
        });

        if all_same_command {
            return Some(LoopSignal::RepeatedCommandWithoutNewOutput {
                tool: last.tool_name.clone(),
                command_or_input: last.input_fingerprint.clone(),
                count: 3,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f09_repeated_work() {
        assert!(repeat_without_progress(
            &[("a", "i", "o"), ("a", "i", "o"), ("a", "i", "o")],
            false
        ));
        assert!(!repeat_without_progress(
            &[("a", "i", "o"), ("a", "j", "o"), ("a", "i", "o")],
            false
        ));
        assert!(!repeat_without_progress(&[("a", "i", "o"); 3], true));
    }

    #[test]
    fn test_oscillating_aba_edits() {
        let mut watchdog = ProgressWatchdog::new();

        watchdog.observe(ProgressObservation {
            tool_name: "edit".into(),
            input_fingerprint: "edit src/lib.rs".into(),
            output_digest: "ok".into(),
            hypothesis_id: None,
            hypothesis_evidence_refs: vec![],
            target_path: Some("src/lib.rs".into()),
            edit_content_hash: Some("hash_A".into()),
            test_failure_signature: None,
            is_pruning_recovery: false,
            is_job_poll: false,
            has_new_evidence: false,
        });

        watchdog.observe(ProgressObservation {
            tool_name: "edit".into(),
            input_fingerprint: "edit src/lib.rs".into(),
            output_digest: "ok".into(),
            hypothesis_id: None,
            hypothesis_evidence_refs: vec![],
            target_path: Some("src/lib.rs".into()),
            edit_content_hash: Some("hash_B".into()),
            test_failure_signature: None,
            is_pruning_recovery: false,
            is_job_poll: false,
            has_new_evidence: false,
        });

        // Reverting back to hash_A: oscillating edit cycle!
        let signal = watchdog.observe(ProgressObservation {
            tool_name: "edit".into(),
            input_fingerprint: "edit src/lib.rs".into(),
            output_digest: "ok".into(),
            hypothesis_id: None,
            hypothesis_evidence_refs: vec![],
            target_path: Some("src/lib.rs".into()),
            edit_content_hash: Some("hash_A".into()),
            test_failure_signature: None,
            is_pruning_recovery: false,
            is_job_poll: false,
            has_new_evidence: false,
        });

        assert_eq!(
            signal,
            Some(LoopSignal::OscillatingEdits {
                path: "src/lib.rs".into(),
                cycle_sequence: vec!["hash_A".into(), "hash_B".into(), "hash_A".into()],
            })
        );
    }

    #[test]
    fn test_new_source_despite_same_command() {
        let mut watchdog = ProgressWatchdog::new();

        for i in 0..3 {
            let signal = watchdog.observe(ProgressObservation {
                tool_name: "bash".into(),
                input_fingerprint: "cargo test".into(),
                output_digest: format!("digest_{i}"), // Output changes!
                hypothesis_id: None,
                hypothesis_evidence_refs: vec![],
                target_path: None,
                edit_content_hash: None,
                test_failure_signature: None,
                is_pruning_recovery: false,
                is_job_poll: false,
                has_new_evidence: false,
            });
            assert_eq!(signal, None);
        }
    }

    #[test]
    fn test_same_failure_new_experiment() {
        let mut watchdog = ProgressWatchdog::new();

        for i in 0..3 {
            let signal = watchdog.observe(ProgressObservation {
                tool_name: "bash".into(),
                input_fingerprint: "cargo test".into(),
                output_digest: "failed".into(),
                hypothesis_id: Some(format!("hypothesis_{i}")), // New diagnosis/experiment each time!
                hypothesis_evidence_refs: vec![format!("ev_{i}")],
                target_path: None,
                edit_content_hash: None,
                test_failure_signature: Some("assertion failed: x == y".into()),
                is_pruning_recovery: false,
                is_job_poll: false,
                has_new_evidence: false,
            });
            assert_eq!(signal, None);
        }
    }

    #[test]
    fn test_noisy_timestamps_normalization() {
        let out1 = "test failed at 2026-09-09T15:30:00Z in 0.45s (pid 12345)";
        let out2 = "test failed at 2026-09-09T15:30:05Z in 0.48s (pid 67890)";
        assert_eq!(normalize_output_noise(out1), normalize_output_noise(out2));
    }

    #[test]
    fn test_retrieval_after_pruning_is_exempt() {
        let mut watchdog = ProgressWatchdog::new();

        for _ in 0..5 {
            let signal = watchdog.observe(ProgressObservation {
                tool_name: "read".into(),
                input_fingerprint: "src/main.rs".into(),
                output_digest: "hash_main".into(),
                hypothesis_id: None,
                hypothesis_evidence_refs: vec![],
                target_path: Some("src/main.rs".into()),
                edit_content_hash: None,
                test_failure_signature: None,
                is_pruning_recovery: true, // Necessary recovery after pruning
                is_job_poll: false,
                has_new_evidence: false,
            });
            assert_eq!(signal, None);
        }
    }

    #[test]
    fn test_legitimately_polling_job_output() {
        let mut watchdog = ProgressWatchdog::new();

        for _ in 0..5 {
            let signal = watchdog.observe(ProgressObservation {
                tool_name: "job_output".into(),
                input_fingerprint: "job_42".into(),
                output_digest: "still running...".into(),
                hypothesis_id: None,
                hypothesis_evidence_refs: vec![],
                target_path: None,
                edit_content_hash: None,
                test_failure_signature: None,
                is_pruning_recovery: false,
                is_job_poll: true, // Legitimate poll of background process
                has_new_evidence: false,
            });
            assert_eq!(signal, None);
        }
    }

    #[test]
    fn test_long_read_only_research_with_new_evidence() {
        let mut watchdog = ProgressWatchdog::new();

        for i in 0..10 {
            let signal = watchdog.observe(ProgressObservation {
                tool_name: "read".into(),
                input_fingerprint: format!("docs/part_{i}.md"),
                output_digest: format!("content_{i}"),
                hypothesis_id: None,
                hypothesis_evidence_refs: vec![format!("ev_doc_{i}")],
                target_path: Some(format!("docs/part_{i}.md")),
                edit_content_hash: None,
                test_failure_signature: None,
                is_pruning_recovery: false,
                is_job_poll: false,
                has_new_evidence: true, // New evidence gathered during research
            });
            assert_eq!(signal, None);
        }
    }

    #[test]
    fn test_watchdog_pause_continue_plan_stop_reducer() {
        let mut watchdog = ProgressWatchdog::new();
        assert_eq!(*watchdog.state(), WatchdogState::Running);
        assert!(watchdog.can_dispatch());

        // Repeated command triggers pause
        for _ in 0..3 {
            watchdog.observe(ProgressObservation {
                tool_name: "bash".into(),
                input_fingerprint: "cargo check".into(),
                output_digest: "same error".into(),
                hypothesis_id: None,
                hypothesis_evidence_refs: vec![],
                target_path: None,
                edit_content_hash: None,
                test_failure_signature: None,
                is_pruning_recovery: false,
                is_job_poll: false,
                has_new_evidence: false,
            });
        }

        assert!(matches!(watchdog.state(), WatchdogState::Paused(_)));
        assert!(!watchdog.can_dispatch());

        // Hard ceiling 0 tokens: Continue returns hard_stop and transitions to Stopped
        assert_eq!(watchdog.reduce_choice(0, "continue"), "hard_stop");
        assert_eq!(*watchdog.state(), WatchdogState::Stopped);
        assert!(!watchdog.can_dispatch());

        // Re-pause for testing other choices
        watchdog.pause(LoopSignal::RepeatedCommandWithoutNewOutput {
            tool: "bash".into(),
            command_or_input: "cargo check".into(),
            count: 3,
        });
        assert!(!watchdog.can_dispatch());

        // User chooses continue with remaining budget 50 -> transitions to Running
        assert_eq!(watchdog.reduce_choice(50, "continue"), "continue");
        assert_eq!(*watchdog.state(), WatchdogState::Running);
        assert!(watchdog.can_dispatch());

        // Pause again -> User chooses plan -> transitions to PlanRequested
        watchdog.pause(LoopSignal::RepeatedCommandWithoutNewOutput {
            tool: "bash".into(),
            command_or_input: "cargo check".into(),
            count: 3,
        });
        assert_eq!(watchdog.reduce_choice(50, "plan"), "return_to_plan");
        assert_eq!(*watchdog.state(), WatchdogState::PlanRequested);
        assert!(!watchdog.can_dispatch());

        // Pause again -> User chooses stop -> transitions to Stopped
        watchdog.pause(LoopSignal::RepeatedCommandWithoutNewOutput {
            tool: "bash".into(),
            command_or_input: "cargo check".into(),
            count: 3,
        });
        assert_eq!(watchdog.reduce_choice(50, "stop"), "stop_checkpoint");
        assert_eq!(*watchdog.state(), WatchdogState::Stopped);
        assert!(!watchdog.can_dispatch());
    }

    #[test]
    fn test_pause_while_writer_active_preserves_in_flight_recovery_and_stops_new_dispatch() {
        let mut watchdog = ProgressWatchdog::new();
        assert!(watchdog.can_dispatch());

        // While writer was executing, watchdog detects loop signal
        watchdog.pause(LoopSignal::OscillatingEdits {
            path: "src/main.rs".into(),
            cycle_sequence: vec!["a".into(), "b".into(), "a".into()],
        });

        // In-flight transaction may finish, but further dispatch is halted
        assert!(!watchdog.can_dispatch());
        assert!(matches!(watchdog.state(), WatchdogState::Paused(_)));
    }
}
