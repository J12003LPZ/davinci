//! Optional Codex CLI competitor adapter.

use std::path::{Path, PathBuf};

use super::command::{
    probe_supported_runner, resolve_runner_binary, CommandHarness, CompetitorRunner,
    ExternalHarness, ExternalRun, ExternalTask,
};
use super::probe::HarnessCapabilities;
use crate::behavior::{VerificationCommand, VerificationResult};

pub const CODEX_BIN_ENV: &str = "DAVINCI_CODEX_BIN";

#[derive(Debug, Clone)]
pub struct CodexRunner {
    inner: CommandHarness,
}

impl CodexRunner {
    pub fn new() -> Self {
        Self::with_binary(resolve_runner_binary(CODEX_BIN_ENV, "codex"))
    }

    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            inner: CommandHarness::new("codex", binary, vec!["exec".into()]),
        }
    }

    pub fn binary(&self) -> &Path {
        &self.inner.binary
    }

    pub fn capabilities(&self) -> Result<HarnessCapabilities, String> {
        probe_supported_runner(self.name(), self.binary())
    }
}

impl Default for CodexRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalHarness for CodexRunner {
    fn name(&self) -> &str {
        "codex"
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

impl CompetitorRunner for CodexRunner {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_codex_is_fail_closed_without_installation() {
        let runner = CodexRunner::with_binary("davinci-missing-codex-runner");
        assert!(!runner.available().unwrap());
        assert!(runner.run(&test_task()).is_err());
    }

    #[test]
    fn codex_uses_noninteractive_exec_mode() {
        let runner = CodexRunner::with_binary("codex");
        assert_eq!(runner.inner.extra_args, vec!["exec"]);
    }

    fn test_task() -> ExternalTask {
        ExternalTask {
            repo_path: tempfile::tempdir().unwrap().path().to_path_buf(),
            request: "inspect".into(),
            timeout: std::time::Duration::from_secs(1),
            artifact_dir: std::env::temp_dir().join("davinci-codex-test-artifacts"),
            environment: Default::default(),
            ignore_paths: Vec::new(),
        }
    }
}
