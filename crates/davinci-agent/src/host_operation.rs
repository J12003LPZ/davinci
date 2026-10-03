//! Reusable, non-serializable consent for one trusted host operation.
use crate::{Agent, PermissionState, PermissionVerdict, ToolContext};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct HostOperationGrant {
    policy: Arc<PermissionState>,
    permit: Option<Arc<crate::approval::DispatchPermit>>,
    context: ToolContext,
    cwd: PathBuf,
    id: String,
    name: String,
    args: Value,
}

impl HostOperationGrant {
    /// Recheck the exact consented action, cancellation and current contract.
    /// This is not a grant for a different action, argument set or workspace.
    pub fn check(&self) -> Result<(), String> {
        if self.context.is_aborted() {
            return Err("host operation cancelled".into());
        }
        if let Some(contract) = self
            .context
            .active_contract
            .lock()
            .map_err(|_| "contract unavailable")?
            .as_ref()
        {
            contract
                .check_call(&self.cwd, &self.name, &self.args)
                .map_err(|e| e.to_string())?;
        }
        let policy = self.policy.lock().map_err(|_| "policy unavailable")?;
        match policy.decide(&self.id, &self.name, &self.args, &self.cwd) {
            PermissionVerdict::Allow => Ok(()),
            PermissionVerdict::Deny { reason } => Err(reason),
            PermissionVerdict::Ask(_) => self
                .permit
                .as_ref()
                .ok_or("host operation requires approval")?
                .recheck(
                    &self.policy,
                    policy.revision(),
                    &self.cwd,
                    &self.name,
                    &self.args,
                )
                .map_err(str::to_owned),
        }
    }
}

impl Agent {
    /// One host-coordinated model turn through the existing admission and
    /// observation owners. Tool calls are returned as data; this method never
    /// dispatches tools or starts a second agent/session writer.
    pub fn complete_host_operation<F>(
        &mut self,
        grant: &HostOperationGrant,
        purpose: &str,
        complete: F,
    ) -> Result<crate::CompleteOutput, String>
    where
        F: FnOnce(&Agent) -> Result<crate::CompleteOutput, String>,
    {
        self.ensure_session_persistence()?;
        grant.check()?;
        if self.abort_requested() {
            return Err("host model request cancelled".into());
        }
        let budget = self
            .root_budget()
            .ok_or("host model operation requires existing root admission")?;
        let before = budget.snapshot()?;
        if before.unknown > 0 || before.pending > 0 || before.halted {
            return Err("root accounting is unresolved".into());
        }
        let started = std::time::Instant::now();
        let observations = self.provider_observation_scope(purpose);
        let result = complete(self);
        let events = observations.finish(if result.is_ok() {
            "completed"
        } else {
            "failed"
        });
        for observation in events {
            self.stats.note_provider_observation(&observation);
            if result.is_ok()
                && observation.kind == "logical_end"
                && observation.requested_service_tier.as_deref() == Some("priority")
                && observation.service_tier_honored() == Some(false)
            {
                self.emit_live(crate::AgentEvent::CompletionNotice {
                    reason_code: "service_tier_downgrade".into(),
                    text: format!("Fast was requested, but the backend served this host operation at {} tier.",
                        observation.returned_service_tier.as_deref().unwrap_or_default()),
                });
            }
            self.emit_live(crate::AgentEvent::ProviderObservation {
                observation: Box::new(observation),
            });
        }
        self.stats.model_turns += 1;
        self.stats.model_wall_ms += started.elapsed().as_millis() as u64;
        let after = self
            .root_budget()
            .ok_or("root budget removed")?
            .snapshot()?;
        if after.requests <= before.requests
            || after.pending > 0
            || after.unknown > 0
            || after.halted
        {
            return Err(
                "host model request did not produce settled root admission evidence".into(),
            );
        }
        grant.check()?;
        let output = result?;
        if !matches!(
            output.message.stop_reason,
            Some(davinci_ai::StopReason::Stop | davinci_ai::StopReason::ToolUse)
        ) {
            return Err(
                "host model response is incomplete; partial tool calls were not applied".into(),
            );
        }
        Ok(output)
    }

    /// Admit a host-owned action through the same approval owner as tool calls.
    /// Callers must retain this exact grant and recheck before publishing effects.
    pub fn authorize_host_operation(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
    ) -> Result<HostOperationGrant, String> {
        self.ensure_session_persistence()?;
        let id = format!("host_{}", uuid::Uuid::new_v4());
        self.check_contract_gate(cwd, &id, name, args)
            .map_err(|e| e.to_string())?;
        if let Some(reason) = self.host_permission_denial(cwd, &id, name, args) {
            return Err(reason);
        }
        let permit = self.approval_registry.take_dispatch(&id);
        if let Some(permit) = &permit {
            let policy = self.permissions.lock().map_err(|_| "policy unavailable")?;
            if let PermissionVerdict::Deny { reason } = policy.decide(&id, name, args, cwd) {
                return Err(reason);
            }
            permit.consume(&self.permissions, policy.revision(), cwd, name, args)?;
        }
        let mut context = self.tool_context.clone();
        context.abort = self
            .runtime
            .as_ref()
            .map(|r| r.cancellation_token.as_atomic_bool())
            .or_else(|| self.abort_signal.clone());
        let grant = HostOperationGrant {
            policy: self.permissions.clone(),
            permit,
            context,
            cwd: cwd.to_path_buf(),
            id,
            name: name.into(),
            args: args.clone(),
        };
        grant.check()?;
        Ok(grant)
    }
}
