//! Agent runtime matching `@earendil-works/pi-agent-core`.

pub mod apply_patch;
pub mod approval;
pub mod cache_stability;
pub mod decision;
pub mod decisions;
mod permission_state;
pub mod process_manager;
pub use permission_state::PermissionState;
mod batch;
mod branch;
pub mod command_receipt;
mod compaction;
mod context;
mod edit_diff;
pub mod effort;
mod events;
mod evidence;
mod file_mutation_queue;
mod images;
pub mod jobs;
pub mod mcp;
pub mod notebook;
mod permission;
pub mod planning;
mod prepared_context;
pub mod prompt;
pub mod provider_budget;
mod pruning;
pub use prepared_context::PreparedContextImage;
mod queues;
mod scheduler;
pub mod semantic;
pub mod shell_policy;
mod skills;
mod stats;
mod subagent;
mod templates;
pub mod todo;
pub mod tool_ledger;
pub mod tools;
mod transaction_verification;
mod turn;
pub mod turn_context;
pub mod web;

pub use batch::{BATCH_MAX_OPERATIONS, VISIBLE_PER_OPERATION, VISIBLE_TOTAL};
pub use branch::{
    build_branch_summary_prompt, collect_entries_for_branch_summary, generate_branch_summary,
    message_from_branch_entry, navigation_target, prepare_branch_entries, BranchPreparation,
    BranchSummaryResult, BRANCH_SUMMARY_PREAMBLE, BRANCH_SUMMARY_PROMPT,
};
pub use compaction::{
    branch_summary_context_message, build_history_prompt, calculate_context_tokens,
    compact_messages, compact_messages_with, compact_messages_with_options,
    compaction_context_message, compute_file_lists, convert_to_llm, env_summarizer,
    estimate_context_tokens, estimate_tokens, extract_file_ops, find_cut_point,
    format_file_operations, generate_summary_with_usage, get_summarization_failure,
    serialize_conversation, should_compact, CompactionDetails, CompactionResult,
    CompactionSettings, CompactionThreshold, CutPointResult, FileOperations, SummarizeRequest,
    SummarizeResponse, Summarizer, BRANCH_SUMMARY_PREFIX, BRANCH_SUMMARY_SUFFIX,
    COMPACTION_SUMMARY_PREFIX, COMPACTION_SUMMARY_SUFFIX, CONTEXT_VM_FOLD_PROMPT,
    CONTEXT_VM_FOLD_SYSTEM_PROMPT, DEFAULT_KEEP_RECENT_TOKENS, DEFAULT_RESERVE_TOKENS,
    SUMMARIZATION_PROMPT, SUMMARIZATION_SYSTEM_PROMPT, TURN_PREFIX_SUMMARIZATION_PROMPT,
    UPDATE_SUMMARIZATION_PROMPT,
};
pub use context::{
    load_context_files, load_context_files_for_targets, ContextBudgetReport, ContextContribution,
    ContextFile, ContextPriority, RootContextAccount, SelectedRootContext,
};
pub use events::AgentEvent;
pub use evidence::{EvidenceStore, EVIDENCE_TTL};
pub use file_mutation_queue::{mutation_queue_key, with_file_mutation_queue};
pub use images::{
    apply_block_images, convert_to_llm_for_provider, normalize_tool_result_images,
    parse_rpc_images, process_image_bytes, IMAGE_READING_DISABLED,
};
pub use jobs::{JobBook, JobNotice, JobStatus, JobSummary};
pub use mcp::{McpRegistry, McpServerRow};
pub(crate) use permission::read_only_capability_allows;
pub use permission::{
    check_path_boundary, glob_matches, is_git_metadata_path, is_outside_or_symlink_escape,
    is_sensitive_file_path, is_symlink_escape, project_relative, session_rule_for,
    strip_verbatim_prefix, subject_of, summary_of, tool_class, FilesystemBoundaryPolicy,
    PermissionMode, PermissionPolicy, PermissionRule, PermissionVerdict, ReadOutsideRootPolicy,
    RuleParseError, RuleSpecifier, ToolApprovalDecision, ToolApprovalRequest, ToolApprover,
    ToolClass,
};
pub use prompt::{
    CapabilityGateOutcome, CapabilityRunState, DebuggingState, FrontendDesignState,
    ReproducerEvidence, ReviewState, MAX_CAPABILITY_COMPLETION_REMINDERS,
};
pub use prompt::{PreparedTurnPrompt, PromptProfile};
pub use pruning::PruneSettings;
pub use queues::{QueueKind, QueueMode, QueuedMessage, RemoteQueue, SteerFollowUpQueues};
pub use scheduler::{lane_for, lane_for_capability, ToolLane, MAX_TOOL_PARALLELISM};
pub use skills::{
    describe_skill, discover_skills, expand_skill_command, expand_user_text,
    expand_user_text_with_metadata, ExpandedUserText, Skill, SkillDescriptor,
};
pub use stats::{RunStats, SharedCounters};
pub use subagent::{
    scoped_tools, scoped_tools_with_policy, scoped_tools_with_registry, AgentSpawnMode,
    SubagentParent, SubagentRequest, SubagentRunner, DEFAULT_SUBAGENT_TOOLS, PLAN_MODE_APPENDIX,
    PLAN_MODE_DENIAL,
};
pub use templates::{
    discover_prompt_templates, expand_prompt_template, parse_command_args, parse_frontmatter,
    strip_frontmatter, substitute_args, PromptTemplate,
};
pub mod living_plan;
pub use living_plan::{LivingPlan, PLAN_ENTRY_TYPE};
pub use todo::{TodoItem, TodoList, TodoStatus, TODO_ENTRY_TYPE};
pub use tool_ledger::{
    classify_side_effect, AttemptOutcome, RecoveryAction, ToolCallLedger, ToolCallRecord,
    ToolExecutionStatus, ToolSideEffect,
};
pub use tools::{
    decision_wait, execute_tool, execute_tool_with, tool_specs, validate_builtin_tool_descriptions,
    AgentTool, DecisionHostReply, DecisionHostRequest, DecisionHostResponse, DecisionResponder,
    ToolContext, ToolError, ToolResult, BUILTIN_TOOLS, CODEX_HOT_TOOLS,
};
pub use turn::retry_delay_ms;

pub mod runtime;
pub use prompt::{
    compose_default_prompt, compose_legacy_default, compose_modules, resolve_resume_prompt_session,
    runtime_state_module, runtime_state_text, ComposedPrompt, PromptBundle, PromptCacheClass,
    PromptCandidateDescriptor, PromptContext, PromptManifest, PromptModule, PromptModuleIdentity,
    PromptSessionRecord, PromptSessionState, PromptSource, RuntimePromptState,
};

pub const PROMPT_SESSION_ENTRY_TYPE: &str = "prompt_session";
pub use runtime::{
    conservative_replay_policy, contract_gate, default_execution_policies, effect_profile_allows,
    find_saved_workflow, hash_system_prompt, hash_system_prompt_with_manifest, hash_tool_names,
    normalize_relative_path, path_scope_allows, save_workflow_to_project, wrap_untrusted_data,
    AgentId, AgentKind, AgentLaunchDisposition, AgentOperationAdapter, AgentOperationError,
    AgentOperationHandle, AgentRecord, AgentState, CacheIdentity, CacheMissReason,
    CancellationToken, CapabilitySource, ChildExecutionContext, ChildExecutionKind,
    ConcurrencyPolicy, ContextBroker, ContextItem, ContextPacket, ContextRequest, ContextSource,
    ContextVmMode, ContractError, ContractExecutor, DeclaredEffect, ExecutionError,
    ExecutorCapabilities, OperationAttempt, OperationSpec, OperationState, OutputPolicy,
    PhaseStatus, PreparedAction, RegistryError, ReplayPolicy, RunId, RuntimeBus, RuntimeCapability,
    RuntimeCapabilityRegistry, RuntimeDecision, RuntimeEvent, RuntimeEventEnvelope, RuntimeHandle,
    RuntimeRegistry, RuntimeSubscriber, ScopeViolation, TaskContract, TaskError, TaskId,
    TaskRecord, TaskRegistry, TaskState, ToolExposureState, UnresolvedChild, WorkflowExecutor,
    WorkflowId, WorkflowSpec, WorkflowStateStore, WorkflowStatus, WorktreeError, WorktreeLease,
    WorktreeManager,
};

use davinci_ai::{
    content_text, AssistantMessage, AssistantMessageEvent, ChatMessage, MessageContent,
    ThinkingBudgets,
};
use davinci_protocol::ThinkingLevel;
use davinci_session::{JsonlSession, SessionEntry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

type CustomToolFn = dyn Fn(&Path, &str, &Value, Option<&ToolContext>) -> Result<ToolResult, ToolError>
    + Send
    + Sync;
type PreToolFn = dyn Fn(&str, &Value) -> Option<String> + Send + Sync;
type PostToolFn = dyn Fn(&str, &Path, &str, &Value, ToolResult) -> ToolResult + Send + Sync;

/// Blocks a tool call when the hook returns a reason (TS `tool_call` `{ block: true }`).
#[derive(Clone)]
pub struct PreToolHook(pub Arc<PreToolFn>);

impl std::fmt::Debug for PreToolHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreToolHook")
    }
}

/// Transforms a completed tool result before it is emitted and persisted.
/// Extensions use this for lossless output compression and other middleware
/// that must run for both built-in and custom tools.
#[derive(Clone)]
pub struct PostToolHook(pub Arc<PostToolFn>);

impl std::fmt::Debug for PostToolHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PostToolHook")
    }
}

/// Live agent-loop subscriber matching TS `AgentSession.subscribe`.
#[derive(Clone)]
pub struct EventSink(pub Arc<dyn Fn(&AgentEvent) + Send + Sync>);

impl std::fmt::Debug for EventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventSink")
    }
}

/// Injected tool runner for JS/manifest tools so `pi-agent` stays independent of the coding-agent host.
#[derive(Clone)]
pub struct CustomToolExecutor {
    inner: Arc<CustomToolFn>,
    requires_context: bool,
}

impl std::fmt::Debug for CustomToolExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CustomToolExecutor")
    }
}

impl CustomToolExecutor {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&Path, &str, &Value) -> Result<ToolResult, ToolError> + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(move |cwd, name, args, _| f(cwd, name, args)),
            requires_context: false,
        }
    }

    /// Host adapter receiving this dispatch's engine context, including consent,
    /// cancellation and session resources. This is not an authorization grant:
    /// resource adapters must still recheck current policy at their boundaries.
    pub fn new_with_context<F>(f: F) -> Self
    where
        F: Fn(&Path, &str, &Value, &ToolContext) -> Result<ToolResult, ToolError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            inner: Arc::new(move |cwd, name, args, context| {
                let context = context.ok_or_else(|| {
                    ToolError::Failed("tool requires engine dispatch context".into())
                })?;
                f(cwd, name, args, context)
            }),
            requires_context: true,
        }
    }

    pub fn execute(&self, cwd: &Path, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        (self.inner)(cwd, name, args, None)
    }

    pub fn execute_with_context(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
        context: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        (self.inner)(cwd, name, args, Some(context))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub last_verification: Option<LastVerification>,
}

/// The last classified verification call, retained across later mutations.
/// Preserve its full execution context when the harness repeats it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastVerification {
    pub tool: String,
    pub command: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

/// Evidence available when a coding turn reaches a normal stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionEvidence {
    Verified,
    Unverified,
    VerificationFailed,
    NotRequired,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ToolOperationOrigin {
    ProviderCall,
    BatchChild {
        parent: crate::runtime::operations::OperationId,
        child_index: usize,
    },
}

#[derive(Clone)]
pub(crate) struct PendingToolOperation {
    pub(crate) runtime: crate::runtime::operations::ToolOperationRuntime,
    pub(crate) plan: crate::runtime::operations::PlannedToolOperation,
    pub(crate) admitted: crate::runtime::operations::AdmittedOperation,
    pub(crate) origin: ToolOperationOrigin,
}

impl std::fmt::Debug for PendingToolOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingToolOperation")
            .field("plan", &self.plan)
            .field("admitted", &self.admitted)
            .field("origin", &self.origin)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub thinking_level: ThinkingLevel,
    pub effort_policy: effort::EffortPolicy,
    /// Repeat the last verification call after later mutations at completion.
    pub auto_verify: bool,
    pub auto_compaction: bool,
    pub compaction: CompactionSettings,
    pub auto_retry: bool,
    pub retry_attempts: u32,
    pub retry_base_delay_ms: u64,
    /// Maximum provider model turns in one run; `Some(0)` disables the limit.
    pub max_model_turns: Option<u32>,
    pub provider_timeout_ms: Option<u64>,
    pub provider_max_retries: Option<u32>,
    pub provider_max_retry_delay_ms: u64,
    pub thinking_budgets: Option<ThinkingBudgets>,
    pub context_window: u64,
    pub context_vm_mode: ContextVmMode,
    pub queues: SteerFollowUpQueues,
    remote: RemoteQueue,
    pub tools: Vec<String>,
    pub tool_registry: Vec<String>,
    pub skills: Vec<Skill>,
    pub templates: Vec<PromptTemplate>,
    pub context_files: Vec<ContextFile>,
    pub session: Option<JsonlSession>,
    pub cwd: PathBuf,
    pub aborted: bool,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub provider: String,
    pub model_id: String,
    pub tool_execution_mode: ToolExecutionMode,
    pub custom_tool_executor: Option<CustomToolExecutor>,
    pub pre_tool: Option<PreToolHook>,
    pub post_tool: Option<PostToolHook>,
    /// Which tools may run without asking (`permission.rs`). Shared, because
    /// the gate reads it from `&self` on the tool thread while the host reads
    /// the mode for its chrome, and a granted rule is written back mid-turn.
    /// Construct with `Arc::new(PermissionState::new(policy))`; update through
    /// `lock()` so mutable access invalidates outstanding approval revisions.
    pub permissions: Arc<PermissionState>,
    /// Who answers when the policy says ask. `None` means the run cannot
    /// ask, and the call is refused with a message that says so.
    pub approver: Option<ToolApprover>,
    /// Typed trusted-host responder; takes precedence over the legacy approver.
    pub approval_responder: Option<approval::ApprovalResponder>,
    approval_registry: Arc<approval::ApprovalRegistry>,
    /// Background shell jobs (`jobs.rs`) and the model's todo ledger
    /// (`todo.rs`), shared with the tool thread and the shell.
    pub tool_context: ToolContext,
    pub summarizer: Option<Summarizer>,
    pub subagent_runner: Option<crate::subagent::SubagentRunner>,
    pub block_images: bool,
    pub auto_resize_images: bool,
    pub retry_aborted: bool,
    pub transport: Option<String>,
    pub install_telemetry: bool,
    pub reload_count: u32,
    pub event_sink: Option<EventSink>,
    /// Cross-thread interrupt: set from the UI thread while `run_loop` runs
    /// on a worker. Checked at every loop step alongside `aborted`.
    pub abort_signal: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Counters for the run (`stats.rs`): turns, batch widths, wall time,
    /// peak context, prunings. Read them through `run_stats`, which also
    /// folds in the counters bumped from inside tool calls.
    pub stats: RunStats,
    pub counters: Arc<SharedCounters>,
    /// When old tool output leaves the provider's view (`pruning.rs`).
    pub prune_settings: PruneSettings,
    /// Where overflowing output is kept for a later `read` (`evidence.rs`).
    /// `None` means overflow is truncated with a note and nothing else.
    pub evidence: Option<EvidenceStore>,
    pub tool_ledger: Arc<std::sync::Mutex<ToolCallLedger>>,
    pub(crate) pending_tool_operations:
        Arc<Mutex<std::collections::HashMap<String, PendingToolOperation>>>,
    pub(crate) operation_presentations: Arc<Mutex<std::collections::HashMap<String, ToolResult>>>,
    /// Tool-call ids whose results are pruned from the provider view. Only
    /// grows; the session file keeps every body.
    pruned_tool_results: std::collections::HashSet<String>,
    pruned_evidence: std::collections::HashMap<String, (PathBuf, String)>,
    base_system_prompt: String,
    /// Return target for /act, not another active mode.
    previous_execution_mode: Option<PermissionMode>,
    /// Historical snapshot for a bounded revision diff, never an active plan.
    previous_plan_revision: Option<LivingPlan>,
    /// Whether the host has registered a backend capable of visual verification.
    visual_verification_available: bool,
    /// Typed lifecycle evidence for the currently prepared real user turn.
    capability_run_state: Arc<Mutex<prompt::CapabilityRunState>>,
    /// Mutation generations and verification evidence for the current run.
    mutation_verification: Arc<Mutex<MutationVerificationState>>,
    pending_transaction_verification:
        Arc<Mutex<std::collections::BTreeMap<String, Vec<transaction_verification::Pending>>>>,
    /// Bounded actual command evidence, populated only by built-in execution.
    command_receipts:
        Arc<Mutex<std::collections::VecDeque<runtime::evidence_store::ExecutionReceipt>>>,
    plan_storage_error: Option<String>,
    pending_bash_messages: Vec<ChatMessage>,
    pending_prompt_messages: Vec<ChatMessage>,
    /// Context supplied by extensions for the next provider request only.
    /// These messages never enter the persisted session history.
    ephemeral_context: Vec<ChatMessage>,
    /// Tests and hosts may force a placement; `None` follows the route family.
    pub turn_context_placement_override: Option<turn_context::TurnContextPlacement>,
    /// Per-turn prompt state prepared for the next `commit_turn_context`.
    turn_state_pending: Option<String>,
    /// Host-supplied schema estimate, excluding the system prompt and messages.
    provider_context_overhead_tokens: Option<u64>,
    provider_output_limit: Option<u64>,
    prepared_context_image: Arc<Mutex<Option<PreparedContextImage>>>,
    prepared_context_generation: u64,
    prepared_manifest_revisions: (u64, u64),
    /// Request-local suffix appended to the current prompt after repository context.
    provider_system_prompt_suffix: Option<String>,
    /// Last prepared context manifest before provider dispatch.
    pub last_prepared_manifest: Option<runtime::context_manifest::PreparedContextManifest>,
    /// Optional shared runtime handle for versioned lifecycle events and coordination.
    pub runtime: Option<RuntimeHandle>,
    /// Optional additive decision-intelligence runtime. It is deliberately
    /// separate from deterministic routing and remains disabled by default.
    pub decision_runtime: Option<Arc<decision::DecisionRuntime>>,
    /// Active prompt manifest identifying modules, hashes, and token budgets.
    pub prompt_manifest: Option<PromptManifest>,
    /// Active prompt session state distinguishing built-in profiles from custom replacement prompts.
    pub prompt_session: prompt::PromptSessionState,
    /// Most recent real user-origin request eligible as capability-routing history.
    /// Internal mailbox/system-origin role=`user` messages must never update this.
    last_real_user_request: Option<String>,
    /// Exact host session source and lineage captured at runtime installation.
    /// Separate from mutable public session/runtime fields to reject stale reuse.
    runtime_session: Option<(PathBuf, String, RunId)>,
}

impl Agent {
    pub fn new(system_prompt: impl Into<String>) -> Self {
        let system_prompt = system_prompt.into();
        let legacy = prompt::compose_legacy_default();
        let (prompt_manifest, system_prompt, prompt_session) = if system_prompt == legacy.text {
            let mut session = prompt::PromptSessionState::builtin(prompt::PromptProfile::LegacyV1);
            session.last_manifest = Some(legacy.manifest.clone());
            session.stable_bundle_hash = Some(legacy.manifest.stable_sha256.clone());
            (Some(legacy.manifest), legacy.text, session)
        } else {
            let session = prompt::PromptSessionState::custom(system_prompt.clone());
            (None, system_prompt, session)
        };
        let agent = Self {
            system_prompt: system_prompt.clone(),
            prompt_manifest,
            prompt_session,
            last_real_user_request: None,
            messages: Vec::new(),
            thinking_level: ThinkingLevel::Off,
            effort_policy: effort::EffortPolicy::default(),
            auto_verify: true,
            auto_compaction: true,
            compaction: CompactionSettings::default(),
            auto_retry: true,
            retry_attempts: 3,
            retry_base_delay_ms: 2_000,
            max_model_turns: Some(200),
            provider_timeout_ms: None,
            provider_max_retries: None,
            provider_max_retry_delay_ms: 60_000,
            thinking_budgets: None,
            context_window: 200_000,
            context_vm_mode: ContextVmMode::from_env_value(
                std::env::var("DAVINCI_CONTEXT_VM").ok().as_deref(),
            ),
            queues: SteerFollowUpQueues::default(),
            remote: RemoteQueue::default(),
            tools: BUILTIN_TOOLS.iter().map(|t| t.to_string()).collect(),
            tool_registry: BUILTIN_TOOLS.iter().map(|t| t.to_string()).collect(),
            skills: Vec::new(),
            templates: Vec::new(),
            context_files: Vec::new(),
            session: None,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            aborted: false,
            is_streaming: false,
            is_compacting: false,
            provider: "google".into(),
            model_id: String::new(),
            // TS `runtimeOptions.toolExecution ?? "parallel"`.
            tool_execution_mode: ToolExecutionMode::Parallel,
            custom_tool_executor: None,
            pre_tool: None,
            post_tool: None,
            permissions: Arc::new(PermissionState::new(PermissionPolicy::default())),
            approver: None,
            approval_responder: None,
            approval_registry: Arc::new(approval::ApprovalRegistry::default()),
            tool_context: ToolContext::default(),
            summarizer: None,
            subagent_runner: None,
            block_images: false,
            auto_resize_images: true,
            retry_aborted: false,
            transport: None,
            install_telemetry: true,
            reload_count: 0,
            event_sink: None,
            abort_signal: None,
            stats: RunStats::default(),
            counters: SharedCounters::shared(),
            prune_settings: PruneSettings::default(),
            evidence: None,
            tool_ledger: Arc::new(std::sync::Mutex::new(ToolCallLedger::default())),
            pending_tool_operations: Arc::new(Mutex::new(std::collections::HashMap::new())),
            operation_presentations: Arc::new(Mutex::new(std::collections::HashMap::new())),
            pruned_tool_results: std::collections::HashSet::new(),
            pruned_evidence: std::collections::HashMap::new(),
            base_system_prompt: system_prompt,
            previous_execution_mode: None,
            previous_plan_revision: None,
            visual_verification_available: false,
            capability_run_state: Arc::new(Mutex::new(prompt::CapabilityRunState::default())),
            mutation_verification: Arc::new(Mutex::new(MutationVerificationState::default())),
            pending_transaction_verification: Arc::new(Mutex::new(
                std::collections::BTreeMap::new(),
            )),
            command_receipts: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            plan_storage_error: None,
            pending_bash_messages: Vec::new(),
            pending_prompt_messages: Vec::new(),
            ephemeral_context: Vec::new(),
            turn_context_placement_override: None,
            turn_state_pending: None,
            provider_context_overhead_tokens: None,
            provider_output_limit: None,
            prepared_context_image: Arc::new(Mutex::new(None)),
            prepared_context_generation: 0,
            prepared_manifest_revisions: (0, 0),
            provider_system_prompt_suffix: None,
            last_prepared_manifest: None,
            runtime: None,
            decision_runtime: None,
            runtime_session: None,
        };
        *agent
            .tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = ToolExposureState::new([
            "read",
            "grep",
            "find",
            "ls",
            "exec_command",
            "apply_patch",
            "update_plan",
            "agent",
            "tool_search",
        ]);
        agent.sync_tool_authorization();
        agent
    }

    pub fn remote_queue(&self) -> RemoteQueue {
        self.remote.clone()
    }

    pub fn new_builtin(profile: prompt::PromptProfile) -> Self {
        let default_mode = PermissionPolicy::default().mode;
        let ctx = prompt::composer::PromptContext {
            provider: "google",
            model_id: "",
            permission_mode: default_mode,
            plan_active: default_mode == PermissionMode::ReadOnly,
        };
        let mut session = prompt::PromptSessionState::builtin(profile);
        let composed = session.render_and_record(&ctx);
        let mut agent = Self::new(&composed.text);
        agent.prompt_manifest = Some(composed.manifest);
        agent.prompt_session = session;
        agent
    }

    pub fn set_runtime(&mut self, mut runtime: RuntimeHandle) {
        runtime.ensure_conversation_identity_current();
        self.tool_context
            .mcp
            .register_with(&runtime.capability_registry);
        runtime
            .cancellation_token
            .bind_job_book(&self.tool_context.jobs);
        let owns_abort_signal = self.abort_signal.as_ref().is_some_and(|signal| {
            self.runtime.as_ref().is_some_and(|previous| {
                Arc::ptr_eq(signal, &previous.cancellation_token.as_atomic_bool())
            })
        });
        if self.abort_signal.is_none() || owns_abort_signal {
            self.abort_signal = Some(runtime.cancellation_token.as_atomic_bool());
        }
        self.tool_context.cache = runtime.cache.clone();
        self.tool_context.runtime = Some(runtime.clone());
        self.runtime_session = self.session.as_ref().and_then(|session| {
            let source = std::fs::canonicalize(&session.path).ok()?;
            (runtime.session_id.as_deref() == Some(session.header.id.as_str()))
                .then(|| (source, session.header.id.clone(), runtime.run_id))
        });
        if let Some(session) = &self.session {
            runtime
                .context_vm
                .bind_session_source(session.path.clone(), session.header.id.clone());
            if let Some((root, through_seq)) = runtime::context_vm::latest_persisted_root(
                &session.entries,
                session.leaf_id.as_deref(),
            ) {
                runtime.context_vm.install_root(root, through_seq);
            }
        }
        self.runtime = Some(runtime);
    }

    pub fn with_runtime(mut self, runtime: RuntimeHandle) -> Self {
        self.set_runtime(runtime);
        self
    }

    pub fn set_decision_runtime(&mut self, runtime: Arc<decision::DecisionRuntime>) {
        self.decision_runtime = Some(runtime);
    }

    pub fn decision_runtime(&self) -> Option<Arc<decision::DecisionRuntime>> {
        self.decision_runtime.clone()
    }

    pub fn enable_decision_runtime(&mut self) {
        if let Some(runtime) = &self.decision_runtime {
            runtime.enable();
        }
    }

    pub fn disable_decision_runtime(&mut self) {
        if let Some(runtime) = &self.decision_runtime {
            runtime.disable();
        }
    }

    pub fn enqueue_decision_shadow(
        &self,
        request: &decision::request::DecisionRequest,
    ) -> Result<(), decision::provider::DecisionError> {
        self.decision_runtime
            .as_ref()
            .ok_or(decision::provider::DecisionError::Disabled)?
            .enqueue_shadow(request.clone())
    }

    pub fn evaluate_decision_shadow(
        &self,
        request: &decision::request::DecisionRequest,
    ) -> Result<decision::response::DecisionResponse, decision::provider::DecisionError> {
        let Some(runtime) = &self.decision_runtime else {
            return Err(decision::provider::DecisionError::Disabled);
        };
        runtime.evaluate(request)
    }

    /// Return the runtime bound to the current session, when reusable by the host.
    pub fn runtime_for_session(&self) -> Option<&RuntimeHandle> {
        let session = self.session.as_ref()?;
        let (path, id, run_id) = self.runtime_session.as_ref()?;
        let source = std::fs::canonicalize(&session.path).ok()?;
        self.runtime.as_ref().filter(|runtime| {
            source == *path
                && session.header.id == *id
                && runtime.session_id.as_deref() == Some(id.as_str())
                && runtime.run_id == *run_id
        })
    }

    pub fn register_context_source(&mut self, source: Arc<dyn crate::runtime::ContextSource>) {
        if let Some(runtime) = &mut self.runtime {
            runtime.context_broker.register_context_source(source);
        }
    }

    pub fn build_context(
        &self,
        request: &crate::runtime::ContextRequest,
    ) -> crate::runtime::ContextPacket {
        if let Some(runtime) = &self.runtime {
            runtime.context_broker.build_context(request)
        } else {
            crate::runtime::ContextPacket::empty()
        }
    }

    /// Restore the base prompt before each extension-aware prompt turn.
    pub fn reset_system_prompt_to_base(&mut self) {
        self.system_prompt = self.base_system_prompt.clone();
        if self.plan_mode()
            && self.turn_context_placement() == turn_context::TurnContextPlacement::SystemPrompt
        {
            self.system_prompt.push_str("\n\n");
            self.system_prompt.push_str(crate::PLAN_MODE_APPENDIX);
        }
    }

    pub fn permission_mode(&self) -> PermissionMode {
        self.permissions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .mode
    }

    /// Snapshot prompt-facing state from the live permission, plan, contract,
    /// and host capability owners without mutating any of them.
    pub fn runtime_prompt_state(&self) -> prompt::RuntimePromptState {
        let plan = self
            .tool_context
            .living_plan
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let plan_revision = (plan.revision != 0).then_some(plan.revision);

        prompt::RuntimePromptState {
            permission_mode: self.permission_mode(),
            plan_revision,
            plan_approved: plan_revision.is_some() && plan.approved_revision == plan_revision,
            active_contract: self.active_contract().is_some(),
            visual_verification_available: self.visual_verification_available,
        }
    }

    /// Update host capability availability before the next prompt is prepared.
    pub fn set_visual_verification_available(&mut self, available: bool) {
        self.visual_verification_available = available;
    }

    /// Return a read-only snapshot of lifecycle evidence for the prepared turn.
    pub fn capability_run_state(&self) -> prompt::CapabilityRunState {
        self.capability_run_state
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Return a read-only snapshot of mutation and verification evidence.
    pub fn mutation_verification_state(&self) -> MutationVerificationState {
        self.mutation_verification
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Classify whether the current run has evidence for its latest mutation.
    pub fn completion_evidence(&self) -> CompletionEvidence {
        let state = self
            .mutation_verification
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if state.mutation_generation == 0 {
            return CompletionEvidence::NotRequired;
        }

        match state.latest_evidence.as_ref() {
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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

    #[cfg(test)]
    pub(crate) fn remember_verification_command(&self, tool: &str, command: &str) {
        self.remember_verification_call(tool, &serde_json::json!({"command": command}), &self.cwd);
    }

    pub(crate) fn remember_verification_call(&self, tool: &str, arguments: &Value, cwd: &Path) {
        let mut state = self
            .mutation_verification
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        state.last_verification = Some(LastVerification {
            tool: tool.to_string(),
            command: arguments
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            arguments: arguments.clone(),
            cwd: Some(cwd.to_path_buf()),
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

    fn reset_capability_run_state(
        &self,
        capabilities: &prompt::CapabilityDecision,
        visual_backend_available: bool,
    ) {
        self.capability_run_state
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .reset_for_user_turn(capabilities, visual_backend_available);
    }

    fn observe_capability_event(&self, event: &AgentEvent) {
        self.capability_run_state
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .observe_event(event);
    }

    pub fn is_plan_mode(&self) -> bool {
        self.permission_mode() == PermissionMode::ReadOnly
    }

    /// Compatibility accessor; both getters read the policy, never a flag.
    pub fn plan_mode(&self) -> bool {
        self.is_plan_mode()
    }

    /// Hosts call this while idle, before another tool can be approved.
    pub fn set_permission_mode(&mut self, mode: PermissionMode) {
        let previous = self.permission_mode();
        if previous != mode {
            if mode == PermissionMode::ReadOnly {
                self.previous_execution_mode = Some(previous);
                self.tool_context
                    .living_plan
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .approved_revision = None;
            } else {
                self.previous_execution_mode = Some(mode);
            }
            let mut policy = self
                .permissions
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            policy.session_allow.clear();
            policy.mode = mode;
        }
        self.reset_system_prompt_to_base();
    }

    pub fn cycle_permission_mode(&mut self) -> PermissionMode {
        let mode = self.permission_mode().next();
        self.set_permission_mode(mode);
        mode
    }

    fn plan_execution_target(&self) -> PermissionMode {
        match self.previous_execution_mode {
            Some(mode @ (PermissionMode::Ask | PermissionMode::Edits | PermissionMode::Auto)) => {
                mode
            }
            _ => PermissionMode::Ask,
        }
    }

    pub fn set_plan_mode(&mut self, on: bool) {
        if on {
            self.set_permission_mode(PermissionMode::ReadOnly);
        } else if self.is_plan_mode() {
            // Never silently restore Always Approve.
            self.set_permission_mode(self.plan_execution_target());
        }
    }

    /// Replace the ephemeral context used for the next provider request.
    /// Extension-provided context is inserted immediately before the latest
    /// user message so it supports, rather than follows, the active prompt.
    pub fn set_ephemeral_context(&mut self, messages: Vec<ChatMessage>) {
        self.ephemeral_context = messages;
    }

    pub fn turn_context_placement(&self) -> turn_context::TurnContextPlacement {
        self.turn_context_placement_override
            .unwrap_or_else(|| turn_context::default_placement(&self.provider, &self.model_id))
    }

    /// Append changing harness state after the current user message on
    /// cache-sensitive routes so prior provider input remains an exact prefix.
    pub fn commit_turn_context(&mut self, memory: Option<String>) {
        if self.turn_context_placement() != turn_context::TurnContextPlacement::Appended {
            return;
        }
        if self.messages.last().map(|message| message.role.as_str()) != Some("user") {
            return;
        }

        let previous = turn_context::TurnContextState::from_messages(&self.messages);
        let runtime_state = self.turn_state_pending.clone().unwrap_or_default();
        let plan = self.plan_turn_context();
        let input = turn_context::TurnContextInput {
            runtime_state: &runtime_state,
            plan_mode_appendix: self.is_plan_mode().then_some(crate::PLAN_MODE_APPENDIX),
            living_plan: plan
                .as_ref()
                .map(|(revision, text)| (*revision, text.as_str())),
            memory: memory.as_deref(),
        };
        if let Some((text, state)) = turn_context::render_turn_context(&previous, &input) {
            self.record_custom_message(&serde_json::json!({
                "customType": turn_context::TURN_CONTEXT_CUSTOM_TYPE,
                "content": text,
                "display": false,
                "details": state,
            }));
        }
    }

    /// Remove extension context after a prompt turn (or when a session is
    /// switched) without touching persisted conversation messages.
    pub fn clear_ephemeral_context(&mut self) {
        self.ephemeral_context.clear();
    }

    pub fn push_event(&self, events: &mut Vec<AgentEvent>, event: AgentEvent) {
        self.observe_capability_event(&event);
        if let Some(sink) = &self.event_sink {
            (sink.0)(&event);
        }
        events.push(event);
    }

    /// Hand an event to the sink without recording it. A provider closure
    /// that streams live uses this for `MessageStart` and every
    /// `MessageUpdate`, and returns `CompleteOutput { streamed_live: true }`
    /// so the loop records them into the event list without sending them a
    /// second time.
    pub fn emit_live(&self, event: AgentEvent) {
        self.observe_capability_event(&event);
        if let Some(sink) = &self.event_sink {
            (sink.0)(&event);
        }
    }

    /// TS `evalSession.reload()` — isolated evals have no extensions; count the step.
    pub fn reload(&mut self) {
        self.reload_count = self.reload_count.saturating_add(1);
        self.aborted = false;
    }

    fn apply_composed_turn_prompt(&mut self, composed: &prompt::composer::ComposedPrompt) {
        match self.turn_context_placement() {
            turn_context::TurnContextPlacement::SystemPrompt => {
                self.system_prompt = composed.text.clone();
                self.base_system_prompt = composed.text.clone();
                self.turn_state_pending = None;
            }
            turn_context::TurnContextPlacement::Appended => {
                let parts = prompt::turn::split_turn_prompt(&self.prompt_session, composed);
                self.system_prompt = parts.instructions.clone();
                self.base_system_prompt = parts.instructions;
                self.turn_state_pending = Some(parts.turn_state);
            }
        }
    }

    pub fn prepare_builtin_prompt_for_user_turn(
        &mut self,
        user_text: &str,
    ) -> Result<prompt::PreparedTurnPrompt, String> {
        if !self.prompt_session.is_builtin() {
            let no_capabilities = prompt::CapabilityDecision {
                capabilities: Vec::new(),
                reasons: Vec::new(),
                evidence: Vec::new(),
            };
            self.reset_capability_run_state(&no_capabilities, self.visual_verification_available);
            return Err(
                "Cannot prepare builtin prompt: session uses custom replacement prompt".to_string(),
            );
        }

        let previous_user_request = self.last_real_user_request.as_deref();

        let recent_tools: Vec<String> = self
            .tool_ledger
            .lock()
            .map(|ledger| ledger.recent_tool_names(10))
            .unwrap_or_default();

        let has_uncommitted_changes = runtime::has_uncommitted_changes(&self.cwd);
        let router_input = prompt::CapabilityRouterInput::new(user_text)
            .with_previous_request(previous_user_request)
            .with_recent_tools(&recent_tools)
            .with_uncommitted_changes(has_uncommitted_changes);

        let capabilities = prompt::route_capabilities(&router_input);

        let runtime_state = self.runtime_prompt_state();
        let permission_mode = runtime_state.permission_mode;

        let ctx = prompt::composer::PromptContext {
            provider: &self.provider,
            model_id: &self.model_id,
            permission_mode,
            plan_active: permission_mode == PermissionMode::ReadOnly,
        };

        let composed = prompt::turn::compose_turn_prompt(
            &self.prompt_session,
            &ctx,
            &capabilities,
            &runtime_state,
        )?;
        self.reset_capability_run_state(&capabilities, runtime_state.visual_verification_available);

        self.apply_composed_turn_prompt(&composed);
        self.prompt_manifest = Some(composed.manifest.clone());
        self.prompt_session.last_manifest = Some(composed.manifest.clone());
        self.prompt_session.stable_bundle_hash = Some(composed.manifest.stable_sha256.clone());

        Ok(prompt::PreparedTurnPrompt {
            composed,
            capabilities,
            runtime_state,
        })
    }

    pub(crate) fn prepare_builtin_prompt_for_user_turn_batch(
        &mut self,
        user_texts: &[&str],
    ) -> Result<prompt::PreparedTurnPrompt, String> {
        if user_texts.is_empty() {
            return Err("Cannot prepare empty user-turn batch".to_string());
        }

        let original_previous = self.last_real_user_request.clone();
        let mut union = prompt::CapabilityDecision {
            capabilities: Vec::new(),
            reasons: Vec::new(),
            evidence: Vec::new(),
        };
        let mut last_runtime_state = None;

        for user_text in user_texts {
            let prepared = match self.prepare_builtin_prompt_for_user_turn(user_text) {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.last_real_user_request = original_previous;
                    return Err(error);
                }
            };
            for capability in prepared.capabilities.capabilities {
                if !union.capabilities.contains(&capability) {
                    union.capabilities.push(capability);
                }
            }
            union.reasons.extend(prepared.capabilities.reasons);
            union.evidence.extend(prepared.capabilities.evidence);
            last_runtime_state = Some(prepared.runtime_state);
            self.last_real_user_request = Some((*user_text).to_string());
        }
        self.last_real_user_request = original_previous;
        union
            .capabilities
            .sort_by_key(|capability| match capability {
                prompt::NativeBehaviorCapability::FrontendDesign => 0,
                prompt::NativeBehaviorCapability::Debugging => 1,
                prompt::NativeBehaviorCapability::CodeReview => 2,
            });

        let runtime_state = last_runtime_state.expect("non-empty batch has runtime state");
        let permission_mode = self.permissions.lock().map(|p| p.mode).unwrap_or_default();
        let ctx = prompt::composer::PromptContext {
            provider: &self.provider,
            model_id: &self.model_id,
            permission_mode,
            plan_active: permission_mode == PermissionMode::ReadOnly,
        };
        let composed =
            prompt::turn::compose_turn_prompt(&self.prompt_session, &ctx, &union, &runtime_state)?;
        self.reset_capability_run_state(&union, runtime_state.visual_verification_available);
        self.apply_composed_turn_prompt(&composed);
        self.prompt_manifest = Some(composed.manifest.clone());
        self.prompt_session.last_manifest = Some(composed.manifest.clone());
        self.prompt_session.stable_bundle_hash = Some(composed.manifest.stable_sha256.clone());

        Ok(prompt::PreparedTurnPrompt {
            composed,
            capabilities: union,
            runtime_state,
        })
    }

    pub fn prompt(&mut self, text: &str) -> ChatMessage {
        self.prompt_user_with(text, &[])
    }

    /// Tool outcomes since the latest real user prompt. Injected reminders and
    /// context messages do not begin a new turn; batch children count separately.
    pub fn effort_signals(&self) -> effort::EffortSignals {
        let start = self
            .messages
            .iter()
            .rposition(|message| {
                message.role == "user" && message.extra_bool(REAL_USER_ORIGIN_FIELD)
            })
            .unwrap_or(0);
        let mut signals = effort::EffortSignals::default();
        for message in &self.messages[start..] {
            if message.role != "toolResult" {
                continue;
            }
            if message.tool_name.as_deref() == Some("batch") {
                if let Some(operations) = message
                    .extra
                    .get("details")
                    .and_then(|details| details.get("operations"))
                    .and_then(Value::as_array)
                {
                    for operation in operations {
                        let error = match operation.get("status").and_then(Value::as_str) {
                            Some("ok") => false,
                            Some("error") => true,
                            _ => continue,
                        };
                        signals.observe(operation.get("tool").and_then(Value::as_str), error);
                    }
                    continue;
                }
            }
            signals.observe(message.tool_name.as_deref(), message.is_error == Some(true));
        }
        signals
    }

    /// The next request's effort; the configured level and prompt stay stable.
    pub fn request_thinking_level(&self) -> ThinkingLevel {
        effort::request_level(
            self.effort_policy,
            self.thinking_level,
            self.effort_signals(),
        )
    }

    pub fn prompt_user_with(
        &mut self,
        text: &str,
        images: &[davinci_ai::MessageContent],
    ) -> ChatMessage {
        let _ = self.prepare_builtin_prompt_for_user_turn(text);
        self.prompt_user_with_prepared(text, images)
    }

    pub(crate) fn prompt_user_with_prepared(
        &mut self,
        text: &str,
        images: &[davinci_ai::MessageContent],
    ) -> ChatMessage {
        let message = self.prompt_with_origin(text, images, true);
        self.last_real_user_request = Some(text.to_string());
        message
    }

    pub fn prompt_with(
        &mut self,
        text: &str,
        images: &[davinci_ai::MessageContent],
    ) -> ChatMessage {
        self.prompt_with_origin(text, images, false)
    }

    fn prompt_with_origin(
        &mut self,
        text: &str,
        images: &[davinci_ai::MessageContent],
        real_user_origin: bool,
    ) -> ChatMessage {
        self.flush_pending_bash_messages();
        // A job that finished while the user was typing is in context
        // before what they typed, so the model reads the news first.
        for notice in self.job_notice_messages() {
            self.messages.push(notice.clone());
            if let Some(session) = &mut self.session {
                let _ = session.append_entry(chat_entry(
                    "user",
                    serde_json::to_value(&notice.content).unwrap_or(Value::Null),
                    &notice.extra,
                ));
            }
            self.pending_prompt_messages.push(notice);
        }
        let mut content = vec![davinci_ai::MessageContent::Text {
            text: text.to_string(),
        }];
        content.extend(images.iter().cloned());
        if self.auto_resize_images {
            content = crate::normalize_tool_result_images(&content, true);
        }
        let mut message = ChatMessage {
            role: "user".into(),
            content,
            ..ChatMessage::default()
        };
        if real_user_origin {
            message
                .extra
                .insert(REAL_USER_ORIGIN_FIELD.into(), Value::Bool(true));
        }
        self.messages.push(message.clone());
        if let Some(session) = &mut self.session {
            let _ = session.append_entry(chat_entry(
                "user",
                serde_json::to_value(&message.content).unwrap_or(Value::Null),
                &message.extra,
            ));
        }
        self.pending_prompt_messages.push(message.clone());
        message
    }

    pub fn context_vm_mode(&self) -> ContextVmMode {
        self.context_vm_mode
    }

    pub fn set_context_vm_mode(&mut self, mode: ContextVmMode) {
        self.context_vm_mode = mode;
        if let Some(runtime) = &mut self.runtime {
            runtime.context_vm.set_mode(mode);
        }
    }

    fn legacy_messages_for_provider(&self) -> Vec<ChatMessage> {
        let plan_context = (self.turn_context_placement()
            == turn_context::TurnContextPlacement::SystemPrompt)
            .then(|| self.plan_provider_context())
            .flatten()
            .map(|text| ChatMessage::text("custom", text));
        let selected = self.select_root_context(self.context_window);
        let ephemeral_context = selected.ephemeral_messages;
        if ephemeral_context.is_empty()
            && self.pruned_tool_results.is_empty()
            && plan_context.is_none()
        {
            return convert_to_llm_for_provider(&self.messages, self.block_images);
        }
        let mut messages = self.project_with_evidence();
        if ephemeral_context.is_empty() && plan_context.is_none() {
            return convert_to_llm_for_provider(&messages, self.block_images);
        }
        let insertion = messages
            .iter()
            .rposition(|message| message.role == "user")
            .unwrap_or(messages.len());
        messages.splice(
            insertion..insertion,
            plan_context.into_iter().chain(ephemeral_context),
        );
        convert_to_llm_for_provider(&messages, self.block_images)
    }

    #[doc(hidden)]
    pub fn legacy_messages_for_provider_for_test(&self) -> Vec<ChatMessage> {
        self.legacy_messages_for_provider()
    }

    fn build_context_vm_image(&self) -> Result<runtime::ContextImage, String> {
        let Some(runtime) = &self.runtime else {
            return Err("context VM runtime is unavailable".into());
        };
        let events = self.context_vm_events_for_runtime();
        let selected = self.select_root_context(self.context_window);
        let mut items = Vec::new();
        for file in selected.repository_files {
            items.push(runtime::ContextItem {
                source: format!("file::{}", file.path.display()),
                content: file.body.clone(),
                estimated_tokens: (file.body.len() as u64).div_ceil(4),
                priority: 100,
                stable_for_cache: true,
                provenance: serde_json::json!({"provenance_kind":"repository_fact"}),
            });
        }
        for (index, message) in selected.ephemeral_messages.iter().enumerate() {
            let content = davinci_ai::content_text(&message.content);
            items.push(runtime::ContextItem {
                source: format!("ephemeral_context::{index}"),
                estimated_tokens: (content.len() as u64).div_ceil(4),
                content,
                priority: 200,
                stable_for_cache: false,
                provenance: serde_json::json!({"provenance_kind":"tool_evidence"}),
            });
        }
        if let Some(plan) = self.plan_provider_context() {
            items.push(runtime::ContextItem {
                source: "agent::living_plan".into(),
                estimated_tokens: (plan.len() as u64).div_ceil(4),
                content: plan,
                priority: 500,
                stable_for_cache: true,
                provenance: serde_json::json!({"provenance_kind":"user_decision"}),
            });
        }
        let budget = self.provider_context_budget();
        let live = self.live_tool_exchange();
        let live_tokens = live
            .iter()
            .map(provider_budget::message_token_ceiling)
            .fold(0u64, u64::saturating_add);
        if budget.reserved().saturating_add(live_tokens) >= budget.window {
            return Err(runtime::context_vm::CONTEXT_BUDGET_EXCEEDED.into());
        }
        let max_tokens = budget.working_set_budget().saturating_sub(live_tokens);
        let direct_tokens = items.iter().map(|item| item.estimated_tokens).sum::<u64>();
        let goal = self
            .last_real_user_request
            .clone()
            .or_else(|| {
                events
                    .iter()
                    .rev()
                    .find(|event| event.kind == runtime::context_vm::ContextEventKind::User)
                    .map(|event| event.visible_text.clone())
            })
            .unwrap_or_else(|| "continue the current task".into());
        let request = runtime::ContextRequest::new(
            runtime.run_id,
            runtime.agent_id,
            goal,
            self.provider.clone(),
            self.model_id.clone(),
            self.tools.clone(),
            max_tokens.saturating_sub(direct_tokens),
            runtime::AgentKind::Main,
        );
        let (broker_packet, ledger) = runtime.context_broker.build_context_with_ledger(&request);
        let mut all_items = items;
        all_items.extend(broker_packet.items);
        // The legacy broker remains bounded. Mandatory candidates omitted by
        // its preselection still reach the authoritative compiler budget gate.
        all_items.extend(ledger.into_iter().filter_map(|(item, selected, reason)| {
            (!selected && reason.as_deref() == Some("over_budget") && item.is_mandatory())
                .then_some(item)
        }));
        let broker_tokens = all_items.iter().map(|item| item.estimated_tokens).sum();
        let broker_packet = runtime::ContextPacket {
            items: all_items,
            estimated_tokens: broker_tokens,
            cache_key: format!("agent-context:{}", broker_packet.cache_key),
        };
        let mut image = runtime
            .context_vm
            .compile(&events, &broker_packet, max_tokens)?;
        // The most recent, complete tool exchange remains protocol data. Older
        // exchanges are evidence only. Reserve this suffix before compilation.
        for (index, message) in live.iter().enumerate() {
            let content = serde_json::to_string(message).map_err(|e| e.to_string())?;
            image.entries.push(runtime::ContextImageEntry {
                id: format!("live-tool-exchange:{index}"),
                source_ref: format!("live-tool-exchange:{index}"),
                category: "live_tool_exchange".into(),
                provenance_kind: runtime::context_manifest::ProvenanceKind::ToolEvidence,
                content_hash: runtime::cache::digest(content.as_bytes()),
                estimated_tokens: provider_budget::message_token_ceiling(message),
                content,
                mandatory: true,
                stable_for_cache: false,
            });
        }
        image.messages.extend(live);
        image.estimated_tokens = image.estimated_tokens.saturating_add(live_tokens);
        Ok(image)
    }

    fn live_tool_exchange(&self) -> Vec<ChatMessage> {
        if self.messages.last().is_none_or(|m| m.role != "toolResult") {
            return Vec::new();
        }
        let Some(start) = self.messages.iter().rposition(|m| m.role == "assistant") else {
            return Vec::new();
        };
        let mut calls = std::collections::HashSet::new();
        for content in &self.messages[start].content {
            if let MessageContent::ToolCall { id, .. } = content {
                if id.is_empty() || !calls.insert(id.as_str()) {
                    return Vec::new();
                }
            }
        }
        let results = &self.messages[start + 1..];
        let ids = results
            .iter()
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect::<std::collections::HashSet<_>>();
        if calls.is_empty()
            || calls != ids
            || results.len() != ids.len()
            || results.iter().any(|m| m.role != "toolResult")
        {
            return Vec::new();
        }
        convert_to_llm_for_provider(&self.messages[start..], self.block_images)
    }

    pub fn provider_context_budget(&self) -> provider_budget::ProviderContextBudget {
        let requested_reasoning = if self.thinking_level == ThinkingLevel::Off {
            0
        } else {
            u64::from(davinci_ai::thinking_budget_for_level(
                self.thinking_level,
                self.thinking_budgets.as_ref(),
            ))
        };
        let total_output = self
            .compaction
            .reserve_tokens
            .max(requested_reasoning.saturating_add(1024))
            .min(self.provider_output_limit.unwrap_or(u64::MAX));
        let reasoning = requested_reasoning.min(total_output.saturating_sub(1024));
        provider_budget::ProviderContextBudget {
            window: self.context_window,
            system: provider_budget::text_token_ceiling(&self.provider_system_prompt()),
            tools: self.provider_context_overhead_tokens.unwrap_or_else(|| {
                serde_json::to_vec(&self.provider_tool_specs())
                    .map_or(u64::MAX, |v| v.len() as u64 + 128)
            }),
            reasoning_reserve: reasoning,
            output_reserve: total_output.saturating_sub(reasoning),
            safety_margin: 256,
        }
    }

    pub fn set_provider_output_limit(&mut self, limit: Option<u64>) {
        self.provider_output_limit = limit;
    }

    pub fn context_vm_provider_output_limit(&self) -> Option<u64> {
        (self.context_vm_mode() == ContextVmMode::Active)
            .then(|| self.provider_context_budget().output_limit())
    }

    pub(crate) fn context_vm_events_for_runtime(&self) -> Vec<runtime::context_vm::ContextEvent> {
        if let Some(session) = &self.session {
            runtime::context_vm::events_from_session_branch(
                &session.entries,
                session.leaf_id.as_deref(),
            )
        } else {
            runtime::context_vm::events_from_messages(&self.messages)
        }
    }

    fn record_context_vm_shadow(&self, legacy: &[ChatMessage]) {
        let Ok(image) = self.prepared_context_image() else {
            return;
        };
        let events = self
            .runtime
            .as_ref()
            .map(|runtime| runtime.context_vm.events())
            .unwrap_or_default();
        let comparison = runtime::context_vm::compare_shadow_views(legacy, &image, &events);
        if let Some(runtime) = &self.runtime {
            runtime.context_vm.record_shadow_comparison(&comparison);
            runtime.emit_observe(crate::RuntimeEvent::ContextVmShadowCompared {
                legacy_tokens: comparison.legacy_estimated_tokens,
                vm_tokens: comparison.vm_estimated_tokens,
                missing_user_refs: comparison.missing_user_refs.len() as u64,
                missing_tool_refs: comparison.missing_tool_refs.len() as u64,
            });
        }
    }

    pub fn native_responses_resume_record(
        &self,
    ) -> Option<davinci_ai::NativeResponsesResumeRecord> {
        let session = self.session.as_ref()?;
        session.entries.iter().rev().find_map(|entry| {
            if entry.entry_type != "custom"
                || entry.custom_type.as_deref()
                    != Some(davinci_ai::NATIVE_RESPONSES_TURN_ENTRY_TYPE)
            {
                return None;
            }
            entry.extra.get("data").and_then(|value| {
                serde_json::from_value::<davinci_ai::NativeResponsesResumeRecord>(value.clone())
                    .ok()
            })
        })
    }

    pub fn messages_for_provider(&self) -> Vec<ChatMessage> {
        match self.context_vm_mode() {
            ContextVmMode::Off => self.legacy_messages_for_provider(),
            ContextVmMode::Shadow => {
                let legacy = self.legacy_messages_for_provider();
                self.record_context_vm_shadow(&legacy);
                legacy
            }
            ContextVmMode::Active => self
                .prepared_context_image()
                .map(|image| image.messages.clone())
                .unwrap_or_else(|_| self.legacy_messages_for_provider()),
        }
    }

    /// The run's counters, complete.
    pub fn run_stats(&self) -> RunStats {
        let mut stats = self.stats;
        self.counters.fold_into(&mut stats);
        stats
    }

    /// The token estimate for what the provider will actually be sent:
    /// pruned tool results count as their placeholder, and an extension's
    /// ephemeral context counts although it is not in `messages`. System and
    /// tool schemas count too. This is a byte heuristic, not a tokenizer or upper bound.
    pub fn estimated_context_tokens(&self) -> u64 {
        if self.context_vm_mode() == ContextVmMode::Active {
            if let Ok(image) = self.prepared_context_image() {
                return self.context_vm_estimated_provider_tokens(&image);
            }
        }
        self.messages
            .iter()
            .map(|message| {
                self.pruned_text(message)
                    .map(|text| (text.len() as u64).div_ceil(4))
                    .unwrap_or_else(|| compaction::estimate_tokens(message))
            })
            .sum::<u64>()
            + estimate_context_tokens(
                &self
                    .select_root_context(self.context_window)
                    .ephemeral_messages,
            )
            + self
                .plan_provider_context()
                .map(|text| (text.len() as u64).div_ceil(4))
                .unwrap_or(0)
            + (self.provider_system_prompt().len() as u64).div_ceil(4)
            + self.provider_context_overhead_tokens.unwrap_or_else(|| {
                let specs = self.provider_tool_specs();
                (serde_json::to_vec(&specs)
                    .expect("tool schemas are JSON")
                    .len() as u64)
                    .div_ceil(4)
            })
    }

    fn context_vm_estimated_provider_tokens(&self, image: &runtime::ContextImage) -> u64 {
        let budget = self.provider_context_budget();
        image
            .estimated_tokens
            .saturating_add(budget.system)
            .saturating_add(budget.tools)
    }

    /// Set once per request configuration using the actual tool catalog.
    /// `None` restores the builtin/MCP estimate.
    pub fn set_provider_context_overhead_tokens(&mut self, tokens: Option<u64>) {
        self.provider_context_overhead_tokens = tokens;
    }

    /// Record host-owned request context that follows the mutable turn prompt.
    pub fn set_provider_system_prompt_suffix(&mut self, suffix: Option<String>) {
        self.provider_system_prompt_suffix = suffix;
    }

    /// Build the exact system prompt for the next provider request.
    pub fn provider_system_prompt(&self) -> String {
        let mut prompt = self.system_prompt.clone();
        context::append_repository_context(&mut prompt, &self.context_files);
        if let Some(suffix) = self.provider_system_prompt_suffix.as_deref() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str(suffix);
        }
        prompt
    }

    /// Captures the complete prepared provider context manifest.
    pub fn prepare_context_manifest(
        &mut self,
        request_id: &str,
        root_run_id: runtime::ids::RunId,
        source_revision: u64,
        overlay_revision: u64,
    ) -> runtime::context_manifest::PreparedContextManifest {
        use runtime::context_manifest::{
            ContextManifestEntry, PreparedContextManifest, ProvenanceKind,
        };
        if self.prepared_manifest_revisions != (source_revision, overlay_revision) {
            self.prepared_manifest_revisions = (source_revision, overlay_revision);
            self.invalidate_context_image();
        }
        let mut entries = Vec::new();

        // 1. Mandatory system prompt
        let provider_system_prompt = self.provider_system_prompt();
        let sys_tokens = (provider_system_prompt.len() as u64).div_ceil(4);
        let sys_hash = ContextManifestEntry::hash_content(&provider_system_prompt);
        entries.push(ContextManifestEntry::new(
            "system_prompt",
            "system",
            ProvenanceKind::MandatoryPolicy,
            "agent::system_prompt",
            sys_hash,
            sys_tokens,
            true,
            Some("mandatory_system_prompt".into()),
            true,
            "fresh",
            None,
        ));

        // 2. Mandatory tool schemas
        let provider_tool_schemas = serde_json::to_string(&self.provider_tool_schema_value())
            .expect("provider tool schemas are JSON");
        let tool_tokens = self
            .provider_context_overhead_tokens
            .unwrap_or_else(|| (provider_tool_schemas.len() as u64).div_ceil(4));
        entries.push(ContextManifestEntry::new(
            "tool_schemas",
            "tools",
            ProvenanceKind::MandatoryPolicy,
            "agent::tool_catalog",
            ContextManifestEntry::hash_content(&provider_tool_schemas),
            tool_tokens,
            true,
            Some("mandatory_tool_schemas".into()),
            true,
            "fresh",
            None,
        ));

        let active_image = if self.context_vm_mode() == ContextVmMode::Active {
            match self.prepared_context_image() {
                Ok(image) => Some(image),
                Err(error) => {
                    if error == runtime::context_vm::CONTEXT_BUDGET_EXCEEDED {
                        entries.push(ContextManifestEntry::new(
                            "context_vm_budget",
                            "context_vm",
                            ProvenanceKind::MandatoryPolicy,
                            "agent::context_vm",
                            ContextManifestEntry::hash_content(&error),
                            0,
                            false,
                            Some(error),
                            true,
                            "unavailable",
                            None,
                        ));
                    }
                    None
                }
            }
        } else {
            None
        };

        // 3. Living plan if present. Active Context VM images already include
        // broker-selected plan and ephemeral items, so adding the legacy
        // projection here would duplicate provider context.
        if active_image.is_none() {
            if let Some(plan_text) = self.plan_provider_context() {
                let plan_tokens = (plan_text.len() as u64).div_ceil(4);
                entries.push(ContextManifestEntry::new(
                    "living_plan",
                    "plan",
                    ProvenanceKind::UserDecision,
                    "agent::living_plan",
                    ContextManifestEntry::hash_content(&plan_text),
                    plan_tokens,
                    true,
                    Some("active_plan".into()),
                    false,
                    "fresh",
                    None,
                ));
            }
        }

        // 4. Ephemeral context
        if active_image.is_none() {
            for (i, msg) in self.ephemeral_context.iter().enumerate() {
                let tokens = compaction::estimate_tokens(msg);
                let content_str = serde_json::to_string(&msg.content).unwrap_or_default();
                entries.push(ContextManifestEntry::new(
                    format!("ephemeral_{i}"),
                    "ephemeral",
                    ProvenanceKind::ToolEvidence,
                    "agent::ephemeral_context",
                    ContextManifestEntry::hash_content(&content_str),
                    tokens,
                    true,
                    Some("ephemeral_injection".into()),
                    false,
                    "fresh",
                    None,
                ));
            }
        }

        // 5. Conversation messages
        if active_image.is_none() {
            for (i, msg) in self.messages_for_provider().iter().enumerate() {
                let tokens = compaction::estimate_tokens(msg);
                let content_str = serde_json::to_string(&msg.content).unwrap_or_default();
                entries.push(ContextManifestEntry::new(
                    format!("message_{i}"),
                    "history",
                    ProvenanceKind::ToolEvidence,
                    format!("message::{}", msg.role),
                    ContextManifestEntry::hash_content(&content_str),
                    tokens,
                    true,
                    Some("conversation_history".into()),
                    false,
                    "fresh",
                    None,
                ));
            }
        }

        let manifest = PreparedContextManifest::new(
            request_id,
            root_run_id,
            source_revision,
            overlay_revision,
            entries,
            0,
        );
        let manifest = if let Some(image) = active_image {
            let root_id = image
                .root
                .checkpoint
                .as_ref()
                .map(|page| page.id.clone())
                .unwrap_or_else(|| format!("ctxvm:epoch:{}", image.root.epoch));
            let vm_entries = self
                .runtime
                .as_ref()
                .map(|runtime| runtime.context_vm.manifest_entries(&image))
                .unwrap_or_default();
            manifest.with_context_vm_entries(vm_entries, root_id, image.root.epoch)
        } else {
            manifest
        };

        self.last_prepared_manifest = Some(manifest.clone());
        manifest
    }

    /// Prune old tool output from the provider view when the context has
    /// grown past the start line. Idempotent between prune passes, so the
    /// provider's prompt cache keeps its prefix until the next pass.
    pub fn prune_context(&mut self) {
        if self.evidence.is_none() || !self.tools.iter().any(|tool| tool == "read") {
            return;
        }
        let tokens = self.estimated_context_tokens();
        let plan = pruning::plan_prune(
            &self.messages,
            &self.pruned_tool_results,
            tokens,
            self.context_window,
            &pruning::PruneSettings::for_route(
                &self.prune_settings,
                turn_context::is_cache_sensitive_route(&self.provider, &self.model_id),
            ),
        );
        if plan.is_empty() {
            return;
        }
        let plan: Vec<String> = plan.into_iter().filter(|id| {
            let Some(message) = self.messages.iter().find(|message| {
                message.role == "toolResult" && message.tool_call_id.as_ref() == Some(id)
            }) else { return false; };
            // Images and other structured blocks must remain lossless in context.
            if !message.content.iter().all(|block| matches!(block, davinci_ai::MessageContent::Text { .. })) {
                return false;
            }
            if !self.pruned_evidence.contains_key(id) {
                let text = davinci_ai::content_text(&message.content);
                let Ok(path) = self.evidence.as_ref().unwrap().store("pruned", &text) else {
                    return false;
                };
                let tool = message.tool_name.as_deref().unwrap_or("tool");
                let hint = format!(
                    "{} Read retained output at {} with offset/limit. Call {}; outcome {}; {} bytes, {} lines.",
                    pruning::placeholder(tool, text.len()), path.display(), id,
                    match message.is_error { Some(true) => "error", Some(false) => "completed", None => "unknown" },
                    text.len(), text.lines().count());
                self.pruned_evidence.insert(id.clone(), (path, hint));
                SharedCounters::add(&self.counters.evidence_files, 1);
            }
            self.evidence_readable(&self.pruned_evidence[id].0)
        }).collect();
        let ids: std::collections::HashSet<&String> = plan.iter().collect();
        let chars: usize = self
            .messages
            .iter()
            .filter(|message| {
                message.role == "toolResult"
                    && message
                        .tool_call_id
                        .as_ref()
                        .is_some_and(|id| ids.contains(id))
            })
            .map(|message| compaction::estimate_content_chars(&message.content))
            .sum();
        self.stats.pruned_results += plan.len() as u64;
        self.stats.pruned_chars += chars as u64;
        self.pruned_tool_results.extend(plan);
    }

    fn evidence_readable(&self, path: &std::path::Path) -> bool {
        self.tools.iter().any(|tool| tool == "read")
            && path.is_file()
            && matches!(
                self.permissions
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .decide(
                        "pruned-evidence",
                        "read",
                        &serde_json::json!({"path": path}),
                        &self.cwd
                    ),
                PermissionVerdict::Allow
            )
    }

    fn pruned_text(&self, message: &ChatMessage) -> Option<&str> {
        let id = message.tool_call_id.as_ref()?;
        let (path, hint) = self.pruned_evidence.get(id)?;
        (message.role == "toolResult"
            && self.pruned_tool_results.contains(id)
            && self.evidence_readable(path))
        .then_some(hint.as_str())
    }

    fn project_with_evidence(&self) -> Vec<ChatMessage> {
        self.messages
            .iter()
            .map(|message| {
                let Some(text) = self.pruned_text(message) else {
                    return message.clone();
                };
                ChatMessage {
                    content: vec![davinci_ai::MessageContent::Text {
                        text: text.to_string(),
                    }],
                    ..message.clone()
                }
            })
            .collect()
    }

    /// Ids of the tool results currently pruned from the provider view.
    pub fn pruned_tool_results(&self) -> &std::collections::HashSet<String> {
        &self.pruned_tool_results
    }

    /// Execution cannot continue after a session write has an uncertain outcome.
    /// Recovery must reopen the durable session before issuing another request.
    pub fn ensure_session_persistence(&self) -> Result<(), String> {
        match self
            .session
            .as_ref()
            .and_then(JsonlSession::persistence_error)
        {
            Some(error) => Err(format!("Session recovery required: {error}")),
            None => Ok(()),
        }
    }

    pub(crate) fn cache_operation_presentation(&self, call_id: &str, result: ToolResult) {
        self.operation_presentations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(call_id.to_owned(), result);
    }

    pub(crate) fn operation_presentation(&self, call_id: &str) -> Option<ToolResult> {
        self.operation_presentations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(call_id)
            .cloned()
    }

    pub(crate) fn clear_operation_presentation(&self, call_id: &str) {
        self.operation_presentations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(call_id);
    }

    fn persist_full_message(&mut self, message: &ChatMessage) {
        if let Some(session) = &mut self.session {
            let mut entry = SessionEntry::message(
                &message.role,
                serde_json::to_value(&message.content).unwrap_or(Value::Null),
            );
            entry.message = Some(serde_json::to_value(message).unwrap_or(Value::Null));
            let _ = session.append_entry(entry);
        }
    }

    fn commit_bash_message(&mut self, message: ChatMessage) {
        self.persist_full_message(&message);
        self.messages.push(message);
    }

    /// TypeScript `AgentSession.recordBashResult`.
    pub fn record_bash_result(
        &mut self,
        command: &str,
        result: &Value,
        exclude_from_context: bool,
    ) {
        let output = result
            .get("output")
            .or_else(|| result.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let exit_code = result
            .get("exitCode")
            .cloned()
            .or_else(|| {
                result
                    .get("details")
                    .and_then(|value| value.get("exitCode"))
                    .cloned()
            })
            .unwrap_or(Value::Null);
        let cancelled = result
            .get("cancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let truncated = result
            .get("truncated")
            .and_then(Value::as_bool)
            .or_else(|| {
                result
                    .get("details")
                    .and_then(|value| value.get("truncation"))
                    .map(|_| true)
            })
            .unwrap_or(false);
        let mut extra = serde_json::Map::new();
        extra.insert("command".into(), Value::String(command.to_string()));
        extra.insert("output".into(), Value::String(output));
        extra.insert("exitCode".into(), exit_code);
        extra.insert("cancelled".into(), Value::Bool(cancelled));
        extra.insert("truncated".into(), Value::Bool(truncated));
        extra.insert(
            "timestamp".into(),
            serde_json::json!(davinci_session::now_ms()),
        );
        extra.insert(
            "excludeFromContext".into(),
            Value::Bool(exclude_from_context),
        );
        if let Some(path) = result.get("fullOutputPath").and_then(Value::as_str) {
            extra.insert("fullOutputPath".into(), Value::String(path.to_string()));
        }
        let message = ChatMessage {
            role: "bashExecution".into(),
            content: Vec::new(),
            extra,
            ..ChatMessage::default()
        };
        if self.is_streaming {
            self.pending_bash_messages.push(message);
        } else {
            self.commit_bash_message(message);
        }
    }

    pub fn flush_pending_bash_messages(&mut self) {
        let pending = std::mem::take(&mut self.pending_bash_messages);
        for message in pending {
            self.commit_bash_message(message);
        }
    }

    /// The finished background jobs the model has not heard about, as the
    /// user messages that tell it (`customType: backgroundJob`). Taking
    /// them marks them announced.
    pub fn job_notice_messages(&self) -> Vec<ChatMessage> {
        let notices = self
            .tool_context
            .jobs
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take_unannounced();
        notices
            .iter()
            .map(|notice| {
                let mut extra = serde_json::Map::new();
                extra.insert(
                    "customType".into(),
                    Value::String(JOB_NOTICE_TYPE.to_string()),
                );
                extra.insert("jobId".into(), Value::from(notice.id));
                ChatMessage {
                    role: "user".into(),
                    content: vec![davinci_ai::MessageContent::Text {
                        text: notice.message_text(),
                    }],
                    extra,
                    ..ChatMessage::default()
                }
            })
            .collect()
    }

    /// Write the ledger to the session after the `todo` tool changed it, so
    /// a resumed session opens on the same plan.
    pub fn persist_todos(&mut self) {
        let value = self
            .tool_context
            .todos
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .to_value();
        if let Some(session) = &mut self.session {
            let mut extra = serde_json::Map::new();
            extra.insert("data".into(), value);
            let _ = session.append_entry(SessionEntry {
                id: String::new(),
                entry_type: "custom".into(),
                parent_id: None,
                seq: 0,
                timestamp: 0,
                message: None,
                custom_type: Some(TODO_ENTRY_TYPE.into()),
                extra,
            });
        }
    }

    /// Compatibility spelling for the checked session persistence path.
    pub fn persist_living_plan(&mut self) -> Result<(), String> {
        self.persist_plan()
    }

    /// Restore from the active branch; a different session cannot inherit a plan.
    /// Human execution approval is renewed after resume, while decisions survive.
    pub fn restore_living_plan(&mut self) -> bool {
        // The compatibility bool reports failure, while restore_plan also
        // exposes the error in /plan show and provider context and fails closed.
        self.restore_plan().unwrap_or(false)
    }

    /// The ledger the session last saved, if any.
    pub fn restore_todos(&mut self) -> bool {
        self.restore_living_plan();
        let Some(session) = &self.session else {
            return false;
        };
        let Some(list) = session
            .entries
            .iter()
            .rev()
            .find(|entry| entry.custom_type.as_deref() == Some(TODO_ENTRY_TYPE))
            .and_then(|entry| entry.extra.get("data"))
            .and_then(TodoList::from_value)
        else {
            return false;
        };
        *self
            .tool_context
            .todos
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = list;
        true
    }

    /// Persist and append a TypeScript extension `CustomMessage`.
    pub fn record_custom_message(&mut self, raw: &Value) -> ChatMessage {
        let content = match raw.get("content") {
            Some(Value::String(text)) => {
                vec![davinci_ai::MessageContent::Text { text: text.clone() }]
            }
            Some(Value::Array(_)) => {
                serde_json::from_value(raw["content"].clone()).unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let session_content = match raw.get("content") {
            Some(Value::String(text)) => Value::String(text.clone()),
            Some(Value::Array(value))
                if serde_json::from_value::<Vec<MessageContent>>(Value::Array(value.clone()))
                    .is_ok() =>
            {
                Value::Array(value.clone())
            }
            _ => serde_json::json!([]),
        };
        let timestamp = davinci_session::now_ms();
        let mut extra = serde_json::Map::new();
        for key in ["customType", "display", "details"] {
            if let Some(value) = raw.get(key) {
                extra.insert(key.to_string(), value.clone());
            }
        }
        extra.insert("timestamp".into(), serde_json::json!(timestamp));
        let message = ChatMessage {
            role: "custom".into(),
            content,
            extra,
            ..ChatMessage::default()
        };
        if let Some(session) = &mut self.session {
            let mut extra = serde_json::Map::new();
            extra.insert("content".into(), session_content);
            for key in ["display", "details"] {
                if let Some(value) = raw.get(key) {
                    extra.insert(key.to_string(), value.clone());
                }
            }
            let _ = session.append_entry(SessionEntry {
                id: String::new(),
                entry_type: "custom_message".into(),
                parent_id: None,
                seq: 0,
                timestamp,
                message: None,
                custom_type: raw
                    .get("customType")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                extra,
            });
        }
        self.messages.push(message.clone());
        self.pending_prompt_messages.push(message.clone());
        message
    }

    pub fn abort_retry(&mut self) {
        self.retry_aborted = true;
    }

    pub fn record_assistant(&mut self, text: &str) {
        let message = ChatMessage::text("assistant", text);
        self.messages.push(message);
        if let Some(session) = &mut self.session {
            let _ = session.append_entry(SessionEntry::message(
                "assistant",
                serde_json::json!([{"type":"text","text": text}]),
            ));
        }
    }

    pub fn last_assistant_text(&self) -> Option<String> {
        self.messages.iter().rev().find_map(|message| {
            if message.role == "assistant" {
                Some(content_text(&message.content))
            } else {
                None
            }
        })
    }

    /// Merge connected MCP tools into the active set and the permission class map.
    pub fn attach_mcp(&mut self, registry: crate::mcp::McpRegistry) {
        let names = registry.tool_names();
        let read_only = registry.read_only_names();
        if let Some(runtime) = &self.runtime {
            registry.register_with(&runtime.capability_registry);
        }
        self.tool_context.mcp = registry;
        self.apply_extension_tools(&names);
        self.permissions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .mcp_read_only = read_only;
    }

    /// Built-in specs plus live MCP tools, filtered by `self.tools`.
    pub fn builtin_and_mcp_specs(&self) -> Vec<AgentTool> {
        let mut specs: Vec<AgentTool> = tool_specs()
            .into_iter()
            .filter(|tool| self.tools.iter().any(|name| name == &tool.name))
            .collect();
        if let Some(spec) = specs.iter_mut().find(|tool| tool.name == "mcp_read") {
            spec.description = self.tool_context.mcp.mcp_read_description();
        }
        specs.extend(
            self.tool_context
                .mcp
                .specs()
                .into_iter()
                .filter(|tool| self.tools.iter().any(|name| name == &tool.name)),
        );
        specs
    }

    /// Synchronize the shared authorization view with the agent's active tool set.
    pub fn sync_tool_authorization(&self) {
        let authorized: std::collections::BTreeSet<String> = self.tools.iter().cloned().collect();
        *self
            .tool_context
            .authorized_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = authorized.clone();
        self.tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain_authorized(&authorized);
    }

    /// Expose every currently active, authorized tool for an explicit tool selection.
    pub fn expose_active_tools(&self) {
        self.sync_tool_authorization();
        let authorized = self
            .tool_context
            .authorized_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let mut exposure = self
            .tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for name in &self.tools {
            exposure.activate_authorized(name, authorized.contains(name));
        }
    }

    /// On cache-sensitive routes, expose the full authorized schema set before
    /// the first request so `tool_search` cannot mutate the provider tool list.
    pub fn freeze_tools_for_cache(&self) {
        if self.turn_context_placement() != turn_context::TurnContextPlacement::Appended {
            return;
        }
        self.expose_active_tools();
        let authorized = self
            .tool_context
            .authorized_tools
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(runtime) = &self.runtime {
            let mut exposure = self
                .tool_context
                .tool_exposure
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for capability in runtime.capability_registry.list() {
                if capability.schema.is_some() {
                    exposure.activate_authorized(
                        &capability.name,
                        authorized.contains(&capability.name),
                    );
                }
            }
        }
    }

    pub fn is_tool_visible(&self, name: &str) -> bool {
        self.tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_visible(name)
    }

    pub fn visible_tool_names(&self) -> std::collections::BTreeSet<String> {
        self.tool_context
            .tool_exposure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .visible_names()
            .clone()
    }

    /// Build the provider schema set from the current exposed view.
    pub fn provider_tool_specs(&self) -> Vec<AgentTool> {
        self.sync_tool_authorization();
        let visible = self.visible_tool_names();
        let mut specs: Vec<AgentTool> = self
            .builtin_and_mcp_specs()
            .into_iter()
            .filter(|tool| visible.contains(&tool.name))
            .collect();
        let mut known: std::collections::BTreeSet<String> =
            specs.iter().map(|tool| tool.name.clone()).collect();

        if let Some(runtime) = &self.runtime {
            for capability in runtime.capability_registry.list() {
                if !visible.contains(&capability.name)
                    || known.contains(&capability.name)
                    || capability.schema.is_none()
                {
                    continue;
                }
                let Some(parameters) = capability.schema.clone() else {
                    continue;
                };
                known.insert(capability.name.clone());
                specs.push(AgentTool {
                    name: capability.name,
                    description: capability.description,
                    parameters,
                });
            }
        }

        specs
    }

    pub fn provider_tool_schema_identity(&self) -> String {
        runtime::compute_schema_hash(&self.provider_tool_schema_value())
    }

    fn provider_tool_schema_value(&self) -> Value {
        Value::Array(
            self.provider_tool_specs()
                .into_iter()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    })
                })
                .collect(),
        )
    }

    /// Account for the normal/root provider request and select optional request-local context.
    pub fn root_context_budget_report(&self, budget: u64) -> ContextBudgetReport {
        let mut account = RootContextAccount::default();
        account.add(
            "system_prompt",
            self.system_prompt.clone(),
            true,
            ContextPriority::Mandatory,
        );
        if let Some(suffix) = self.provider_system_prompt_suffix.as_ref() {
            account.add(
                "provider_system_prompt_suffix",
                suffix.clone(),
                false,
                ContextPriority::Mandatory,
            );
        }
        for file in &self.context_files {
            account.add(
                format!("repository_instruction::{}", file.path.display()),
                file.body.clone(),
                true,
                ContextPriority::Mandatory,
            );
        }
        account.add(
            "provider_tool_schemas",
            serde_json::to_string(&self.provider_tool_schema_value()).unwrap_or_default(),
            true,
            ContextPriority::Mandatory,
        );
        if let Some(contract) = self
            .tool_context
            .active_contract
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            account.add(
                "active_task_contract",
                serde_json::to_string(contract).unwrap_or_default(),
                false,
                ContextPriority::Mandatory,
            );
        }
        if let Some(plan) = self.plan_provider_context() {
            account.add("living_plan", plan, false, ContextPriority::Mandatory);
        }

        // Current extension/memory evidence gets first claim on Important space.
        for (index, message) in self.ephemeral_context.iter().enumerate() {
            account.add(
                format!("ephemeral_context::{index}"),
                serde_json::to_string(message).unwrap_or_default(),
                false,
                ContextPriority::Important,
            );
        }

        let latest_user = self
            .messages
            .iter()
            .rposition(|message| message.role == "user");
        for (index, message) in self.messages.iter().enumerate() {
            let priority = if Some(index) == latest_user {
                ContextPriority::Mandatory
            } else {
                ContextPriority::Important
            };
            account.add(
                format!("conversation::{index}"),
                serde_json::to_string(message).unwrap_or_default(),
                false,
                priority,
            );
        }
        for skill in &self.skills {
            account.add(
                format!("skill::{}", skill.name),
                skill.body.clone(),
                true,
                ContextPriority::Deferred,
            );
        }
        for template in &self.templates {
            account.add(
                format!("prompt_template::{}", template.name),
                template.body.clone(),
                true,
                ContextPriority::Deferred,
            );
        }
        account.report_for_budget(budget)
    }

    pub fn select_root_context(&self, budget: u64) -> SelectedRootContext {
        let report = self.root_context_budget_report(budget);
        let selected_sources = report
            .contributions
            .iter()
            .filter(|entry| entry.selected)
            .map(|entry| entry.source.as_str())
            .collect::<std::collections::HashSet<_>>();
        let ephemeral_messages = self
            .ephemeral_context
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                selected_sources.contains(format!("ephemeral_context::{index}").as_str())
            })
            .map(|(_, message)| message.clone())
            .collect();
        SelectedRootContext {
            report,
            repository_files: self.context_files.clone(),
            ephemeral_messages,
        }
    }

    pub fn apply_extension_tools(&mut self, names: &[String]) {
        for name in names {
            if !self.tool_registry.contains(name) {
                self.tool_registry.push(name.clone());
            }
            if !self.tools.contains(name) {
                self.tools.push(name.clone());
            }
        }
        self.sync_tool_authorization();
    }

    /// TS `setActiveToolsByName` — only registry names are enabled; unknown names ignored.
    pub fn set_active_tools_by_name(&mut self, names: &[String]) {
        self.tools = names
            .iter()
            .filter(|name| self.tool_registry.iter().any(|known| known == *name))
            .cloned()
            .collect();
        self.expose_active_tools();
    }

    pub fn fold_context(
        &mut self,
        reason: runtime::context_vm::FoldReason,
        custom_instructions: Option<&str>,
    ) -> Result<runtime::ContextRoot, String> {
        let Some(runtime) = &self.runtime else {
            return Err("context VM runtime is unavailable".into());
        };
        let events = self.context_vm_events_for_runtime();
        let parent = runtime
            .context_vm
            .load_state_from_root()
            .unwrap_or_default();
        let proposal = self.summarizer.as_ref().and_then(|summarizer| {
            let request = runtime::context_vm::fold_request(
                &parent,
                &events,
                custom_instructions,
                self.context_window,
                &self.provider,
                &self.model_id,
            )?;
            let response = summarizer.summarize(&request).ok()?;
            if compaction::get_summarization_failure(&response, "context fold").is_some() {
                return None;
            }
            runtime::context_vm::parse_checkpoint_proposal(&response.text).ok()
        });
        let root = runtime
            .context_vm
            .fold_with_proposal(reason, &events, proposal)?;
        let prefix_digest = self
            .prepared_context_image()
            .map(|image| image.prefix_digest.clone())
            .unwrap_or_default();
        let (before_tokens, after_tokens) = runtime.context_vm.last_fold_tokens().unwrap_or((0, 0));
        if let Some(session) = &mut self.session {
            let seq = session
                .entries
                .iter()
                .map(|entry| entry.seq)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let entry = runtime::context_vm::context_checkpoint_entry(
                &root,
                events.iter().map(|event| event.seq).max().unwrap_or(0),
                &prefix_digest,
                session.leaf_id.clone(),
                seq,
            );
            session
                .append_entry(entry)
                .map_err(|error| format!("context checkpoint persistence failed: {error}"))?;
        }
        runtime.emit_observe(crate::RuntimeEvent::ContextVmFolded {
            epoch: root.epoch,
            reason: reason.as_str().into(),
            checkpoint_id: root
                .checkpoint
                .as_ref()
                .map(|page| page.id.clone())
                .unwrap_or_default(),
            before_tokens,
            after_tokens,
        });
        Ok(root)
    }

    pub fn context_vm_cache_affinity(&self) -> Option<String> {
        if self.context_vm_mode() != ContextVmMode::Active {
            return None;
        }
        self.runtime
            .as_ref()
            .map(|runtime| runtime.context_vm.cache_affinity())
    }

    pub fn compact(&mut self, custom_instructions: Option<&str>) -> CompactionResult {
        if self.context_vm_mode() == ContextVmMode::Active {
            self.is_compacting = true;
            let estimated_before = self.estimated_context_tokens();
            if let Some(runtime) = &self.runtime {
                runtime.emit_observe(crate::RuntimeEvent::PreCompact {
                    estimated_tokens: estimated_before,
                });
            }
            let fold =
                self.fold_context(runtime::context_vm::FoldReason::Manual, custom_instructions);
            let estimated_after = self.estimated_context_tokens();
            if let Some(runtime) = &self.runtime {
                runtime.emit_observe(crate::RuntimeEvent::PostCompact {
                    before_tokens: estimated_before,
                    after_tokens: estimated_after,
                });
            }
            self.is_compacting = false;
            return match fold {
                Ok(root) => CompactionResult {
                    summary: format!("Context VM epoch {} checkpointed", root.epoch),
                    messages: self.messages.clone(),
                    compacted: true,
                    details: CompactionDetails::default(),
                    first_kept_entry_id: String::new(),
                    tokens_before: estimated_before,
                    tokens_after: estimated_after,
                    usage: None,
                },
                Err(error) => CompactionResult {
                    summary: error,
                    messages: self.messages.clone(),
                    compacted: false,
                    details: CompactionDetails::default(),
                    first_kept_entry_id: String::new(),
                    tokens_before: estimated_before,
                    tokens_after: estimated_before,
                    usage: None,
                },
            };
        }
        self.is_compacting = true;
        let estimated_before = self.estimated_context_tokens();
        if let Some(runtime) = &self.runtime {
            runtime.emit_observe(crate::RuntimeEvent::PreCompact {
                estimated_tokens: estimated_before,
            });
        }
        let previous_summary = self.session.as_ref().and_then(|session| {
            session
                .entries
                .iter()
                .rev()
                .find(|entry| entry.entry_type == "compaction")
                .and_then(|entry| {
                    entry
                        .extra
                        .get("summary")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                })
        });
        let provider = self.provider.clone();
        let model_id = self.model_id.clone();
        let bound = self.summarizer.clone().map(|inner| {
            Summarizer::new(move |request| {
                let mut request = request.clone();
                if request.provider.is_empty() {
                    request.provider = provider.clone();
                }
                if request.model_id.is_empty() {
                    request.model_id = model_id.clone();
                }
                inner.summarize(&request)
            })
        });
        let mut result = compact_messages_with_options(
            &self.messages,
            custom_instructions,
            self.compaction.keep_recent_tokens,
            self.compaction.reserve_tokens,
            previous_summary.as_deref(),
            bound.as_ref(),
        );
        if result.compacted {
            if let Some(session) = &mut self.session {
                let first_kept = first_kept_entry_id(session, &self.messages, &result.messages);
                result.first_kept_entry_id = first_kept.clone();
                let mut extra = serde_json::Map::new();
                extra.insert("summary".into(), serde_json::json!(result.summary));
                extra.insert("firstKeptEntryId".into(), serde_json::json!(first_kept));
                extra.insert(
                    "details".into(),
                    serde_json::to_value(&result.details).unwrap_or_default(),
                );
                extra.insert("fromHook".into(), serde_json::json!(false));
                if let Some(usage) = &result.usage {
                    extra.insert(
                        "usage".into(),
                        serde_json::to_value(usage).unwrap_or_default(),
                    );
                }
                let _ = session.append_entry(SessionEntry {
                    id: String::new(),
                    entry_type: "compaction".into(),
                    parent_id: session.leaf_id.clone(),
                    seq: 0,
                    timestamp: 0,
                    message: None,
                    custom_type: None,
                    extra,
                });
            }
        }
        if result.compacted {
            self.messages = result.messages.clone();
        }
        let estimated_after = self.estimated_context_tokens();
        if let Some(runtime) = &self.runtime {
            runtime.emit_observe(crate::RuntimeEvent::PostCompact {
                before_tokens: estimated_before,
                after_tokens: estimated_after,
            });
        }
        self.is_compacting = false;
        result
    }

    pub fn abort(&mut self) {
        self.aborted = true;
    }

    /// True when the local flag, host signal, or active runtime is cancelled.
    pub fn abort_requested(&self) -> bool {
        self.aborted
            || self
                .abort_signal
                .as_ref()
                .is_some_and(|signal| signal.load(std::sync::atomic::Ordering::Relaxed))
            || self
                .runtime
                .as_ref()
                .is_some_and(|runtime| runtime.cancellation_token.is_cancelled())
    }

    /// Revoke ephemeral consent when an idle host replaces or reloads a session.
    /// Keep the configured policy and transport callbacks for the next request.
    pub fn reset_session_approvals(&mut self) {
        let mut policy = self
            .permissions
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        policy.session_allow.clear();
        self.approval_registry.revoke_all();
    }

    /// Activate durable task state before replacing the active session.
    /// Reloading the live source preserves its writer lease and worker handles.
    pub fn load_from_session(&mut self, mut session: JsonlSession) -> Result<(), String> {
        let source = std::fs::canonicalize(&session.path).map_err(|error| {
            format!("Runtime recovery required: session source could not be resolved: {error}")
        })?;
        let current = self.runtime_for_session().filter(|runtime| {
            self.runtime_session
                .as_ref()
                .is_some_and(|(path, id, _)| *path == source && *id == session.header.id)
                && (runtime.task_registry.is_durable() || runtime.parent_agent_id.is_some())
        });
        let worker = self
            .runtime
            .as_ref()
            .filter(|runtime| runtime.parent_agent_id.is_some());
        if worker.is_some() && self.session.is_some() && current.is_none() {
            return Err(
                "Runtime recovery required: a worker cannot switch its bound conversation".into(),
            );
        }
        let session_changed = current.is_none();
        let candidate = match (current, worker) {
            (Some(runtime), _) => runtime.clone(),
            (None, Some(worker)) => {
                runtime::session::restore_worker_session_runtime(worker.clone(), &mut session)
                    .map_err(|error| format!("Runtime recovery required: {error}"))?
            }
            (None, None) => runtime::session::restore_session_runtime(
                RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new()),
                &session,
            )
            .map_err(|error| format!("Runtime recovery required: {error}"))?,
        };
        let ledger_path = session.path.with_extension("tool-ledger.json");
        let legacy_observations =
            ToolCallLedger::legacy_observations_from_path(&ledger_path, &session.header.id)
                .map_err(|error| format!("Runtime recovery required: {error}"))?;
        let candidate_ledger = ToolCallLedger::load_bound(&ledger_path, &session.header.id)
            .map_err(|error| format!("Runtime recovery required: {error}"))?;
        if let Some(operations) = candidate.operations.as_ref() {
            operations
                .dispatcher()
                .journal()
                .import_legacy_observations(&legacy_observations)
                .map_err(|error| {
                    format!(
                        "Runtime recovery required: legacy operation observations could not be imported: {error}"
                    )
                })?;
        }
        let messages = messages_from_session(&session);
        if session_changed {
            if let Some(processes) = &self.tool_context.processes {
                self.tool_context.processes = Some(processes.new_session()?);
            }
        }
        self.last_real_user_request = last_real_user_request_from_messages(&messages);
        self.reset_session_approvals();
        self.messages = messages;
        self.pending_prompt_messages.clear();
        self.session = Some(session);
        self.tool_ledger = Arc::new(std::sync::Mutex::new(candidate_ledger));
        self.set_runtime(candidate);
        self.restore_living_plan();
        let _ = self.restore_prompt_session();
        Ok(())
    }

    /// Persist current prompt identity as a custom session entry if an active session exists.
    pub fn persist_prompt_session(&mut self) -> Result<(), String> {
        self.ensure_session_persistence()?;
        let Some(record) = PromptSessionRecord::from_session_state(&self.prompt_session) else {
            return Ok(());
        };
        let data = serde_json::to_value(&record).map_err(|e| e.to_string())?;
        let Some(session) = &mut self.session else {
            return Ok(());
        };
        if session.entries.iter().rev().any(|entry| {
            entry.entry_type == "custom"
                && entry.custom_type.as_deref() == Some(PROMPT_SESSION_ENTRY_TYPE)
                && entry.extra.get("data") == Some(&data)
        }) {
            return Ok(());
        }
        let mut extra = serde_json::Map::new();
        extra.insert("data".into(), data);
        session
            .append_entry(davinci_session::SessionEntry {
                id: String::new(),
                entry_type: "custom".into(),
                parent_id: None,
                seq: 0,
                timestamp: 0,
                message: None,
                custom_type: Some(PROMPT_SESSION_ENTRY_TYPE.into()),
                extra,
            })
            .map_err(|error| format!("Session recovery required: {error}"))
    }

    /// Restore prompt session state and identity from persisted custom session entries.
    /// Returns Ok(true) if a prompt session record was found and restored, Ok(false) otherwise.
    pub fn restore_prompt_session(&mut self) -> Result<bool, String> {
        let Some(session) = &self.session else {
            return Ok(false);
        };
        let record = session.entries.iter().rev().find_map(|entry| {
            if entry.entry_type == "custom"
                && entry.custom_type.as_deref() == Some(PROMPT_SESSION_ENTRY_TYPE)
            {
                if let Some(data) = entry.extra.get("data") {
                    serde_json::from_value::<PromptSessionRecord>(data.clone()).ok()
                } else {
                    serde_json::from_value::<PromptSessionRecord>(serde_json::Value::Object(
                        entry.extra.clone(),
                    ))
                    .ok()
                }
            } else {
                None
            }
        });

        if record.is_none() && self.prompt_session.is_custom() {
            return Ok(false);
        }

        let profile_override = if record.is_none() && self.prompt_session.is_builtin() {
            self.prompt_session.profile()
        } else {
            None
        };

        let (mut resumed_session, _) =
            resolve_resume_prompt_session(record.as_ref(), profile_override);
        resumed_session.append_text = self.prompt_session.append_text.clone();
        let ctx = PromptContext {
            provider: &self.provider,
            model_id: &self.model_id,
            permission_mode: self.permission_mode(),
            plan_active: self.is_plan_mode(),
        };
        let composed = resumed_session.render_and_record(&ctx);
        let diag = record
            .as_ref()
            .and_then(|record| prompt::prompt_transition_diagnostic(record, &composed.manifest));
        self.system_prompt = composed.text;
        self.prompt_manifest = Some(composed.manifest);
        resumed_session.transition_diagnostic = diag;
        self.prompt_session = resumed_session;

        Ok(record.is_some())
    }

    /// Navigate the session tree. When `summarize` is true, generates a branch
    /// summary of the abandoned path and appends a `branch_summary` entry.
    pub fn navigate_tree_entry(
        &mut self,
        target_id: &str,
        summarize: bool,
        custom_instructions: Option<&str>,
        replace_instructions: bool,
        reserve_tokens: u64,
    ) -> Result<TreeNavigateResult, String> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| "No session to navigate".to_string())?;
        let target = session
            .entries
            .iter()
            .find(|entry| entry.id == target_id)
            .cloned()
            .ok_or_else(|| format!("Entry {target_id} not found"))?;
        let old_leaf = session.leaf_id.clone();
        let (collected, _) =
            collect_entries_for_branch_summary(&session.entries, old_leaf.as_deref(), target_id);
        let (new_leaf, editor_text) = navigation_target(&target);
        let mut summary_text = None;
        if summarize && !collected.is_empty() {
            let provider = self.provider.clone();
            let model_id = self.model_id.clone();
            let bound = self.summarizer.clone().map(|inner| {
                Summarizer::new(move |request| {
                    let mut request = request.clone();
                    if request.provider.is_empty() {
                        request.provider = provider.clone();
                    }
                    if request.model_id.is_empty() {
                        request.model_id = model_id.clone();
                    }
                    inner.summarize(&request)
                })
            });
            let result = generate_branch_summary(
                &collected,
                self.context_window,
                reserve_tokens,
                custom_instructions,
                replace_instructions,
                bound.as_ref(),
            );
            if result.aborted {
                return Ok(TreeNavigateResult {
                    cancelled: true,
                    editor_text: None,
                    summary: None,
                });
            }
            if let Some(error) = result.error {
                return Err(error);
            }
            if let Some(summary) = result.summary.clone() {
                let details = serde_json::to_value(&result.details).unwrap_or_default();
                let usage = result
                    .usage
                    .as_ref()
                    .and_then(|usage| serde_json::to_value(usage).ok());
                self.session
                    .as_mut()
                    .ok_or_else(|| "No session to navigate".to_string())?
                    .branch_with_summary(new_leaf.clone(), &summary, details, usage, false)
                    .map_err(|err| err.to_string())?;
                summary_text = Some(summary);
            }
        } else if let Some(session) = &mut self.session {
            session.set_leaf(new_leaf);
        }
        if let Some(session) = &self.session {
            self.messages = messages_from_session(session);
        }
        self.restore_plan()?;
        Ok(TreeNavigateResult {
            cancelled: false,
            editor_text,
            summary: summary_text,
        })
    }

    pub fn session_tree(&self) -> Vec<serde_json::Value> {
        self.session
            .as_ref()
            .map(|session| {
                session
                    .entries
                    .iter()
                    .map(|entry| {
                        serde_json::json!({
                            "id": entry.id,
                            "parentId": entry.parent_id,
                            "type": entry.entry_type,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn entries_since(&self, since: Option<&str>) -> Vec<davinci_session::SessionEntry> {
        let Some(session) = &self.session else {
            return Vec::new();
        };
        match since {
            Some(id) => session
                .entries
                .iter()
                .skip_while(|entry| entry.id != id)
                .skip(1)
                .cloned()
                .collect(),
            None => session.entries.clone(),
        }
    }
}

fn command_package_target(command: &str) -> Option<String> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        if matches!(*part, "-p" | "--package") {
            return parts
                .get(index + 1)
                .map(|value| value.trim_matches('"').to_string());
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
        if !changed_packages.is_empty()
            && changed_packages.iter().all(|changed| changed == &package)
        {
            return (VerificationCoverage::Targeted, targets);
        }
        return (VerificationCoverage::Unrelated, targets);
    }
    (VerificationCoverage::Unknown, Vec::new())
}

#[derive(Debug, Clone, Default)]
pub struct TreeNavigateResult {
    pub cancelled: bool,
    pub editor_text: Option<String>,
    pub summary: Option<String>,
}

pub(crate) fn custom_message_from_session_entry(entry: &SessionEntry) -> Option<ChatMessage> {
    let content = entry
        .extra
        .get("content")
        .cloned()
        .or_else(|| entry.message.clone())
        .unwrap_or_else(|| serde_json::json!([]));
    let content = match &content {
        Value::String(text) => vec![MessageContent::Text { text: text.clone() }],
        Value::Array(_) => serde_json::from_value(content).unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut extra = serde_json::Map::new();
    if let Some(custom_type) = &entry.custom_type {
        extra.insert("customType".into(), Value::String(custom_type.clone()));
    }
    for key in ["display", "details"] {
        if let Some(value) = entry.extra.get(key) {
            extra.insert(key.to_string(), value.clone());
        }
    }
    extra.insert("timestamp".into(), serde_json::json!(entry.timestamp));
    Some(ChatMessage {
        role: "custom".into(),
        content,
        extra,
        ..ChatMessage::default()
    })
}

fn entry_to_chat(entry: &SessionEntry) -> Option<ChatMessage> {
    match entry.entry_type.as_str() {
        "compaction" => {
            let summary = entry.extra.get("summary")?.as_str()?;
            Some(compaction_context_message(summary))
        }
        "branch_summary" => {
            let summary = entry.extra.get("summary")?.as_str()?;
            Some(branch_summary_context_message(summary))
        }
        "custom_message" => custom_message_from_session_entry(entry),
        "message" => {
            let message = entry.message.as_ref()?;
            serde_json::from_value(message.clone()).ok()
        }
        _ => None,
    }
}

fn messages_from_session(session: &JsonlSession) -> Vec<ChatMessage> {
    davinci_session::build_context_entries(&session.entries, session.leaf_id.as_deref())
        .into_iter()
        .filter_map(entry_to_chat)
        .collect()
}

fn last_real_user_request_from_messages(messages: &[ChatMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        if message.role != "user"
            || message.extra.get(REAL_USER_ORIGIN_FIELD) != Some(&Value::Bool(true))
        {
            return None;
        }
        message.content.iter().find_map(|content| match content {
            davinci_ai::MessageContent::Text { text } => Some(text.clone()),
            _ => None,
        })
    })
}

fn first_kept_entry_id(
    session: &JsonlSession,
    before: &[ChatMessage],
    after: &[ChatMessage],
) -> String {
    let kept = after.len().saturating_sub(1);
    let first_kept_index = before.len().saturating_sub(kept);
    let mut message_index = 0usize;
    for entry in &session.entries {
        if entry.entry_type == "compaction" {
            continue;
        }
        if entry_to_chat(entry).is_none() {
            continue;
        }
        if message_index == first_kept_index {
            return entry.id.clone();
        }
        message_index += 1;
    }
    session
        .entries
        .last()
        .map(|entry| entry.id.clone())
        .unwrap_or_default()
}

/// Provider complete callback output. `From<AssistantMessage>` keeps existing closures working.
#[derive(Debug, Clone)]
pub struct CompleteOutput {
    pub message: AssistantMessage,
    pub stream_events: Option<Vec<AssistantMessageEvent>>,
    pub native_responses_resume: Option<davinci_ai::NativeResponsesResumeRecord>,
    /// The closure already sent `MessageStart` and one `MessageUpdate` per
    /// stream event through `Agent::emit_live` while the provider was
    /// answering. The loop then only records them.
    pub streamed_live: bool,
}

impl From<AssistantMessage> for CompleteOutput {
    fn from(message: AssistantMessage) -> Self {
        Self {
            message,
            stream_events: None,
            native_responses_resume: None,
            streamed_live: false,
        }
    }
}

/// The `customType` of the user message that tells the model a background
/// job finished.
pub const JOB_NOTICE_TYPE: &str = "backgroundJob";
const REAL_USER_ORIGIN_FIELD: &str = "davinciRealUserOrigin";

pub fn default_system_prompt() -> String {
    prompt::compose_legacy_default().text
}

/// The orchestration rules every pi prompt carries: they are what turns a
/// thirty-turn investigation into a six-turn one. Runtime and prompt have
/// to agree — the scheduler overlaps independent calls (`scheduler.rs`),
/// `batch` hides several operations behind one boundary, `agent` fans out
/// workers — and this is the prompt's half of that agreement.
pub const TOOL_USE_STRATEGY: &str = prompt::tool_strategy::TOOL_USE_STRATEGY;

pub fn new_message_id() -> String {
    Uuid::new_v4().to_string()
}

/// A session entry for one chat message, carrying the message's `extra`
/// fields (`customType: backgroundJob`, `jobId`) so a resumed session reads
/// a job notice back as a job notice and not as something the user typed.
pub fn chat_entry(
    role: &str,
    content: Value,
    extra: &serde_json::Map<String, Value>,
) -> SessionEntry {
    let mut entry = SessionEntry::message(role, content);
    if let Some(Value::Object(message)) = &mut entry.message {
        for (key, value) in extra {
            message.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    entry
}

/// Measure deferred root schemas against the same agent with every authorized
/// schema exposed. The returned units are serialized provider-schema bytes.
pub fn deferred_root_schema_ablation() -> Result<(bool, bool, u64, u64), String> {
    let agent = Agent::new("offline-root-schema-ablation");
    let deferred_names = agent.visible_tool_names();
    let deferred = serde_json::to_vec(&agent.provider_tool_specs()).map_err(|e| e.to_string())?;
    agent.expose_active_tools();
    let full_names = agent.visible_tool_names();
    let full = serde_json::to_vec(&agent.provider_tool_specs()).map_err(|e| e.to_string())?;
    Ok((
        !full.is_empty(),
        !deferred.is_empty() && deferred_names.is_subset(&full_names),
        full.len() as u64,
        deferred.len() as u64,
    ))
}

/// Measure query-driven capability exposure against exposing every authorized
/// schema. The candidate is correct only if the queried schema becomes visible.
pub fn capability_toolbox_ablation() -> Result<(bool, bool, u64, u64), String> {
    let mut agent = Agent::new("offline-capability-ablation");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    let query = execute_tool_with(
        Path::new("."),
        "tool_search",
        &serde_json::json!({"query": "web_search"}),
        &agent.tool_context,
    )
    .map_err(|error| error.to_string())?;
    let activated = query
        .details
        .as_ref()
        .and_then(|details| details.get("activated"))
        .and_then(Value::as_array)
        .is_some_and(|names| names.iter().any(|name| name == "web_search"));
    let queried = serde_json::to_vec(&agent.provider_tool_specs()).map_err(|e| e.to_string())?;
    agent.expose_active_tools();
    let full = serde_json::to_vec(&agent.provider_tool_specs()).map_err(|e| e.to_string())?;
    Ok((
        !full.is_empty(),
        activated && agent.is_tool_visible("web_search"),
        full.len() as u64,
        queried.len() as u64,
    ))
}

#[cfg(test)]
mod worker_session_tests;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
