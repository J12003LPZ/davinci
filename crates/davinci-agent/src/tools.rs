use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::jobs::JobBook;
use crate::todo::TodoList;

mod foreground;

#[allow(dead_code)]
pub fn decision_wait(interactive: bool, deferred: bool, cancelled: bool) -> &'static str {
    if cancelled {
        "cancelled"
    } else if deferred {
        "deferred"
    } else if interactive {
        "wait_for_user"
    } else {
        "decision_required"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionHostReply {
    pub action: crate::decisions::HostDecisionAction,
    pub host_event_id: String,
    pub answered_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionHostResponse {
    Reply(DecisionHostReply),
    Cancelled,
    Timeout,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct DecisionHostRequest {
    pub question: crate::decisions::DecisionQuestion,
}

#[derive(Clone)]
pub struct DecisionResponder(
    Arc<dyn Fn(DecisionHostRequest) -> DecisionHostResponse + Send + Sync + 'static>,
);

impl std::fmt::Debug for DecisionResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DecisionResponder(..)")
    }
}

impl DecisionResponder {
    pub fn new(
        responder: impl Fn(DecisionHostRequest) -> DecisionHostResponse + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(responder))
    }

    pub fn respond(&self, request: DecisionHostRequest) -> DecisionHostResponse {
        (self.0)(request)
    }
}

pub const BUILTIN_TOOLS: &[&str] = &[
    "read",
    "retrieve_context",
    "write",
    "edit",
    "bash",
    "powershell",
    "grep",
    "find",
    "ls",
    "web_fetch",
    "web_search",
    "todo",
    "job_output",
    "job_kill",
    "process_start",
    "process_status",
    "process_output",
    "process_write",
    "process_stop",
    "process_list",
    "notebook_edit",
    "mcp_read",
    "agent",
    "batch",
    "apply_patch",
    "patch_preview",
    "patch_apply",
    "patch_status",
    "patch_rollback",
    "exec_command",
    "write_stdin",
    "update_plan",
    "propose_plan",
    "ask_user_question",
    "tool_search",
    "code_definition",
    "code_references",
    "code_outline",
    "code_diagnostics",
    "code_call_hierarchy",
    "code_rename_preview",
];

pub const CODEX_HOT_TOOLS: &[&str] = &[
    "exec_command",
    "write_stdin",
    "apply_patch",
    "read",
    "grep",
    "find",
    "ls",
    "update_plan",
    "propose_plan",
    "agent",
    "tool_search",
];

/// What the built-in tools share across calls: the background jobs and the
/// model's ledger. Both are behind `Arc<Mutex>` because the agent loop, the
/// tool thread and the davinci shell all read them.
#[derive(Debug, Clone, Default)]
pub struct ToolContext {
    /// Trusted host entry point for foreground process ownership. Without this,
    /// legacy commands remain available but cannot emit verification receipts.
    pub foreground_supervisor: Option<crate::jobs::supervisor::SupervisorCommand>,
    /// Per-dispatch host capture; never reconstructed from tool result JSON.
    pub command_receipt: Option<crate::command_receipt::CommandReceiptCapture>,
    /// Trusted host setting; ordinary mutation safety cannot be disabled.
    pub transactions_disabled: bool,
    /// Stable provenance for trusted library calls without a RuntimeHandle.
    pub transaction_owner: crate::runtime::transactions::TransactionOwner,
    /// Live, engine-installed permission and contract checks for file mutations.
    pub mutation_authority: Option<crate::runtime::transactions::MutationAuthority>,
    /// Per-dispatch signal used to invalidate verification even after a failed recovery.
    pub mutation_attempted: Arc<std::sync::atomic::AtomicBool>,
    /// Session-owned process service installed by a trusted host, lazily starts children.
    pub processes: Option<crate::process_manager::ProcessManager>,
    /// Engine-issued consent for this exact dispatch; never model input.
    pub dispatch_permit: Option<Arc<crate::approval::DispatchPermit>>,
    pub cache: crate::runtime::cache::CacheRuntime,
    pub jobs: Arc<Mutex<JobBook>>,
    pub todos: Arc<Mutex<TodoList>>,
    pub living_plan: Arc<Mutex<crate::LivingPlan>>,
    pub mcp: crate::mcp::McpRegistry,
    /// The turn's abort flag, when the host gave the agent one. A foreground
    /// shell command or a `job_output` wait stops at the next poll instead
    /// of holding the tool thread until the process ends on its own.
    pub abort: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub runtime: Option<crate::runtime::RuntimeHandle>,
    /// Provider-facing schemas that have been authorized and exposed for this run.
    pub tool_exposure: Arc<Mutex<crate::runtime::ToolExposureState>>,
    /// The run's current tool authorization set, shared with `tool_search`.
    pub authorized_tools: Arc<Mutex<BTreeSet<String>>>,
    pub task_coordinator: Option<crate::runtime::task_transport::TaskCoordinatorClient>,
    pub active_contract: Arc<Mutex<Option<crate::runtime::contracts::TaskContract>>>,
    pub semantic: Option<Arc<dyn crate::semantic::SemanticService>>,
    /// Trusted host bridge for synchronous user decisions. This is kept
    /// distinct from permission approval so a recommendation or answer can
    /// never issue tool authority.
    pub decision_responder: Option<DecisionResponder>,
    /// Process-local guard for the one-outstanding-question invariant.
    pub decision_slot: Arc<Mutex<Option<String>>>,
    /// Host-question deadline. `None` uses the production default; tests and
    /// embedders may choose a shorter explicit bound.
    pub decision_timeout: Option<std::time::Duration>,
}

pub fn is_managed_process_tool(name: &str) -> bool {
    matches!(
        name,
        "process_start"
            | "process_status"
            | "process_output"
            | "process_write"
            | "process_stop"
            | "process_list"
    )
}

pub(crate) fn is_coordinated_mutation(name: &str) -> bool {
    matches!(
        name,
        "write" | "edit" | "notebook_edit" | "apply_patch" | "patch_apply" | "patch_rollback"
    )
}

pub fn team_tools_enabled() -> bool {
    std::env::var("DAVINCI_EXPERIMENTAL_AGENT_TEAMS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub fn workflow_tools_enabled() -> bool {
    std::env::var("DAVINCI_EXPERIMENTAL_WORKFLOWS")
        .or_else(|_| std::env::var("DAVINCI_RUNTIME_WORKFLOWS"))
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

impl ToolContext {
    pub fn is_aborted(&self) -> bool {
        self.abort
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
    }
}

const DEFAULT_MAX_LINES: usize = 2000;
const DEFAULT_MAX_BYTES: usize = 50 * 1024;
const GREP_MAX_LINE_LENGTH: usize = 500;
const GREP_DEFAULT_LIMIT: usize = 100;
const FIND_DEFAULT_LIMIT: usize = 1000;
const LS_DEFAULT_LIMIT: usize = 500;
const POWERSHELL_UTF8_PREFIX: &str =
    "try { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8 } catch {}\n";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ToolResult {
    pub fn take_updates(details: &mut Option<serde_json::Value>) -> Vec<serde_json::Value> {
        let Some(Value::Object(map)) = details.as_mut() else {
            return Vec::new();
        };
        match map.remove("_piUpdates") {
            Some(Value::Array(items)) => items,
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Unknown tool: {0}")]
    Unknown(String),
    #[error("{0}")]
    Failed(String),
    #[error("{0}")]
    Durability(String),
}

pub fn tool_specs() -> Vec<AgentTool> {
    let mut specs = vec![
        AgentTool {
            name: "read".into(),
            description: "Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"number"},"limit":{"type":"number"}},"required":["path"]}),
        },
        AgentTool {
            name: "retrieve_context".into(),
            description: "Retrieve an exact Context VM page or authoritative source by reference. Use query and offset/limit to page large evidence without exposing hidden reasoning.".into(),
            parameters: serde_json::json!({
                "type":"object",
                "properties":{
                    "page":{"type":"string"},
                    "sourceRef":{"type":"string"},
                    "query":{"type":"string"},
                    "offset":{"type":"integer","minimum":0},
                    "limit":{"type":"integer","minimum":0,"maximum":400}
                },
                "oneOf":[{"required":["page"]},{"required":["sourceRef"]}]
            }),
        },
        AgentTool {
            name: "write".into(),
            description: "Write files (creates/overwrites)".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
        },
        AgentTool {
            name: "edit".into(),
            description: "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.".into(),
            parameters: serde_json::json!({
                "type":"object",
                "properties":{
                    "path":{"type":"string","description":"Path to the file to edit (relative or absolute)"},
                    "edits":{
                        "type":"array",
                        "description":"One or more targeted replacements. Each edit is matched against the original file, not incrementally.",
                        "items":{
                            "type":"object",
                            "properties":{
                                "oldText":{"type":"string"},
                                "newText":{"type":"string"}
                            },
                            "required":["oldText","newText"]
                        }
                    },
                    "oldText":{"type":"string"},
                    "newText":{"type":"string"}
                },
                "required":["path"]
            }),
        },
        AgentTool {
            name: "bash".into(),
            description: "Execute bash commands. With background: true the command keeps running while you continue; the call returns a job id at once, job_output reads what it printed, job_kill stops it, and you are told when it finishes. Use background for builds, test suites and servers that take more than a few seconds.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"command":{"type":"string"},"timeout":{"type":"number","description":"Timeout in seconds (optional, no default timeout)"},"background":{"type":"boolean","description":"Run in the background and return a job id immediately (optional)"}},"required":["command"]}),
        },
        AgentTool {
            name: "powershell".into(),
            description: "Execute PowerShell commands. With background: true the command runs as a background job (see bash).".into(),
            parameters: serde_json::json!({"type":"object","properties":{"command":{"type":"string"},"background":{"type":"boolean","description":"Run in the background and return a job id immediately (optional)"}},"required":["command"]}),
        },
        AgentTool {
            name: "grep".into(),
            description: "Search repository text and return matching file paths, line numbers, and optional context. Respects .gitignore. Use it to locate symbols, strings, config keys, and call sites before opening broader files.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"glob":{"type":"string"},"ignoreCase":{"type":"boolean"},"literal":{"type":"boolean"},"context":{"type":"number"},"limit":{"type":"number"}},"required":["pattern"]}),
        },
        AgentTool {
            name: "find".into(),
            description: "Search for files by glob pattern relative to the search directory. Respects .gitignore. Use to find file locations without reading contents.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"limit":{"type":"number"}},"required":["pattern"]}),
        },
        AgentTool {
            name: "ls".into(),
            description: "List directory contents sorted alphabetically with '/' suffix for directories. Includes dotfiles. Use to inspect folder layout.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"limit":{"type":"number"}}}),
        },
        AgentTool {
            name: "web_fetch".into(),
            description: "Fetch a web page or file over http(s) and read it as text. HTML is reduced to its readable content (headings, paragraphs, lists, links as `text (url)`, code blocks); JSON and plain text pass through. Output is truncated to 2000 lines or 50KB. Fetch a search result before quoting it.".into(),
            parameters: crate::web::fetch_parameters(),
        },
        AgentTool {
            name: "web_search".into(),
            description: "Search the web. Returns numbered results with title, url and snippet. Follow up with web_fetch on a result to read it.".into(),
            parameters: crate::web::search_parameters(),
        },
        AgentTool {
            name: "todo".into(),
            description: "Keep your task list current. Send the whole list every time (it replaces the previous one): each item has text and a status of pending, active or done. Use it for tasks of three or more steps, mark the step you are on active, and mark steps done as you finish them.".into(),
            parameters: crate::todo::tool_parameters(),
        },
        AgentTool {
            name: "job_output".into(),
            description: "Read the output of a background job started with bash/powershell background: true. Returns what it has printed so far and whether it is still running; wait blocks up to N seconds for it to exit; tail returns only the last N lines.".into(),
            parameters: crate::jobs::output_parameters(),
        },
        AgentTool {
            name: "job_kill".into(),
            description: "Stop a background job and its child processes.".into(),
            parameters: crate::jobs::kill_parameters(),
        },
        AgentTool {
            name: "notebook_edit".into(),
            description: "Replace, insert or delete one cell of a Jupyter notebook (.ipynb). Cells are numbered as `read` shows them. To change text inside a cell, `edit` also works on notebooks and matches inside cell sources.".into(),
            parameters: crate::notebook::tool_parameters(),
        },
        AgentTool {
            name: "mcp_read".into(),
            description: "Read a resource from a connected MCP server. Pass { server, uri }.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"server":{"type":"string","description":"MCP server name"},"uri":{"type":"string","description":"Resource URI"}},"required":["server","uri"]}),
        },
        AgentTool {
            name: "agent".into(),
            description: "Start nested workers or persistent teammates with their own context. Pass a prompt (one worker) for bounded independent research, or tasks: [{prompt, description?, tools?}] for up to 8 workers that run concurrently. Supports custom agent profiles (via `agent`), execution modes (`oneshot`, `background`, `teammate`), model overrides, and isolation.".into(),
            parameters: crate::subagent::tool_parameters(),
        },
        AgentTool {
            name: "batch".into(),
            description: crate::batch::batch_description(),
            parameters: crate::batch::batch_parameters(),
        },
        AgentTool {
            name: "apply_patch".into(),
            description: "Apply structured multi-file or multi-hunk patches in Codex format (*** Begin Patch ... *** End Patch). Supports Add, Update, and Delete file operations. Prefer edit for small single-file replacements.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "The patch text enclosed in *** Begin Patch and *** End Patch"
                    }
                },
                "required": ["input"]
            }),
        },
        AgentTool {
            name: "exec_command".into(),
            description: "Start a bounded shell command using the platform-appropriate shell (PowerShell on Windows, bash/sh on Unix). Supports background execution.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "timeout": { "type": "number", "description": "Timeout in seconds (optional)" },
                    "background": { "type": "boolean", "description": "Run in background (optional)" }
                },
                "required": ["command"]
            }),
        },
        AgentTool {
            name: "write_stdin".into(),
            description: "Send input text to an interactive process or background job.".into(),
            parameters: crate::jobs::stdin_parameters(),
        },
        AgentTool {
            name: "propose_plan".into(),
            description: "Create or revise the session's structured implementation plan after inspecting repository files. Include source evidence, assumptions, concrete steps, dependencies, and validation. Never edits implementation files or approves itself.".into(),
            parameters: crate::living_plan::tool_parameters(),
        },
        AgentTool {
            name: "update_plan".into(),
            description: "Track execution progress using plan:[{step,status}]. This progress ledger never approves a plan or changes permissions; use propose_plan for evidence-backed implementation decisions.".into(),
            parameters: update_plan_parameters(),
        },
        AgentTool {
            name: "ask_user_question".into(),
            description: "Ask one material, evidence-backed structured question of the controlling user. The tool carries question content only; answers and host authority are never accepted from model JSON.".into(),
            parameters: serde_json::json!({
                "type":"object",
                "additionalProperties": false,
                "properties":{
                    "id":{"type":"string"},
                    "kind":{"type":"string","enum":["scope","approach","tradeoff","compatibility","persistence","behavior","verification","other"]},
                    "title":{"type":"string"},
                    "question":{"type":"string"},
                    "materiality":{"type":"string"},
                    "evidence_refs":{"type":"array","items":{"type":"string"}},
                    "options":{"type":"array","items":{"type":"object","additionalProperties":false,"properties":{
                        "id":{"type":"string"},"label":{"type":"string"},"explanation":{"type":"string"},"recommended":{"type":"boolean"}
                    },"required":["id","label","explanation"]}},
                    "allow_custom":{"type":"boolean"},
                    "custom_only":{"type":"boolean"}
                },
                "required":["id","kind","title","question","materiality","evidence_refs","options"]
            }),
        },
        AgentTool {
            name: "tool_search".into(),
            description: "Discover deferred tools and namespaces by keyword query.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Keyword query for tool discovery" }
                },
                "required": ["query"]
            }),
        },
        AgentTool {
            name: "code_definition".into(),
            description: "Find symbol definition via language server or text search fallback.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Target file path" },
                    "line": { "type": "number", "description": "1-indexed line number" },
                    "character": { "type": "number", "description": "1-indexed character offset" },
                    "symbol": { "type": "string", "description": "Symbol name for fallback" }
                },
                "required": ["path"]
            }),
        },
        AgentTool {
            name: "code_references".into(),
            description: "Find symbol references across the workspace via language server or text search fallback.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path containing symbol" },
                    "line": { "type": "number", "description": "1-indexed line number" },
                    "character": { "type": "number", "description": "1-indexed character offset" },
                    "symbol": { "type": "string", "description": "Symbol name to find references for" },
                    "includeDeclaration": { "type": "boolean", "description": "Include declaration in results" }
                },
                "required": ["path"]
            }),
        },
        AgentTool {
            name: "code_outline".into(),
            description: "Extract symbol outline (functions, classes, types) for a file.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Target file path" }
                },
                "required": ["path"]
            }),
        },
        AgentTool {
            name: "code_diagnostics".into(),
            description: "Retrieve compiler and linter diagnostics for a file from the language server.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Target file path" }
                },
                "required": ["path"]
            }),
        },
        AgentTool {
            name: "code_call_hierarchy".into(),
            description: "Trace incoming or outgoing call hierarchy for a function or method.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Target file path" },
                    "line": { "type": "number", "description": "1-indexed line number" },
                    "character": { "type": "number", "description": "1-indexed character offset" },
                    "direction": { "type": "string", "enum": ["incoming", "outgoing"], "description": "Call direction (incoming or outgoing)" }
                },
                "required": ["path", "line", "character"]
            }),
        },
        AgentTool {
            name: "code_rename_preview".into(),
            description: "Generate a preview of a workspace-wide symbol rename operation.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Target file path" },
                    "line": { "type": "number", "description": "1-indexed line number" },
                    "character": { "type": "number", "description": "1-indexed character offset" },
                    "newName": { "type": "string", "description": "Proposed new symbol name" }
                },
                "required": ["path", "line", "character", "newName"]
            }),
        },
    ];
    if team_tools_enabled() {
        specs.extend(crate::runtime::agent_tool_specs());
        specs.extend(crate::runtime::task_tool_specs());
    }
    if workflow_tools_enabled() {
        specs.extend(crate::runtime::workflow_tool_specs());
    }
    specs.extend(crate::process_manager::tool_specs());
    specs.extend(crate::runtime::transactions::tool_specs());
    specs
}

pub fn validate_builtin_tool_descriptions(specs: &[AgentTool]) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    for tool in specs {
        if tool.description.trim().is_empty() {
            errors.push(format!("Tool '{}' has an empty description", tool.name));
            continue;
        }
        if tool.description.len() > 700 {
            errors.push(format!(
                "Tool '{}' description exceeds 700 characters (length: {})",
                tool.name,
                tool.description.len()
            ));
        }
        let lower = tool.description.to_lowercase();
        if lower.contains("unrestricted") || lower.contains("bypass permission") {
            errors.push(format!(
                "Tool '{}' claims authority that contradicts permission engine",
                tool.name
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn execute_tool(
    cwd: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<ToolResult, ToolError> {
    execute_tool_with(cwd, name, input, &ToolContext::default())
}

/// Run a built-in tool with the shared state of the run: background jobs
/// and the todo ledger. `execute_tool` runs with fresh, throw-away state.
pub fn execute_tool_with(
    cwd: &Path,
    name: &str,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    match name {
        transaction if crate::runtime::transactions::is_tool(transaction) => {
            crate::runtime::transactions::execute(cwd, transaction, input, context)
        }
        process if is_managed_process_tool(process) => {
            if let Some(coordinator) = &context.task_coordinator {
                return coordinator.call_with_timeout(process, input, context.abort.as_deref(), std::time::Duration::from_secs(15));
            }
            let manager = context.processes.as_ref().ok_or_else(|| ToolError::Failed("Managed processes are disabled or unavailable in this host".into()))?;
            let contract = context.active_contract.lock().unwrap_or_else(|e| e.into_inner());
            let provenance = crate::jobs::managed::Provenance {
                session_id: context.runtime.as_ref().and_then(|runtime| runtime.session_id.clone()),
                agent_id: context.runtime.as_ref().map(|runtime| runtime.agent_id),
                task_id: contract.as_ref().map(|contract| contract.task_id),
                graph_node: None,
            };
            drop(contract);
            manager.clone().with_provenance(provenance)
                .execute(cwd, process, input, context.abort.as_deref(), context.dispatch_permit.as_deref())
                .map_err(ToolError::Failed)
        }
        "read" => read_tool_cached(cwd, input, context),
        "retrieve_context" => crate::runtime::context_vm::retrieve_context_tool(input, context),
        "write" => write_tool(cwd, input, context),
        "edit" => edit_tool(cwd, input, context),
        "apply_patch" => apply_patch_tool(cwd, input, context),
        "exec_command" => exec_command_tool(cwd, input, context),
        "write_stdin" => write_stdin_tool(input, context),
        "bash" => shell_tool(cwd, input, context),
        "powershell" => powershell_tool(cwd, input, context),
        "ls" => ls_tool(cwd, input),
        "grep" => grep_tool(cwd, input, context),
        "find" => find_tool(cwd, input),
        "web_fetch" => crate::web::fetch_tool(input).map_err(ToolError::Failed),
        "web_search" => crate::web::search_tool(input).map_err(ToolError::Failed),
        "todo" => todo_tool(input, context),
        "update_plan" => update_plan_tool(input, context),
        "propose_plan" => {
            let mut plan = context.living_plan.lock().unwrap_or_else(|e| e.into_inner());
            plan.update(input, cwd).map_err(ToolError::Failed)?;
            *context.todos.lock().unwrap_or_else(|e| e.into_inner()) = TodoList {
                items: plan.steps.iter().map(|step| crate::TodoItem {
                    text: format!("[{}] {}", step.id, step.change),
                    status: crate::TodoStatus::Pending,
                }).collect(),
            };
            Ok(ToolResult {
                content: plan.render(),
                is_error: false,
                details: Some(serde_json::json!({"revision": plan.revision, "changes": plan.changes})),
            })
        },
        "ask_user_question" => ask_user_question_tool(cwd, input, context),
        "tool_search" => tool_search_tool(input, context),
        "job_output" => crate::jobs::output_tool(&context.jobs, input, context.abort.as_deref())
            .map_err(ToolError::Failed),
        "job_kill" => crate::jobs::kill_tool(&context.jobs, input).map_err(ToolError::Failed),
        "notebook_edit" => notebook_edit_tool(cwd, input, context),
        "mcp_read" => mcp_read_tool(input, context),
        "agent_status" | "agent_message" | "agent_stop"
            if !team_tools_enabled() =>
        {
            Err(ToolError::Failed(
                "Team coordination tools are disabled; set DAVINCI_EXPERIMENTAL_AGENT_TEAMS=1 to enable".into(),
            ))
        }
        "task_create" | "task_update" | "task_list" | "task_get"
            if !team_tools_enabled() && context.task_coordinator.is_none() =>
        {
            Err(ToolError::Failed(
                "Team coordination tools are disabled; set DAVINCI_EXPERIMENTAL_AGENT_TEAMS=1 to enable".into(),
            ))
        }
        "agent_status" => crate::runtime::agent_status_tool(input, context),
        "agent_message" => crate::runtime::agent_message_tool(input, context),
        "agent_stop" => crate::runtime::agent_stop_tool(input, context),
        "task_create" => crate::runtime::task_create_tool(input, context),
        "task_update" => crate::runtime::task_update_tool(input, context),
        "task_list" => crate::runtime::task_list_tool(input, context),
        "task_get" => crate::runtime::task_get_tool(input, context),
        "workflow_run" | "workflow_status"
            if !workflow_tools_enabled() =>
        {
            Err(ToolError::Failed(
                "Workflow coordination tools are disabled; set DAVINCI_EXPERIMENTAL_WORKFLOWS=1 to enable".into(),
            ))
        }
        "workflow_run" => crate::runtime::workflow_run_tool(cwd, input, context),
        "workflow_status" => crate::runtime::workflow_status_tool(input, context),
        "code_definition" => code_definition_tool(cwd, input, context),
        "code_references" => code_references_tool(cwd, input, context),
        "code_outline" => code_outline_tool(cwd, input, context),
        "code_diagnostics" => code_diagnostics_tool(cwd, input, context),
        "code_call_hierarchy" => code_call_hierarchy_tool(cwd, input, context),
        "code_rename_preview" => code_rename_preview_tool(cwd, input, context),
        other if other.starts_with("mcp__") => mcp_call_tool(other, input, context),
        other => Err(ToolError::Unknown(other.to_string())),
    }
}

fn apply_patch_tool(
    cwd: &Path,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let patch = input
        .get("input")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed("Missing `input` argument for apply_patch".into()))?;
    let transaction = crate::runtime::transactions::tools::ToolTransaction::new(cwd, context)?;
    let result = (|| {
        let (changes, message) = crate::apply_patch::prepare_patch(cwd, patch, |path| {
            transaction.snapshot(path).map_err(|e| e.to_string())
        })?;
        let summary = transaction.apply(changes).map_err(|e| e.to_string())?;
        Ok::<_, String>((message, summary))
    })();
    match result {
        Ok((msg, transaction)) => Ok(ToolResult {
            content: msg,
            is_error: false,
            details: Some(serde_json::json!({"transaction":transaction})),
        }),
        Err(err) => Ok(ToolResult {
            content: err,
            is_error: true,
            details: None,
        }),
    }
}

fn exec_command_tool(
    cwd: &Path,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    if cfg!(windows) {
        powershell_tool(cwd, input, context)
    } else {
        shell_tool(cwd, input, context)
    }
}

fn write_stdin_tool(
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    match crate::jobs::stdin_tool(&context.jobs, input) {
        Ok(result) => Ok(result),
        Err(err) => Ok(ToolResult {
            content: err,
            is_error: true,
            details: None,
        }),
    }
}

fn wait_for_decision_host(
    responder: DecisionResponder,
    request: DecisionHostRequest,
    context: &ToolContext,
) -> DecisionHostResponse {
    use std::sync::mpsc::RecvTimeoutError;
    const DEFAULT_DECISION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
    const POLL: std::time::Duration = std::time::Duration::from_millis(25);
    let deadline = context.decision_timeout.unwrap_or(DEFAULT_DECISION_TIMEOUT);
    let started = std::time::Instant::now();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(responder.respond(request));
    });
    loop {
        if context.is_aborted() {
            return DecisionHostResponse::Cancelled;
        }
        if started.elapsed() >= deadline {
            return DecisionHostResponse::Timeout;
        }
        let remaining = deadline.saturating_sub(started.elapsed());
        match receiver.recv_timeout(POLL.min(remaining)) {
            Ok(response) => return response,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return DecisionHostResponse::Unavailable,
        }
    }
}

fn ask_user_question_tool(
    cwd: &Path,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let raw: crate::decisions::DecisionQuestionInput = serde_json::from_value(input.clone())
        .map_err(|error| ToolError::Failed(format!("Invalid structured question: {error}")))?;
    let snapshot = context
        .living_plan
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let question = crate::decisions::validate_question_for_plan(raw, &snapshot, cwd)
        .map_err(ToolError::Failed)?;

    let Some(responder) = &context.decision_responder else {
        return Ok(ToolResult {
            content: decision_wait(false, false, false).into(),
            is_error: true,
            details: Some(serde_json::json!({
                "status": decision_wait(false, false, false),
                "question": question,
                "interactive": false
            })),
        });
    };
    if context.is_aborted() {
        return Ok(ToolResult {
            content: decision_wait(true, false, true).into(),
            is_error: true,
            details: Some(serde_json::json!({"status": decision_wait(true, false, true)})),
        });
    }

    {
        let mut slot = context
            .decision_slot
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(active) = slot.as_ref() {
            return Ok(ToolResult {
                content: format!(
                    "{}: another question is already pending ({active})",
                    decision_wait(false, false, false)
                ),
                is_error: true,
                details: Some(serde_json::json!({
                    "status": decision_wait(false, false, false),
                    "pending_decision_id":active,
                    "question":question
                })),
            });
        }
        *slot = Some(question.id.clone());
    }

    struct SlotGuard<'a>(&'a Arc<Mutex<Option<String>>>);
    impl Drop for SlotGuard<'_> {
        fn drop(&mut self) {
            *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
    let _slot_guard = SlotGuard(&context.decision_slot);
    let wait_started = std::time::Instant::now();
    let response = wait_for_decision_host(
        responder.clone(),
        DecisionHostRequest {
            question: question.clone(),
        },
        context,
    );
    let elapsed_ms = wait_started.elapsed().as_millis() as u64;

    match response {
        DecisionHostResponse::Cancelled => Ok(ToolResult {
            content: decision_wait(true, false, true).into(),
            is_error: true,
            details: Some(serde_json::json!({
                "status": decision_wait(true, false, true),
                "question":question,
                "elapsed_ms":elapsed_ms
            })),
        }),
        DecisionHostResponse::Unavailable => Ok(ToolResult {
            content: decision_wait(false, false, false).into(),
            is_error: true,
            details: Some(serde_json::json!({
                "status": decision_wait(false, false, false),
                "question":question,
                "interactive":false,
                "elapsed_ms":elapsed_ms
            })),
        }),
        DecisionHostResponse::Timeout => Ok(ToolResult {
            content: format!(
                "{}: timed out waiting for user",
                decision_wait(false, false, false)
            ),
            is_error: true,
            details: Some(serde_json::json!({
                "status": decision_wait(false, false, false),
                "reason":"timeout",
                "question":question,
                "elapsed_ms":elapsed_ms
            })),
        }),
        DecisionHostResponse::Reply(reply) => {
            let mut pending = snapshot.clone();
            pending
                .structured_decisions
                .insert(question.id.clone(), question.clone());
            let host_reply = crate::decisions::HostDecisionReply {
                decision_id: question.id.clone(),
                expected_plan_revision: snapshot.revision,
                expected_question_revision: question.plan_revision,
                host_event_id: reply.host_event_id,
                answered_at_ms: reply.answered_at_ms,
                action: reply.action,
            };
            let (next, _) = crate::decisions::apply_host_decision(&pending, &host_reply, cwd)
                .map_err(ToolError::Failed)?;
            let state = next
                .structured_decisions
                .get(&question.id)
                .map(|decision| format!("{:?}", decision.state).to_ascii_lowercase())
                .unwrap_or_else(|| "unknown".into());
            *context
                .living_plan
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = next.clone();
            Ok(ToolResult {
                content: state.clone(),
                is_error: false,
                details: Some(serde_json::json!({
                    "status":state,
                    "decision_id":question.id,
                    "revision":next.revision,
                    "elapsed_ms":elapsed_ms
                })),
            })
        }
    }
}

fn tool_search_tool(
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let authorized = context
        .authorized_tools
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let mut matches = Vec::new();
    let mut activated = Vec::new();

    if let Some(runtime) = &context.runtime {
        for capability in runtime.capability_registry.list() {
            let searchable = format!(
                "{} {} {}",
                capability.name, capability.description, capability.source
            )
            .to_lowercase();
            if !authorized.contains(&capability.name)
                || (!query.is_empty() && !searchable.contains(&query))
            {
                continue;
            }
            matches.push(capability.name.clone());
            if capability.schema.is_some()
                && context
                    .tool_exposure
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .activate_authorized(&capability.name, true)
            {
                activated.push(capability.name);
            }
            if matches.len() == 5 {
                break;
            }
        }
    } else {
        matches = context
            .mcp
            .tool_names()
            .into_iter()
            .filter(|name| authorized.contains(name))
            .filter(|name| query.is_empty() || name.to_lowercase().contains(&query))
            .take(5)
            .collect();
    }
    let content = if matches.is_empty() {
        format!("No tools found matching query `{query}`")
    } else {
        format!("Found tools: {}", matches.join(", "))
    };
    Ok(ToolResult {
        content,
        is_error: false,
        details: Some(serde_json::json!({
            "matches": matches,
            "activated": activated,
        })),
    })
}

fn mcp_read_tool(
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let server = required_str(input, "server")?;
    let uri = required_str(input, "uri")?;
    context.mcp.read(server, uri)
}

fn mcp_call_tool(
    name: &str,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let Some((server, tool)) = davinci_mcp::split_agent_tool_name(name) else {
        return Err(ToolError::Unknown(name.to_string()));
    };
    context.mcp.call(server, tool, input)
}

fn update_plan_parameters() -> Value {
    let mut schema = crate::todo::tool_parameters();
    schema
        .as_object_mut()
        .expect("todo schema is an object")
        .remove("required");
    schema["properties"]["plan"] = serde_json::json!({
        "type":"array", "items":{"type":"object", "properties":{
            "step":{"type":"string"}, "status":{"type":"string"}
        }, "required":["step","status"]}
    });
    schema["properties"]["explanation"] = serde_json::json!({"type":"string"});
    schema["anyOf"] = serde_json::json!([{"required":["items"]},{"required":["plan"]}]);
    schema
}

fn update_plan_tool(input: &Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    let Some(steps) = input.get("plan") else {
        return todo_tool(input, context);
    };
    if input.get("items").is_some() {
        return Err(ToolError::Failed(
            "Use either plan or items, not both.".into(),
        ));
    }
    let steps = steps
        .as_array()
        .ok_or_else(|| ToolError::Failed("plan must be an array.".into()))?;
    let items: Vec<Value> = steps
        .iter()
        .map(|step| {
            let mut item = step.clone();
            if let Some(text) = step.get("step") {
                if let Some(object) = item.as_object_mut() {
                    object.insert("text".into(), text.clone());
                }
            }
            item
        })
        .collect();
    todo_tool(&serde_json::json!({"items":items}), context)
}

/// `todo { items }`: the list is replaced whole and echoed back rendered.
fn todo_tool(input: &serde_json::Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
    let list = TodoList::from_args(input).map_err(ToolError::Failed)?;
    let content = list.render();
    let details = serde_json::json!({
        "items": list.items,
        "done": list.done(),
        "total": list.items.len(),
        "summary": list.summary(),
    });
    *context.todos.lock().unwrap_or_else(|err| err.into_inner()) = list;
    Ok(ToolResult {
        content,
        is_error: false,
        details: Some(details),
    })
}

fn read_tool_cached(
    cwd: &Path,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    use crate::runtime::cache::*;
    if !context.cache.config().enabled {
        return read_tool(cwd, input);
    }
    let path = resolve(cwd, required_str(input, "path")?)?;
    let root = match cwd.canonicalize() {
        Ok(root) => root,
        Err(_) => return read_tool(cwd, input),
    };
    let absolute = match path.canonicalize() {
        Ok(path) => path,
        Err(_) => return read_tool(cwd, input),
    };
    let Ok(relative) = absolute.strip_prefix(&root) else {
        return read_tool(cwd, input);
    };
    // The agent's permission gate executes before this dispatch on every call.
    // A fresh confined read additionally proves that cached bytes cannot grant file access.
    let snapshot = match context
        .cache
        .read_current_file(&root, relative, DEFAULT_MAX_BYTES, || Ok(()))
    {
        Ok(snapshot) => snapshot,
        Err(_) => return read_tool(cwd, input),
    };
    if detect_image_mime(&path, &snapshot.bytes[..snapshot.bytes.len().min(12)]).is_some()
        || crate::notebook::is_notebook_path(&path)
    {
        return read_tool(cwd, input);
    }
    let offset = input
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_MAX_LINES);
    let key = CacheKey::new(
        CacheNamespace::File,
        format!("text-window:{offset}:{limit}"),
        1,
        "read-window-v1",
        vec![CacheDependency::ContentHash(snapshot.content_hash.clone())],
    );
    let window = context.cache.get_or_compute(
        &CacheRequest::new(key, CachePolicy::MemoryOnly),
        || Ok(()),
        None,
        || {
            read_text_window_from(
                std::io::Cursor::new(&snapshot.bytes),
                offset,
                limit,
                DEFAULT_MAX_BYTES,
            )
            .map_err(|e| CacheError::Compute(e.to_string()))
        },
    );
    let Ok(window) = window else {
        return read_tool(cwd, input);
    };
    // Only line/byte counts are eligible for disk; no source or tool output is serialized.
    let counts_key = CacheKey::new(
        CacheNamespace::File,
        "text-counts",
        1,
        "lossy-utf8-lines-v1",
        vec![CacheDependency::ContentHash(snapshot.content_hash)],
    );
    let counts = context.cache.get_or_compute(
        &CacheRequest::new(counts_key, CachePolicy::PersistentImmutable),
        || Ok(()),
        None,
        || {
            let text = String::from_utf8_lossy(&snapshot.bytes);
            let lines = if text.is_empty() {
                0
            } else {
                text.split('\n')
                    .count()
                    .saturating_sub(usize::from(text.ends_with('\n')))
            };
            Ok((lines, text.len()))
        },
    );
    let Ok(counts) = counts else {
        return read_tool(cwd, input);
    };
    let truncated_by = if window.truncated {
        Some(if window.lines_returned >= limit {
            "lines"
        } else {
            "bytes"
        })
    } else {
        None
    };
    Ok(ToolResult {
        content: window.content.clone(),
        is_error: false,
        details: Some(serde_json::json!({
            "path":path, "truncation": {
                "truncated":window.truncated, "truncatedBy":truncated_by, "firstLine":window.first_line,
                "totalLines":counts.0, "totalBytes":counts.1, "outputLines":window.lines_returned,
                "outputBytes":window.content.len(), "maxLines":limit, "maxBytes":DEFAULT_MAX_BYTES
            }
        })),
    })
}

fn read_tool(cwd: &Path, input: &serde_json::Value) -> Result<ToolResult, ToolError> {
    let raw_path = required_str(input, "path")?;
    let path = resolve(cwd, raw_path)?;
    let mut prefix_file =
        fs::File::open(&path).map_err(|err| ToolError::Failed(err.to_string()))?;
    let mut prefix = [0_u8; 12];
    let prefix_len = prefix_file
        .read(&mut prefix)
        .map_err(|err| ToolError::Failed(err.to_string()))?;
    if let Some(mime) = detect_image_mime(&path, &prefix[..prefix_len]) {
        let bytes = fs::read(&path).map_err(|err| ToolError::Failed(err.to_string()))?;
        return read_image(&path, &bytes, mime);
    }
    let mut content = String::new();
    let mut notebook = None;
    let is_notebook = crate::notebook::is_notebook_path(&path);
    if is_notebook {
        let bytes = fs::read(&path).map_err(|err| ToolError::Failed(err.to_string()))?;
        content = String::from_utf8_lossy(&bytes).into_owned();
        if let Some(parsed) = crate::notebook::parse(&content) {
            content = crate::notebook::render(&parsed);
            notebook = Some(serde_json::json!({
                "cells": parsed.get("cells").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
                "language": crate::notebook::language(&parsed),
            }));
        }
    }
    let offset = input
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let limit = limit.unwrap_or(DEFAULT_MAX_LINES);
    if is_notebook {
        let (content, truncation) = truncate_read(&content, offset, Some(limit));
        let mut details = serde_json::json!({"path": path, "truncation": truncation});
        if let Some(notebook) = notebook {
            details["notebook"] = notebook;
        }
        return Ok(ToolResult {
            content,
            is_error: false,
            details: Some(details),
        });
    }

    let window = read_text_window(&path, offset, limit, DEFAULT_MAX_BYTES)?;
    let truncated_by = if window.truncated {
        if window.lines_returned >= limit {
            Some("lines")
        } else {
            Some("bytes")
        }
    } else {
        None
    };
    let (total_lines, total_bytes) = match small_text_totals(&path)? {
        Some((lines, bytes)) => (Some(lines), Some(bytes)),
        None => (None, None),
    };
    let truncation = serde_json::json!({
        "truncated": window.truncated,
        "truncatedBy": truncated_by,
        "firstLine": window.first_line,
        "totalLines": total_lines,
        "totalBytes": total_bytes,
        "outputLines": window.lines_returned,
        "outputBytes": window.content.len(),
        "maxLines": limit,
        "maxBytes": DEFAULT_MAX_BYTES,
    });
    Ok(ToolResult {
        content: window.content,
        is_error: false,
        details: Some(serde_json::json!({"path": path, "truncation": truncation})),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TextWindow {
    content: String,
    first_line: usize,
    lines_returned: usize,
    truncated: bool,
}

struct RawLine {
    bytes: Vec<u8>,
    too_long: bool,
}

fn read_raw_line(
    reader: &mut BufReader<impl Read>,
    capture: bool,
    max_bytes: Option<usize>,
) -> Result<Option<RawLine>, ToolError> {
    let mut bytes = Vec::new();
    let mut saw_any = false;
    loop {
        let buffer = reader
            .fill_buf()
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        if buffer.is_empty() {
            return Ok(saw_any.then_some(RawLine {
                bytes,
                too_long: false,
            }));
        }

        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(buffer.len());
        if capture {
            if max_bytes.is_some_and(|max| bytes.len().saturating_add(content_len) > max) {
                return Ok(Some(RawLine {
                    bytes: Vec::new(),
                    too_long: true,
                }));
            }
            bytes.extend_from_slice(&buffer[..content_len]);
        }

        let consume_len = newline.map_or(buffer.len(), |index| index + 1);
        reader.consume(consume_len);
        saw_any = true;
        if newline.is_some() {
            return Ok(Some(RawLine {
                bytes,
                too_long: false,
            }));
        }
    }
}

fn read_text_window(
    path: &Path,
    offset: usize,
    limit: usize,
    max_bytes: usize,
) -> Result<TextWindow, ToolError> {
    let file = fs::File::open(path).map_err(|err| ToolError::Failed(err.to_string()))?;
    read_text_window_from(file, offset, limit, max_bytes)
}

fn read_text_window_from(
    file: impl Read,
    offset: usize,
    limit: usize,
    max_bytes: usize,
) -> Result<TextWindow, ToolError> {
    let mut reader = BufReader::new(file);
    let first_line = offset.max(1);

    for _ in 1..first_line {
        if read_raw_line(&mut reader, false, None)?.is_none() {
            return Ok(TextWindow {
                content: String::new(),
                first_line,
                lines_returned: 0,
                truncated: false,
            });
        }
    }

    let mut content = String::new();
    let mut lines_returned = 0;
    let mut truncated = false;
    while lines_returned < limit {
        let separator_bytes = usize::from(lines_returned > 0);
        let remaining_bytes = max_bytes.saturating_sub(content.len() + separator_bytes);
        let Some(line) = read_raw_line(&mut reader, true, Some(remaining_bytes))? else {
            break;
        };
        if line.too_long {
            truncated = true;
            break;
        }

        let line = String::from_utf8_lossy(&line.bytes).into_owned();
        if content.len() + separator_bytes + line.len() > max_bytes {
            truncated = true;
            break;
        }
        if lines_returned > 0 {
            content.push('\n');
        }
        content.push_str(&line);
        lines_returned += 1;
    }

    if !truncated && lines_returned >= limit {
        truncated = read_raw_line(&mut reader, false, None)?.is_some();
    }

    Ok(TextWindow {
        content,
        first_line,
        lines_returned,
        truncated,
    })
}

fn small_text_totals(path: &Path) -> Result<Option<(usize, usize)>, ToolError> {
    let metadata = fs::metadata(path).map_err(|err| ToolError::Failed(err.to_string()))?;
    if metadata.len() > DEFAULT_MAX_BYTES as u64 {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|err| ToolError::Failed(err.to_string()))?;
    let content = String::from_utf8_lossy(&bytes);
    let total_lines = if content.is_empty() {
        0
    } else {
        let mut lines = content.split('\n').count();
        if content.ends_with('\n') {
            lines = lines.saturating_sub(1);
        }
        lines
    };
    Ok(Some((total_lines, content.len())))
}

fn read_image(path: &Path, bytes: &[u8], mime: &str) -> Result<ToolResult, ToolError> {
    match process_image(bytes, mime) {
        Ok(processed) => {
            let mut note = format!("Read image file [{}]", processed.mime_type);
            for hint in &processed.hints {
                note.push('\n');
                note.push_str(hint);
            }
            Ok(ToolResult {
                content: note,
                is_error: false,
                details: Some(serde_json::json!({
                    "path": path,
                    "image": {
                        "type": "image",
                        "data": processed.data,
                        "mimeType": processed.mime_type,
                    }
                })),
            })
        }
        Err(message) => Ok(ToolResult {
            content: format!("Read image file [{mime}]\n{message}"),
            is_error: true,
            details: Some(serde_json::json!({"path": path})),
        }),
    }
}

struct ProcessedImage {
    data: String,
    mime_type: String,
    hints: Vec<String>,
}

fn process_image(bytes: &[u8], mime: &str) -> Result<ProcessedImage, String> {
    let normalized = match mime {
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" => mime.to_string(),
        _ => {
            let png = image::load_from_memory(bytes)
                .map_err(|err| format!("Unsupported image type: {err}"))?;
            let mut out = Vec::new();
            png.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .map_err(|err| err.to_string())?;
            return Ok(ProcessedImage {
                data: base64::engine::general_purpose::STANDARD.encode(&out),
                mime_type: "image/png".into(),
                hints: vec![format!("[Image converted from {mime} to image/png.]")],
            });
        }
    };
    Ok(ProcessedImage {
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        mime_type: normalized,
        hints: Vec::new(),
    })
}

fn detect_image_mime(path: &Path, bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

fn write_tool(
    cwd: &Path,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let path = resolve_for_mutation(cwd, required_str(input, "path")?)?;
    crate::file_mutation_queue::with_file_mutation_queue(&path, || {
        let manager = crate::runtime::transactions::tools::ToolTransaction::new(cwd, context)?;
        let snapshot = manager.snapshot(&path)?;
        let created = snapshot.bytes().is_none();
        let content = required_str(input, "content")?;
        // The change is what the transcript shows, so the previous content
        // is read before it is gone; a fresh file diffs against nothing.
        let previous = snapshot.text().unwrap_or_default().to_owned();
        let transaction =
            manager.apply(vec![snapshot.change(Some(content.as_bytes().to_vec()))])?;
        let (diff, first_changed_line) = crate::edit_diff::generate_diff_string(
            &crate::edit_diff::normalize_to_lf(&previous),
            &crate::edit_diff::normalize_to_lf(content),
            4,
        );
        Ok(ToolResult {
            content: format!("Wrote {}", path.display()),
            is_error: false,
            details: Some(serde_json::json!({
                "path": path,
                "diff": diff,
                "firstChangedLine": first_changed_line,
                "created": created,
                "transaction":transaction,
            })),
        })
    })
}

fn edit_tool(
    cwd: &Path,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let (display_path, edits) =
        crate::edit_diff::prepare_edit_arguments(input).map_err(ToolError::Failed)?;
    let path = resolve_for_mutation(cwd, &display_path)?;
    let manager = crate::runtime::transactions::tools::ToolTransaction::new(cwd, context)?;
    crate::file_mutation_queue::with_file_mutation_queue(&path, || {
        if crate::notebook::is_notebook_path(&path) {
            if let Some(result) = notebook_edit_locked(&path, &display_path, &edits, &manager)? {
                return Ok(result);
            }
        }
        edit_tool_locked(&path, &display_path, &edits, &manager)
    })
}

/// `edit` on a notebook: the replacements land inside cell sources. `None`
/// when the file is not notebook JSON, so it is edited as text.
fn notebook_edit_locked(
    path: &Path,
    display_path: &str,
    edits: &[crate::edit_diff::Edit],
    manager: &crate::runtime::transactions::tools::ToolTransaction<'_>,
) -> Result<Option<ToolResult>, ToolError> {
    let snapshot = manager.snapshot(path)?;
    let raw = snapshot.text().map_err(ToolError::Failed)?;
    let Some(mut notebook) = crate::notebook::parse(raw) else {
        return Ok(None);
    };
    let changes = crate::notebook::edit_in_cells(&mut notebook, edits, display_path)
        .map_err(ToolError::Failed)?;
    let text = crate::notebook::serialize(&notebook, crate::notebook::detect_indent(raw));
    let transaction = manager.apply(vec![snapshot.change(Some(text.into_bytes()))])?;
    let (diff, first_changed_line) = crate::notebook::changes_diff(&changes);
    let cells: Vec<usize> = changes.iter().map(|change| change.index + 1).collect();
    Ok(Some(ToolResult {
        content: format!(
            "Edited {display_path} (cell {})",
            cells
                .iter()
                .map(|cell| cell.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        is_error: false,
        details: Some(serde_json::json!({
            "path": display_path,
            "edits": edits.len(),
            "cells": cells,
            "diff": diff,
            "firstChangedLine": first_changed_line,
            "transaction":transaction,
        })),
    }))
}

/// `notebook_edit { path, cell, mode, source?, cellType? }`.
fn notebook_edit_tool(
    cwd: &Path,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    use crate::notebook::{self, EditMode};
    let display_path = required_str(input, "path")?.to_string();
    let path = resolve_for_mutation(cwd, &display_path)?;
    let cell = input
        .get("cell")
        .and_then(Value::as_u64)
        .ok_or_else(|| ToolError::Failed("Missing cell (1-based cell number)".into()))?
        as usize;
    let mode = input
        .get("mode")
        .and_then(Value::as_str)
        .and_then(EditMode::parse)
        .ok_or_else(|| ToolError::Failed("mode must be replace, insert or delete".into()))?;
    let source = input.get("source").and_then(Value::as_str);
    let cell_type = notebook::apply_kind(input.get("cellType").and_then(Value::as_str))
        .map_err(ToolError::Failed)?;
    crate::file_mutation_queue::with_file_mutation_queue(&path, || {
        let manager = crate::runtime::transactions::tools::ToolTransaction::new(cwd, context)?;
        let snapshot = manager.snapshot(&path)?;
        let raw = snapshot.text().map_err(ToolError::Failed)?;
        let mut parsed = notebook::parse(raw).ok_or_else(|| {
            ToolError::Failed(format!("{display_path} is not a Jupyter notebook"))
        })?;
        let outcome =
            notebook::structural_edit(&mut parsed, &display_path, cell, mode, source, cell_type)
                .map_err(ToolError::Failed)?;
        let text = notebook::serialize(&parsed, notebook::detect_indent(raw));
        let transaction = manager.apply(vec![snapshot.change(Some(text.into_bytes()))])?;
        Ok(ToolResult {
            content: outcome.summary,
            is_error: false,
            details: Some(serde_json::json!({
                "path": display_path,
                "cell": cell,
                "mode": input.get("mode").and_then(Value::as_str).unwrap_or("replace").to_ascii_lowercase(),
                "cells": outcome.cells,
                "diff": outcome.diff,
                "transaction":transaction,
            })),
        })
    })
}

fn edit_tool_locked(
    path: &Path,
    display_path: &str,
    edits: &[crate::edit_diff::Edit],
    manager: &crate::runtime::transactions::tools::ToolTransaction<'_>,
) -> Result<ToolResult, ToolError> {
    let snapshot = manager.snapshot(path)?;
    let raw = snapshot.text().map_err(ToolError::Failed)?;
    let (bom, content) = crate::edit_diff::split_bom(raw);
    let ending = crate::edit_diff::detect_line_ending(content);
    let normalized = crate::edit_diff::normalize_to_lf(content);
    let applied =
        crate::edit_diff::apply_edits_to_normalized_content(&normalized, edits, display_path)
            .map_err(ToolError::Failed)?;
    let final_content = format!(
        "{bom}{}",
        crate::edit_diff::restore_line_endings(&applied.new_content, ending)
    );
    let transaction = manager.apply(vec![snapshot.change(Some(final_content.into_bytes()))])?;
    let (diff, first_changed_line) =
        crate::edit_diff::generate_diff_string(&applied.base_content, &applied.new_content, 4);
    Ok(ToolResult {
        content: format!("Edited {display_path}"),
        is_error: false,
        details: Some(serde_json::json!({
            "path": display_path,
            "edits": edits.len(),
            "tokensBefore": applied.base_content.len(),
            "diff": diff,
            "firstChangedLine": first_changed_line,
            "transaction":transaction,
        })),
    })
}

fn resolve_bash_timeout_ms(input: &serde_json::Value) -> Result<Option<u64>, ToolError> {
    let Some(value) = input.get("timeout") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let seconds = value.as_f64().ok_or_else(|| {
        ToolError::Failed("Invalid timeout: must be a finite number of seconds".into())
    })?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(ToolError::Failed(
            "Invalid timeout: must be a finite number of seconds".into(),
        ));
    }
    let timeout_ms = seconds * 1000.0;
    const MAX_TIMEOUT_MS: f64 = 2_147_483_647.0;
    if timeout_ms > MAX_TIMEOUT_MS {
        return Err(ToolError::Failed(format!(
            "Invalid timeout: maximum is {} seconds",
            MAX_TIMEOUT_MS / 1000.0
        )));
    }
    Ok(Some(timeout_ms as u64))
}

fn wants_background(input: &serde_json::Value) -> bool {
    input
        .get("background")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Spawn the shell with the command, stdout and stderr piped, exactly as a
/// foreground call would — a background job is the same process, only
/// nobody waits for it.
fn spawn_shell(
    cwd: &Path,
    command: &str,
    background: bool,
) -> Result<std::process::Child, ToolError> {
    let custom = std::env::var("PI_SHELL")
        .ok()
        .filter(|value| !value.is_empty());
    let config = davinci_ai::resolve_shell_config(custom.as_deref()).map_err(ToolError::Failed)?;
    let mut process = Command::new(&config.shell);
    process
        .args(&config.args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    match config.command_transport {
        davinci_ai::CommandTransport::Argv => {
            if background {
                process.arg(command).stdin(std::process::Stdio::piped());
            } else {
                process.arg(command).stdin(std::process::Stdio::null());
            }
        }
        davinci_ai::CommandTransport::Stdin => {
            process.stdin(std::process::Stdio::piped());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own group, so `job_kill` can take the whole tree down.
        process.process_group(0);
    }
    let mut child = process
        .spawn()
        .map_err(|err| ToolError::Failed(err.to_string()))?;
    if matches!(
        config.command_transport,
        davinci_ai::CommandTransport::Stdin
    ) {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin
                .write_all(command.as_bytes())
                .map_err(|err| ToolError::Failed(err.to_string()))?;
        }
    }
    Ok(child)
}

fn shell_tool(
    cwd: &Path,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let command = required_str(input, "command")?;
    let timeout_ms = resolve_bash_timeout_ms(input)?;
    let command = match std::env::var("PI_SHELL_COMMAND_PREFIX") {
        Ok(prefix) if !prefix.is_empty() => format!("{prefix}; {command}"),
        _ => command.to_string(),
    };
    let background = wants_background(input);
    let started_at_ms = crate::command_receipt::now();
    if background {
        let child = spawn_shell(cwd, &command, true)?;
        let shown = required_str(input, "command")?;
        let pid = child.id();
        let id = context
            .jobs
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .register(shown, child);
        return Ok(crate::jobs::started_result(id, pid, shown));
    }
    let timeout_label = input.get("timeout").map(|value| match value {
        serde_json::Value::Number(number) => number.to_string(),
        other => other.to_string(),
    });
    let output = if let Some(host) = &context.foreground_supervisor {
        let custom = std::env::var("PI_SHELL")
            .ok()
            .filter(|value| !value.is_empty());
        let shell =
            davinci_ai::resolve_shell_config(custom.as_deref()).map_err(ToolError::Failed)?;
        let mut argv = shell.args;
        let stdin = match shell.command_transport {
            davinci_ai::CommandTransport::Argv => {
                argv.push(command.clone());
                &[][..]
            }
            davinci_ai::CommandTransport::Stdin => command.as_bytes(),
        };
        let output = foreground::run(
            host,
            foreground::config(cwd, shell.shell.into(), argv, context)?,
            stdin,
            timeout_ms,
            timeout_label.as_deref(),
            context,
        )?;
        if let Some(capture) = &context.command_receipt {
            capture.completed(cwd, &command, started_at_ms, &output);
        }
        output
    } else {
        wait_shell_output(
            spawn_shell(cwd, &command, false)?,
            timeout_ms,
            timeout_label.as_deref(),
            context,
        )?
    };
    let mut content = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.stderr.is_empty() {
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    Ok(ToolResult {
        content,
        is_error: !output.status.success(),
        details: Some(serde_json::json!({"exitCode": output.status.code()})),
    })
}

const MAX_SHELL_STREAM_BYTES: usize = 16 * 1024 * 1024;

fn read_shell_stream(mut pipe: impl std::io::Read, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut overflow = false;
    loop {
        let count = match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let retained = count.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..retained]);
        overflow |= retained != count;
        // Keep draining after the cap: stopping here could block the child on
        // a full pipe. Incomplete output must never become verification evidence.
    }
    if overflow {
        return Err(std::io::Error::other(
            "command output exceeded stream byte limit",
        ));
    }
    Ok(bytes)
}

fn join_shell_stream(
    handle: Option<std::thread::JoinHandle<std::io::Result<Vec<u8>>>>,
) -> Result<Vec<u8>, ToolError> {
    match handle {
        None => Ok(Vec::new()),
        Some(handle) => handle
            .join()
            .map_err(|_| ToolError::Failed("command output reader panicked".into()))?
            .map_err(|error| ToolError::Failed(format!("command output capture failed: {error}"))),
    }
}

fn wait_shell_output(
    mut child: std::process::Child,
    timeout_ms: Option<u64>,
    timeout_label: Option<&str>,
    context: &ToolContext,
) -> Result<std::process::Output, ToolError> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_handle = stdout
        .map(|pipe| std::thread::spawn(move || read_shell_stream(pipe, MAX_SHELL_STREAM_BYTES)));
    let stderr_handle = stderr
        .map(|pipe| std::thread::spawn(move || read_shell_stream(pipe, MAX_SHELL_STREAM_BYTES)));
    // Poll rather than block in `wait`: the turn's abort flag has to be able
    // to end the command, timeout or not.
    let start = std::time::Instant::now();
    let limit = timeout_ms.map(std::time::Duration::from_millis);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                let timed_out = limit.is_some_and(|limit| start.elapsed() >= limit);
                let aborted = context.is_aborted();
                if !timed_out && !aborted {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
                let _ = child.kill();
                let _ = child.wait();
                let stdout = join_shell_stream(stdout_handle);
                let stderr = join_shell_stream(stderr_handle);
                let stdout = stdout?;
                let stderr = stderr?;
                let mut content = String::from_utf8_lossy(&stdout).into_owned();
                if !stderr.is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&String::from_utf8_lossy(&stderr));
                }
                let status = if timed_out {
                    let seconds = timeout_label.unwrap_or("0");
                    format!("Command timed out after {seconds} seconds")
                } else {
                    "Command aborted".to_string()
                };
                return Err(ToolError::Failed(if content.is_empty() {
                    status
                } else {
                    format!("{content}\n\n{status}")
                }));
            }
            Err(err) => return Err(ToolError::Failed(err.to_string())),
        }
    };
    // Join both even when one failed, so no reader is detached on this path.
    let stdout = join_shell_stream(stdout_handle);
    let stderr = join_shell_stream(stderr_handle);
    Ok(std::process::Output {
        status,
        stdout: stdout?,
        stderr: stderr?,
    })
}

fn powershell_tool(
    cwd: &Path,
    input: &serde_json::Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let command = required_str(input, "command")?;
    let timeout_ms = resolve_bash_timeout_ms(input)?;
    let timeout_label = input.get("timeout").map(ToString::to_string);
    if let Ok(reply) = std::env::var("PI_POWERSHELL_REPLY") {
        return Ok(ToolResult {
            content: reply,
            is_error: false,
            details: Some(serde_json::json!({"exitCode": 0})),
        });
    }
    let wrapped = format!("{POWERSHELL_UTF8_PREFIX}{command}");
    let background = wants_background(input);
    if !background {
        if let Some(host) = &context.foreground_supervisor {
            let executable = ["pwsh", "powershell"]
                .into_iter()
                .find_map(|program| {
                    crate::process_manager::resolve_native_executable(program, cwd).ok()
                })
                .ok_or_else(|| {
                    ToolError::Failed(
                        "PowerShell is not available and could not be launched".into(),
                    )
                })?;
            let started_at_ms = crate::command_receipt::now();
            let config = foreground::config(
                cwd,
                executable,
                vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    wrapped,
                ],
                context,
            )?;
            let output = foreground::run(
                host,
                config,
                &[],
                timeout_ms,
                timeout_label.as_deref(),
                context,
            )?;
            if let Some(capture) = &context.command_receipt {
                capture.completed(cwd, command, started_at_ms, &output);
            }
            let mut content = String::from_utf8_lossy(&output.stdout).into_owned();
            if !output.stderr.is_empty() {
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            return Ok(ToolResult {
                content,
                is_error: !output.status.success(),
                details: Some(serde_json::json!({"exitCode": output.status.code()})),
            });
        }
    }
    for program in ["pwsh", "powershell"] {
        let stdin = if background {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        };
        let spawned = Command::new(program)
            .args(["-NoProfile", "-NonInteractive", "-Command", &wrapped])
            .current_dir(cwd)
            .stdin(stdin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();
        let child = match spawned {
            Ok(child) => child,
            Err(_) => continue,
        };
        if background {
            let pid = child.id();
            let id = context
                .jobs
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .register(command, child);
            return Ok(crate::jobs::started_result(id, pid, command));
        }
        let output = wait_shell_output(child, timeout_ms, timeout_label.as_deref(), context)?;
        let mut content = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.stderr.is_empty() {
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        return Ok(ToolResult {
            content,
            is_error: !output.status.success(),
            details: Some(serde_json::json!({"exitCode": output.status.code()})),
        });
    }
    Err(ToolError::Failed(
        "PowerShell is not available and could not be launched".into(),
    ))
}

fn ls_tool(cwd: &Path, input: &serde_json::Value) -> Result<ToolResult, ToolError> {
    let path = resolve(
        cwd,
        input.get("path").and_then(|v| v.as_str()).unwrap_or("."),
    )?;
    if !path.exists() {
        return Err(ToolError::Failed(format!(
            "Path not found: {}",
            path.display()
        )));
    }
    if !path.is_dir() {
        return Err(ToolError::Failed(format!(
            "Not a directory: {}",
            path.display()
        )));
    }
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(LS_DEFAULT_LIMIT)
        .max(1);
    let mut entries: Vec<String> = fs::read_dir(&path)
        .map_err(|err| ToolError::Failed(format!("Cannot read directory: {err}")))?
        .flatten()
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            // A symlink's own type says nothing about its target; only
            // then is the `stat` worth paying for.
            let is_dir = match entry.file_type() {
                Ok(kind) if !kind.is_symlink() => kind.is_dir(),
                _ => entry.path().is_dir(),
            };
            if is_dir {
                format!("{name}/")
            } else {
                name
            }
        })
        .collect();
    entries.sort_by_key(|name| name.to_ascii_lowercase());
    if entries.is_empty() {
        return Ok(ToolResult {
            content: "(empty directory)".into(),
            is_error: false,
            details: None,
        });
    }
    let entry_limit_reached = entries.len() > limit;
    entries.truncate(limit);
    let mut output = entries.join("\n");
    let mut details = serde_json::Map::new();
    if entry_limit_reached {
        output.push_str(&format!(
            "\n\n[{limit} entries limit reached. Use limit={} for more]",
            limit.saturating_mul(2)
        ));
        details.insert("entryLimitReached".into(), serde_json::json!(limit));
    }
    Ok(ToolResult {
        content: output,
        is_error: false,
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
    })
}

fn path_is_inside_git_repo(search_path: &Path) -> bool {
    let mut current = if search_path.is_file() {
        search_path.parent().unwrap_or(search_path).to_path_buf()
    } else {
        search_path.to_path_buf()
    };
    loop {
        if current.join(".git").exists() {
            return true;
        }
        let Some(parent) = current.parent() else {
            return false;
        };
        if parent == current {
            return false;
        }
        current = parent.to_path_buf();
    }
}

fn build_fd_args(pattern: &str, search_path: &Path, limit: usize) -> Vec<String> {
    let mut args = vec!["--glob".into(), "--color=never".into(), "--hidden".into()];
    if !path_is_inside_git_repo(search_path) {
        args.push("--no-require-git".into());
    }
    args.push("--max-results".into());
    args.push(limit.to_string());
    let mut effective_pattern = pattern.to_string();
    if pattern.contains('/') {
        args.push("--full-path".into());
        if !pattern.starts_with('/') && !pattern.starts_with("**/") && pattern != "**" {
            effective_pattern = format!("**/{pattern}");
        }
        if cfg!(windows) {
            effective_pattern = effective_pattern.replace('/', "[/\\\\]");
        }
    }
    args.push("--".into());
    args.push(effective_pattern);
    args.push(search_path.to_string_lossy().into_owned());
    args
}

fn build_rg_args(
    pattern: &str,
    search_path: &Path,
    glob: Option<&str>,
    ignore_case: bool,
    literal: bool,
) -> Vec<String> {
    let mut args = vec![
        "--json".into(),
        "--line-number".into(),
        "--color=never".into(),
        "--hidden".into(),
    ];
    if ignore_case {
        args.push("--ignore-case".into());
    }
    if literal {
        args.push("--fixed-strings".into());
    }
    if let Some(glob) = glob {
        args.push("--glob".into());
        args.push(glob.to_string());
    }
    args.push("--".into());
    args.push(pattern.to_string());
    args.push(search_path.to_string_lossy().into_owned());
    args
}

fn run_managed_tool(
    env_name: &str,
    default_name: &str,
    args: &[String],
) -> Result<Option<std::process::Output>, ToolError> {
    let program = std::env::var(env_name)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_name.to_string());
    match Command::new(&program).args(args).output() {
        Ok(output) => Ok(Some(output)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(ToolError::Failed(format!(
            "Failed to run {default_name}: {err}"
        ))),
    }
}

/// One `match` event out of ripgrep's `--json` stream: file, line number
/// and the line's text when ripgrep sent it.
type RgMatch = (PathBuf, usize, Option<String>);

/// Parse a `--json` match event. Anything that is not a complete match
/// (summaries, `begin`/`end` markers, a malformed line) yields `None`.
fn parse_rg_match(line: &str) -> Option<RgMatch> {
    let event = serde_json::from_str::<Value>(line).ok()?;
    if event.get("type").and_then(Value::as_str) != Some("match") {
        return None;
    }
    let data = event.get("data")?;
    let file = data.get("path")?.get("text")?.as_str()?;
    let line_number = data.get("line_number")?.as_u64()?;
    let line_text = data
        .get("lines")
        .and_then(|value| value.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((PathBuf::from(file), line_number as usize, line_text))
}

/// What reading a ripgrep stream ended with.
struct RgStream {
    matches: Vec<RgMatch>,
    /// `limit` matches are in hand; the caller stops the child rather
    /// than waiting for it to finish walking the tree.
    limit_reached: bool,
    /// The turn's abort flag was raised while the stream was still open.
    aborted: bool,
}

fn is_secret_search_path(path: &Path) -> bool {
    crate::permission::is_secret_file_path(&path.to_string_lossy())
        || path.canonicalize().is_ok_and(|resolved| {
            crate::permission::is_secret_file_path(&resolved.to_string_lossy())
        })
}

/// Recursive permission applies to the search root, not every descendant.
/// Secret files require an explicit path that the permission gate can approve.
fn excludes_secret_descendants(search_path: &Path) -> bool {
    search_path.is_dir() && !is_secret_search_path(search_path)
}

/// Read ripgrep's `--json` output as it arrives (grep.ts reads it line by
/// line through `readline` for the same reason): the read stops the moment
/// `limit` matches have been collected or the turn is aborted, so the
/// caller can kill ripgrep instead of waiting for it to visit every file
/// under the search path. Before this the tool sat in `Command::output`
/// for as long as ripgrep took — a worker that grepped its home directory
/// held its turn for half an hour with the matches already in the pipe.
///
/// The pipe is drained on its own thread and handed over a channel so the
/// abort flag can be polled while nothing arrives.
fn stream_rg_matches<R: std::io::Read + Send + 'static>(
    pipe: R,
    limit: usize,
    context: &ToolContext,
    exclude_secrets: bool,
) -> RgStream {
    use std::io::BufRead;
    use std::sync::mpsc::{self, RecvTimeoutError};
    let (sender, receiver) = mpsc::sync_channel::<String>(64);
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(pipe).lines() {
            let Ok(line) = line else {
                break;
            };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let mut stream = RgStream {
        matches: Vec::new(),
        limit_reached: false,
        aborted: false,
    };
    loop {
        if context.is_aborted() {
            stream.aborted = true;
            break;
        }
        match receiver.recv_timeout(std::time::Duration::from_millis(10)) {
            Ok(line) => {
                if let Some(found) = parse_rg_match(&line) {
                    if exclude_secrets && is_secret_search_path(&found.0) {
                        continue;
                    }
                    stream.matches.push(found);
                    if stream.matches.len() >= limit {
                        stream.limit_reached = true;
                        break;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    stream
}

/// Run ripgrep and collect up to `limit` matches from its stream, killing
/// it as soon as they are in hand. `None` when ripgrep is not installed,
/// which sends the caller to the native walk.
fn run_rg_streaming(
    args: &[String],
    limit: usize,
    context: &ToolContext,
    exclude_secrets: bool,
) -> Result<Option<(Vec<RgMatch>, bool)>, ToolError> {
    use std::io::Read;
    use std::process::Stdio;
    let program = std::env::var("PI_RG_PATH")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "rg".to_string());
    let mut child = match Command::new(&program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(ToolError::Failed(format!("Failed to run ripgrep: {err}")));
        }
    };
    // stderr is drained on its own thread so a chatty ripgrep (permission
    // errors under a home directory) can never block on a full pipe.
    let stderr_handle = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    });
    let stream = match child.stdout.take() {
        Some(pipe) => stream_rg_matches(pipe, limit, context, exclude_secrets),
        None => RgStream {
            matches: Vec::new(),
            limit_reached: false,
            aborted: false,
        },
    };
    if stream.limit_reached || stream.aborted {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|err| ToolError::Failed(format!("Failed to run ripgrep: {err}")))?;
    let stderr = stderr_handle
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    if stream.aborted {
        return Err(ToolError::Failed("Operation aborted".into()));
    }
    let code = status.code().unwrap_or(-1);
    if !stream.limit_reached && code != 0 && code != 1 {
        let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(ToolError::Failed(if stderr.is_empty() {
            format!("ripgrep exited with code {code}")
        } else {
            stderr
        }));
    }
    Ok(Some((stream.matches, stream.limit_reached)))
}

fn grep_tool(
    cwd: &Path,
    input: &serde_json::Value,
    tool_context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let pattern = required_str(input, "pattern")?;
    let search_path = resolve(
        cwd,
        input.get("path").and_then(Value::as_str).unwrap_or("."),
    )?;
    if !search_path.exists() {
        return Err(ToolError::Failed(format!(
            "Path not found: {}",
            search_path.display()
        )));
    }
    if tool_context.is_aborted() {
        return Err(ToolError::Failed("Operation aborted".into()));
    }
    let glob = input.get("glob").and_then(Value::as_str);
    let ignore_case = input
        .get("ignoreCase")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let literal = input
        .get("literal")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let context = input.get("context").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(GREP_DEFAULT_LIMIT)
        .max(1);
    let args = build_rg_args(pattern, &search_path, glob, ignore_case, literal);
    let Some((raw_matches, match_limit_reached)) = run_rg_streaming(
        &args,
        limit,
        tool_context,
        excludes_secret_descendants(&search_path),
    )?
    else {
        return grep_tool_native(cwd, input, tool_context);
    };
    let is_dir = search_path.is_dir();
    if raw_matches.is_empty() {
        return Ok(ToolResult {
            content: "No matches found".into(),
            is_error: false,
            details: None,
        });
    }
    let mut lines_truncated = false;
    let mut matches = Vec::new();
    for (file, line_number, line_text) in raw_matches {
        let display = format_grep_path(&file, &search_path, is_dir);
        if context == 0 {
            let text = line_text
                .as_deref()
                .unwrap_or("")
                .replace("\r\n", "\n")
                .replace('\r', "")
                .trim_end_matches('\n')
                .to_string();
            let (text, truncated) = truncate_line(&text);
            lines_truncated |= truncated;
            matches.push(format!("{display}:{line_number}: {text}"));
            continue;
        }
        let Ok(body) = fs::read_to_string(&file) else {
            matches.push(format!("{display}:{line_number}: (unable to read file)"));
            continue;
        };
        let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
        let file_lines: Vec<&str> = normalized.split('\n').collect();
        let start = line_number.saturating_sub(context).max(1);
        let end = (line_number + context).min(file_lines.len());
        for current in start..=end {
            let (text, truncated) =
                truncate_line(file_lines.get(current - 1).copied().unwrap_or(""));
            lines_truncated |= truncated;
            if current == line_number {
                matches.push(format!("{display}:{current}: {text}"));
            } else {
                matches.push(format!("{display}-{current}- {text}"));
            }
        }
    }
    let mut output_text = matches.join("\n");
    let mut details = serde_json::Map::new();
    let mut notices = Vec::new();
    if match_limit_reached {
        notices.push(format!(
            "{limit} matches limit reached. Use limit={} for more, or refine pattern",
            limit.saturating_mul(2)
        ));
        details.insert("matchLimitReached".into(), serde_json::json!(limit));
    }
    if lines_truncated {
        notices.push(format!(
            "Some lines truncated to {GREP_MAX_LINE_LENGTH} chars. Use read tool to see full lines"
        ));
        details.insert("linesTruncated".into(), Value::Bool(true));
    }
    if !notices.is_empty() {
        output_text.push_str("\n\n[");
        output_text.push_str(&notices.join(". "));
        output_text.push(']');
    }
    Ok(ToolResult {
        content: output_text,
        is_error: false,
        details: if details.is_empty() {
            None
        } else {
            Some(Value::Object(details))
        },
    })
}

/// How many files the native grep scans at once. Reading is the cost, and
/// it overlaps well on the SSDs the tool runs against.
const GREP_SCAN_THREADS: usize = 8;
/// Below this many files a pool costs more than it saves.
const GREP_PARALLEL_MIN_FILES: usize = 32;
/// Bytes inspected for a NUL to decide a file is binary (what ripgrep does).
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// The compiled form of a grep pattern: built once per call, not once per
/// line as the first version did.
enum Matcher {
    Literal(String),
    LiteralIgnoreCase(String),
    Regex(regex::Regex),
}

impl Matcher {
    fn new(pattern: &str, ignore_case: bool, literal: bool) -> Self {
        if literal {
            return if ignore_case {
                Self::LiteralIgnoreCase(pattern.to_ascii_lowercase())
            } else {
                Self::Literal(pattern.to_string())
            };
        }
        match regex::RegexBuilder::new(pattern)
            .case_insensitive(ignore_case)
            .build()
        {
            Ok(regex) => Self::Regex(regex),
            // An invalid regex falls back to a substring match, as before.
            Err(_) => Self::new(pattern, ignore_case, true),
        }
    }

    fn is_match(&self, line: &str) -> bool {
        match self {
            Self::Literal(needle) => line.contains(needle.as_str()),
            Self::LiteralIgnoreCase(needle) => {
                // Only lower-case a line that could match at all.
                line.len() >= needle.len() && line.to_ascii_lowercase().contains(needle.as_str())
            }
            Self::Regex(regex) => regex.is_match(line),
        }
    }
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_SNIFF_BYTES).any(|byte| *byte == 0)
}

/// A scanned file's rendered match lines and whether any line was cut.
type ScannedFile = (Vec<String>, bool);

/// Scan one file for the native grep. Returns the rendered match lines (with
/// context) and whether any line was cut, or `None` for a file that is not
/// text.
fn grep_scan_file(
    file: &Path,
    matcher: &Matcher,
    context: usize,
    display: &str,
    remaining: usize,
) -> Option<(Vec<String>, bool)> {
    let bytes = fs::read(file).ok()?;
    if looks_binary(&bytes) {
        return None;
    }
    let body = String::from_utf8_lossy(&bytes);
    let mut out = Vec::new();
    let mut lines_truncated = false;
    if context == 0 {
        for (index, line) in body.lines().enumerate() {
            if out.len() >= remaining {
                break;
            }
            if matcher.is_match(line) {
                let (text, truncated) = truncate_line(line);
                lines_truncated |= truncated;
                out.push(format!("{display}:{}: {text}", index + 1));
            }
        }
        return Some((out, lines_truncated));
    }
    let file_lines: Vec<&str> = body.lines().collect();
    for (index, line) in file_lines.iter().enumerate() {
        if out.len() >= remaining {
            break;
        }
        if !matcher.is_match(line) {
            continue;
        }
        let start = index.saturating_sub(context);
        let end = (index + context + 1).min(file_lines.len());
        for (current, line) in file_lines.iter().enumerate().take(end).skip(start) {
            let (text, truncated) = truncate_line(line);
            lines_truncated |= truncated;
            if current == index {
                out.push(format!("{display}:{}: {text}", current + 1));
            } else {
                out.push(format!("{display}-{}- {text}", current + 1));
            }
        }
    }
    Some((out, lines_truncated))
}

fn grep_tool_native(
    cwd: &Path,
    input: &serde_json::Value,
    tool_context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let pattern = required_str(input, "pattern")?;
    let search_path = resolve(
        cwd,
        input.get("path").and_then(|v| v.as_str()).unwrap_or("."),
    )?;
    if !search_path.exists() {
        return Err(ToolError::Failed(format!(
            "Path not found: {}",
            search_path.display()
        )));
    }
    let glob = input.get("glob").and_then(|v| v.as_str());
    let ignore_case = input
        .get("ignoreCase")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let literal = input
        .get("literal")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let context = input
        .get("context")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(GREP_DEFAULT_LIMIT)
        .max(1);
    let is_dir = search_path.is_dir();
    let matcher = Matcher::new(pattern, ignore_case, literal);

    // Walk first, scan after: the walk is cheap directory metadata, the
    // scan is the file reads, and only the scan is worth spreading out.
    let mut files: Vec<PathBuf> = Vec::new();
    let exclude_secrets = excludes_secret_descendants(&search_path);
    walk_files(
        &search_path,
        &IgnoreRules::load(&search_path),
        &mut |file| {
            if glob.is_none_or(|glob| path_glob_match(glob, file, &search_path))
                && !(exclude_secrets && is_secret_search_path(file))
            {
                files.push(file.to_path_buf());
            }
            // A large tree is abandoned at the next file once the turn is
            // aborted, like the ripgrep path.
            !tool_context.is_aborted()
        },
    );
    if tool_context.is_aborted() {
        return Err(ToolError::Failed("Operation aborted".into()));
    }

    let scan = |file: &Path, remaining: usize| {
        let display = format_grep_path(file, &search_path, is_dir);
        grep_scan_file(file, &matcher, context, &display, remaining)
    };
    let mut matches: Vec<String> = Vec::new();
    let mut lines_truncated = false;
    if files.len() < GREP_PARALLEL_MIN_FILES {
        for file in &files {
            if matches.len() >= limit {
                break;
            }
            if let Some((lines, truncated)) = scan(file, limit - matches.len()) {
                lines_truncated |= truncated;
                matches.extend(lines);
            }
        }
    } else {
        // Every thread pulls the next file index; results land in the
        // file's slot so the merged order is the walk order, exactly as
        // the sequential scan would have produced it. `found` lets the
        // pool stop early once the limit is clearly reached.
        use std::sync::atomic::{AtomicUsize, Ordering};
        let next = AtomicUsize::new(0);
        let found = AtomicUsize::new(0);
        let slots: Mutex<Vec<Option<ScannedFile>>> =
            Mutex::new((0..files.len()).map(|_| None).collect());
        let threads = GREP_SCAN_THREADS.min(files.len());
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= files.len() || found.load(Ordering::Relaxed) >= limit {
                        break;
                    }
                    if let Some(result) = scan(&files[index], limit) {
                        found.fetch_add(result.0.len(), Ordering::Relaxed);
                        slots.lock().unwrap_or_else(|err| err.into_inner())[index] = Some(result);
                    }
                });
            }
        });
        for slot in slots.into_inner().unwrap_or_else(|err| err.into_inner()) {
            if matches.len() >= limit {
                break;
            }
            if let Some((lines, truncated)) = slot {
                lines_truncated |= truncated;
                matches.extend(lines);
            }
        }
    }
    if matches.is_empty() {
        return Ok(ToolResult {
            content: "No matches found".into(),
            is_error: false,
            details: None,
        });
    }
    let match_limit_reached = matches.len() >= limit;
    matches.truncate(limit);
    let mut output = matches.join("\n");
    let mut details = serde_json::Map::new();
    let mut notices = Vec::new();
    if match_limit_reached {
        notices.push(format!(
            "{limit} matches limit reached. Use limit={} for more, or refine pattern",
            limit.saturating_mul(2)
        ));
        details.insert("matchLimitReached".into(), serde_json::json!(limit));
    }
    if lines_truncated {
        notices.push("some lines truncated".into());
        details.insert("linesTruncated".into(), serde_json::json!(true));
    }
    if !notices.is_empty() {
        output.push_str("\n\n[");
        output.push_str(&notices.join(". "));
        output.push(']');
    }
    Ok(ToolResult {
        content: output,
        is_error: false,
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
    })
}

fn find_tool(cwd: &Path, input: &serde_json::Value) -> Result<ToolResult, ToolError> {
    let pattern = required_str(input, "pattern")?;
    let search_path = resolve(
        cwd,
        input.get("path").and_then(Value::as_str).unwrap_or("."),
    )?;
    if !search_path.exists() {
        return Err(ToolError::Failed(format!(
            "Path not found: {}",
            search_path.display()
        )));
    }
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(FIND_DEFAULT_LIMIT)
        .max(1);
    let args = build_fd_args(pattern, &search_path, limit);
    let Some(output) = run_managed_tool("PI_FD_PATH", "fd", &args)? else {
        return find_tool_native(cwd, input);
    };
    if !output.status.success() && output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ToolError::Failed(if stderr.is_empty() {
            format!("fd exited with code {}", output.status.code().unwrap_or(-1))
        } else {
            stderr
        }));
    }
    let mut hits = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let path = PathBuf::from(line);
            relativize_find_result_path(&path, &search_path)
        })
        .collect::<Vec<_>>();
    if hits.is_empty() {
        return Ok(ToolResult {
            content: "No files found matching pattern".into(),
            is_error: false,
            details: None,
        });
    }
    let result_limit_reached = hits.len() >= limit;
    hits.truncate(limit);
    let mut output_text = hits.join("\n");
    let mut details = serde_json::Map::new();
    if result_limit_reached {
        output_text.push_str(&format!("\n\n[{limit} results limit reached]"));
        details.insert("resultLimitReached".into(), serde_json::json!(limit));
    }
    Ok(ToolResult {
        content: output_text,
        is_error: false,
        details: if details.is_empty() {
            None
        } else {
            Some(Value::Object(details))
        },
    })
}

fn find_tool_native(cwd: &Path, input: &serde_json::Value) -> Result<ToolResult, ToolError> {
    let pattern = required_str(input, "pattern")?;
    let search_path = resolve(
        cwd,
        input.get("path").and_then(|v| v.as_str()).unwrap_or("."),
    )?;
    if !search_path.exists() {
        return Err(ToolError::Failed(format!(
            "Path not found: {}",
            search_path.display()
        )));
    }
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(FIND_DEFAULT_LIMIT)
        .max(1);
    let mut hits = Vec::new();
    walk_files(
        &search_path,
        &IgnoreRules::load(&search_path),
        &mut |file| {
            if hits.len() >= limit {
                return false;
            }
            if path_glob_match(pattern, file, &search_path) {
                hits.push(relativize_find_result_path(file, &search_path));
            }
            true
        },
    );
    if hits.is_empty() {
        return Ok(ToolResult {
            content: "No files found matching pattern".into(),
            is_error: false,
            details: None,
        });
    }
    let result_limit_reached = hits.len() >= limit;
    hits.truncate(limit);
    let mut output = hits.join("\n");
    let mut details = serde_json::Map::new();
    if result_limit_reached {
        output.push_str(&format!("\n\n[{limit} results limit reached]"));
        details.insert("resultLimitReached".into(), serde_json::json!(limit));
    }
    Ok(ToolResult {
        content: output,
        is_error: false,
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
    })
}

pub fn relativize_find_result_path(result_path: &Path, search_path: &Path) -> String {
    let display = result_path.to_string_lossy();
    let had_trailing = display.ends_with('/') || display.ends_with('\\');
    let relative = if result_path.is_absolute() || display.starts_with('/') {
        match result_path.strip_prefix(search_path) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(_) => {
                // Keep the TypeScript-style root relativization even when a
                // fixture uses POSIX paths on Windows.
                let root = search_path.to_string_lossy();
                display
                    .strip_prefix(root.as_ref())
                    .map(|path| path.trim_start_matches(['/', '\\']).to_owned())
                    .unwrap_or_else(|| display.into_owned())
            }
        }
    } else {
        display.into_owned()
    };
    let mut posix = relative.replace('\\', "/");
    if posix.is_empty() {
        posix = ".".into();
    }
    if had_trailing && !posix.ends_with('/') {
        posix.push('/');
    }
    posix
}

fn required_str<'a>(input: &'a serde_json::Value, field: &str) -> Result<&'a str, ToolError> {
    input
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::Failed(format!("Missing {field}")))
}

fn resolve(cwd: &Path, path: &str) -> Result<PathBuf, ToolError> {
    let path = PathBuf::from(path);
    Ok(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

pub fn resolve_for_mutation(cwd: &Path, raw_path: &str) -> Result<PathBuf, ToolError> {
    let path = resolve(cwd, raw_path)?;
    let (_, symlink_escape) = crate::permission::check_path_boundary(cwd, &path);
    if symlink_escape {
        return Err(ToolError::Failed(format!(
            "Untrusted symlink escape rejected for mutation: '{raw_path}'"
        )));
    }
    Ok(path)
}

struct IgnoreRules {
    /// Rules without a `/`: matched against an entry's own name.
    name_patterns: Vec<String>,
    /// Rules with a `/`: matched against the whole path.
    path_patterns: Vec<String>,
}

impl IgnoreRules {
    fn load(root: &Path) -> Self {
        let mut patterns = vec![".git".into(), "node_modules".into()];
        let mut current = if root.is_file() {
            root.parent().unwrap_or(root).to_path_buf()
        } else {
            root.to_path_buf()
        };
        loop {
            let gitignore = current.join(".gitignore");
            if let Ok(body) = fs::read_to_string(&gitignore) {
                for line in body.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                        continue;
                    }
                    patterns.push(line.trim_end_matches('/').to_string());
                }
            }
            if current.join(".git").exists() {
                break;
            }
            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => break,
            }
        }
        let (path_patterns, name_patterns): (Vec<String>, Vec<String>) = patterns
            .into_iter()
            .partition(|pattern| pattern.contains('/'));
        Self {
            name_patterns,
            path_patterns,
        }
    }

    /// The walk prunes an ignored directory before descending, so an entry
    /// only has to be judged by its own name against the name rules
    /// (`.git`, `target`, `*.log`); the path rules (`docs/build`) see the
    /// whole path. Neither needs the string of every ancestor rebuilt per
    /// entry, which the first version did.
    fn ignored(&self, path: &Path) -> bool {
        if !self.name_patterns.is_empty() {
            if let Some(name) = path.file_name() {
                let name = name.to_string_lossy();
                if self
                    .name_patterns
                    .iter()
                    .any(|pattern| glob_match(pattern, &name))
                {
                    return true;
                }
            }
        }
        if self.path_patterns.is_empty() {
            return false;
        }
        let posix = path.to_string_lossy().replace('\\', "/");
        self.path_patterns
            .iter()
            .any(|pattern| glob_match(pattern, &posix) || posix.ends_with(pattern))
    }
}

/// Depth-first walk in a stable order (entries sorted by name, so results
/// do not depend on the file system's iteration order). The entry's own
/// file type is used — `read_dir` already knows it — instead of a `stat`
/// per path, and symlinked directories are not followed.
fn walk_files(root: &Path, ignore: &IgnoreRules, visit: &mut dyn FnMut(&Path) -> bool) {
    if root.is_file() {
        let _ = visit(root);
        return;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<fs::DirEntry> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        // Directories are pushed in reverse so the stack pops them in name
        // order.
        let mut dirs = Vec::new();
        for entry in entries {
            let path = entry.path();
            if ignore.ignored(&path) {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                dirs.push(path);
            } else if file_type.is_file() {
                if !visit(&path) {
                    return;
                }
            } else if file_type.is_symlink() {
                // A link to a file is searched like the file; a link to a
                // directory is not followed (cycles).
                if path.is_file() && !visit(&path) {
                    return;
                }
            }
        }
        stack.extend(dirs.into_iter().rev());
    }
}

fn format_grep_path(file: &Path, search_path: &Path, is_dir: bool) -> String {
    if is_dir {
        let relative = file
            .strip_prefix(search_path)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| {
                file.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        if !relative.is_empty() && !relative.starts_with("..") {
            return relative;
        }
    }
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string())
}

fn truncate_line(line: &str) -> (String, bool) {
    let sanitized = line.replace('\r', "");
    if sanitized.chars().count() > GREP_MAX_LINE_LENGTH {
        (sanitized.chars().take(GREP_MAX_LINE_LENGTH).collect(), true)
    } else {
        (sanitized, false)
    }
}

pub(crate) fn truncate_read(
    content: &str,
    offset: usize,
    limit: Option<usize>,
) -> (String, serde_json::Value) {
    let lines: Vec<&str> = if content.is_empty() {
        Vec::new()
    } else {
        let mut lines: Vec<&str> = content.split('\n').collect();
        if content.ends_with('\n') {
            lines.pop();
        }
        lines
    };
    let start = offset.saturating_sub(1).min(lines.len());
    let max_lines = limit.unwrap_or(DEFAULT_MAX_LINES);
    let mut out = Vec::new();
    let mut bytes = 0usize;
    let mut truncated_by = None;
    for (index, line) in lines.iter().enumerate().skip(start) {
        if out.len() >= max_lines {
            truncated_by = Some("lines");
            break;
        }
        let add = if index > start || !out.is_empty() {
            line.len() + 1
        } else {
            line.len()
        };
        if bytes + add > DEFAULT_MAX_BYTES {
            truncated_by = Some("bytes");
            break;
        }
        out.push(*line);
        bytes += add;
    }
    let output = out.join("\n");
    let details = serde_json::json!({
        "truncated": truncated_by.is_some(),
        "truncatedBy": truncated_by,
        "totalLines": lines.len(),
        "totalBytes": content.len(),
        "outputLines": out.len(),
        "outputBytes": output.len(),
        "maxLines": max_lines,
        "maxBytes": DEFAULT_MAX_BYTES,
    });
    (output, details)
}

fn path_glob_match(pattern: &str, file: &Path, search_path: &Path) -> bool {
    let relative = file
        .strip_prefix(search_path)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| file.to_string_lossy().replace('\\', "/"));
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    glob_match(pattern, &relative) || glob_match(pattern, &name)
}

fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == "**" || pattern == "**/*" {
        return true;
    }
    let pattern = pattern.replace('\\', "/");
    let name = name.replace('\\', "/");
    if let Some(stripped) = pattern.strip_prefix("**/") {
        return glob_match(stripped, &name)
            || name
                .rsplit('/')
                .next()
                .is_some_and(|part| glob_match(stripped, part))
            || name.split('/').any(|part| glob_match(stripped, part));
    }
    match_glob_chars(&pattern, &name)
}

fn match_glob_chars(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    fn rec(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) if p.get(1) == Some(&'*') => {
                let rest = if p.get(2) == Some(&'/') {
                    &p[3..]
                } else {
                    &p[2..]
                };
                rec(rest, n)
                    || (!n.is_empty() && rec(p, &n[1..]))
                    || (p.get(2) == Some(&'/') && n.first() == Some(&'/') && rec(&p[3..], &n[1..]))
            }
            (Some('*'), _) => rec(&p[1..], n) || (!n.is_empty() && rec(p, &n[1..])),
            (Some('?'), Some(_)) => rec(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => rec(&p[1..], &n[1..]),
            _ => false,
        }
    }
    rec(&p, &n)
}

fn code_definition_tool(
    cwd: &Path,
    input: &Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    if context.is_aborted() {
        return Err(ToolError::Failed("Operation aborted".into()));
    }
    let raw_path = required_str(input, "path")?;
    let path = resolve(cwd, raw_path)?;
    let line = input.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
    let character = input.get("character").and_then(Value::as_u64).unwrap_or(1) as u32;
    let symbol = input.get("symbol").and_then(Value::as_str);

    if let Some(semantic) = &context.semantic {
        match semantic.definition(
            cwd,
            raw_path,
            line.saturating_sub(1),
            character.saturating_sub(1),
        ) {
            Ok(res) => {
                let content = serde_json::to_string_pretty(&res).unwrap_or_default();
                return Ok(ToolResult {
                    content,
                    is_error: false,
                    details: Some(serde_json::to_value(&res).unwrap_or_default()),
                });
            }
            Err(err) => return Err(ToolError::Failed(err)),
        }
    }

    let sym = symbol.ok_or_else(|| {
        ToolError::Failed(
            "No language server available and no `symbol` argument provided for text fallback"
                .into(),
        )
    })?;
    let res =
        crate::semantic::text_fallback_definition(cwd, sym, Some(&path), context.abort.as_deref())
            .map_err(ToolError::Failed)?;
    let content = serde_json::to_string_pretty(&res).unwrap_or_default();
    Ok(ToolResult {
        content,
        is_error: false,
        details: Some(serde_json::to_value(&res).unwrap_or_default()),
    })
}

fn code_references_tool(
    cwd: &Path,
    input: &Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    if context.is_aborted() {
        return Err(ToolError::Failed("Operation aborted".into()));
    }
    let raw_path = required_str(input, "path")?;
    let path = resolve(cwd, raw_path)?;
    let line = input.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
    let character = input.get("character").and_then(Value::as_u64).unwrap_or(1) as u32;
    let symbol = input.get("symbol").and_then(Value::as_str);
    let include_decl = input
        .get("includeDeclaration")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    if let Some(semantic) = &context.semantic {
        match semantic.references(
            cwd,
            raw_path,
            line.saturating_sub(1),
            character.saturating_sub(1),
            include_decl,
        ) {
            Ok(res) => {
                let content = serde_json::to_string_pretty(&res).unwrap_or_default();
                return Ok(ToolResult {
                    content,
                    is_error: false,
                    details: Some(serde_json::to_value(&res).unwrap_or_default()),
                });
            }
            Err(err) => return Err(ToolError::Failed(err)),
        }
    }

    let sym = symbol.ok_or_else(|| {
        ToolError::Failed(
            "No language server available and no `symbol` argument provided for text fallback"
                .into(),
        )
    })?;
    let res =
        crate::semantic::text_fallback_references(cwd, sym, Some(&path), context.abort.as_deref())
            .map_err(ToolError::Failed)?;
    let content = serde_json::to_string_pretty(&res).unwrap_or_default();
    Ok(ToolResult {
        content,
        is_error: false,
        details: Some(serde_json::to_value(&res).unwrap_or_default()),
    })
}

fn code_outline_tool(
    cwd: &Path,
    input: &Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    if context.is_aborted() {
        return Err(ToolError::Failed("Operation aborted".into()));
    }
    let raw_path = required_str(input, "path")?;
    let _path = resolve(cwd, raw_path)?;

    if let Some(semantic) = &context.semantic {
        match semantic.outline(cwd, raw_path) {
            Ok(res) => {
                let content = serde_json::to_string_pretty(&res).unwrap_or_default();
                return Ok(ToolResult {
                    content,
                    is_error: false,
                    details: Some(serde_json::to_value(&res).unwrap_or_default()),
                });
            }
            Err(err) => return Err(ToolError::Failed(err)),
        }
    }

    let res = crate::semantic::text_fallback_outline(cwd, raw_path).map_err(ToolError::Failed)?;
    let content = serde_json::to_string_pretty(&res).unwrap_or_default();
    Ok(ToolResult {
        content,
        is_error: false,
        details: Some(serde_json::to_value(&res).unwrap_or_default()),
    })
}

fn code_diagnostics_tool(
    cwd: &Path,
    input: &Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    if context.is_aborted() {
        return Err(ToolError::Failed("Operation aborted".into()));
    }
    let raw_path = required_str(input, "path")?;
    let _path = resolve(cwd, raw_path)?;

    if let Some(semantic) = &context.semantic {
        match semantic.diagnostics(cwd, raw_path) {
            Ok(res) => {
                let content = serde_json::to_string_pretty(&res).unwrap_or_default();
                return Ok(ToolResult {
                    content,
                    is_error: false,
                    details: Some(serde_json::to_value(&res).unwrap_or_default()),
                });
            }
            Err(err) => return Err(ToolError::Failed(err)),
        }
    }

    let res =
        crate::semantic::text_fallback_diagnostics(cwd, raw_path).map_err(ToolError::Failed)?;
    let content = serde_json::to_string_pretty(&res).unwrap_or_default();
    Ok(ToolResult {
        content,
        is_error: false,
        details: Some(serde_json::to_value(&res).unwrap_or_default()),
    })
}

fn code_call_hierarchy_tool(
    cwd: &Path,
    input: &Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    if context.is_aborted() {
        return Err(ToolError::Failed("Operation aborted".into()));
    }
    let raw_path = required_str(input, "path")?;
    let _path = resolve(cwd, raw_path)?;
    let line = input.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
    let character = input.get("character").and_then(Value::as_u64).unwrap_or(1) as u32;
    let direction = input
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("incoming");
    let incoming = direction != "outgoing";

    if let Some(semantic) = &context.semantic {
        match semantic.call_hierarchy(
            cwd,
            raw_path,
            line.saturating_sub(1),
            character.saturating_sub(1),
            incoming,
        ) {
            Ok(res) => {
                let content = serde_json::to_string_pretty(&res).unwrap_or_default();
                return Ok(ToolResult {
                    content,
                    is_error: false,
                    details: Some(serde_json::to_value(&res).unwrap_or_default()),
                });
            }
            Err(err) => return Err(ToolError::Failed(err)),
        }
    }

    Err(ToolError::Failed(
        "Call hierarchy is unsupported without an active language server".into(),
    ))
}

fn code_rename_preview_tool(
    cwd: &Path,
    input: &Value,
    context: &ToolContext,
) -> Result<ToolResult, ToolError> {
    if context.is_aborted() {
        return Err(ToolError::Failed("Operation aborted".into()));
    }
    let raw_path = required_str(input, "path")?;
    let _path = resolve(cwd, raw_path)?;
    let line = input.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
    let character = input.get("character").and_then(Value::as_u64).unwrap_or(1) as u32;
    let new_name = required_str(input, "newName")?;

    if let Some(semantic) = &context.semantic {
        match semantic.rename_preview(
            cwd,
            raw_path,
            line.saturating_sub(1),
            character.saturating_sub(1),
            new_name,
        ) {
            Ok(preview) => {
                let content = serde_json::to_string_pretty(&preview).unwrap_or_default();
                return Ok(ToolResult {
                    content,
                    is_error: false,
                    details: Some(serde_json::to_value(&preview).unwrap_or_default()),
                });
            }
            Err(err) => return Err(ToolError::Failed(err)),
        }
    }

    Err(ToolError::Failed("Rename preview is unavailable without an active language server; plain text search cannot safely guarantee semantic rename".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn shell_capture_rejects_partial_reads_and_drains_overflow() {
        use std::io::{self, Read};
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("injected read failure"))
            }
        }
        assert!(read_shell_stream(io::Cursor::new(b"prefix").chain(Broken), 16).is_err());
        let mut oversized = io::Cursor::new(b"123456789");
        assert!(read_shell_stream(&mut oversized, 8).is_err());
        assert_eq!(
            oversized.position(),
            9,
            "overflow must still drain the pipe"
        );
        assert_eq!(
            read_shell_stream(io::Cursor::new(b"12345678"), 8).unwrap(),
            b"12345678"
        );
        assert!(read_shell_stream(io::empty(), 0).unwrap().is_empty());
    }

    #[test]
    fn shell_capture_reader_failures_are_not_empty_success() {
        let panic =
            std::thread::spawn(|| -> std::io::Result<Vec<u8>> { panic!("injected reader panic") });
        assert!(join_shell_stream(Some(panic)).is_err());
        let error = std::thread::spawn(|| Err(std::io::Error::other("injected pipe error")));
        assert!(join_shell_stream(Some(error)).is_err());
        assert!(join_shell_stream(None).unwrap().is_empty());
    }

    #[test]
    fn shell_capture_output_fixture() {
        use std::io::Write;
        let Ok(stream) = std::env::var("DAVINCI_TEST_SHELL_CAPTURE_STREAM") else {
            return;
        };
        let mut pipe: Box<dyn Write> = match stream.as_str() {
            "stdout" => Box::new(std::io::stdout()),
            "stderr" => Box::new(std::io::stderr()),
            _ => panic!("invalid fixture stream"),
        };
        let bytes = [b'x'; 8192];
        for _ in 0..=MAX_SHELL_STREAM_BYTES / bytes.len() {
            pipe.write_all(&bytes).unwrap();
        }
    }

    #[test]
    fn shell_capture_real_child_overflow_is_an_error() {
        for stream in ["stdout", "stderr"] {
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tools::tests::shell_capture_output_fixture",
                    "--nocapture",
                ])
                .env("DAVINCI_TEST_SHELL_CAPTURE_STREAM", stream)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let error = wait_shell_output(child, Some(10_000), Some("10"), &ToolContext::default())
                .unwrap_err();
            assert!(
                error.to_string().contains("stream byte limit"),
                "{stream}: {error}"
            );
        }
    }

    #[test]
    fn read_large_text_file_returns_requested_window() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("large.txt");
        let content = (1..=5000)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, content).unwrap();

        let window = read_text_window(&path, 2500, 3, 1024).unwrap();

        assert_eq!(window.first_line, 2500);
        assert_eq!(window.lines_returned, 3);
        assert_eq!(window.content, "line 2500\nline 2501\nline 2502");
        assert!(window.truncated);
    }

    #[test]
    fn read_window_preserves_utf8_at_byte_limit() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("utf8.txt");
        fs::write(&path, "éclair\nnext\n").unwrap();

        let window = read_text_window(&path, 1, 1, "éclair".len()).unwrap();

        assert_eq!(window.content, "éclair");
        assert!(!window.content.contains('\u{fffd}'));
        assert!(window.truncated);
    }

    #[test]
    fn f02_waiting_mode() {
        assert_eq!(decision_wait(false, false, false), "decision_required");
        assert_eq!(decision_wait(true, false, false), "wait_for_user");
        assert_eq!(decision_wait(true, true, false), "deferred");
        assert_eq!(decision_wait(true, false, true), "cancelled");
    }

    fn f02_question_fixture() -> (tempfile::TempDir, ToolContext, serde_json::Value) {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("src.rs"), "fn existing() {}\n").unwrap();
        let mut plan = crate::LivingPlan::default();
        plan.update(
            &serde_json::json!({
                "expected_revision":0,
                "goal":"Choose cache persistence",
                "evidence":[{"path":"src.rs","finding":"Current cache entry point"}]
            }),
            dir.path(),
        )
        .unwrap();
        let context = ToolContext {
            living_plan: Arc::new(Mutex::new(plan)),
            ..ToolContext::default()
        };
        let question = serde_json::json!({
            "id":"cache-scope",
            "kind":"persistence",
            "title":"Cache scope",
            "question":"Which cache should be used?",
            "materiality":"Changes persistence semantics",
            "evidence_refs":["src.rs"],
            "options":[
                {"id":"memory","label":"Memory","explanation":"Process local","recommended":true},
                {"id":"sqlite","label":"SQLite","explanation":"Persistent","recommended":false}
            ],
            "allow_custom":true,
            "custom_only":false
        });
        (dir, context, question)
    }

    #[test]
    fn f02_question_schema_has_content_but_no_authority_fields() {
        let spec = tool_specs()
            .into_iter()
            .find(|tool| tool.name == "ask_user_question")
            .unwrap();
        let properties = spec.parameters["properties"].as_object().unwrap();
        for forbidden in [
            "answer",
            "state",
            "actor",
            "approved_revision",
            "permission_mode",
        ] {
            assert!(
                !properties.contains_key(forbidden),
                "schema exposed {forbidden}"
            );
        }
        assert_eq!(spec.parameters["additionalProperties"], false);
    }

    #[test]
    fn f02_no_ui_and_timeout_return_pending_without_plan_mutation() {
        let (dir, context, question) = f02_question_fixture();
        let before = context.living_plan.lock().unwrap().clone();
        let unavailable =
            execute_tool_with(dir.path(), "ask_user_question", &question, &context).unwrap();
        assert!(unavailable.is_error);
        assert_eq!(
            unavailable.details.as_ref().unwrap()["status"],
            "decision_required"
        );
        assert_eq!(*context.living_plan.lock().unwrap(), before);

        let timeout_context = ToolContext {
            living_plan: context.living_plan.clone(),
            decision_responder: Some(DecisionResponder::new(|_| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                DecisionHostResponse::Unavailable
            })),
            decision_timeout: Some(std::time::Duration::from_millis(25)),
            ..ToolContext::default()
        };
        let started = std::time::Instant::now();
        let timed_out =
            execute_tool_with(dir.path(), "ask_user_question", &question, &timeout_context)
                .unwrap();
        assert!(timed_out.is_error);
        assert_eq!(timed_out.details.as_ref().unwrap()["reason"], "timeout");
        assert!(
            timed_out.details.as_ref().unwrap()["elapsed_ms"]
                .as_u64()
                .unwrap()
                >= 20
        );
        assert!(started.elapsed() < std::time::Duration::from_millis(150));
        assert_eq!(*timeout_context.living_plan.lock().unwrap(), before);
    }

    #[test]
    fn f02_host_answer_uses_authenticated_reply_and_never_changes_files() {
        let (dir, context, question) = f02_question_fixture();
        let source_before = fs::read(dir.path().join("src.rs")).unwrap();
        let answer_context = ToolContext {
            living_plan: context.living_plan.clone(),
            decision_responder: Some(DecisionResponder::new(|_| {
                DecisionHostResponse::Reply(DecisionHostReply {
                    action: crate::decisions::HostDecisionAction::AnswerChoice("sqlite".into()),
                    host_event_id: "ui-event-1".into(),
                    answered_at_ms: 42,
                })
            })),
            ..ToolContext::default()
        };
        let result =
            execute_tool_with(dir.path(), "ask_user_question", &question, &answer_context).unwrap();
        assert!(!result.is_error);
        let plan = answer_context.living_plan.lock().unwrap().clone();
        let decision = &plan.structured_decisions["cache-scope"];
        assert_eq!(
            decision.state,
            crate::decisions::DecisionState::AnsweredByUser
        );
        assert_eq!(
            decision.answer.as_ref().unwrap().host_event_id,
            "ui-event-1"
        );
        assert_eq!(fs::read(dir.path().join("src.rs")).unwrap(), source_before);
    }

    #[test]
    fn f02_one_question_capacity_and_parent_stop_are_fail_closed() {
        let (dir, context, question) = f02_question_fixture();
        *context.decision_slot.lock().unwrap() = Some("already-pending".into());
        let occupied = ToolContext {
            living_plan: context.living_plan.clone(),
            decision_slot: context.decision_slot.clone(),
            decision_responder: Some(DecisionResponder::new(|_| panic!("must not dispatch"))),
            ..ToolContext::default()
        };
        let result =
            execute_tool_with(dir.path(), "ask_user_question", &question, &occupied).unwrap();
        assert!(result.is_error);
        assert_eq!(
            result.details.as_ref().unwrap()["pending_decision_id"],
            "already-pending"
        );

        let stopped = ToolContext {
            living_plan: context.living_plan.clone(),
            abort: Some(Arc::new(std::sync::atomic::AtomicBool::new(true))),
            decision_responder: Some(DecisionResponder::new(|_| panic!("must not dispatch"))),
            ..ToolContext::default()
        };
        let result =
            execute_tool_with(dir.path(), "ask_user_question", &question, &stopped).unwrap();
        assert!(result.is_error);
        assert_eq!(result.details.as_ref().unwrap()["status"], "cancelled");

        let abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let during_wait = ToolContext {
            living_plan: context.living_plan.clone(),
            abort: Some(abort.clone()),
            decision_responder: Some(DecisionResponder::new(|_| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                DecisionHostResponse::Unavailable
            })),
            decision_timeout: Some(std::time::Duration::from_secs(1)),
            ..ToolContext::default()
        };
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(25));
            abort.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        let result =
            execute_tool_with(dir.path(), "ask_user_question", &question, &during_wait).unwrap();
        assert!(result.is_error);
        assert_eq!(result.details.as_ref().unwrap()["status"], "cancelled");
        assert!(started.elapsed() < std::time::Duration::from_millis(150));
    }

    #[test]
    fn f02_hanging_responder_times_out_promptly_without_plan_mutation() {
        let (dir, context, question) = f02_question_fixture();
        let before = context.living_plan.lock().unwrap().clone();
        let timeout_context = ToolContext {
            living_plan: context.living_plan.clone(),
            decision_responder: Some(DecisionResponder::new(|_| {
                std::thread::sleep(std::time::Duration::from_secs(5));
                DecisionHostResponse::Reply(DecisionHostReply {
                    action: crate::decisions::HostDecisionAction::AnswerChoice("sqlite".into()),
                    host_event_id: "late-event".into(),
                    answered_at_ms: 100,
                })
            })),
            decision_timeout: Some(std::time::Duration::from_millis(25)),
            ..ToolContext::default()
        };
        let started = std::time::Instant::now();
        let timed_out =
            execute_tool_with(dir.path(), "ask_user_question", &question, &timeout_context)
                .unwrap();
        assert!(timed_out.is_error);
        assert_eq!(timed_out.details.as_ref().unwrap()["reason"], "timeout");
        assert!(started.elapsed() < std::time::Duration::from_millis(200));
        assert_eq!(*timeout_context.living_plan.lock().unwrap(), before);
        assert!(timeout_context.decision_slot.lock().unwrap().is_none());
    }

    #[test]
    fn f02_abort_while_responder_blocked_cancels_promptly() {
        let (dir, context, question) = f02_question_fixture();
        let before = context.living_plan.lock().unwrap().clone();
        let abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let abort_context = ToolContext {
            living_plan: context.living_plan.clone(),
            abort: Some(abort.clone()),
            decision_responder: Some(DecisionResponder::new(|_| {
                std::thread::sleep(std::time::Duration::from_secs(5));
                DecisionHostResponse::Reply(DecisionHostReply {
                    action: crate::decisions::HostDecisionAction::AnswerChoice("sqlite".into()),
                    host_event_id: "late-event".into(),
                    answered_at_ms: 100,
                })
            })),
            decision_timeout: Some(std::time::Duration::from_secs(10)),
            ..ToolContext::default()
        };
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(25));
            abort.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        let result =
            execute_tool_with(dir.path(), "ask_user_question", &question, &abort_context).unwrap();
        assert!(result.is_error);
        assert_eq!(result.details.as_ref().unwrap()["status"], "cancelled");
        assert!(started.elapsed() < std::time::Duration::from_millis(200));
        assert_eq!(*abort_context.living_plan.lock().unwrap(), before);
        assert!(abort_context.decision_slot.lock().unwrap().is_none());
    }

    #[test]
    fn parallel_edits_on_the_same_file_are_serialized() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("parallel-edit.txt");
        std::fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();
        let cwd = dir.path().to_path_buf();
        let left = std::thread::spawn({
            let cwd = cwd.clone();
            move || {
                execute_tool(
                    &cwd,
                    "edit",
                    &serde_json::json!({
                        "path":"parallel-edit.txt",
                        "edits":[{"oldText":"alpha","newText":"ALPHA"}]
                    }),
                )
            }
        });
        let right = std::thread::spawn({
            let cwd = cwd.clone();
            move || {
                execute_tool(
                    &cwd,
                    "edit",
                    &serde_json::json!({
                        "path":"parallel-edit.txt",
                        "edits":[{"oldText":"beta","newText":"BETA"}]
                    }),
                )
            }
        });
        left.join().unwrap().unwrap();
        right.join().unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "ALPHA\nBETA\ngamma\n"
        );
    }

    #[test]
    fn read_write_edit_semantics() {
        let dir = tempdir().unwrap();
        execute_tool(
            dir.path(),
            "write",
            &serde_json::json!({"path":"a.txt","content":"hello"}),
        )
        .unwrap();
        let read = execute_tool(dir.path(), "read", &serde_json::json!({"path":"a.txt"})).unwrap();
        assert_eq!(read.content, "hello");
        execute_tool(
            dir.path(),
            "edit",
            &serde_json::json!({"path":"a.txt","oldText":"hello","newText":"world"}),
        )
        .unwrap();
        let read = execute_tool(dir.path(), "read", &serde_json::json!({"path":"a.txt"})).unwrap();
        assert_eq!(read.content, "world");
        execute_tool(
            dir.path(),
            "write",
            &serde_json::json!({"path":"b.txt","content":"one\ntwo\nthree\n"}),
        )
        .unwrap();
        let sliced = execute_tool(
            dir.path(),
            "read",
            &serde_json::json!({"path":"b.txt","offset":2,"limit":1}),
        )
        .unwrap();
        assert_eq!(sliced.content, "two");
        assert_eq!(
            sliced.details.as_ref().unwrap()["truncation"]["totalLines"],
            3
        );
        execute_tool(
            dir.path(),
            "write",
            &serde_json::json!({"path":"c.txt","content":"alpha\nbeta\ngamma\n"}),
        )
        .unwrap();
        execute_tool(
            dir.path(),
            "edit",
            &serde_json::json!({
                "path":"c.txt",
                "edits":[
                    {"oldText":"alpha","newText":"ALPHA"},
                    {"oldText":"gamma","newText":"GAMMA"}
                ]
            }),
        )
        .unwrap();
        let on_disk = fs::read_to_string(dir.path().join("c.txt")).unwrap();
        assert_eq!(on_disk, "ALPHA\nbeta\nGAMMA\n");
        let missing = execute_tool(
            dir.path(),
            "edit",
            &serde_json::json!({"path":"c.txt","edits":[{"oldText":"nope","newText":"x"}]}),
        )
        .unwrap_err();
        assert!(missing
            .to_string()
            .contains("Could not find the exact text in c.txt"));
    }

    #[test]
    fn bash_timeout_matches_ts_errors() {
        let dir = tempdir().unwrap();
        let invalid = execute_tool(
            dir.path(),
            "bash",
            &serde_json::json!({"command":"true","timeout":0}),
        )
        .unwrap_err();
        assert_eq!(
            invalid.to_string(),
            "Invalid timeout: must be a finite number of seconds"
        );
        let timed_out = execute_tool(
            dir.path(),
            "bash",
            &serde_json::json!({"command":"sleep 2","timeout":0.2}),
        )
        .unwrap_err();
        assert!(timed_out
            .to_string()
            .contains("Command timed out after 0.2 seconds"));
    }

    #[cfg(windows)]
    #[test]
    fn legacy_powershell_honors_and_validates_timeout() {
        let dir = tempdir().unwrap();
        let capture =
            crate::command_receipt::CommandReceiptCapture::new("timeout", "powershell", None);
        let context = ToolContext {
            command_receipt: Some(capture.clone()),
            ..Default::default()
        };
        let started = std::time::Instant::now();
        let error = execute_tool_with(
            dir.path(),
            "powershell",
            &serde_json::json!({"command":"Start-Sleep -Seconds 2", "timeout":0.2}),
            &context,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Command timed out after 0.2 seconds"),
            "{error}"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(capture.take().is_none());
        let invalid = execute_tool_with(dir.path(), "powershell", &serde_json::json!({"command":"Set-Content -Path should-not-exist.txt -Value ran", "timeout":0}), &context).unwrap_err();
        assert_eq!(
            invalid.to_string(),
            "Invalid timeout: must be a finite number of seconds"
        );
        assert!(!dir.path().join("should-not-exist.txt").exists());
    }

    #[test]
    fn grep_find_ls_match_ts_strings() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/app.ts"), "const needle = 1;\nkeep\n").unwrap();
        fs::write(dir.path().join(".gitignore"), "secret.txt\n").unwrap();
        fs::write(dir.path().join("secret.txt"), "needle hidden").unwrap();
        let grep = execute_tool(
            dir.path(),
            "grep",
            &serde_json::json!({"pattern":"needle","glob":"*.ts"}),
        )
        .unwrap();
        assert!(grep.content.contains("src/app.ts:1: const needle = 1;"));
        assert!(!grep.content.contains("secret.txt"));
        let missing = execute_tool(
            dir.path(),
            "grep",
            &serde_json::json!({"pattern":"nope","path":"missing"}),
        )
        .unwrap_err();
        assert!(missing.to_string().starts_with("Path not found:"));
        let none =
            execute_tool(dir.path(), "grep", &serde_json::json!({"pattern":"zzzz"})).unwrap();
        assert_eq!(none.content, "No matches found");
        let found =
            execute_tool(dir.path(), "find", &serde_json::json!({"pattern":"*.ts"})).unwrap();
        assert_eq!(found.content, "src/app.ts");
        let empty =
            execute_tool(dir.path(), "find", &serde_json::json!({"pattern":"*.rs"})).unwrap();
        assert_eq!(empty.content, "No files found matching pattern");
        let listed = execute_tool(dir.path(), "ls", &serde_json::json!({})).unwrap();
        assert!(listed.content.contains("src/"));
        let empty_dir = dir.path().join("blank");
        fs::create_dir_all(&empty_dir).unwrap();
        let empty_ls =
            execute_tool(dir.path(), "ls", &serde_json::json!({"path":"blank"})).unwrap();
        assert_eq!(empty_ls.content, "(empty directory)");
        let not_dir =
            execute_tool(dir.path(), "ls", &serde_json::json!({"path":"src/app.ts"})).unwrap_err();
        assert!(not_dir.to_string().starts_with("Not a directory:"));
    }

    #[test]
    fn audit_regression_recursive_grep_omits_credentials() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("nested/secrets")).unwrap();
        for name in [
            ".env",
            ".env.local",
            "ID_RSA_backup",
            "key.pem",
            "nested/secrets/token.txt",
        ] {
            fs::write(dir.path().join(name), "needle PRIVATE_FIXTURE\n").unwrap();
        }
        fs::write(dir.path().join("public.txt"), "needle PUBLIC_FIXTURE\n").unwrap();
        for native in [false, true] {
            for glob in ["**/*", ".env", "**/.env*", "[.]env", "*.pem"] {
                let input =
                    serde_json::json!({"pattern":"needle", "path":".", "glob":glob, "context":1});
                let result = if native {
                    grep_tool_native(dir.path(), &input, &ToolContext::default())
                } else {
                    grep_tool(dir.path(), &input, &ToolContext::default())
                }
                .unwrap();
                assert!(
                    !result.content.contains("PRIVATE_FIXTURE"),
                    "credential exposed for glob {glob}, native={native}"
                );
                if glob == "**/*" {
                    assert!(result.content.contains("PUBLIC_FIXTURE"));
                }
            }
            // The caller's permission gate still authorizes an explicit secret path.
            let input = serde_json::json!({"pattern":"needle", "path":".env"});
            let explicit = if native {
                grep_tool_native(dir.path(), &input, &ToolContext::default())
            } else {
                grep_tool(dir.path(), &input, &ToolContext::default())
            }
            .unwrap();
            assert!(explicit.content.contains("PRIVATE_FIXTURE"));
        }
    }

    /// A ripgrep stdout that never ends: one match event per read, forever.
    struct EndlessRg(usize);

    impl std::io::Read for EndlessRg {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0 += 1;
            let line = format!(
                "{{\"type\":\"match\",\"data\":{{\"path\":{{\"text\":\"f{}.rs\"}},\"line_number\":{},\"lines\":{{\"text\":\"needle\\n\"}}}}}}\n",
                self.0, self.0
            );
            let bytes = line.as_bytes();
            let n = bytes.len().min(buf.len());
            buf[..n].copy_from_slice(&bytes[..n]);
            Ok(n)
        }
    }

    /// A ripgrep stdout that produces nothing and never closes, like a
    /// walk over a huge tree that has found nothing yet.
    struct SilentRg {
        receiver: std::sync::mpsc::Receiver<u8>,
        /// Held so the receiver blocks instead of seeing a closed channel.
        _sender: std::sync::mpsc::Sender<u8>,
    }

    impl std::io::Read for SilentRg {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            let _ = self.receiver.recv();
            Ok(0)
        }
    }

    #[test]
    fn grep_stops_reading_ripgrep_at_the_match_limit() {
        let stream = stream_rg_matches(EndlessRg(0), 5, &ToolContext::default(), false);
        assert_eq!(stream.matches.len(), 5);
        assert!(stream.limit_reached);
        assert!(!stream.aborted);
        assert_eq!(stream.matches[4].0, PathBuf::from("f5.rs"));
        assert_eq!(stream.matches[4].1, 5);
    }

    #[test]
    fn grep_stops_reading_ripgrep_when_the_turn_is_aborted() {
        let abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let context = ToolContext {
            abort: Some(abort.clone()),
            ..ToolContext::default()
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn({
            let abort = abort.clone();
            move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                abort.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let started = std::time::Instant::now();
        let stream = stream_rg_matches(
            SilentRg {
                receiver,
                _sender: sender,
            },
            5,
            &context,
            false,
        );
        assert!(stream.aborted);
        assert!(stream.matches.is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn grep_honours_the_limit_and_the_abort_flag_end_to_end() {
        let dir = tempdir().unwrap();
        for index in 0..6 {
            fs::write(
                dir.path().join(format!("file{index}.txt")),
                "needle one\nneedle two\n",
            )
            .unwrap();
        }
        let limited = execute_tool(
            dir.path(),
            "grep",
            &serde_json::json!({"pattern":"needle","limit":2}),
        )
        .unwrap();
        let match_lines = limited
            .content
            .lines()
            .filter(|line| line.contains(": needle"))
            .count();
        assert_eq!(match_lines, 2, "{}", limited.content);
        assert!(limited.content.contains("2 matches limit reached"));
        assert_eq!(
            limited
                .details
                .and_then(|d| d.get("matchLimitReached").cloned()),
            Some(serde_json::json!(2))
        );
        let aborted = execute_tool_with(
            dir.path(),
            "grep",
            &serde_json::json!({"pattern":"needle"}),
            &ToolContext {
                abort: Some(Arc::new(std::sync::atomic::AtomicBool::new(true))),
                ..ToolContext::default()
            },
        )
        .unwrap_err();
        assert_eq!(aborted.to_string(), "Operation aborted");
    }

    #[test]
    fn powershell_and_image_read() {
        // Keep the reply override out of other concurrently running command tests.
        if std::env::var_os("DAVINCI_POWERSHELL_REPLY_FIXTURE").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tools::tests::powershell_and_image_read",
                    "--nocapture",
                ])
                .env("DAVINCI_POWERSHELL_REPLY_FIXTURE", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "fixture must execute its child test"
            );
            return;
        }
        std::env::set_var("PI_POWERSHELL_REPLY", "ps-ok");
        let dir = tempdir().unwrap();
        let capture =
            crate::command_receipt::CommandReceiptCapture::new("simulated", "powershell", None);
        let context = ToolContext {
            command_receipt: Some(capture.clone()),
            ..Default::default()
        };
        let ps = execute_tool_with(
            dir.path(),
            "powershell",
            &serde_json::json!({"command":"Get-Date"}),
            &context,
        )
        .unwrap();
        std::env::remove_var("PI_POWERSHELL_REPLY");
        assert_eq!(ps.content, "ps-ok");
        assert!(
            capture.take().is_none(),
            "simulated output must not produce execution evidence"
        );
        assert!(tool_specs().iter().any(|tool| tool.name == "powershell"));
        let png = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(png)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        fs::write(dir.path().join("dot.png"), &bytes).unwrap();
        let read =
            execute_tool(dir.path(), "read", &serde_json::json!({"path":"dot.png"})).unwrap();
        assert!(read.content.starts_with("Read image file [image/png]"));
        assert!(read.details.as_ref().unwrap()["image"]["data"].is_string());
    }

    #[test]
    fn fd_and_rg_argv_match_typescript_tools() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        let fd = build_fd_args("src/*.rs", dir.path(), 25);
        let expected_pattern = if cfg!(windows) {
            "**[/\\\\]src[/\\\\]*.rs"
        } else {
            "**/src/*.rs"
        };
        assert_eq!(
            fd,
            vec![
                "--glob",
                "--color=never",
                "--hidden",
                "--max-results",
                "25",
                "--full-path",
                "--",
                expected_pattern,
                dir.path().to_string_lossy().as_ref(),
            ]
        );
        let rg = build_rg_args("Needle", dir.path(), Some("*.rs"), true, true);
        assert_eq!(
            rg,
            vec![
                "--json",
                "--line-number",
                "--color=never",
                "--hidden",
                "--ignore-case",
                "--fixed-strings",
                "--glob",
                "*.rs",
                "--",
                "Needle",
                dir.path().to_string_lossy().as_ref(),
            ]
        );
    }

    #[test]
    fn fd_argv_disables_git_requirement_outside_a_repo() {
        let dir = tempdir().unwrap();
        let args = build_fd_args("*.rs", dir.path(), 10);
        assert!(args.iter().any(|arg| arg == "--no-require-git"));
    }

    #[test]
    fn relativize_find_result_is_posix() {
        assert_eq!(
            relativize_find_result_path(Path::new("/tmp/root/src/app.ts"), Path::new("/tmp/root")),
            "src/app.ts"
        );
    }

    #[test]
    fn execute_tool_apply_patch_works() {
        let dir = tempdir().unwrap();
        let patch = r#"*** Begin Patch
*** Add File: test.txt
+Hello Codex
*** End Patch"#;
        let res = execute_tool(
            dir.path(),
            "apply_patch",
            &serde_json::json!({ "input": patch }),
        )
        .unwrap();
        assert!(!res.is_error);
        assert!(res.content.contains("1 added"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("test.txt")).unwrap(),
            "Hello Codex\n"
        );
    }

    #[test]
    fn write_stdin_tool_reports_error_on_missing_args_or_invalid_job() {
        let dir = tempdir().unwrap();
        let context = ToolContext::default();

        // Missing job_id
        let res = execute_tool_with(
            dir.path(),
            "write_stdin",
            &serde_json::json!({ "input": "test" }),
            &context,
        )
        .unwrap();
        assert!(res.is_error);
        assert!(res.content.contains("Missing jobId"));

        // Missing input
        let res = execute_tool_with(
            dir.path(),
            "write_stdin",
            &serde_json::json!({ "job_id": 1 }),
            &context,
        )
        .unwrap();
        assert!(res.is_error);
        assert!(res.content.contains("Missing required parameter: input"));

        // Non-existent job
        let res = execute_tool_with(
            dir.path(),
            "write_stdin",
            &serde_json::json!({ "job_id": 42, "input": "test" }),
            &context,
        )
        .unwrap();
        assert!(res.is_error);
        assert!(res.content.contains("No background job 42"));
    }

    #[test]
    fn test_team_tools_gating() {
        let dir = tempfile::tempdir().unwrap();
        let context = ToolContext::default();

        // When disabled (default)
        std::env::remove_var("DAVINCI_EXPERIMENTAL_AGENT_TEAMS");
        let specs = tool_specs();
        assert!(!specs.iter().any(|s| s.name == "agent_status"));
        assert!(!specs.iter().any(|s| s.name == "task_create"));
        assert!(!specs.iter().any(|s| s.name == "task_get"));

        let err = execute_tool_with(dir.path(), "agent_status", &serde_json::json!({}), &context)
            .unwrap_err();
        assert!(
            matches!(err, ToolError::Failed(ref msg) if msg.contains("Team coordination tools are disabled"))
        );

        // When enabled
        std::env::set_var("DAVINCI_EXPERIMENTAL_AGENT_TEAMS", "1");
        let specs_enabled = tool_specs();
        let capabilities = crate::runtime::capabilities::builtin_capabilities();
        let get_dispatch = execute_tool_with(
            dir.path(),
            "task_get",
            &serde_json::json!({"task_id": uuid::Uuid::new_v4().to_string()}),
            &context,
        );
        std::env::remove_var("DAVINCI_EXPERIMENTAL_AGENT_TEAMS");
        assert!(
            matches!(get_dispatch, Err(ToolError::Failed(message)) if message == "Runtime subsystem not initialized")
        );
        assert!(capabilities
            .iter()
            .any(|cap| cap.name == "task_get" && cap.read_only));
        assert!(specs_enabled.iter().any(|s| s.name == "agent_status"));
        assert!(specs_enabled.iter().any(|s| s.name == "task_create"));
        assert!(specs_enabled.iter().any(|s| s.name == "task_update"));
        assert!(specs_enabled.iter().any(|s| s.name == "task_get"));
    }

    #[test]
    fn test_workflow_tools_gating() {
        let dir = tempfile::tempdir().unwrap();
        let context = ToolContext::default();

        // When disabled (default)
        std::env::remove_var("DAVINCI_EXPERIMENTAL_WORKFLOWS");
        std::env::remove_var("DAVINCI_RUNTIME_WORKFLOWS");
        let specs = tool_specs();
        assert!(!specs.iter().any(|s| s.name == "workflow_run"));
        assert!(!specs.iter().any(|s| s.name == "workflow_status"));

        let err = execute_tool_with(
            dir.path(),
            "workflow_status",
            &serde_json::json!({}),
            &context,
        )
        .unwrap_err();
        assert!(
            matches!(err, ToolError::Failed(ref msg) if msg.contains("Workflow coordination tools are disabled"))
        );

        // When enabled
        std::env::set_var("DAVINCI_EXPERIMENTAL_WORKFLOWS", "1");
        let specs_enabled = tool_specs();
        std::env::remove_var("DAVINCI_EXPERIMENTAL_WORKFLOWS");
        assert!(specs_enabled.iter().any(|s| s.name == "workflow_run"));
        assert!(specs_enabled.iter().any(|s| s.name == "workflow_status"));
    }

    #[test]
    fn semantic_tools_registration_and_fallback_execution() {
        let dir = tempdir().unwrap();
        let context = ToolContext::default();

        // 1. Tool specs exist
        let specs = tool_specs();
        assert!(specs.iter().any(|s| s.name == "code_definition"));
        assert!(specs.iter().any(|s| s.name == "code_references"));
        assert!(specs.iter().any(|s| s.name == "code_outline"));
        assert!(specs.iter().any(|s| s.name == "code_diagnostics"));
        assert!(specs.iter().any(|s| s.name == "code_call_hierarchy"));
        assert!(specs.iter().any(|s| s.name == "code_rename_preview"));

        // 2. Prepare sample source file
        let src = r#"
pub struct User {
    pub name: String,
}

impl User {
    pub fn greet(&self) -> String {
        format!("Hello, {}", self.name)
    }
}
"#;
        std::fs::write(dir.path().join("user.rs"), src).unwrap();

        // 3. Fallback definition
        let def_res = execute_tool_with(
            dir.path(),
            "code_definition",
            &serde_json::json!({"path": "user.rs", "symbol": "greet"}),
            &context,
        )
        .unwrap();
        assert!(!def_res.is_error);
        assert!(def_res.content.contains("greet"));
        assert!(def_res.content.contains("fallback"));

        // 4. Fallback references
        let ref_res = execute_tool_with(
            dir.path(),
            "code_references",
            &serde_json::json!({"path": "user.rs", "symbol": "User"}),
            &context,
        )
        .unwrap();
        assert!(!ref_res.is_error);
        assert!(ref_res.content.contains("user.rs"));

        // 5. Fallback outline
        let outline_res = execute_tool_with(
            dir.path(),
            "code_outline",
            &serde_json::json!({"path": "user.rs"}),
            &context,
        )
        .unwrap();
        assert!(!outline_res.is_error);
        assert!(outline_res.content.contains("User"));
        assert!(outline_res.content.contains("greet"));

        // 6. Diagnostics without a server are explicitly partial, never a
        // false claim that an empty result proves the file is error-free.
        let diag_res = execute_tool_with(
            dir.path(),
            "code_diagnostics",
            &serde_json::json!({"path": "user.rs"}),
            &context,
        )
        .unwrap();
        assert!(!diag_res.is_error);
        assert_eq!(
            diag_res
                .details
                .as_ref()
                .and_then(|value| value["partial"].as_bool()),
            Some(true)
        );
        assert!(diag_res.content.contains("no compiler diagnostics"));

        // 7. Call hierarchy without server returns unsupported error
        let hier_res = execute_tool_with(
            dir.path(),
            "code_call_hierarchy",
            &serde_json::json!({"path": "user.rs", "line": 7, "character": 12}),
            &context,
        );
        assert!(hier_res.is_err());
        assert!(hier_res
            .unwrap_err()
            .to_string()
            .contains("Call hierarchy is unsupported"));

        // 8. Rename preview without server returns error
        let rename_res = execute_tool_with(
            dir.path(),
            "code_rename_preview",
            &serde_json::json!({"path": "user.rs", "line": 7, "character": 12, "newName": "say_hello"}),
            &context,
        );
        assert!(rename_res.is_err());
        assert!(rename_res
            .unwrap_err()
            .to_string()
            .contains("Rename preview is unavailable"));
    }
}
