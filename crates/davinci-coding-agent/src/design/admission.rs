//! Authority is captured from the live host, never from a browser or model DTO.
use super::error::{DesignError, DesignResult};
use davinci_agent::{Agent, PermissionState, PermissionVerdict, ToolContext};
use davinci_session::JsonlSession;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct AuthorizedDesignContext {
    session_id: String,
    workspace: PathBuf,
    workspace_id: String,
    tools: ToolContext,
    policy: Arc<PermissionState>,
    grant: Option<davinci_agent::host_operation::HostOperationGrant>,
    static_reads_allowed: bool,
}
impl AuthorizedDesignContext {
    /// Trusted SDK/CLI entry point. Never expose this constructor through RPC.
    pub fn from_agent(agent: &Agent, workspace: &Path) -> DesignResult<Self> {
        let session = agent
            .session
            .as_ref()
            .ok_or_else(|| DesignError::MissingCapability("persistent session required".into()))?;
        let workspace = workspace.canonicalize()?;
        if Path::new(&session.header.cwd).canonicalize()? != workspace {
            return Err(DesignError::Denied("session workspace mismatch".into()));
        }
        let workspace_id = format!(
            "{:x}",
            Sha256::digest(workspace.to_string_lossy().as_bytes())
        );
        Ok(Self {
            session_id: session.header.id.clone(),
            workspace,
            workspace_id,
            tools: agent.tool_context.clone(),
            policy: agent.permissions.clone(),
            grant: None,
            static_reads_allowed: !agent.named_file_hooks_active && agent.pre_tool.is_none(),
        })
    }
    pub(crate) fn for_operation(
        agent: &Agent,
        workspace: &Path,
        operation: &str,
        args: &Value,
    ) -> DesignResult<Self> {
        let mut context = Self::from_agent(agent, workspace)?;
        context.grant = Some(
            agent
                .authorize_host_operation(&context.workspace, operation, args)
                .map_err(DesignError::Denied)?,
        );
        Ok(context)
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
    pub fn tools(&self) -> &ToolContext {
        &self.tools
    }
    pub(crate) fn grant(&self) -> DesignResult<&davinci_agent::host_operation::HostOperationGrant> {
        self.grant
            .as_ref()
            .ok_or_else(|| DesignError::Denied("host operation grant required".into()))
    }
    /// Static extraction must not bypass per-file policy or invisible read hooks.
    pub fn check_static_read(&self, path: &Path) -> DesignResult<()> {
        if !self.static_reads_allowed {
            return Err(DesignError::Denied(
                "static reads are unavailable while read hooks are active".into(),
            ));
        }
        if self.tools.is_aborted() {
            return Err(DesignError::Cancelled);
        }
        let args = serde_json::json!({"path":path});
        if let Some(contract) = self
            .tools
            .active_contract
            .lock()
            .map_err(|_| DesignError::Denied("contract unavailable".into()))?
            .as_ref()
        {
            contract
                .check_call(&self.workspace, "read", &args)
                .map_err(|e| DesignError::Denied(e.to_string()))?;
        }
        let policy = self
            .policy
            .lock()
            .map_err(|_| DesignError::Denied("permission policy unavailable".into()))?;
        match policy.decide("design-static-read", "read", &args, &self.workspace) {
            PermissionVerdict::Allow => Ok(()),
            _ => Err(DesignError::Denied(
                "static source read is not allowed by current policy".into(),
            )),
        }
    }
    pub(crate) fn with_cancellation(mut self, flag: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.tools.abort = Some(flag);
        self
    }
    pub fn check(&self, operation: &str, args: &Value) -> DesignResult<()> {
        if self.tools.is_aborted() {
            return Err(DesignError::Cancelled);
        }
        if let Some(contract) = self
            .tools
            .active_contract
            .lock()
            .map_err(|_| DesignError::Denied("contract unavailable".into()))?
            .as_ref()
        {
            contract
                .check_call(&self.workspace, operation, args)
                .map_err(|e| DesignError::Denied(e.to_string()))?;
        }
        if let Some(grant) = &self.grant {
            return grant.check().map_err(DesignError::Denied);
        }
        let policy = self
            .policy
            .lock()
            .map_err(|_| DesignError::Denied("permission policy unavailable".into()))?;
        match policy.decide("design", operation, args, &self.workspace) {
            PermissionVerdict::Allow => Ok(()),
            PermissionVerdict::Deny { reason } => Err(DesignError::Denied(reason)),
            PermissionVerdict::Ask(_) => Err(DesignError::Denied(
                "design operation requires approval through the current host permission policy"
                    .into(),
            )),
        }
    }
    pub fn check_session(&self, session: &JsonlSession) -> DesignResult<()> {
        if session.header.id != self.session_id
            || Path::new(&session.header.cwd).canonicalize()? != self.workspace
        {
            return Err(DesignError::Denied("artifact owner mismatch".into()));
        }
        if self.tools.is_aborted() {
            return Err(DesignError::Cancelled);
        }
        if let Some(grant) = &self.grant {
            grant.check().map_err(DesignError::Denied)?;
        }
        Ok(())
    }
}
