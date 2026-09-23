//! Host-owned process observations, separate from model-visible tool JSON.
use crate::runtime::{evidence_store::ExecutionReceipt, TaskId};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct CommandReceiptCapture {
    operation_id: String,
    tool_name: String,
    task_id: Option<TaskId>,
    process_operation_binding: Option<crate::runtime::operations::ProcessOperationBinding>,
    receipt: Arc<Mutex<Option<ExecutionReceipt>>>,
}

impl CommandReceiptCapture {
    pub(crate) fn new(operation_id: &str, tool_name: &str, task_id: Option<TaskId>) -> Self {
        Self {
            operation_id: operation_id.into(),
            tool_name: tool_name.into(),
            task_id,
            process_operation_binding: None,
            receipt: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn with_process_operation_binding(
        mut self,
        binding: crate::runtime::operations::ProcessOperationBinding,
    ) -> Self {
        self.process_operation_binding = Some(binding);
        self
    }

    pub(crate) fn process_operation_binding(
        &self,
    ) -> Option<crate::runtime::operations::ProcessOperationBinding> {
        self.process_operation_binding.clone()
    }

    pub(crate) fn take(&self) -> Option<ExecutionReceipt> {
        self.receipt.lock().ok()?.take()
    }

    pub(crate) fn completed(
        &self,
        cwd: &std::path::Path,
        command: &str,
        started_at_ms: i64,
        output: &std::process::Output,
    ) {
        if command.len() > 16 * 1024 || self.operation_id.len() > 256 || self.tool_name.len() > 128
        {
            return;
        }
        // Evidence failure cannot turn command execution into an application failure.
        let Ok(cwd) = cwd.canonicalize() else { return };
        let Ok(mut slot) = self.receipt.lock() else {
            return;
        };
        let process_evidence = slot
            .as_ref()
            .and_then(|receipt| receipt.process_evidence.clone());
        *slot = Some(ExecutionReceipt {
            operation_id: self.operation_id.clone(),
            tool_name: self.tool_name.clone(),
            task_id: self.task_id,
            argv: vec![command.into()],
            compiler_source_roots: compiler_source_roots(&cwd, command, &output.stdout),
            cwd: cwd.to_string_lossy().into(),
            started: true,
            exit_code: output.status.code(),
            started_at_ms,
            finished_at_ms: now(),
            stdout_hash: Some(crate::runtime::checkpoints::compute_sha256(&output.stdout)),
            stderr_hash: Some(crate::runtime::checkpoints::compute_sha256(&output.stderr)),
            process_evidence,
            ..Default::default()
        });
    }

    pub(crate) fn process_observed(
        &self,
        evidence: crate::jobs::supervisor::ProcessExecutionEvidence,
        started_at_ms: i64,
        stdout: &[u8],
        stderr: &[u8],
    ) {
        if self.process_operation_binding.as_ref() != evidence.identity.operation.as_ref() {
            return;
        }
        let Ok(mut slot) = self.receipt.lock() else {
            return;
        };
        let started = matches!(
            evidence.launch_state,
            crate::jobs::supervisor::ProcessLaunchState::Started
                | crate::jobs::supervisor::ProcessLaunchState::Exited
                | crate::jobs::supervisor::ProcessLaunchState::Stopped
        );
        let mut receipt = slot.take().unwrap_or_else(|| ExecutionReceipt {
            operation_id: self.operation_id.clone(),
            task_id: self.task_id,
            tool_name: self.tool_name.clone(),
            argv: evidence.argv.clone(),
            cwd: evidence.cwd.clone(),
            started,
            started_at_ms,
            ..Default::default()
        });
        receipt.started |= started;
        receipt.exit_code = evidence.exit_code;
        receipt.started_at_ms = if receipt.started_at_ms == 0 {
            started_at_ms
        } else {
            receipt.started_at_ms.min(started_at_ms)
        };
        receipt.finished_at_ms = now();
        receipt.stdout_hash = Some(crate::runtime::checkpoints::compute_sha256(stdout));
        receipt.stderr_hash = Some(crate::runtime::checkpoints::compute_sha256(stderr));
        receipt.process_evidence = Some(evidence);
        *slot = Some(receipt);
    }
}

pub(crate) fn attach_to_tool_result(
    result: &mut crate::ToolResult,
    receipt: &ExecutionReceipt,
) -> Result<(), String> {
    let details = result.details.get_or_insert_with(|| serde_json::json!({}));
    if !details.is_object() {
        *details = serde_json::json!({});
    }
    details["_command_receipt"] = serde_json::to_value(receipt)
        .map_err(|error| format!("command receipt encoding failed: {error}"))?;
    Ok(())
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn compiler_source_roots(root: &std::path::Path, command: &str, stdout: &[u8]) -> Vec<String> {
    if !crate::transaction_verification::workspace_command(command)
        || !command
            .split_whitespace()
            .any(|word| word == "--message-format=json")
        || stdout.len() > 4 * 1024 * 1024
    {
        return Vec::new();
    }
    let root = crate::permission::strip_verbatim_prefix(root);
    let mut paths = std::collections::BTreeSet::new();
    for line in stdout.split(|byte| *byte == b'\n') {
        if line.len() > 128 * 1024 {
            return Vec::new();
        }
        let Ok(message) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        match message["reason"].as_str() {
            Some("build-finished") => {
                return if message["success"].as_bool() == Some(true) {
                    paths.into_iter().collect()
                } else {
                    Vec::new()
                }
            }
            Some("compiler-artifact") => {
                let Some(source) = message["target"]["src_path"].as_str() else {
                    continue;
                };
                if source.len() > 8192 {
                    return Vec::new();
                }
                let source = crate::permission::strip_verbatim_prefix(std::path::Path::new(source));
                let Ok(relative) = source.strip_prefix(&root) else {
                    continue;
                };
                if relative
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
                {
                    continue;
                }
                paths.insert(relative.to_string_lossy().replace('\\', "/"));
                if paths.len() > 128 || paths.iter().map(String::len).sum::<usize>() > 32 * 1024 {
                    return Vec::new();
                }
            }
            _ => {}
        }
    }
    Vec::new()
}

#[cfg(test)]
pub(crate) fn test_supervisor() -> crate::jobs::supervisor::SupervisorCommand {
    crate::jobs::supervisor::SupervisorCommand {
        executable: std::env::current_exe().unwrap(),
        argv: vec![
            "--exact".into(),
            "tools::foreground::tests::foreground_supervisor_fixture".into(),
            "--nocapture".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{execute_tool_with, ToolContext};

    #[test]
    fn unsupervised_commands_do_not_produce_verification_receipts() {
        let root = tempfile::tempdir().unwrap();
        for tool in ["exec_command", "powershell"] {
            if tool == "powershell" && !cfg!(windows) {
                continue;
            }
            let capture = CommandReceiptCapture::new("unowned-process", tool, None);
            let context = ToolContext {
                command_receipt: Some(capture.clone()),
                ..Default::default()
            };
            let result = execute_tool_with(
                root.path(),
                tool,
                &serde_json::json!({"command":"exit 0"}),
                &context,
            )
            .unwrap();
            assert!(!result.is_error, "{}", result.content);
            assert!(
                capture.take().is_none(),
                "{tool}: an unowned process cannot prove bounded verification"
            );
        }
    }

    #[test]
    fn compiler_roots_require_completion_and_cannot_escape_workspace() {
        let root = tempfile::tempdir().unwrap();
        let command = "cargo check --workspace --message-format=json";
        let artifact = |path: &std::path::Path| {
            serde_json::json!({"reason":"compiler-artifact", "target":{"src_path":path}})
                .to_string()
        };
        let source = artifact(&root.path().join("src/lib.rs"));
        let outside = artifact(&root.path().join("../other.rs"));
        assert!(compiler_source_roots(root.path(), command, source.as_bytes()).is_empty());
        let output =
            format!("{source}\n{outside}\n{{\"reason\":\"build-finished\",\"success\":true}}\n");
        assert_eq!(
            compiler_source_roots(root.path(), command, output.as_bytes()),
            ["src/lib.rs"]
        );
        let after_build = format!(
            "{output}{}\n",
            artifact(&root.path().join("src/not_compiled.rs"))
        );
        assert_eq!(
            compiler_source_roots(root.path(), command, after_build.as_bytes()),
            ["src/lib.rs"]
        );
        assert!(compiler_source_roots(
            root.path(),
            "echo cargo check --workspace --message-format=json",
            output.as_bytes()
        )
        .is_empty());
        assert!(compiler_source_roots(
            root.path(),
            command,
            output.replace("true", "false").as_bytes()
        )
        .is_empty());
        assert!(
            compiler_source_roots(root.path(), command, &vec![b'x'; 4 * 1024 * 1024 + 1])
                .is_empty()
        );
    }

    #[test]
    fn actual_command_populates_private_receipt_once() {
        let root = tempfile::tempdir().unwrap();
        for (command, expected) in [("exit 0", 0), ("exit 7", 7)] {
            let capture = CommandReceiptCapture::new("actual-process", "exec_command", None);
            let context = ToolContext {
                foreground_supervisor: Some(test_supervisor()),
                command_receipt: Some(capture.clone()),
                ..Default::default()
            };
            let result = execute_tool_with(
                root.path(),
                "exec_command",
                &serde_json::json!({"command":command}),
                &context,
            )
            .unwrap();
            assert_eq!(result.is_error, expected != 0);
            let receipt = capture
                .take()
                .expect("actual process must produce a receipt");
            assert_eq!(receipt.exit_code, Some(expected));
            assert_eq!(receipt.operation_id, "actual-process");
            assert_eq!(receipt.argv, [command]);
            assert!(receipt.started && receipt.started_at_ms > 0);
            assert!(receipt.finished_at_ms >= receipt.started_at_ms);
            assert!(receipt.stdout_hash.is_some() && receipt.stderr_hash.is_some());
            assert_eq!(receipt.is_passed(), expected == 0);
            assert!(capture.take().is_none());
        }
    }
}
