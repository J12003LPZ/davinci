//! Optional Hermes CLI competitor adapter.

use std::path::{Path, PathBuf};

use super::command::{
    probe_supported_runner, resolve_runner_binary, CommandHarness, CompetitorRunner,
    ExternalHarness, ExternalRun, ExternalTask,
};
use super::probe::HarnessCapabilities;
use crate::behavior::{VerificationCommand, VerificationResult};

pub const HERMES_BIN_ENV: &str = "DAVINCI_HERMES_BIN";

#[derive(Debug, Clone)]
pub struct HermesRunner {
    inner: CommandHarness,
}

impl HermesRunner {
    pub fn new() -> Self {
        Self::with_binary(resolve_runner_binary(HERMES_BIN_ENV, "hermes"))
    }

    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            inner: CommandHarness::new("hermes", binary, vec!["-z".into()]),
        }
    }

    pub fn binary(&self) -> &Path {
        &self.inner.binary
    }

    pub fn capabilities(&self) -> Result<HarnessCapabilities, String> {
        probe_supported_runner(self.name(), self.binary())
    }
}

impl Default for HermesRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalHarness for HermesRunner {
    fn name(&self) -> &str {
        "hermes"
    }

    fn available(&self) -> Result<bool, String> {
        Ok(self.capabilities().is_ok())
    }

    fn run(&self, task: &ExternalTask) -> Result<ExternalRun, String> {
        self.capabilities()?;
        self.inner.run(task)
    }

    fn run_with_verification(
        &self,
        task: &ExternalTask,
        verification_commands: &[VerificationCommand],
    ) -> Result<(ExternalRun, Vec<VerificationResult>), String> {
        self.capabilities()?;
        self.inner
            .run_with_verification(task, verification_commands)
    }
}

impl CompetitorRunner for HermesRunner {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_hermes_is_fail_closed_without_installation() {
        let runner = HermesRunner::with_binary("davinci-missing-hermes-runner");
        assert!(!runner.available().unwrap());
    }

    #[test]
    fn hermes_uses_one_shot_mode() {
        let runner = HermesRunner::with_binary("hermes");
        assert_eq!(runner.inner.extra_args, vec!["-z"]);
    }
}
