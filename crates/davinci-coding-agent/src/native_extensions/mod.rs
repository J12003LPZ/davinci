//! Native Rust ports of the bundled pi extensions.

pub mod browser;
pub mod build_intelligence;
pub mod change_impact;
pub mod content_router;
pub mod ecosystem;
pub mod engineering_snapshot;
pub mod git_intelligence;
pub mod graph;
pub mod language_intelligence;
pub mod learning;
pub mod memory_page;
pub mod package_intelligence;
pub mod repo_intelligence;
pub mod security_scan;
pub mod test_impact;
pub mod token_governor;
pub mod vector_memory;
pub mod verification_planner;
pub mod workspace_metadata;
pub mod workspace_snapshot;

#[allow(unused_imports)]
pub use content_router::*;
#[allow(unused_imports)]
pub use ecosystem::*;
pub use graph::*;
pub use learning::*;
#[allow(unused_imports)]
pub use security_scan::{
    enumerate_scope, normalize_relative_path, render_report, render_sarif, tool_spec,
    FindingSeverity, ScanConfig, ScanStatus, SecurityArtifactSeal, SecurityArtifactStore,
    SecurityCandidate, SecurityCoverage, SecurityFinding, SecurityScan, SecurityScanConfig,
    SecurityScanController, SecurityScanManifest, SecurityVerifyRequest,
};
pub use token_governor::*;
pub use vector_memory::*;

use crate::native_tools::visual_snapshot::{
    visual_snapshot_tool_spec, VisualSnapshotBackend, VisualSnapshotHost, VISUAL_SNAPSHOT_TOOL,
};
use davinci_agent::{ToolError, ToolResult};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Shared adapter for Context VM artifact retrieval. Keeping this at the
/// native-extension boundary lets callers use the governor's existing
/// retrieve_output store rather than duplicating tool-result bytes.
// Public library API; this module is also compiled privately into the binary.
#[cfg_attr(not(test), allow(dead_code))]
pub fn retrieve_context_artifact(
    governor: &mut TokenGovernor,
    uri: &str,
) -> Result<String, ToolError> {
    governor.retrieve_artifact(uri)
}

pub const NATIVE_TOOLS: &[&str] = &[
    "browser_open",
    "browser_snapshot",
    "browser_click",
    "browser_type",
    "browser_select",
    "browser_console",
    "browser_network",
    "browser_accessibility",
    "browser_screenshot",
    "browser_close",
    "test_related",
    "test_impacted",
    "test_plan",
    "package_info",
    "package_exports",
    "package_symbol",
    "package_dependents",
    "package_why",
    "workspace_packages",
    "build_targets",
    "build_dependencies",
    "build_affected",
    "build_command",
    "git_symbol_history",
    "git_related_commits",
    "git_changed_symbols",
    "git_branch_diff",
    "git_blame_symbol",
    "git_commit_context",
    "git_conflict_explain",
    "impact_analyze",
    "verification_plan",
    "workspace_checkpoint",
    "workspace_diff",
    "workspace_restore",
    "repo_map",
    "symbol_search",
    "file_symbols",
    "file_dependencies",
    "symbol_relationships",
    "related_files",
    "code_query",
    "lsp_definition",
    "lsp_references",
    "lsp_hover",
    "lsp_document_symbols",
    "lsp_workspace_symbols",
    "lsp_implementations",
    "lsp_type_definition",
    "lsp_diagnostics",
    "memory_search",
    "retrieve_output",
    "graph_status",
    "graph_run",
    "sec_scan_start",
    "sec_scan_context",
    "sec_scan_progress",
    "sec_scan_draft",
    "sec_scan_complete",
    "sec_scan_cancel",
    "sec_candidates_record",
    "sec_candidates_list",
    "sec_candidates_validate",
    "sec_candidates_attack_path",
    "sec_scope_files",
    "sec_policy_resolve",
    "sec_tracking_validate",
    "sec_deep_scan",
    "skill_list",
    "skill_view",
    "skill_manage",
];

pub const NATIVE_COMMANDS: &[&str] = &[
    "repo-index-status",
    "cache-status",
    "test-impact-status",
    "package-status",
    "build-status",
    "git-status",
    "impact-status",
    "verification-status",
    "workspace-status",
    "lsp-status",
    "memory-status",
    "memory-search",
    "memory-reindex",
    "memory-clear",
    "memory-page",
    "governor-status",
    "governor-reset",
    "graph",
    // Internal graph lifecycle operations. Deliberately omitted from
    // `command_specs`: the terminal exposes one context-sensitive `/graph`.
    "graph-resume",
    "graph-status",
    "graph-view",
    "graph-abort",
    "security-scan",
    "sec-resume",
    "sec-status",
    "sec-report",
    "sec-abort",
    "learning-status",
    "learning-pending",
    "learning-approve",
    "learning-reject",
    "skill-list",
    "skill-view",
    "hook-status",
];

/// Metadata shared by the interactive and RPC command discovery surfaces.
/// Native commands are Rust implementations, but they should look like any
/// other invocable pi command to clients and autocomplete.
pub fn command_specs() -> Vec<(&'static str, &'static str, Option<&'static str>)> {
    vec![
        (
            "repo-index-status",
            "Show structural repository index counts, cache and refresh state.",
            None,
        ),
        (
            "cache-status",
            "Show local cache hits separately from provider read/write usage and transport reuse.",
            None,
        ),
        (
            "test-impact-status",
            "Show test-impact availability and repository observation state.",
            None,
        ),
        (
            "git-status",
            "Show bounded Git intelligence status and repository history availability.",
            None,
        ),
        (
            "impact-status",
            "Show change-impact analysis status and dependency coverage.",
            None,
        ),
        (
            "verification-status",
            "Show bounded verification-planner limits and planning telemetry.",
            None,
        ),
        (
            "workspace-status",
            "Show bounded workspace checkpoint limits and restore telemetry.",
            None,
        ),
        (
            "package-status",
            "Show installed dependency resolution status, lockfile kinds, and cache telemetry.",
            None,
        ),
        (
            "build-status",
            "Show repository build intelligence status, detected task runners, and cache telemetry.",
            None,
        ),
        (
            "lsp-status",
            "Show native TypeScript/JavaScript language-server sessions and availability.",
            None,
        ),
        (
            "security-scan",
            "Start an experimental source-grounded security review.",
            Some("[path] [--mode quick|standard|deep] [--format terminal|json|sarif]"),
        ),
        (
            "sec-resume",
            "Resume an interrupted security review.",
            Some("<scanId>"),
        ),
        (
            "sec-status",
            "Show the active security review status.",
            Some("[scanId]"),
        ),
        (
            "sec-report",
            "Show the current security review report.",
            Some("[scanId]"),
        ),
        (
            "sec-abort",
            "Cancel the active security review.",
            Some("[scanId]"),
        ),
        (
            "memory-status",
            "Show vector-memory health and record counts.",
            None,
        ),
        (
            "memory-search",
            "Search durable vector and lexical memory.",
            Some("<query>"),
        ),
        (
            "memory-reindex",
            "Reload vector memory, repair record ids and embed records missing a vector.",
            None,
        ),
        ("memory-clear", "Clear local vector-memory records.", None),
        (
            "memory-page",
            "Build and open a page showing vector-memory connection, records and a live retrieval check.",
            Some("[--no-open] [query]"),
        ),
        (
            "governor-status",
            "Show token-governor compression and deduplication counters.",
            None,
        ),
        (
            "governor-reset",
            "Reset token-governor session state.",
            None,
        ),
        (
            "graph",
            "Start, watch, save, or run an execution graph.",
            Some("[goal|save <name>|run <name>] [--simple|--complex] [--dry-run]"),
        ),
        (
            "learning-status",
            "Show learning subsystem status and statistics.",
            None,
        ),
        (
            "learning-pending",
            "List candidates awaiting user approval.",
            None,
        ),
        (
            "learning-approve",
            "Approve a learning candidate to activate it as a skill.",
            Some("<candidateId|all>"),
        ),
        (
            "learning-reject",
            "Reject a learning candidate.",
            Some("<candidateId|all> [reason]"),
        ),
        (
            "skill-list",
            "List compact reusable skill descriptors.",
            Some("[query]"),
        ),
        (
            "skill-view",
            "Read the full content of a skill.",
            Some("<name> [file]"),
        ),
        (
            "hook-status",
            "Show deterministic hook and policy engine diagnostics, trust state, and telemetry.",
            None,
        ),
    ]
}

pub fn native_invocable_commands() -> Vec<Value> {
    command_specs()
        .into_iter()
        .map(|(name, description, argument_hint)| {
            let mut command = json!({
                "name": name,
                "description": description,
                "source": "native",
                "sourceInfo": {"runtime": "rust"},
            });
            if let Some(argument_hint) = argument_hint {
                command["argumentHint"] = Value::String(argument_hint.to_string());
            }
            command
        })
        .collect()
}

/// Set inside a graph worker child process, absent in an ordinary session.
/// It gates the `graph_submit` tool and the per-role tool and shell policy.
pub fn graph_worker_context() -> Option<GraphWorkerContext> {
    GraphWorkerContext::from_env()
}

pub type SharedVectorMemory = Arc<Mutex<VectorMemory>>;
pub type SharedTokenGovernor = Arc<Mutex<TokenGovernor>>;

#[derive(Debug, Clone, Default)]
pub struct NativeExtensionHost {
    pub browser: browser::BrowserController,
    pub test_impact: test_impact::TestImpact,
    pub engineering: engineering_snapshot::EngineeringSnapshots,
    pub package_intelligence: package_intelligence::PackageIntelligence,
    pub build_intelligence: build_intelligence::BuildIntelligence,
    pub git_intelligence: git_intelligence::GitIntelligence,
    pub change_impact: change_impact::ChangeImpact,
    pub verification_planner: verification_planner::VerificationPlanner,
    pub workspace_snapshot: workspace_snapshot::WorkspaceSnapshot,
    pub repo_intelligence: repo_intelligence::RepoIntelligence,
    pub cache: davinci_agent::runtime::cache::CacheRuntime,
    pub language_intelligence: language_intelligence::LanguageIntelligence,
    pub governor: SharedTokenGovernor,
    pub memory: SharedVectorMemory,
    pub graph: GraphController,
    pub security: SecurityScanController,
    pub learning: LearningController,
    pub visual_snapshot: VisualSnapshotHost,
    /// Set by the native visual backend registration path when one exists.
    pub visual_verification_available: bool,
    pub cwd: std::path::PathBuf,
    agent_dir: std::path::PathBuf,
}

const CODEX_CREDITS_PER_USD: f64 = 25.0;

fn codex_credits_estimate(usd: f64) -> f64 {
    usd.max(0.0) * CODEX_CREDITS_PER_USD
}

impl NativeExtensionHost {
    pub fn new_with_agent_dir(
        session_key: impl Into<String>,
        cwd: &Path,
        agent_dir: Option<&Path>,
    ) -> Self {
        let session_key = session_key.into();
        let repo_agent_dir = agent_dir
            .map(Path::to_path_buf)
            .unwrap_or_else(davinci_session::default_agent_dir);
        let merged_settings = crate::settings::load_merged_settings(&repo_agent_dir, cwd);
        let cache = match agent_dir {
            Some(dir) => davinci_agent::runtime::cache::CacheRuntime::shared(
                merged_settings.cache.clone().unwrap_or_default(),
                dir.into(),
            ),
            None => davinci_agent::runtime::cache::CacheRuntime::default(),
        };
        let governor_config = agent_dir
            .map(|dir| TokenGovernorConfig::from_file(&dir.join("token-governor.json")))
            .unwrap_or_else(TokenGovernorConfig::from_env);
        let memory_config = agent_dir
            .map(|dir| VectorMemoryConfig::from_file(&dir.join("vector-memory.json")))
            .unwrap_or_else(VectorMemoryConfig::from_env);
        let governor_value = TokenGovernor::new(session_key.clone(), governor_config);
        let _ = governor_value.sweep_stale_outputs();
        let language_governor = governor_value.clone();
        let governor = Arc::new(Mutex::new(governor_value));
        let learning_config = if agent_dir.is_some() {
            merged_settings.learning.clone()
        } else {
            None
        };
        let learning = LearningController::new(cwd, agent_dir, learning_config);
        let memory = Arc::new(Mutex::new(VectorMemory::with_config(
            cwd.to_path_buf(),
            memory_config,
        )));
        let mut graph = GraphController::new(cwd.to_path_buf());
        graph.memory = Some(Arc::clone(&memory));
        graph.learning = Some(learning.clone());
        graph.governor = Some(Arc::clone(&governor));
        let visual_snapshot = VisualSnapshotHost::discover(cwd);

        let repo_config = merged_settings.repo_intelligence.clone().unwrap_or_default();
        let repo_intelligence =
            repo_intelligence::RepoIntelligence::new(cwd, &repo_agent_dir, repo_config);
        let engineering = engineering_snapshot::EngineeringSnapshots::default();
        let test_impact = test_impact::TestImpact::new(
            cwd,
            repo_intelligence.clone(),
            cache.clone(),
            merged_settings.test_impact.clone().unwrap_or_default(),
        )
        .with_snapshots(engineering.clone());
        let package_config = merged_settings.package_intelligence.clone().unwrap_or_default();
        let package_intelligence =
            package_intelligence::PackageIntelligence::new(cwd, cache.clone(), package_config);
        let build_config = merged_settings.build_intelligence.clone().unwrap_or_default();
        let build_intelligence =
            build_intelligence::BuildIntelligence::new(cwd, cache.clone(), build_config)
                .with_snapshots(engineering.clone());
        let git_config = merged_settings.git_intelligence.clone().unwrap_or_default();
        let git_intelligence =
            git_intelligence::GitIntelligence::new(cwd, cache.clone(), git_config);
        let language_config = if agent_dir.is_some() {
            merged_settings
                .language_intelligence
                .clone()
                .unwrap_or_default()
        } else {
            Default::default()
        };
        let language_intelligence =
            language_intelligence::LanguageIntelligence::new(cwd, language_config);
        language_intelligence.set_governor(language_governor);
        graph.language_intelligence = Some(language_intelligence.clone());
        let change_config = merged_settings.change_impact.clone().unwrap_or_default();
        let change_impact = change_impact::ChangeImpact::new(
            cwd,
            cache.clone(),
            change_config,
            repo_intelligence.clone(),
            test_impact.clone(),
            package_intelligence.clone(),
            build_intelligence.clone(),
            git_intelligence.clone(),
            language_intelligence.clone(),
        );
        let repo_intelligence = repo_intelligence.with_semantic_provider(Arc::new(
            change_impact::LanguageIntelligenceAdapter::new(language_intelligence.clone()),
        ));
        let verification_planner_config = merged_settings
            .verification_planner
            .clone()
            .unwrap_or_default();
        let verification_planner =
            verification_planner::VerificationPlanner::new(cwd, verification_planner_config)
                .with_snapshots(engineering.clone());
        let workspace_snapshot_config =
            merged_settings.workspace_snapshots.clone().unwrap_or_default();
        let workspace_snapshot =
            workspace_snapshot::WorkspaceSnapshot::new(cwd, workspace_snapshot_config);

        let browser_config = agent_dir
            .map(|dir| {
                let mut config = crate::settings::load_settings(dir)
                    .browser_verification
                    .unwrap_or_default();
                if merged_settings
                    .browser_verification
                    .as_ref()
                    .is_some_and(|settings| !settings.enabled)
                {
                    config.enabled = false;
                }
                config
            })
            .unwrap_or_default();

        Self {
            browser: browser::BrowserController::new(cwd, browser_config),
            test_impact,
            engineering,
            package_intelligence,
            build_intelligence,
            git_intelligence,
            change_impact,
            verification_planner,
            workspace_snapshot,
            repo_intelligence,
            cache,
            language_intelligence,
            governor,
            memory,
            graph,
            security: SecurityScanController::new(cwd.to_path_buf()),
            learning,
            visual_verification_available: visual_snapshot.is_available(),
            visual_snapshot,
            cwd: cwd.to_path_buf(),
            agent_dir: repo_agent_dir,
        }
    }

    pub fn visual_verification_available(&self) -> bool {
        self.visual_verification_available && self.visual_snapshot.is_available()
    }

    #[allow(dead_code)]
    pub fn register_visual_snapshot_backend(
        &mut self,
        backend: Arc<dyn VisualSnapshotBackend>,
        cwd: &Path,
    ) {
        self.visual_snapshot = VisualSnapshotHost::from_backend(backend, cwd);
        self.visual_verification_available = self.visual_snapshot.is_available();
    }

    pub fn has_tool(&self, name: &str) -> bool {
        NATIVE_TOOLS.contains(&name)
            || name == VISUAL_SNAPSHOT_TOOL && self.visual_verification_available()
            || name == GRAPH_SUBMIT_TOOL && graph_worker_context().is_some()
    }

    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = NATIVE_TOOLS
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        if self.visual_verification_available() {
            names.push(VISUAL_SNAPSHOT_TOOL.to_string());
        }
        if graph_worker_context().is_some() {
            names.push(GRAPH_SUBMIT_TOOL.to_string());
        }
        names
    }

    /// `state_hash` is only evaluated for the tools whose ledger needs it
    /// (it runs git), so ordinary reads and edits never pay for it.
    pub fn before_tool(
        &mut self,
        name: &str,
        args: &Value,
        state_hash: impl FnOnce() -> String,
    ) -> Option<String> {
        // Inside a graph worker, least privilege is enforced here as well as by
        // the child's --tools allowlist: "bash" is one tool whose danger lives
        // in its command text.
        if let Some(context) = graph_worker_context() {
            if let Some(reason) = context.block_reason(name, args) {
                return Some(reason);
            }
        }
        self.governor
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .before_tool(name, args, state_hash)
    }

    pub fn after_tool(&mut self, name: &str, args: &Value, result: ToolResult) -> ToolResult {
        if !matches!(name, "read" | "grep" | "find" | "ls")
            && !test_impact::TOOL_NAMES.contains(&name)
            && !package_intelligence::TOOL_NAMES.contains(&name)
            && !build_intelligence::TOOL_NAMES.contains(&name)
            && !git_intelligence::TOOL_NAMES.contains(&name)
            && !change_impact::TOOL_NAMES.contains(&name)
            && !verification_planner::TOOL_NAMES.contains(&name)
        {
            self.engineering.invalidate();
        }
        self.governor
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .after_tool(name, args, result)
    }

    pub fn memory_inject(&self, query: &str) -> Option<String> {
        self.memory
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .inject(query)
    }

    pub fn memory_index_messages(
        &mut self,
        messages: &[MemoryMessage],
    ) -> Result<usize, ToolError> {
        self.memory
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .index_messages(messages)
    }

    pub fn session_start(&mut self) {
        self.governor
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .session_start();
        self.memory
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .session_start();
    }

    pub fn session_compact(&mut self) {
        self.governor
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .session_compact();
        self.memory
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .session_compact();
    }

    /// A background graph run must not outlive the session that started it.
    pub fn session_shutdown(&mut self) {
        if let Some(processes) = &self.graph.processes {
            processes.shutdown();
        }
        self.language_intelligence.shutdown();
        graph::abort_all_runs();
        self.security.abort_review();
        self.learning.cancel_active_review();
    }

    pub fn sync_active_learning_memories(&mut self) {
        let mut to_sync = Vec::new();
        for c in self.learning.project_store.candidates() {
            if c.status == ArtifactStatus::Active {
                to_sync.push(c.clone());
            }
        }
        for c in self.learning.global_store.candidates() {
            if c.status == ArtifactStatus::Active {
                to_sync.push(c.clone());
            }
        }

        let mut memory = self
            .memory
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for candidate in to_sync {
            match &candidate.artifact {
                LearningArtifact::Memory {
                    memory_kind,
                    text,
                    importance,
                } => {
                    let kind = match memory_kind.to_ascii_lowercase().as_str() {
                        "decision" => vector_memory::MemoryKind::Decision,
                        "architecture" => vector_memory::MemoryKind::Architecture,
                        "discovery" => vector_memory::MemoryKind::Discovery,
                        "bug" => vector_memory::MemoryKind::Bug,
                        "fix" => vector_memory::MemoryKind::Fix,
                        "constraint" => vector_memory::MemoryKind::Constraint,
                        "task_result" => vector_memory::MemoryKind::TaskResult,
                        "compaction" => vector_memory::MemoryKind::Compaction,
                        "fact" => vector_memory::MemoryKind::Fact,
                        "task" => vector_memory::MemoryKind::Task,
                        "summary" => vector_memory::MemoryKind::Summary,
                        "conversation" => vector_memory::MemoryKind::Conversation,
                        _ => vector_memory::MemoryKind::Fact,
                    };
                    let _ = memory.index_learning_memory(
                        text,
                        kind,
                        *importance,
                        candidate.confidence,
                        &candidate.source_session_id,
                        candidate.source_turn,
                        candidate.evidence.graph_run_id.as_deref(),
                    );
                }
                LearningArtifact::FailureLesson { text, importance } => {
                    let _ = memory.index_learning_memory(
                        text,
                        vector_memory::MemoryKind::Bug,
                        *importance,
                        candidate.confidence,
                        &candidate.source_session_id,
                        candidate.source_turn,
                        candidate.evidence.graph_run_id.as_deref(),
                    );
                }
                _ => {}
            }
        }
    }

    pub fn review_settled_turn(&mut self, evidence: LearningEvidence) -> Option<String> {
        let result = self.learning.review_settled_turn(evidence);
        self.sync_active_learning_memories();
        result
    }

    pub fn cancel_active_learning_review(&mut self) {
        self.learning.cancel_active_review();
    }

    pub fn record_skill_outcome_for_content_hash(
        &mut self,
        name: &str,
        content_hash: &str,
        outcome: SkillOutcome,
    ) {
        let _ = self
            .learning
            .record_skill_outcome_for_content_hash(name, content_hash, outcome);
    }

    pub fn set_learning_project_trusted(&mut self, trusted: bool) {
        self.learning.set_project_trusted(trusted);
    }

    pub fn drain_learning_notifications(&mut self) -> Vec<String> {
        self.learning.drain_notifications()
    }

    pub fn execute_tool(
        &mut self,
        _cwd: &Path,
        name: &str,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        match name {
            name if browser::TOOL_NAMES.contains(&name) => Err(ToolError::Failed(
                "browser tool requires engine dispatch context".into(),
            )),
            name if test_impact::TOOL_NAMES.contains(&name) => self.test_impact.execute(name, args),
            name if package_intelligence::TOOL_NAMES.contains(&name) => {
                self.package_intelligence.execute_tool(name, args)
            }
            name if build_intelligence::TOOL_NAMES.contains(&name) => {
                self.build_intelligence.execute_tool(name, args)
            }
            name if git_intelligence::TOOL_NAMES.contains(&name) => {
                self.git_intelligence.execute_tool(name, args)
            }
            name if change_impact::TOOL_NAMES.contains(&name) => {
                self.change_impact.execute_tool(name, args)
            }
            name if verification_planner::TOOL_NAMES.contains(&name) => {
                self.verification_planner.execute_tool(name, args)
            }
            name if workspace_snapshot::TOOL_NAMES.contains(&name) => {
                self.workspace_snapshot.execute_tool(name, args)
            }
            name if repo_intelligence::is_repo_tool(name) => {
                self.repo_intelligence.execute_tool(name, args)
            }
            name if language_intelligence::TOOL_NAMES.contains(&name) => {
                if std::env::var_os("PI_GRAPH_ROLE").is_some() {
                    let client = davinci_agent::runtime::task_transport::TaskCoordinatorClient::from_env()
                        .ok_or_else(||ToolError::Failed("Parent language-intelligence transport unavailable; no worker-local server is allowed".into()))?;
                    client.call_with_timeout(name, args, None, std::time::Duration::from_secs(35))
                } else {
                    self.language_intelligence.execute(name, args)
                }
            }
            VISUAL_SNAPSHOT_TOOL => self.visual_snapshot.execute_tool(_cwd, args),
            "memory_search" => self
                .memory
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .search_tool(args),
            "retrieve_output" => self
                .governor
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .retrieve(args)
                .or_else(|error| {
                if std::env::var_os("PI_GRAPH_ROLE").is_some() {
                    if let Some(client) =
                        davinci_agent::runtime::task_transport::TaskCoordinatorClient::from_env()
                    {
                        return client.call(name, args);
                    }
                }
                Err(error)
            }),
            "skill_list" => {
                let query = args.get("query").and_then(Value::as_str).unwrap_or("");
                let query_embedding = {
                    let memory = self
                        .memory
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if !query.trim().is_empty() && memory.dense_available() {
                        memory.embed_query_text(query).ok()
                    } else {
                        None
                    }
                };
                self.learning.skill_list_tool_with_query_embedding(
                    _cwd,
                    args,
                    query_embedding.as_deref(),
                )
            }
            "skill_view" => self.learning.skill_view_tool(_cwd, args),
            "skill_manage" => self.learning.skill_manage_tool(_cwd, args),
            name if name.starts_with("sec_") => self.security.execute_tool(name, args),
            "graph_status" | "graph_run" | GRAPH_SUBMIT_TOOL => self.graph.execute_tool(name, args),
            _ => Err(ToolError::Unknown(name.to_string())),
        }
    }

    pub fn command(&mut self, name: &str, args: &str) -> Result<Option<Value>, String> {
        match name {
            "repo-index-status" => Ok(Some(self.repo_intelligence.status())),
            "lsp-status" => Ok(Some(self.language_intelligence.status())),
            "test-impact-status" => Ok(Some(self.test_impact.status())),
            "package-status" => Ok(Some(self.package_intelligence.status())),
            "build-status" => Ok(Some(self.build_intelligence.status())),
            "git-status" => Ok(Some(self.git_intelligence.status())),
            "impact-status" => Ok(Some(self.change_impact.status())),
            "verification-status" => Ok(Some(self.verification_planner.status())),
            "workspace-status" => Ok(Some(self.workspace_snapshot.status())),
            "memory-status" => Ok(Some(self.memory.status())),
            "memory-search" => Ok(Some(self.memory.search_text(args))),
            "memory-reindex" => Ok(Some(self.memory.reindex().map_err(|err| err.to_string())?)),
            "memory-clear" => Ok(Some(self.memory.clear().map_err(|err| err.to_string())?)),
            "memory-page" => Ok(Some(memory_page::command(&self.memory, args)?)),
            "cache-status" => {
                let stats = self.cache.stats();
                let raw_input = stats
                    .provider
                    .input_tokens
                    .saturating_add(stats.provider.cache_read_tokens)
                    .saturating_add(stats.provider.cache_write_tokens);
                Ok(Some(json!({
                    "enabled": self.cache.config().enabled,
                    "runtimeFeatures": davinci_ai::openai_cache_policy::runtime_features(),
                    "codexUsage": davinci_ai::codex_usage::latest(),
                    "codexCreditsEstimate": codex_credits_estimate(stats.provider.total_cost_usd),
                    "creditsSource": "estimate: session USD cost x 25, Codex rate card 2026-09",
                    "summary": stats.summary(),
                    "namespaces": stats.namespaces,
                    "diskUsage":"last observed on write or explicit sweep; no startup scan",
                    "providerSource":"provider-reported usage only; not local cache hits or websocket reuse",
                    "provider":{
                        "ordinaryInputTokens":stats.provider.input_tokens,
                        "cacheReadTokens":stats.provider.cache_read_tokens,
                        "cacheWriteTokens":stats.provider.cache_write_tokens,
                        "rawInputTokens":raw_input,
                        "cacheReadRatio":if raw_input > 0 {
                            Value::from(stats.provider.cache_read_tokens as f64 / raw_input as f64)
                        } else {
                            Value::Null
                        }
                    },
                    "localEvidenceSource":"memory/persistent namespace counters above",
                    "transportContinuationSource":"separate Codex websocket/session diagnostics"
                })))
            }
            "governor-status" => Ok(Some(self.governor.status())),
            "governor-reset" => {
                self.governor.reset();
                Ok(Some(self.governor.status()))
            }
            "learning-status" => Ok(Some(self.learning.status_command())),
            "learning-pending" => Ok(Some(self.learning.pending_command())),
            "learning-approve" => {
                let res = self.learning.approve_command(args);
                self.sync_active_learning_memories();
                res.map(Some)
            }
            "learning-reject" => self.learning.reject_command(args).map(Some),
            "skill-list" => self.learning.skill_list_command(args).map(Some),
            "skill-view" => self.learning.skill_view_command(args).map(Some),
            "hook-status" => {
                let cwd = if self.cwd.as_os_str().is_empty() {
                    std::env::current_dir().unwrap_or_default()
                } else {
                    self.cwd.clone()
                };
                Ok(Some(crate::hooks::status_report_with_agent_dir(
                    &self.agent_dir,
                    &cwd,
                )))
            }
            name if name.starts_with("graph") => self.graph.command(name, args),
            name if name == "security-scan" || name.starts_with("sec-") => {
                self.security.command(name, args)
            }
            _ => Ok(None),
        }
    }

    pub fn describe_tool(name: &str) -> Option<davinci_ai::ToolSpec> {
        if browser::TOOL_NAMES.contains(&name) {
            return browser::tool_spec(name);
        }
        if repo_intelligence::is_repo_tool(name) {
            return repo_intelligence::tool_spec(name);
        }
        if test_impact::TOOL_NAMES.contains(&name) {
            return test_impact::tool_spec(name);
        }
        if package_intelligence::TOOL_NAMES.contains(&name) {
            return package_intelligence::tool_spec(name);
        }
        if build_intelligence::TOOL_NAMES.contains(&name) {
            return build_intelligence::tool_spec(name);
        }
        if git_intelligence::TOOL_NAMES.contains(&name) {
            return git_intelligence::tool_spec(name);
        }
        if change_impact::TOOL_NAMES.contains(&name) {
            return change_impact::tool_spec(name);
        }
        if verification_planner::TOOL_NAMES.contains(&name) {
            return verification_planner::tool_spec(name);
        }
        if workspace_snapshot::TOOL_NAMES.contains(&name) {
            return workspace_snapshot::tool_spec(name);
        }
        if language_intelligence::TOOL_NAMES.contains(&name) {
            return language_intelligence::tool_spec(name);
        }
        let (description, parameters) = match name {
            "memory_search" => (
                "Search durable vector and lexical memory for supporting context.",
                json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":20}},"required":["query"]}),
            ),
            "retrieve_output" => (
                "Retrieve a lossless full or ranged tool output saved by token governor.",
                json!({"type":"object","properties":{"id":{"type":"string","pattern":"^out-[0-9a-f]{12}$"},"startLine":{"type":"integer","minimum":1},"lineByteOffset":{"type":"integer","minimum":0},"endLine":{"type":"integer","minimum":1},"grep":{"type":"string"}},"required":["id"]}),
            ),
            "graph_status" => (
                "Inspect the active and recent graph runs in this project.",
                json!({"type":"object","properties":{"runId":{"type":"string"}}}),
            ),
            "graph_run" => (
                "Solve a coding task as an execution graph of isolated, least-privileged worker processes (classify, research, plan, implement, verify, review) with deterministic plan, verification, review coverage, and security audit guarantees. Runs to completion, which can take a long time.",
                json!({"type":"object","properties":{"goal":{"type":"string","minLength":1},"mode":{"type":"string","enum":["simple","complex"]},"dryRun":{"type":"boolean"}},"required":["goal"]}),
            ),
            "skill_list" => (
                "List compact reusable skill descriptors relevant to a task without loading full skill bodies.",
                json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "scope": {"type": "string", "enum": ["project", "global", "all"]},
                        "status": {"type": "string", "enum": ["candidate", "pending_approval", "active", "archived", "all"]},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 20}
                    }
                }),
            ),
            "skill_view" => (
                "Read the current full SKILL.md or an allowed supporting file for one skill.",
                json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "file": {"type": "string"}
                    },
                    "required": ["name"]
                }),
            ),
            "skill_manage" => (
                "Safely create, patch, support, or archive agent skills with verification and ownership guards.",
                json!({
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["create", "patch", "write_file", "archive", "activate", "reject"]},
                        "name": {"type": "string"},
                        "scope": {"type": "string", "enum": ["project", "global"]},
                        "description": {"type": "string"},
                        "body": {"type": "string"},
                        "oldText": {"type": "string"},
                        "newText": {"type": "string"},
                        "expectedHash": {"type": "string"},
                        "filePath": {"type": "string"},
                        "content": {"type": "string"},
                        "candidateId": {"type": "string"}
                    },
                    "required": ["action", "name"]
                }),
            ),
            VISUAL_SNAPSHOT_TOOL => return Some(visual_snapshot_tool_spec()),
            name if name.starts_with("sec_") => security_scan::tool_spec(name),
            // Only a graph worker child sees this tool; it is the worker's one
            // exit door and its schema names the artifact that node owes.
            GRAPH_SUBMIT_TOOL => {
                let context = graph_worker_context()?;
                let (description, parameters) = context.tool_spec();
                return Some(davinci_ai::ToolSpec {
                    name: name.to_string(),
                    description,
                    parameters,
                    constrained_sampling: None,
                });
            }
            _ => return None,
        };
        Some(davinci_ai::ToolSpec {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
            constrained_sampling: None,
        })
    }

    pub fn tool_specs() -> Vec<davinci_ai::ToolSpec> {
        NATIVE_TOOLS
            .iter()
            .copied()
            .chain(graph_worker_context().map(|_| GRAPH_SUBMIT_TOOL))
            .filter_map(Self::describe_tool)
            .collect()
    }

    pub fn available_tool_specs(&self) -> Vec<davinci_ai::ToolSpec> {
        let mut specs = Self::tool_specs();
        if self.visual_verification_available() {
            specs.push(visual_snapshot_tool_spec());
        }
        specs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_external_native_command_is_listed() {
        let listed: std::collections::BTreeSet<_> =
            command_specs().into_iter().map(|(name, _, _)| name).collect();
        let internal_graph = ["graph-resume", "graph-status", "graph-view", "graph-abort"];
        for name in NATIVE_COMMANDS {
            if internal_graph.contains(name) {
                continue;
            }
            assert!(listed.contains(name), "{name} is dispatched but not listed");
        }
    }
    use std::sync::Arc;

    #[test]
    fn codex_credit_estimate_follows_rate_card_ratio() {
        assert_eq!(codex_credits_estimate(5.0), 125.0);
        assert_eq!(codex_credits_estimate(0.2), 5.0);
        assert_eq!(codex_credits_estimate(-1.0), 0.0);
    }

    #[test]
    fn repository_language_and_cache_surfaces_coexist() {
        let _guard = graph::worker_hooks::submit_test_guard();
        let mut host = NativeExtensionHost::default();
        for name in language_intelligence::TOOL_NAMES.iter().chain(
            [
                "repo_map",
                "symbol_search",
                "file_symbols",
                "file_dependencies",
                "symbol_relationships",
                "related_files",
                "code_query",
            ]
            .iter(),
        ) {
            assert!(host.has_tool(name));
            assert!(host.tool_names().iter().any(|n| n == name));
            assert!(host
                .available_tool_specs()
                .iter()
                .any(|spec| spec.name == *name));
            assert_eq!(
                davinci_agent::tool_class(name),
                davinci_agent::ToolClass::Read
            );
        }
        for command in ["repo-index-status", "cache-status", "lsp-status"] {
            assert!(NATIVE_COMMANDS.contains(&command));
            assert!(command_specs().iter().any(|(name, _, _)| *name == command));
            assert!(host.command(command, "").unwrap().is_some());
        }
        assert_eq!(
            host.command("lsp-status", "").unwrap().unwrap()["sessions"],
            json!([])
        );
        let result = host
            .execute_tool(
                Path::new("."),
                "lsp_hover",
                &json!({"path":"missing.ts","line":1,"column":1}),
            )
            .unwrap();
        assert!(result.is_error);
        assert_eq!(
            result.details.unwrap()["error"]["code"],
            "invalid_source_path"
        );
    }

    #[test]
    fn cache_status_is_lazy_and_separates_provider_usage() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let mut host = NativeExtensionHost::new_with_agent_dir(
            "cache-status-test",
            root.path(),
            Some(state.path()),
        );
        host.cache.record_provider_usage(100, 30, 4);
        let status = host.command("cache-status", "").unwrap().unwrap();
        assert_eq!(status["summary"]["provider"]["cacheReadTokens"], 30);
        assert_eq!(status["summary"]["memory"]["hits"], 0);
        assert!(!state.path().join("cache-runtime").exists());
    }

    struct FakeVisualBackend {
        image_path: std::path::PathBuf,
    }

    impl VisualSnapshotBackend for FakeVisualBackend {
        fn name(&self) -> &str {
            "fake_visual_backend"
        }

        fn available(&self, _cwd: &Path) -> bool {
            true
        }

        fn capture(
            &self,
            _cwd: &Path,
            request: &crate::native_tools::VisualSnapshotRequest,
        ) -> Result<crate::native_tools::VisualSnapshotResult, String> {
            Ok(crate::native_tools::VisualSnapshotResult {
                image_path: self.image_path.clone(),
                width: request.viewport_width,
                height: request.viewport_height,
            })
        }
    }

    struct WorkerEnv;

    impl WorkerEnv {
        fn set(artifact_path: &Path) -> Self {
            std::env::set_var("PI_GRAPH_ROLE", "reviewer");
            std::env::set_var("PI_GRAPH_EXPECT", "review");
            std::env::set_var("PI_GRAPH_ARTIFACT_PATH", artifact_path);
            std::env::set_var("PI_GRAPH_EXTRA_TOOLS", "read,graph_submit");
            Self
        }
    }

    impl Drop for WorkerEnv {
        fn drop(&mut self) {
            for key in [
                "PI_GRAPH_ROLE",
                "PI_GRAPH_EXPECT",
                "PI_GRAPH_ARTIFACT_PATH",
                "PI_GRAPH_EXTRA_TOOLS",
            ] {
                std::env::remove_var(key);
            }
        }
    }

    #[test]
    fn graph_submit_exists_only_inside_a_worker_process() {
        let _lock = graph::worker_hooks::submit_test_guard();
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("artifact.json");

        let host = NativeExtensionHost::default();
        assert!(!host
            .tool_names()
            .iter()
            .any(|name| name == GRAPH_SUBMIT_TOOL));
        assert!(!NativeExtensionHost::tool_specs()
            .iter()
            .any(|spec| spec.name == GRAPH_SUBMIT_TOOL));

        let _env = WorkerEnv::set(&artifact);
        let host = NativeExtensionHost::default();
        assert!(host
            .tool_names()
            .iter()
            .any(|name| name == GRAPH_SUBMIT_TOOL));
        let spec = NativeExtensionHost::tool_specs()
            .into_iter()
            .find(|spec| spec.name == GRAPH_SUBMIT_TOOL)
            .expect("graph_submit is offered to a worker");
        assert!(spec.description.contains("final review artifact"));
    }

    #[test]
    fn registered_visual_backend_is_advertised_by_the_native_host() {
        let dir = tempfile::tempdir().unwrap();
        let image_path = dir.path().join("snapshot.png");
        std::fs::write(&image_path, b"fake-png").unwrap();
        let mut host = NativeExtensionHost::default();

        assert!(!host.has_tool(VISUAL_SNAPSHOT_TOOL));
        host.register_visual_snapshot_backend(
            Arc::new(FakeVisualBackend { image_path }),
            dir.path(),
        );

        assert!(host.visual_verification_available());
        assert!(host.has_tool(VISUAL_SNAPSHOT_TOOL));
        assert!(host
            .tool_names()
            .iter()
            .any(|name| name == VISUAL_SNAPSHOT_TOOL));
        assert!(host
            .available_tool_specs()
            .iter()
            .any(|spec| spec.name == VISUAL_SNAPSHOT_TOOL));
    }

    #[test]
    fn a_worker_submits_and_is_policed_through_the_native_host() {
        let _lock = graph::worker_hooks::submit_test_guard();
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("artifact.json");
        let _env = WorkerEnv::set(&artifact);
        let mut host = NativeExtensionHost::default();

        // The reviewer may run tests but never mutate the repository.
        assert_eq!(
            host.before_tool("bash", &json!({"command": "cargo test"}), || "hash".into()),
            None
        );
        assert!(host
            .before_tool("write", &json!({"path": "src/lib.rs"}), || "hash".into())
            .is_some());
        assert!(host
            .before_tool("bash", &json!({"command": "rm -rf src"}), || "hash".into())
            .is_some());

        let result = host
            .execute_tool(
                dir.path(),
                GRAPH_SUBMIT_TOOL,
                &json!({"artifact": {"verdict": "approve", "issues": [], "notes": "ok"}}),
            )
            .expect("submit accepted");
        assert!(!result.is_error);
        assert!(artifact.is_file(), "the artifact reached disk");

        // Once the artifact is in, the node is done: no more work is accepted.
        assert!(host
            .before_tool("bash", &json!({"command": "cargo test"}), || "hash".into())
            .is_some());
    }

    #[test]
    fn public_native_commands_have_discoverable_metadata() {
        let specs = command_specs();
        assert!(specs.iter().any(|(name, _, _)| *name == "security-scan"));
        for internal in ["sec-status", "sec-report", "sec-abort", "sec-resume"] {
            assert!(!specs.iter().any(|(name, _, _)| *name == internal));
            assert!(NATIVE_COMMANDS.contains(&internal));
        }
        for (name, description, _) in specs {
            assert!(NATIVE_COMMANDS.contains(&name));
            assert!(!description.is_empty());
        }
    }

    #[test]
    fn graph_has_one_user_facing_slash_command() {
        let names: Vec<_> = command_specs().into_iter().map(|spec| spec.0).collect();
        assert!(names.contains(&"graph"));
        for legacy in ["graph-resume", "graph-status", "graph-view", "graph-abort"] {
            assert!(!names.contains(&legacy), "{legacy} must remain internal");
            assert!(NATIVE_COMMANDS.contains(&legacy));
        }
    }

    #[test]
    fn native_invocable_commands_identify_rust_runtime() {
        let commands = native_invocable_commands();
        assert_eq!(commands.len(), command_specs().len());
        assert!(commands.iter().any(|command| {
            command["name"] == "memory-search"
                && command["source"] == "native"
                && command["argumentHint"] == "<query>"
        }));
    }

    #[test]
    fn test_graph_internal_lifecycle_not_advertised() {
        let specs = command_specs();
        let advertised_names: Vec<&str> = specs.iter().map(|(name, _, _)| *name).collect();

        // /graph is advertised
        assert!(advertised_names.contains(&"graph"));

        // Internal lifecycle operations are deliberately omitted from public autocomplete / help
        for internal in ["graph-resume", "graph-status", "graph-view", "graph-abort"] {
            assert!(
                !advertised_names.contains(&internal),
                "internal lifecycle command '{internal}' should not be advertised in command_specs"
            );
        }

        // But they are retained in NATIVE_COMMANDS for internal/backward compatibility
        for internal in ["graph-resume", "graph-status", "graph-view", "graph-abort"] {
            assert!(
                NATIVE_COMMANDS.contains(&internal),
                "internal lifecycle command '{internal}' must remain in NATIVE_COMMANDS"
            );
        }

        // native_invocable_commands also excludes them
        let invocable = native_invocable_commands();
        for internal in ["graph-resume", "graph-status", "graph-view", "graph-abort"] {
            assert!(
                !invocable.iter().any(|c| c["name"] == internal),
                "invocable commands must not advertise '{internal}'"
            );
        }
    }

    #[test]
    fn native_learning_commands_dispatch() {
        let mut host = NativeExtensionHost::default();
        let status = host
            .command("learning-status", "")
            .unwrap()
            .expect("status value");
        assert_eq!(status["enabled"], true);

        let pending = host
            .command("learning-pending", "")
            .unwrap()
            .expect("pending value");
        assert!(pending["pending"].is_array());
    }

    #[test]
    fn test_sync_active_learning_memories_into_vector_memory() {
        let root = tempfile::tempdir().unwrap();
        let agent_dir = tempfile::tempdir().unwrap();
        let mut host = NativeExtensionHost {
            learning: LearningController::new(root.path(), Some(agent_dir.path()), None),
            memory: VectorMemory::with_config(root.path().into(), VectorMemoryConfig::default()),
            ..NativeExtensionHost::default()
        };
        let cand = LearningCandidate {
            id: "cand-mem-1".into(),
            scope: LearningScope::Project,
            status: ArtifactStatus::Active,
            artifact: LearningArtifact::Memory {
                memory_kind: "constraint".into(),
                text: "PostgreSQL pool must not exceed 20 connections in staging".into(),
                importance: 0.85,
            },
            confidence: 0.9,
            source_session_id: "sess-sync".into(),
            source_repo_id: host.memory.repo_id.clone(),
            source_turn: 1,
            created_at_ms: 1000,
            evidence: VerificationEvidence::default(),
            rationale: "reusable DB constraint".into(),
        };
        host.learning.project_store.upsert_candidate(cand).unwrap();
        host.sync_active_learning_memories();

        let search_res = host.memory.search("PostgreSQL pool connections", 5);
        assert!(!search_res.is_empty());
        assert!(search_res[0]
            .record
            .text
            .contains("PostgreSQL pool must not exceed 20"));
        assert_eq!(
            search_res[0].record.kind,
            vector_memory::MemoryKind::Constraint
        );
    }

    #[test]
    fn context_vm_artifact_adapter_delegates_to_governor_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = OutputStore::new(dir.path());
        let original = "exact governor bytes\nwith evidence";
        let reference = store.save(original).unwrap();
        let mut governor =
            TokenGovernor::with_store("context-vm-test", TokenGovernorConfig::default(), store);

        assert_eq!(
            retrieve_context_artifact(
                &mut governor,
                &format!("governor://output/{}", reference.id),
            )
            .unwrap(),
            original
        );
    }
}
