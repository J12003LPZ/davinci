//! Shared Codemode contracts. Authority-bearing context is host-created only.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum CodeModeMode {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "read-only")]
    ReadOnly,
    #[serde(rename = "controlled")]
    Controlled,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeModeRequest {
    pub code: String,
    pub timeout_ms: Option<u64>,
    pub max_output_bytes: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum CodeModeStatus {
    Completed,
    Partial,
    Failed,
    Cancelled,
    RecoveryRequired,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeModeIdentity {
    pub invocation_id: String,
    pub session_id: Option<String>,
    pub branch_leaf: Option<String>,
    pub workspace_binding: String,
    pub runtime_run_id: String,
    pub parent_operation_ref: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeModeLimits {
    pub script_bytes: usize,
    pub wall_ms: u64,
    pub vm_heap_bytes: usize,
    pub tool_calls: u32,
    pub parallelism: usize,
    pub pending_calls: usize,
    pub argument_bytes: usize,
    pub json_depth: usize,
    pub child_result_bytes: usize,
    pub total_result_bytes: usize,
    pub frame_bytes: usize,
    pub output_bytes: usize,
    pub collected_output_bytes: usize,
    pub metadata_calls: u32,
    pub metadata_bytes: usize,
    pub cleanup_grace_ms: u64,
}
// Host-created only: never Deserialize this context from model or IPC input.
#[derive(Clone)]
pub struct CodeModeRunContext {
    pub identity: CodeModeIdentity,
    pub mode: CodeModeMode,
    pub limits: CodeModeLimits,
    pub capability_revision: String,
    pub cancellation: crate::runtime::CancellationToken,
    pub root_budget: Option<crate::runtime::capacity::RootBudget>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeModeCall {
    pub request_id: u32,
    pub tool: String,
    pub args: serde_json::Value,
}
// Serialize the sole variant as the fixed string "tool-output".
// Only the Rust host constructs artifact references.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum CodeModeArtifactKind {
    #[serde(rename = "tool-output")]
    ToolOutput,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeModeArtifact {
    /// Evidence-store path issued by the Rust host. Retrieval goes through the
    /// ordinary `read` tool, so its permission check applies on every access.
    pub id: String,
    pub kind: CodeModeArtifactKind,
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeModeToolValue {
    pub text: String,
    pub structured_content: Option<serde_json::Value>,
    pub complete: bool,
    pub artifact: Option<CodeModeArtifact>,
    pub operation_ref: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum CodeModeChildStatus {
    Succeeded,
    Failed,
    NotStarted,
    Cancelled,
    RecoveryRequired,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeModeChildOutcome {
    pub request_id: u32,
    pub ordinal: u32,
    pub tool: String,
    pub operation_ref: String,
    pub status: CodeModeChildStatus,
    pub error: Option<CodeModeError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeModeOutcome {
    pub status: CodeModeStatus,
    pub script_completed: bool,
    pub output_text: String,
    pub output_complete: bool,
    pub output_artifact: Option<CodeModeArtifact>,
    pub children: Vec<CodeModeChildOutcome>,
    pub host_notes: Vec<String>,
    pub operation_ref: String,
    pub error: Option<CodeModeError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeModeError {
    pub code: String,
    pub message: String,
    pub operation_ref: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolQuery {
    pub query: String,
    pub limit: usize,
    pub cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolMetadata {
    pub canonical_name: String,
    pub js_name: String,
    pub description: String,
    pub read_only: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolPage {
    pub tools: Vec<ToolMetadata>,
    pub total: usize,
    pub cursor: Option<String>,
}
pub trait CodeModeBroker: Send + Sync {
    fn search(&self, query: ToolQuery) -> Result<ToolPage, CodeModeError>;
    fn describe(&self, canonical_name: &str) -> Result<serde_json::Value, CodeModeError>;
    fn call(&self, call: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError>;
}
pub trait CodeModeHost: Send + Sync {
    /// The broker is owned so a host can return at its deadline while a
    /// non-cooperative child callback is still running on another thread.
    fn execute(
        &self,
        request: &CodeModeRequest,
        context: &CodeModeRunContext,
        broker: std::sync::Arc<dyn CodeModeBroker>,
    ) -> CodeModeOutcome;
}
