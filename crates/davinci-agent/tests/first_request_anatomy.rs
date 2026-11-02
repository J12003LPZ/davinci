//! Offline first-request anatomy for the Droid gap study (plan P0.2).
//!
//! Builds DaVinci's first provider request for the prompt `hi` on the
//! `openai-codex` backend route, for three configurations — Full, Lean, and
//! Full + Codemode — and records the byte categories the plan needs to decide
//! whether the tool-surface workstream (W2) is worth opening.
//!
//! Offline only: no provider request, no mock server, no `droid exec` prompt.
//! Everything is built from existing seams:
//!
//! * `cache_stability::wire_body_for_next_request` builds the exact request
//!   body (`crates/davinci-agent/src/cache_stability.rs`).
//! * `PreparedProviderRequest::new` derives the `WireManifest` segments
//!   (`crates/davinci-ai/src/responses_request.rs`) — the byte source.
//! * `Agent::prompt` → `prepare_builtin_prompt_for_user_turn` composes the
//!   system prompt for the real provider/model, and `commit_turn_context`
//!   appends the per-turn `<turn_context>` block.
//! * `ToolSurface::{Full, Lean}` + `freeze_tools_for_cache` select the up-front
//!   schema set (`crates/davinci-agent/src/lib.rs`).
//! * `enable_read_only_codemode` registers the real `codemode` schema.

use davinci_agent::cache_stability::wire_body_for_next_request;
use davinci_agent::codemode::{
    CodeModeBroker, CodeModeHost, CodeModeOutcome, CodeModeRequest, CodeModeRunContext,
    CodeModeStatus,
};
use davinci_agent::runtime::operations::{
    CallerType, ExecutionOwner, ExecutionOwnerId, JournalId, JournalIdentity, OperationContext,
    OperationJournal, RootNamespaceId, ToolOperationRuntime, WorkspaceId, WorkspaceIdentity,
};
use davinci_agent::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_agent::turn_context::TurnContextPlacement;
use davinci_agent::{Agent, PromptProfile, ToolSurface};
use davinci_ai::{
    load_builtin_models, MessageContent, Model, PreparedProviderRequest, WireSegmentCategory,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The checkout root (nearest ancestor of the working directory holding a
/// `.git` entry), where the backend route reads its `AGENTS.md`.
fn repo_root() -> PathBuf {
    let mut dir = std::env::current_dir().expect("current_dir");
    loop {
        if dir.join(".git").exists() {
            return dir;
        }
        if !dir.pop() {
            panic!("first_request_anatomy must run inside a git checkout");
        }
    }
}

/// The model the existing backend-route seam (`cache_stability::codex_test_model`)
/// uses. The tool-schema and instruction byte figures are model-independent
/// except for the one-line `model` field and the provider adapter text.
fn backend_model() -> Model {
    load_builtin_models()
        .into_iter()
        .find(|model| model.provider == "openai-codex" && model.id == "gpt-5.6-luna")
        .expect("catalog has openai-codex/gpt-5.6-luna")
}

/// A host that is never executed: `enable_read_only_codemode` only registers
/// the schema and binds the host, it does not run a script.
struct NoopCodeModeHost;

impl CodeModeHost for NoopCodeModeHost {
    fn execute(
        &self,
        _: &CodeModeRequest,
        _: &CodeModeRunContext,
        _: Arc<dyn CodeModeBroker>,
    ) -> CodeModeOutcome {
        CodeModeOutcome {
            status: CodeModeStatus::Completed,
            script_completed: true,
            output_text: String::new(),
            output_complete: true,
            output_artifact: None,
            children: vec![],
            host_notes: vec![],
            operation_ref: "noop".into(),
            error: None,
            retained_output: None,
            omitted_bytes: 0,
            omitted_start_line: None,
        }
    }
}

/// A runtime whose operation journal exists, so `enable_read_only_codemode`
/// can register the real `codemode` capability (it refuses without a journal).
fn runtime_with_journal(workspace: &Path) -> RuntimeHandle {
    let root_namespace_id = RootNamespaceId::new();
    let identity = JournalIdentity::new(
        JournalId::new(),
        WorkspaceIdentity {
            id: WorkspaceId::new(),
            binding_version: 1,
        },
    )
    .unwrap();
    let run_id = RunId::new();
    let agent_id = AgentId::new();
    let context = OperationContext {
        journal_id: identity.journal_id,
        root_namespace_id,
        session_id: "first-request-anatomy-fixture".into(),
        runtime_run_id: run_id,
        parent_operation_id: None,
        agent_id,
        worker_id: None,
        task_id: None,
        graph: None,
        workspace: identity.workspace.clone(),
        caller: CallerType::ProviderToolCall,
        wire_tool_call_id: None,
    };
    let journal = Arc::new(
        OperationJournal::open(&workspace.join("journal"), identity, root_namespace_id).unwrap(),
    );
    let operations = ToolOperationRuntime::new(
        journal.clone(),
        context,
        ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
        workspace,
    )
    .unwrap();
    RuntimeHandle::new(run_id, agent_id, RuntimeBus::new()).with_operation_runtime(operations)
}

#[derive(Clone, Copy)]
enum Config {
    Full,
    Lean,
    FullCodemode,
}

impl Config {
    fn name(&self) -> &'static str {
        match self {
            Config::Full => "DaVinci Full",
            Config::Lean => "DaVinci Lean",
            Config::FullCodemode => "DaVinci Full+Codemode",
        }
    }
}

/// Build the backend-route agent and its first request body for `hi`.
fn build_first_request(config: Config) -> (Agent, PreparedProviderRequest) {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.provider = "openai-codex".into();
    agent.model_id = "gpt-5.6-luna".into();
    agent.turn_context_placement_override = Some(TurnContextPlacement::Appended);
    agent.tool_surface = match config {
        Config::Lean => ToolSurface::Lean,
        _ => ToolSurface::Full,
    };
    // The backend route reads the checkout's AGENTS.md into the system prompt.
    agent.cwd = repo_root();
    agent.load_instruction_files(true);

    match config {
        Config::Full | Config::Lean => {
            agent.set_runtime(RuntimeHandle::new(
                RunId::new(),
                AgentId::new(),
                RuntimeBus::new(),
            ));
        }
        Config::FullCodemode => {
            // The operation journal lives beside a throwaway workspace; the
            // agent's `cwd` stays on the checkout so AGENTS.md still loads.
            let workspace = tempfile::tempdir().unwrap();
            agent.set_runtime(runtime_with_journal(workspace.path()));
            agent
                .enable_read_only_codemode(Arc::new(NoopCodeModeHost))
                .unwrap();
        }
    }

    agent.freeze_tools_for_cache();
    agent.prompt("hi");
    agent.commit_turn_context(None);

    let model = backend_model();
    let body = wire_body_for_next_request(&agent, &model);
    (agent, PreparedProviderRequest::new(body))
}

/// Sum of `byte_len` for one `WireSegmentCategory`.
fn category_bytes(prepared: &PreparedProviderRequest, category: WireSegmentCategory) -> usize {
    prepared
        .manifest()
        .segments
        .iter()
        .filter(|segment| segment.category == category)
        .map(|segment| segment.byte_len)
        .sum()
}

/// Byte length of the first `open`…`close` block in `text`, or 0 when absent.
fn block_bytes(text: &str, open: &str, close: &str) -> usize {
    let Some(start) = text.find(open) else {
        return 0;
    };
    let Some(rel) = text[start + open.len()..].find(close) else {
        return 0;
    };
    open.len() + rel + close.len()
}

/// Raw text of the appended `<turn_context>` block, or empty when absent.
fn turn_context_text(agent: &Agent) -> String {
    agent
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.extra.get("customType").and_then(Value::as_str)
                == Some(davinci_agent::turn_context::TURN_CONTEXT_CUSTOM_TYPE)
        })
        .and_then(|message| {
            message.content.iter().find_map(|block| match block {
                MessageContent::Text { text } => Some(text.clone()),
                _ => None,
            })
        })
        .unwrap_or_default()
}

struct Row {
    name: &'static str,
    tool_count: usize,
    tool_schema_bytes: usize,
    instruction_bytes: usize,
    project_instruction_bytes: usize,
    turn_context_bytes: usize,
    conversation_bytes: usize,
    total_bytes: usize,
    structural_overhead: usize,
}

fn measure(config: Config) -> Row {
    let (agent, prepared) = build_first_request(config);
    let manifest = prepared.manifest();
    let body = prepared.body();

    let tool_count = body
        .get("tools")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let tool_schema_bytes = category_bytes(&prepared, WireSegmentCategory::ToolContract);
    let instruction_bytes = category_bytes(&prepared, WireSegmentCategory::TrustedInstructions);
    let conversation_bytes = category_bytes(&prepared, WireSegmentCategory::ConversationItem);

    let instructions = agent.provider_system_prompt();
    let project_instruction_bytes =
        block_bytes(&instructions, "<project_context>", "</project_context>");
    let turn_context_bytes = turn_context_text(&agent).len();

    let total_bytes = manifest.request_bytes_before_compression;
    let sum_segments: usize = manifest
        .segments
        .iter()
        .map(|segment| segment.byte_len)
        .sum();
    let structural_overhead = total_bytes - sum_segments;

    // Independent recomputation straight from the body, not from the manifest.
    let json_len = |value: &Value| serde_json::to_vec(value).unwrap().len();
    let object = body.as_object().expect("body is an object");
    let is_trusted_item = |item: &Value| {
        matches!(
            item.get("role").and_then(Value::as_str),
            Some("developer" | "system")
        )
    };
    let items: &[Value] = object
        .get("input")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let expected_tools = object.get("tools").map(json_len).unwrap_or(0);
    let expected_instructions = object.get("instructions").map(json_len).unwrap_or(0)
        + object.get("system").map(json_len).unwrap_or(0)
        + items
            .iter()
            .filter(|item| is_trusted_item(item))
            .map(json_len)
            .sum::<usize>();
    let expected_conversation: usize = items
        .iter()
        .filter(|item| !is_trusted_item(item))
        .map(json_len)
        .sum();
    // Remaining top-level keys are re-wrapped in one object per group.
    let group = |keys: &[&str], invert: bool| {
        let map: serde_json::Map<String, Value> = object
            .iter()
            .filter(|(key, _)| keys.contains(&key.as_str()) != invert)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if map.is_empty() {
            0
        } else {
            json_len(&Value::Object(map))
        }
    };
    let routing_keys = [
        "prompt_cache_key",
        "prompt_cache_retention",
        "prompt_cache_options",
        "stream",
        "store",
    ];
    let claimed_keys = [
        "input",
        "messages",
        "tools",
        "instructions",
        "system",
        "prompt_cache_key",
        "prompt_cache_retention",
        "prompt_cache_options",
        "stream",
        "store",
    ];
    let expected_config = group(&claimed_keys, true);
    let expected_routing = group(&routing_keys, false);

    assert_eq!(
        tool_schema_bytes,
        expected_tools,
        "{}: ToolContract bytes != serialized body[\"tools\"]",
        config.name()
    );
    assert_eq!(
        instruction_bytes,
        expected_instructions,
        "{}: TrustedInstructions bytes != serialized instructions (+ developer items)",
        config.name()
    );
    assert_eq!(
        conversation_bytes,
        expected_conversation,
        "{}: ConversationItem bytes != serialized non-developer input items",
        config.name()
    );
    assert_eq!(
        sum_segments,
        expected_tools
            + expected_instructions
            + expected_conversation
            + expected_config
            + expected_routing,
        "{}: manifest segments do not cover tools + instructions + input + config + routing",
        config.name()
    );

    // Internal consistency that catches a broken measurement.
    assert!(tool_count > 0, "{}: empty tool set", config.name());
    assert_eq!(
        tool_count,
        agent.provider_tool_specs().len(),
        "{}: body tools array length != schema count",
        config.name()
    );
    assert!(
        tool_schema_bytes > 0,
        "{}: empty ToolContract segment",
        config.name()
    );
    assert!(
        instruction_bytes > 0,
        "{}: empty TrustedInstructions segment",
        config.name()
    );

    Row {
        name: config.name(),
        tool_count,
        tool_schema_bytes,
        instruction_bytes,
        project_instruction_bytes,
        turn_context_bytes,
        conversation_bytes,
        total_bytes,
        structural_overhead,
    }
}

#[test]
fn first_request_anatomy_table() {
    let rows = [
        measure(Config::Full),
        measure(Config::Lean),
        measure(Config::FullCodemode),
    ];

    // The top-level key set is identical in all three configurations, so the
    // structural remainder (key names, braces, commas) must be too.
    assert!(
        rows.iter()
            .all(|row| row.structural_overhead == rows[0].structural_overhead),
        "structural overhead differs across configurations"
    );

    println!(
        "{:<22} {:>8} {:>16} {:>16} {:>26} {:>26} {:>16} {:>16} {:>16}",
        "configuration",
        "tools",
        "tool-schema B",
        "instruction B",
        "project-instr B (in instr)",
        "turn-context B (in conv)",
        "conversation B",
        "total B",
        "structural B"
    );
    for row in &rows {
        println!(
            "{:<22} {:>8} {:>16} {:>16} {:>26} {:>26} {:>16} {:>16} {:>16}",
            row.name,
            row.tool_count,
            row.tool_schema_bytes,
            row.instruction_bytes,
            row.project_instruction_bytes,
            row.turn_context_bytes,
            row.conversation_bytes,
            row.total_bytes,
            row.structural_overhead
        );
    }
    println!("note: project-instr B is a raw-text substring contained in instruction B;");
    println!("note: turn-context B is raw text contained in conversation B. Do not add columns.");
}
