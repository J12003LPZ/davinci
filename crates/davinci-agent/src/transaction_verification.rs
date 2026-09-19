//! Bind transaction observations to host-captured foreground command receipts.
use crate::runtime::transactions::{
    tools::ToolTransaction, MutationAuthority, TransactionCoordinator, VerificationObservation,
};
use crate::{Agent, PermissionVerdict};
use std::path::Path;

#[derive(Debug)]
pub(crate) struct Pending {
    transaction_id: String,
    coordinator: TransactionCoordinator,
    observation: VerificationObservation,
    authority: MutationAuthority,
    command: String,
}

// Conservative bootstrap until verification plans supply explicit coverage.
// Reject shell composition, target exclusion and filters rather than guessing.
pub(crate) fn workspace_command(command: &str) -> bool {
    let words: Vec<_> = command.split_whitespace().collect();
    words.len() >= 3
        && words[0] == "cargo"
        && matches!(words[1], "check" | "test" | "clippy")
        && words[2..].contains(&"--workspace")
        && words[2..].iter().all(|word| {
            matches!(
                *word,
                "--workspace"
                    | "--message-format=json"
                    | "--offline"
                    | "--locked"
                    | "--quiet"
                    | "--all-targets"
                    | "--all-features"
            )
        })
}

impl Agent {
    pub(crate) fn begin_transaction_verification(
        &self,
        cwd: &Path,
        id: &str,
        name: &str,
        args: &serde_json::Value,
        context: &crate::ToolContext,
    ) {
        if !matches!(name, "bash" | "powershell" | "exec_command") {
            return;
        }
        let Some(command) = args.get("command").and_then(|value| value.as_str()) else {
            return;
        };
        if !workspace_command(command) {
            return;
        }
        let Ok(manager) = ToolTransaction::new(cwd, context) else {
            return;
        };
        let coordinator = manager.coordinator();
        let Ok(ids) = coordinator.verification_candidates() else {
            return;
        };
        let policy = self.permissions.clone();
        let contract = context.active_contract.clone();
        let root = cwd.to_path_buf();
        let authority = MutationAuthority::new(move |path| {
            let args = serde_json::json!({"path":path});
            if let Some(contract) = contract
                .lock()
                .map_err(|_| "contract lock poisoned")?
                .as_ref()
            {
                contract
                    .check_call(&root, "read", &args)
                    .map_err(|e| e.to_string())?;
            }
            match policy
                .lock()
                .map_err(|_| "permission lock poisoned")?
                .decide("transaction-verification", "read", &args, &root)
            {
                PermissionVerdict::Allow => Ok(()),
                _ => Err("transaction verification requires current read authority".into()),
            }
        });
        let mut observations = Vec::new();
        for transaction_id in ids {
            if let Ok(observation) =
                coordinator.begin_verification(&transaction_id, &|path| authority.check(path))
            {
                observations.push(Pending {
                    transaction_id,
                    coordinator: coordinator.clone(),
                    observation,
                    authority: authority.clone(),
                    command: command.into(),
                });
            }
        }
        if observations.is_empty() {
            return;
        }
        if let Ok(mut pending) = self.pending_transaction_verification.lock() {
            // Saturation drops optional evidence, never authorizes an unobserved source.
            if pending.len() < 128 {
                pending.insert(id.into(), observations);
            }
        }
    }

    pub(crate) fn finish_transaction_verification(
        &self,
        id: &str,
        passed: bool,
    ) -> Vec<serde_json::Value> {
        let observations = self
            .pending_transaction_verification
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(id))
            .unwrap_or_default();
        if observations.is_empty() {
            return Vec::new();
        }
        let receipt = self.command_receipts.lock().ok().and_then(|receipts| {
            receipts
                .iter()
                .find(|receipt| receipt.operation_id == id)
                .cloned()
        });
        let receipt = receipt.filter(|receipt| passed && receipt.is_passed());
        let mut outcomes = Vec::new();
        for pending in observations {
            let result = match receipt
                .as_ref()
                .filter(|receipt| receipt.argv == [pending.command.clone()]
                    && pending.observation.covered_by_compiler_roots(&receipt.compiler_source_roots))
            {
                Some(receipt) => pending
                    .coordinator
                    .finish_verification(pending.observation, receipt.clone(), &|path| {
                        pending.authority.check(path)
                    })
                    .map(|_| ()),
                None => Err("no successful matching command receipt with observed source coverage after hooks".into()),
            };
            outcomes.push(
                serde_json::json!({"id":pending.transaction_id, "verified":result.is_ok(),
                "reason":result.err()}),
            );
        }
        outcomes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_composition_and_narrowed_workspace_commands() {
        assert!(workspace_command("cargo check --workspace --offline"));
        for command in [
            "echo cargo check --workspace",
            "cargo check --workspace; exit 0",
            "cargo test --workspace --exclude other",
            "cargo test --workspace one_test",
            "cargo check -p one",
            "cargo check --workspace || true",
        ] {
            assert!(!workspace_command(command), "{command}");
        }
    }
}
