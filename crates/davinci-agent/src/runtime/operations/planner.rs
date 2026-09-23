use super::{
    CallerType, EffectClass, EffectProfile, IdempotencyScope, ModelError, OperationContext,
    OperationKind, OperationSpec, PayloadDigest, Precondition, PreconditionKind,
    ScopedIdempotencyKey,
};
use crate::runtime::{
    conservative_replay_policy, CapabilitySource, DeclaredEffect, ReplayPolicy, RuntimeCapability,
};
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum ToolOperationPlanError {
    #[error("tool name must not be empty")]
    EmptyToolName,
    #[error("batch child index must be one-based")]
    InvalidBatchChildIndex,
    #[error(transparent)]
    Identity(#[from] super::IdentityError),
    #[error(transparent)]
    Model(#[from] ModelError),
}

/// Pure translation of a trusted tool invocation into a durable operation intent.
#[derive(Debug, Clone)]
pub struct PlannedToolOperation {
    spec: OperationSpec,
    replay_policy: ReplayPolicy,
}

impl PlannedToolOperation {
    pub fn spec(&self) -> &OperationSpec {
        &self.spec
    }

    pub fn into_spec(self) -> OperationSpec {
        self.spec
    }

    pub fn replay_policy(&self) -> ReplayPolicy {
        self.replay_policy
    }
}

pub struct ToolOperationPlanner;

impl ToolOperationPlanner {
    /// Plan a durable child execution. The payload contains the stable logical
    /// identity and child context, while the actual prompt or command remains
    /// owned by the host adapter.
    pub fn managed_execution(
        mut context: OperationContext,
        caller: CallerType,
        scope: IdempotencyScope,
        logical_id: &str,
        kind: OperationKind,
        effects: EffectProfile,
        payload: Value,
    ) -> Result<PlannedToolOperation, ToolOperationPlanError> {
        let key = ScopedIdempotencyKey::new(scope, logical_id.to_owned())?;
        context.caller = caller;
        context.wire_tool_call_id = Some(logical_id.to_owned());
        let spec = OperationSpec::new(context, key, kind, effects, payload, Vec::new())?;
        Ok(PlannedToolOperation {
            spec,
            replay_policy: ReplayPolicy::NeverAutoReplay,
        })
    }

    pub fn provider_call(
        mut context: OperationContext,
        call_id: &str,
        tool: &str,
        args: &Value,
        capability: Option<&RuntimeCapability>,
        contract_digest: Option<&str>,
    ) -> Result<PlannedToolOperation, ToolOperationPlanError> {
        if tool.trim().is_empty() {
            return Err(ToolOperationPlanError::EmptyToolName);
        }
        context.caller = CallerType::ProviderToolCall;
        context.wire_tool_call_id = Some(call_id.to_owned());
        let key = ScopedIdempotencyKey::new(
            IdempotencyScope::ProviderCall,
            provider_key(&context.session_id, call_id),
        )?;
        Self::plan(context, key, tool, args, capability, contract_digest)
    }

    pub fn batch_child(
        mut context: OperationContext,
        parent_operation_id: super::OperationId,
        child_index: usize,
        tool: &str,
        args: &Value,
        capability: Option<&RuntimeCapability>,
        contract_digest: Option<&str>,
    ) -> Result<PlannedToolOperation, ToolOperationPlanError> {
        if child_index == 0 {
            return Err(ToolOperationPlanError::InvalidBatchChildIndex);
        }
        if tool.trim().is_empty() {
            return Err(ToolOperationPlanError::EmptyToolName);
        }
        context.caller = CallerType::BatchToolCall;
        context.parent_operation_id = Some(parent_operation_id);
        context.wire_tool_call_id = Some(format!("{parent_operation_id}#{child_index}"));
        let key = ScopedIdempotencyKey::new(
            IdempotencyScope::BatchChild,
            format!("{parent_operation_id}:{child_index}"),
        )?;
        Self::plan(context, key, tool, args, capability, contract_digest)
    }

    fn plan(
        context: OperationContext,
        key: ScopedIdempotencyKey,
        tool: &str,
        args: &Value,
        capability: Option<&RuntimeCapability>,
        contract_digest: Option<&str>,
    ) -> Result<PlannedToolOperation, ToolOperationPlanError> {
        let effects = effect_profile(capability);
        let replay_policy = match capability {
            Some(capability) if capability.source == CapabilitySource::Builtin => {
                capability.replay_policy
            }
            _ => conservative_replay_policy(tool),
        };
        let mut preconditions = Vec::new();
        if let Some(digest) = contract_digest {
            preconditions.push(Precondition {
                kind: PreconditionKind::Custom,
                resource: "task_contract".to_owned(),
                expected_digest: Some(PayloadDigest::of_bytes(digest.as_bytes())),
            });
        }
        let payload = json!({
            "tool": tool,
            "arguments": args,
        });
        let spec = OperationSpec::new(
            context,
            key,
            operation_kind(tool),
            effects,
            payload,
            preconditions,
        )?;
        Ok(PlannedToolOperation {
            spec,
            replay_policy,
        })
    }
}

fn provider_key(session_id: &str, call_id: &str) -> String {
    format!(
        "{}:{session_id}{}:{call_id}",
        session_id.len(),
        call_id.len()
    )
}

fn operation_kind(tool: &str) -> OperationKind {
    match tool {
        "bash" | "powershell" | "exec_command" => OperationKind::ShellCommand,
        "write" | "apply_patch" | "notebook_edit" => OperationKind::FileWrite,
        "edit" => OperationKind::FileEdit,
        "delete" => OperationKind::FileDelete,
        "move" => OperationKind::FileMove,
        name if name.starts_with("browser_") => OperationKind::BrowserAction,
        name if name.starts_with("mcp_") => OperationKind::McpCall,
        _ => OperationKind::ToolInvocation,
    }
}

fn effect_profile(capability: Option<&RuntimeCapability>) -> EffectProfile {
    let Some(capability) = capability.filter(|capability| {
        capability.source == CapabilitySource::Builtin && !capability.declared_effects.is_empty()
    }) else {
        return EffectProfile {
            classification: EffectClass::ExternalMutation,
            supports_idempotency_key: false,
            supports_postcondition_probe: false,
            supports_compensation: false,
            requires_live_owner: true,
        };
    };

    let classification = if capability.read_only
        && capability.declared_effects.iter().all(|effect| {
            matches!(
                effect,
                DeclaredEffect::FileSystemRead | DeclaredEffect::McpRead
            )
        }) {
        EffectClass::ReadOnly
    } else if capability
        .declared_effects
        .contains(&DeclaredEffect::ProcessExecution)
    {
        EffectClass::ProcessMutation
    } else if capability
        .declared_effects
        .contains(&DeclaredEffect::ExternalServicePublish)
        || capability
            .declared_effects
            .contains(&DeclaredEffect::NetworkAccess)
        || capability
            .declared_effects
            .contains(&DeclaredEffect::McpMutation)
    {
        EffectClass::ExternalMutation
    } else if capability
        .declared_effects
        .contains(&DeclaredEffect::FileSystemWrite)
    {
        EffectClass::ReversibleMutation
    } else if capability
        .declared_effects
        .contains(&DeclaredEffect::HostInteraction)
    {
        EffectClass::IrreversibleMutation
    } else {
        EffectClass::ExternalMutation
    };

    EffectProfile {
        classification,
        supports_idempotency_key: false,
        supports_postcondition_probe: false,
        supports_compensation: false,
        requires_live_owner: true,
    }
}
