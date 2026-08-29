use crate::tools::AgentTool;
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    AllowSession,
    Deny,
}

#[async_trait]
pub trait PermissionPolicy: Send + Sync {
    async fn check_permission(
        &self,
        tool: &dyn AgentTool,
        tool_call_id: &str,
        arguments: &serde_json::Value,
    ) -> PermissionDecision;
}

pub struct AllowAllPermissionPolicy;

#[async_trait]
impl PermissionPolicy for AllowAllPermissionPolicy {
    async fn check_permission(
        &self,
        _tool: &dyn AgentTool,
        _tool_call_id: &str,
        _arguments: &serde_json::Value,
    ) -> PermissionDecision {
        PermissionDecision::Allow
    }
}
