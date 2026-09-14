#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new, 1)


def add_tests() -> None:
    path = ROOT / "crates/davinci-agent/src/lib.rs"
    text = path.read_text()
    if "fn unrelated_successful_test_does_not_verify_mutation()" in text:
        return
    test = r'''

    #[test]
    fn unrelated_successful_test_does_not_verify_mutation() {
        let agent = Agent::new("x");
        agent.record_successful_mutation_paths(vec![PathBuf::from(
            "crates/davinci-agent/src/lib.rs",
        )]);
        agent.record_verification_command("cargo test -p unrelated-crate", true);
        assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
        let state = agent.mutation_verification_state();
        assert_eq!(
            state.latest_evidence.as_ref().map(|e| e.coverage),
            Some(VerificationCoverage::Unrelated)
        );
    }
'''
    idx = text.rfind("\n}")
    if idx < 0:
        raise SystemExit("cannot locate lib test module end")
    path.write_text(text[:idx] + test + text[idx:])


def apply_impl() -> None:
    lib_path = ROOT / "crates/davinci-agent/src/lib.rs"
    lib = lib_path.read_text()

    old_state = r'''/// Lifecycle evidence for mutations and the verification commands that follow
/// them. A verification attempt is associated with the current generation so
/// a later mutation cannot inherit an earlier success.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationVerificationState {
    pub mutation_generation: u64,
    pub verified_generation: Option<u64>,
    pub last_verification_succeeded: bool,
}
'''
    new_state = r'''#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCoverage {
    Targeted,
    Broad,
    Unrelated,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub generation: u64,
    pub command: String,
    pub succeeded: bool,
    pub mutation_paths: Vec<PathBuf>,
    pub verification_targets: Vec<String>,
    pub coverage: VerificationCoverage,
}

/// Lifecycle evidence for mutations and the verification commands that follow
/// them. A verification attempt is associated with the current generation so
/// a later mutation cannot inherit an earlier success.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationVerificationState {
    pub mutation_generation: u64,
    pub verified_generation: Option<u64>,
    pub last_verification_succeeded: bool,
    #[serde(default)]
    pub mutation_paths: Vec<PathBuf>,
    #[serde(default)]
    pub latest_evidence: Option<VerificationEvidence>,
}
'''
    lib = replace_once(lib, old_state, new_state, "verification state types")

    old_completion = r'''        match state.verified_generation {
            Some(generation) if generation == state.mutation_generation => {
                if state.last_verification_succeeded {
                    CompletionEvidence::Verified
                } else {
                    CompletionEvidence::VerificationFailed
                }
            }
            _ => CompletionEvidence::Unverified,
        }
    }

    /// Invalidate any earlier verification after a successful mutation.
    pub(crate) fn record_successful_mutation(&self) {
        let mut state = self
            .mutation_verification
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        state.mutation_generation = state.mutation_generation.saturating_add(1);
        state.verified_generation = None;
        state.last_verification_succeeded = false;
    }

    /// Record the result of a recognized verification command for the current
    /// mutation generation. The generation is retained for failed attempts so
    /// completion can distinguish failure from no verification at all.
    pub(crate) fn record_verification_result(&self, succeeded: bool) {
        let mut state = self
            .mutation_verification
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        state.verified_generation = Some(state.mutation_generation);
        state.last_verification_succeeded = succeeded;
    }
'''
    new_completion = r'''        match state.latest_evidence.as_ref() {
            Some(evidence) if evidence.generation == state.mutation_generation => {
                if !evidence.succeeded {
                    CompletionEvidence::VerificationFailed
                } else if matches!(
                    evidence.coverage,
                    VerificationCoverage::Targeted | VerificationCoverage::Broad
                ) {
                    CompletionEvidence::Verified
                } else {
                    CompletionEvidence::Unverified
                }
            }
            _ => CompletionEvidence::Unverified,
        }
    }

    /// Compatibility path for mutation sources that cannot yet provide a path.
    pub(crate) fn record_successful_mutation(&self) {
        self.record_successful_mutation_paths(Vec::new());
    }

    pub(crate) fn record_successful_mutation_paths(&self, paths: Vec<PathBuf>) {
        let mut state = self
            .mutation_verification
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let prior_was_verified = state.verified_generation == Some(state.mutation_generation)
            && state.last_verification_succeeded;
        if prior_was_verified {
            state.mutation_paths.clear();
        }
        state.mutation_generation = state.mutation_generation.saturating_add(1);
        for path in paths {
            if !state.mutation_paths.contains(&path) {
                state.mutation_paths.push(path);
            }
        }
        state.verified_generation = None;
        state.last_verification_succeeded = false;
        state.latest_evidence = None;
    }

    /// Compatibility entry point: a caller with no command/target information
    /// records broad evidence so existing explicit verification APIs retain
    /// their prior meaning. Live shell verification uses the scoped method.
    pub(crate) fn record_verification_result(&self, succeeded: bool) {
        let mut state = self
            .mutation_verification
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let generation = state.mutation_generation;
        let paths = state.mutation_paths.clone();
        state.verified_generation = Some(generation);
        state.last_verification_succeeded = succeeded;
        state.latest_evidence = Some(VerificationEvidence {
            generation,
            command: "legacy_explicit_verifier".into(),
            succeeded,
            mutation_paths: paths,
            verification_targets: Vec::new(),
            coverage: VerificationCoverage::Broad,
        });
    }

    pub(crate) fn record_verification_command(&self, command: &str, succeeded: bool) {
        let mut state = self
            .mutation_verification
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let generation = state.mutation_generation;
        let mutation_paths = state.mutation_paths.clone();
        let (coverage, verification_targets) =
            verification_coverage_for_command(command, &mutation_paths);
        state.verified_generation = Some(generation);
        state.last_verification_succeeded = succeeded;
        state.latest_evidence = Some(VerificationEvidence {
            generation,
            command: command.to_string(),
            succeeded,
            mutation_paths,
            verification_targets,
            coverage,
        });
    }
'''
    lib = replace_once(lib, old_completion, new_completion, "verification lifecycle methods")

    helper_marker = "\n#[derive(Debug, Clone, Default)]\npub struct TreeNavigateResult"
    helpers = r'''

fn command_package_target(command: &str) -> Option<String> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        if matches!(*part, "-p" | "--package") {
            return parts.get(index + 1).map(|value| value.trim_matches('"').to_string());
        }
        if let Some(value) = part.strip_prefix("--package=") {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

fn mutation_package(path: &Path) -> Option<String> {
    let parts = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    parts
        .windows(2)
        .find(|pair| pair[0] == "crates")
        .map(|pair| pair[1].to_string())
}

fn verification_coverage_for_command(
    command: &str,
    mutation_paths: &[PathBuf],
) -> (VerificationCoverage, Vec<String>) {
    if mutation_paths.is_empty() {
        return (VerificationCoverage::Broad, Vec::new());
    }
    let lower = command.to_ascii_lowercase();
    if lower.contains("--workspace")
        || lower.contains("cargo fmt --")
        || (lower.starts_with("cargo test") && command_package_target(command).is_none())
        || (lower.starts_with("cargo clippy") && command_package_target(command).is_none())
        || (lower.starts_with("cargo check") && command_package_target(command).is_none())
    {
        return (VerificationCoverage::Broad, vec!["workspace".into()]);
    }
    if let Some(package) = command_package_target(command) {
        let changed_packages = mutation_paths
            .iter()
            .filter_map(|path| mutation_package(path))
            .collect::<std::collections::BTreeSet<_>>();
        let targets = vec![package.clone()];
        if !changed_packages.is_empty() && changed_packages.iter().all(|changed| changed == &package) {
            return (VerificationCoverage::Targeted, targets);
        }
        return (VerificationCoverage::Unrelated, targets);
    }
    (VerificationCoverage::Unknown, Vec::new())
}
'''
    if "fn verification_coverage_for_command(" not in lib:
        lib = replace_once(lib, helper_marker, helpers + helper_marker, "verification helper insertion")
    lib_path.write_text(lib)

    turn_path = ROOT / "crates/davinci-agent/src/turn.rs"
    turn = turn_path.read_text()
    turn = replace_once(turn, "use std::path::Path;", "use std::path::{Path, PathBuf};", "turn path imports")
    turn = replace_once(
        turn,
        '''            if matches!(name, "write" | "edit" | "apply_patch" | "notebook_edit")
                && !pre_hook_error
                && !result.is_error
            {
                self.record_successful_mutation();
            }
            if matches!(name, "bash" | "powershell" | "exec_command") {
                let cmd = args
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if is_verification_command(cmd) {
                    self.record_verification_result(!pre_hook_error && !result.is_error);
                }
            }
''',
        '''            if matches!(name, "write" | "edit" | "apply_patch" | "notebook_edit")
                && !pre_hook_error
                && !result.is_error
            {
                self.record_successful_mutation_paths(mutation_paths_from_tool(name, args));
            }
            if matches!(name, "bash" | "powershell" | "exec_command") {
                let cmd = args
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if is_verification_command(cmd) {
                    self.record_verification_command(cmd, !pre_hook_error && !result.is_error);
                }
            }
''',
        "turn verification recording",
    )
    marker = "\npub(crate) fn is_verification_command(cmd: &str) -> bool {"
    helper = r'''

pub(crate) fn mutation_paths_from_tool(name: &str, args: &Value) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for key in ["path", "file_path", "notebook_path"] {
        if let Some(value) = args.get(key).and_then(Value::as_str) {
            let path = PathBuf::from(value);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    if name == "apply_patch" {
        if let Some(patch) = args
            .get("patch")
            .or_else(|| args.get("input"))
            .and_then(Value::as_str)
        {
            for line in patch.lines() {
                for prefix in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
                    if let Some(path) = line.strip_prefix(prefix) {
                        let path = PathBuf::from(path.trim());
                        if !paths.contains(&path) {
                            paths.push(path);
                        }
                    }
                }
            }
        }
    }
    paths
}
'''
    if "pub(crate) fn mutation_paths_from_tool" not in turn:
        turn = replace_once(turn, marker, helper + marker, "mutation path helper")
    turn_path.write_text(turn)

    batch_path = ROOT / "crates/davinci-agent/src/batch.rs"
    batch = batch_path.read_text()
    batch = replace_once(
        batch,
        '''                            agent.record_successful_mutation();
                        }
                        if matches!(tool.as_str(), "bash" | "powershell" | "exec_command") {
                            let command = args
                                .get("command")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            if crate::turn::is_verification_command(command) {
                                agent.record_verification_result(
                                    !pre_hook_error && !result.is_error,
                                );
                            }
                        }
''',
        '''                            agent.record_successful_mutation_paths(
                                crate::turn::mutation_paths_from_tool(&tool, &args),
                            );
                        }
                        if matches!(tool.as_str(), "bash" | "powershell" | "exec_command") {
                            let command = args
                                .get("command")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            if crate::turn::is_verification_command(command) {
                                agent.record_verification_command(
                                    command,
                                    !pre_hook_error && !result.is_error,
                                );
                            }
                        }
''',
        "batch verification recording",
    )
    batch_path.write_text(batch)


def main() -> None:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--tests", action="store_true")
    mode.add_argument("--impl", action="store_true")
    args = parser.parse_args()
    if args.tests:
        add_tests()
    else:
        apply_impl()


if __name__ == "__main__":
    main()
