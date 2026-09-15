//! Optional OpenCode CLI competitor adapter.

use std::path::{Path, PathBuf};

use super::command::{
    probe_supported_runner, resolve_runner_binary, CommandHarness, CompetitorRunner,
    ExternalHarness, ExternalRun, ExternalTask,
};
use super::probe::HarnessCapabilities;
use crate::behavior::{VerificationCommand, VerificationResult};

pub const OPENCODE_BIN_ENV: &str = "DAVINCI_OPENCODE_BIN";

#[derive(Debug, Clone)]
pub struct OpenCodeRunner {
    inner: CommandHarness,
}

impl OpenCodeRunner {
    pub fn new() -> Self {
        Self::with_binary(resolve_runner_binary(OPENCODE_BIN_ENV, "opencode"))
    }

    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            inner: CommandHarness::new("opencode", binary, vec!["run".into()]),
        }
    }

    pub fn binary(&self) -> &Path {
        &self.inner.binary
    }

    pub fn capabilities(&self) -> Result<HarnessCapabilities, String> {
        probe_supported_runner(self.name(), self.binary())
    }
}

impl Default for OpenCodeRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalHarness for OpenCodeRunner {
    fn name(&self) -> &str {
        "opencode"
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

impl CompetitorRunner for OpenCodeRunner {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_opencode_is_fail_closed_without_installation() {
        let runner = OpenCodeRunner::with_binary("davinci-missing-opencode-runner");
        assert!(!runner.available().unwrap());
    }

    #[test]
    fn opencode_uses_noninteractive_run_mode() {
        let runner = OpenCodeRunner::with_binary("opencode");
        assert_eq!(runner.inner.extra_args, vec!["run"]);
    }
}
