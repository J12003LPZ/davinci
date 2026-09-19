//! Deterministic coverage and metric contracts for the engineering program.
//!
//! This module deliberately records missing measurements as null-plus-reason.
//! A fixture receipt, a recommendation, or an unavailable live dependency can
//! never be promoted to a successful browser, CI, or model result.

use crate::behavior::trace::{BehaviorEvent, BehaviorTrace};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const ENGINEERING_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineeringCategory {
    RepoNavigation,
    SymbolFinding,
    TypescriptDebugging,
    SafeRefactor,
    DependencyApiUsage,
    FrontendBugFixing,
    TestSelection,
    BrowserVerification,
    GitContext,
    TransactionSafety,
    LongRunningTask,
    NormalVsGraph,
}

impl EngineeringCategory {
    pub const ALL: [Self; 12] = [
        Self::RepoNavigation,
        Self::SymbolFinding,
        Self::TypescriptDebugging,
        Self::SafeRefactor,
        Self::DependencyApiUsage,
        Self::FrontendBugFixing,
        Self::TestSelection,
        Self::BrowserVerification,
        Self::GitContext,
        Self::TransactionSafety,
        Self::LongRunningTask,
        Self::NormalVsGraph,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineeringWorkflowStep {
    RepoMap,
    LspSymbols,
    PackageApi,
    GitContext,
    ChangeImpact,
    StartDevServer,
    TransactionCreate,
    EditCode,
    LspDiagnostics,
    TestImpact,
    TargetedTests,
    TypeBuildChecks,
    OpenBrowser,
    VerifyLogin,
    ConsoleNetwork,
    TransactionVerify,
    EvidenceCompletion,
}

impl EngineeringWorkflowStep {
    pub const ALL: [Self; 17] = [
        Self::RepoMap,
        Self::LspSymbols,
        Self::PackageApi,
        Self::GitContext,
        Self::ChangeImpact,
        Self::StartDevServer,
        Self::TransactionCreate,
        Self::EditCode,
        Self::LspDiagnostics,
        Self::TestImpact,
        Self::TargetedTests,
        Self::TypeBuildChecks,
        Self::OpenBrowser,
        Self::VerifyLogin,
        Self::ConsoleNetwork,
        Self::TransactionVerify,
        Self::EvidenceCompletion,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineeringMetric {
    TaskSuccess,
    VerificationSuccess,
    IncorrectCompletionClaims,
    ToolCalls,
    FullFileReads,
    BytesRead,
    InputTokens,
    OutputTokens,
    CacheHits,
    WallClockLatencyMs,
    TestsExecuted,
    TestRuntimeMs,
    ProcessStartups,
    LspColdStarts,
    BrowserVerificationSuccess,
    RollbackCorrectness,
    CiSuccess,
}

impl EngineeringMetric {
    pub const ALL: [Self; 17] = [
        Self::TaskSuccess,
        Self::VerificationSuccess,
        Self::IncorrectCompletionClaims,
        Self::ToolCalls,
        Self::FullFileReads,
        Self::BytesRead,
        Self::InputTokens,
        Self::OutputTokens,
        Self::CacheHits,
        Self::WallClockLatencyMs,
        Self::TestsExecuted,
        Self::TestRuntimeMs,
        Self::ProcessStartups,
        Self::LspColdStarts,
        Self::BrowserVerificationSuccess,
        Self::RollbackCorrectness,
        Self::CiSuccess,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineeringRunMode {
    OfflineNormal,
    OfflineGraph,
    LiveBrowserNormal,
    LiveModelNormal,
    LiveGraph,
}

impl EngineeringRunMode {
    pub fn is_live(self) -> bool {
        !matches!(self, Self::OfflineNormal | Self::OfflineGraph)
    }

    pub fn is_graph(self) -> bool {
        matches!(self, Self::OfflineGraph | Self::LiveGraph)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricReceipt {
    pub value: Option<f64>,
    pub unit: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub observed: bool,
}

impl MetricReceipt {
    pub fn observed(value: f64, unit: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            value: Some(value),
            unit: unit.into(),
            source: source.into(),
            reason: None,
            observed: true,
        }
    }

    pub fn missing(
        unit: impl Into<String>,
        source: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            value: None,
            unit: unit.into(),
            source: source.into(),
            reason: Some(reason.into()),
            observed: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioReceipt {
    pub category: EngineeringCategory,
    pub mode: EngineeringRunMode,
    pub passed: bool,
    #[serde(default)]
    pub workflow_steps: Vec<EngineeringWorkflowStep>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub failure_reasons: Vec<String>,
}

/// Deterministic observations extracted from one normalized behavior trace.
///
/// This is intentionally narrower than a complete engineering run. It records
/// only steps and measurements that the trace can prove; callers must combine
/// receipts from the required scenarios and provide independent evidence IDs
/// before a manifest can validate.
#[derive(Debug, Clone)]
pub struct EngineeringTraceObservation {
    pub workflow_steps: BTreeSet<EngineeringWorkflowStep>,
    pub metrics: BTreeMap<EngineeringMetric, MetricReceipt>,
}

/// Convert normalized agent behavior into objective engineering observations.
/// Missing provider, browser, CI, and rollback data remains a nullable metric.
pub fn observe_behavior_trace(trace: &BehaviorTrace) -> EngineeringTraceObservation {
    let mut workflow_steps = BTreeSet::new();
    let mut labels = BTreeSet::new();

    for event in &trace.events {
        match event {
            BehaviorEvent::Read { .. } => {
                workflow_steps.insert(EngineeringWorkflowStep::RepoMap);
            }
            BehaviorEvent::Search { tool, .. } => {
                if matches!(tool.as_str(), "symbol_search" | "file_symbols") {
                    workflow_steps.insert(EngineeringWorkflowStep::LspSymbols);
                } else {
                    workflow_steps.insert(EngineeringWorkflowStep::RepoMap);
                }
                labels.insert(tool.clone());
            }
            BehaviorEvent::Edit { .. } => {
                workflow_steps.insert(EngineeringWorkflowStep::EditCode);
            }
            BehaviorEvent::Shell { command_class, .. } => match command_class.as_str() {
                "dev_server" => {
                    workflow_steps.insert(EngineeringWorkflowStep::StartDevServer);
                }
                "git" => {
                    workflow_steps.insert(EngineeringWorkflowStep::GitContext);
                }
                "test" => {
                    workflow_steps.insert(EngineeringWorkflowStep::TargetedTests);
                }
                "build" | "lint" => {
                    workflow_steps.insert(EngineeringWorkflowStep::TypeBuildChecks);
                }
                _ => {}
            },
            BehaviorEvent::CapabilityIdentity { capability } => {
                labels.insert(capability.clone());
            }
            BehaviorEvent::PlanEvent { kind } => {
                labels.insert(kind.clone());
            }
            BehaviorEvent::FinalResponse
            | BehaviorEvent::VerificationClaim { .. }
            | BehaviorEvent::SubagentSpawn { .. }
            | BehaviorEvent::PermissionAsked { .. }
            | BehaviorEvent::PermissionDenied { .. }
            | BehaviorEvent::MessageLifecycle { .. }
            | BehaviorEvent::PromptProfile { .. } => {}
        }
    }

    for label in &labels {
        add_workflow_label(label, &mut workflow_steps);
    }

    let metrics = behavior_trace_metrics(trace, &labels);
    EngineeringTraceObservation {
        workflow_steps,
        metrics,
    }
}

/// Build one scenario receipt from a trace without inventing missing gates.
pub fn scenario_receipt_from_trace(
    category: EngineeringCategory,
    mode: EngineeringRunMode,
    trace: &BehaviorTrace,
    evidence: Vec<String>,
) -> ScenarioReceipt {
    let observation = observe_behavior_trace(trace);
    let expected = EngineeringWorkflowStep::ALL
        .into_iter()
        .collect::<BTreeSet<_>>();
    let missing = expected
        .difference(&observation.workflow_steps)
        .map(|step| format!("{step:?}"))
        .collect::<Vec<_>>();

    let mut failure_reasons = Vec::new();
    if !missing.is_empty() {
        failure_reasons.push(format!(
            "workflow steps not observed: {}",
            missing.join(", ")
        ));
    }
    if trace
        .verification
        .iter()
        .any(|verification| !verification.passed)
    {
        failure_reasons.push("a verification command failed".into());
    }
    if !trace
        .events
        .iter()
        .any(|event| matches!(event, BehaviorEvent::FinalResponse))
    {
        failure_reasons.push("final response was not observed".into());
    }
    if evidence.is_empty() {
        failure_reasons.push("no evidence artifact was supplied".into());
    }

    ScenarioReceipt {
        category,
        mode,
        passed: failure_reasons.is_empty(),
        workflow_steps: observation.workflow_steps.into_iter().collect(),
        evidence,
        failure_reasons,
    }
}

fn add_workflow_label(label: &str, steps: &mut BTreeSet<EngineeringWorkflowStep>) {
    let normalized = label.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    let normalized = normalized
        .strip_prefix("workflow:")
        .unwrap_or(&normalized)
        .to_string();
    let normalized = normalized
        .strip_prefix("workflow_")
        .unwrap_or(&normalized)
        .to_string();

    match normalized.as_str() {
        "repo_intelligence" | "repo_map" | "repo_navigation" => {
            steps.insert(EngineeringWorkflowStep::RepoMap);
        }
        "lsp_symbols"
        | "lsp_definition"
        | "lsp_references"
        | "lsp_hover"
        | "lsp_document_symbols"
        | "lsp_workspace_symbols"
        | "lsp_implementations"
        | "lsp_type_definition" => {
            steps.insert(EngineeringWorkflowStep::LspSymbols);
        }
        "lsp_diagnostics" | "diagnostics" => {
            steps.insert(EngineeringWorkflowStep::LspDiagnostics);
        }
        "package_intelligence"
        | "package_api"
        | "package_info"
        | "package_exports"
        | "package_symbol"
        | "package_dependents"
        | "package_why" => {
            steps.insert(EngineeringWorkflowStep::PackageApi);
        }
        "git_intelligence"
        | "git_context"
        | "git_symbol_history"
        | "git_related_commits"
        | "git_changed_symbols"
        | "git_branch_diff"
        | "git_blame_symbol"
        | "git_commit_context"
        | "git_conflict_explain" => {
            steps.insert(EngineeringWorkflowStep::GitContext);
        }
        "change_impact" | "impact_analyze" => {
            steps.insert(EngineeringWorkflowStep::ChangeImpact);
        }
        "process_start" | "dev_server" | "start_dev_server" => {
            steps.insert(EngineeringWorkflowStep::StartDevServer);
        }
        "edit" | "write" | "apply_patch" | "edit_code" => {
            steps.insert(EngineeringWorkflowStep::EditCode);
        }
        "transaction_create" | "workspace_checkpoint" => {
            steps.insert(EngineeringWorkflowStep::TransactionCreate);
        }
        "test_impact" | "test_related" | "test_impacted" | "test_plan" => {
            steps.insert(EngineeringWorkflowStep::TestImpact);
        }
        "targeted_tests" => {
            steps.insert(EngineeringWorkflowStep::TargetedTests);
        }
        "build_intelligence" | "build_targets" | "build_dependencies" | "build_affected"
        | "build_command" | "workspace_packages" | "type_build_checks" => {
            steps.insert(EngineeringWorkflowStep::TypeBuildChecks);
        }
        "browser_open" | "open_browser" => {
            steps.insert(EngineeringWorkflowStep::OpenBrowser);
        }
        "browser_snapshot" | "browser_login" | "verify_login" | "login_verified" => {
            steps.insert(EngineeringWorkflowStep::VerifyLogin);
        }
        "browser_console" | "browser_network" | "console_network" => {
            steps.insert(EngineeringWorkflowStep::ConsoleNetwork);
        }
        "workspace_diff" | "workspace_restore" | "transaction_verify" => {
            steps.insert(EngineeringWorkflowStep::TransactionVerify);
        }
        "verification_plan" | "evidence_completion" | "evidence_complete" => {
            steps.insert(EngineeringWorkflowStep::EvidenceCompletion);
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineeringEvalManifest {
    pub schema_version: u32,
    pub run_id: String,
    pub fixture_id: String,
    pub source_identity: String,
    pub toolchain: String,
    pub mode: EngineeringRunMode,
    pub feature_flags: BTreeMap<String, bool>,
    pub categories: Vec<EngineeringCategory>,
    pub workflow_steps: Vec<EngineeringWorkflowStep>,
    pub metrics: BTreeMap<EngineeringMetric, MetricReceipt>,
    pub scenarios: Vec<ScenarioReceipt>,
    #[serde(default)]
    pub planted_failures: Vec<String>,
}

impl EngineeringEvalManifest {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.schema_version != ENGINEERING_SCHEMA_VERSION {
            errors.push(format!(
                "unsupported engineering eval schema {}",
                self.schema_version
            ));
        }
        if self.run_id.trim().is_empty() {
            errors.push("run_id is required".to_string());
        }
        if self.fixture_id.trim().is_empty() {
            errors.push("fixture_id is required".to_string());
        }
        if self.source_identity.trim().is_empty() {
            errors.push("source_identity is required".to_string());
        }
        if self.toolchain.trim().is_empty() {
            errors.push("toolchain is required".to_string());
        }
        self.validate_unique_categories(&mut errors);
        self.validate_unique_steps(&mut errors);
        self.validate_flags(&mut errors);
        self.validate_metrics(&mut errors);
        self.validate_scenarios(&mut errors);
        self.validate_executable_workflow_coverage(&mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn validate_unique_categories(&self, errors: &mut Vec<String>) {
        let actual = self.categories.iter().copied().collect::<BTreeSet<_>>();
        let expected = EngineeringCategory::ALL
            .into_iter()
            .collect::<BTreeSet<_>>();
        if actual != expected || self.categories.len() != expected.len() {
            errors.push(
                "categories must contain each of the twelve required categories exactly once"
                    .into(),
            );
        }
    }

    fn validate_unique_steps(&self, errors: &mut Vec<String>) {
        let actual = self.workflow_steps.iter().copied().collect::<BTreeSet<_>>();
        let expected = EngineeringWorkflowStep::ALL
            .into_iter()
            .collect::<BTreeSet<_>>();
        if actual != expected || self.workflow_steps.len() != expected.len() {
            errors.push(
                "workflow_steps must contain each of the seventeen required steps exactly once"
                    .into(),
            );
        }
    }

    fn validate_flags(&self, errors: &mut Vec<String>) {
        for name in REQUIRED_FEATURE_FLAGS {
            if !self.feature_flags.contains_key(*name) {
                errors.push(format!("missing independent feature flag '{name}'"));
            }
        }
    }

    fn validate_metrics(&self, errors: &mut Vec<String>) {
        for metric in EngineeringMetric::ALL {
            let Some(receipt) = self.metrics.get(&metric) else {
                errors.push(format!("missing metric receipt for {metric:?}"));
                continue;
            };
            if receipt.source.trim().is_empty() || receipt.unit.trim().is_empty() {
                errors.push(format!("metric {metric:?} needs source and unit"));
            }
            match (receipt.value, receipt.observed, receipt.reason.as_deref()) {
                (Some(value), true, _) if value.is_finite() && value >= 0.0 => {}
                (Some(_), false, _) => {
                    errors.push(format!("metric {metric:?} has a value but is not observed"));
                }
                (Some(_), true, _) => {
                    errors.push(format!(
                        "metric {metric:?} has a non-finite or negative value"
                    ));
                }
                (None, _, Some(reason)) if !reason.trim().is_empty() => {}
                (None, _, _) => {
                    errors.push(format!("missing metric {metric:?} must include a reason"));
                }
            }
        }
    }

    fn validate_scenarios(&self, errors: &mut Vec<String>) {
        let actual = self
            .scenarios
            .iter()
            .map(|scenario| scenario.category)
            .collect::<BTreeSet<_>>();
        let expected = EngineeringCategory::ALL
            .into_iter()
            .collect::<BTreeSet<_>>();
        if actual != expected || self.scenarios.len() != expected.len() {
            errors.push("scenarios must cover all twelve categories exactly once".into());
        }
        for scenario in &self.scenarios {
            if scenario.mode != self.mode {
                errors.push(format!(
                    "scenario {:?} mode {:?} does not match manifest mode {:?}",
                    scenario.category, scenario.mode, self.mode
                ));
            }
            if scenario.passed && !scenario.failure_reasons.is_empty() {
                errors.push(format!(
                    "passed scenario {:?} cannot carry failure reasons",
                    scenario.category
                ));
            }
            if scenario.passed && scenario.evidence.is_empty() {
                errors.push(format!(
                    "passed scenario {:?} needs evidence",
                    scenario.category
                ));
            }
            if !scenario.passed && scenario.failure_reasons.is_empty() {
                errors.push(format!(
                    "failed scenario {:?} needs a failure reason",
                    scenario.category
                ));
            }
            if scenario.workflow_steps.is_empty() {
                errors.push(format!(
                    "scenario {:?} needs executable workflow-step coverage",
                    scenario.category
                ));
            }
        }
    }

    fn validate_executable_workflow_coverage(&self, errors: &mut Vec<String>) {
        let actual = self
            .scenarios
            .iter()
            .flat_map(|scenario| scenario.workflow_steps.iter().copied())
            .collect::<BTreeSet<_>>();
        let expected = EngineeringWorkflowStep::ALL
            .into_iter()
            .collect::<BTreeSet<_>>();
        if actual != expected {
            errors.push(
                "scenario receipts must provide executable coverage for all seventeen workflow steps"
                    .into(),
            );
        }
    }
}

fn behavior_trace_metrics(
    trace: &BehaviorTrace,
    labels: &BTreeSet<String>,
) -> BTreeMap<EngineeringMetric, MetricReceipt> {
    let verification_count = trace.verification.len();
    let passed_verifications = trace
        .verification
        .iter()
        .filter(|verification| verification.passed)
        .count();
    let incorrect_claims = if passed_verifications == 0 {
        trace
            .events
            .iter()
            .filter(|event| matches!(event, BehaviorEvent::VerificationClaim { .. }))
            .count() as f64
    } else {
        0.0
    };
    let full_file_reads = trace
        .events
        .iter()
        .filter(|event| {
            matches!(
                event,
                BehaviorEvent::Read {
                    start: None,
                    end: None,
                    ..
                }
            )
        })
        .count() as f64;

    let mut metrics = BTreeMap::new();
    metrics.insert(
        EngineeringMetric::TaskSuccess,
        explicit_boolean_metric(
            labels,
            "task_success",
            "task_failure",
            "behavior_trace",
            "task outcome was not supplied by the trace",
        ),
    );
    metrics.insert(
        EngineeringMetric::VerificationSuccess,
        if verification_count == 0 {
            MetricReceipt::missing(
                "boolean",
                "behavior_trace.verification",
                "no verification command was observed",
            )
        } else {
            MetricReceipt::observed(
                (passed_verifications == verification_count) as u8 as f64,
                "boolean",
                "behavior_trace.verification",
            )
        },
    );
    metrics.insert(
        EngineeringMetric::IncorrectCompletionClaims,
        MetricReceipt::observed(
            incorrect_claims,
            "count",
            "behavior_trace.verification_claims",
        ),
    );
    metrics.insert(
        EngineeringMetric::ToolCalls,
        MetricReceipt::observed(
            trace.stats.tool_calls as f64,
            "count",
            "behavior_trace.stats",
        ),
    );
    metrics.insert(
        EngineeringMetric::FullFileReads,
        MetricReceipt::observed(full_file_reads, "count", "behavior_trace.reads"),
    );
    metrics.insert(
        EngineeringMetric::BytesRead,
        MetricReceipt::missing(
            "bytes",
            "behavior_trace",
            "read byte counts were not emitted",
        ),
    );
    metrics.insert(
        EngineeringMetric::InputTokens,
        token_metric(trace.stats.input_tokens, "input_tokens"),
    );
    metrics.insert(
        EngineeringMetric::OutputTokens,
        token_metric(trace.stats.output_tokens, "output_tokens"),
    );
    metrics.insert(
        EngineeringMetric::CacheHits,
        MetricReceipt::missing(
            "count",
            "cache_telemetry",
            "cache telemetry was not attached",
        ),
    );
    metrics.insert(
        EngineeringMetric::WallClockLatencyMs,
        if trace.stats.wall_ms == 0 {
            MetricReceipt::missing(
                "milliseconds",
                "behavior_trace.stats",
                "wall-clock timing was not emitted",
            )
        } else {
            MetricReceipt::observed(
                trace.stats.wall_ms as f64,
                "milliseconds",
                "behavior_trace.stats",
            )
        },
    );
    metrics.insert(
        EngineeringMetric::TestsExecuted,
        MetricReceipt::observed(
            trace
                .verification
                .iter()
                .filter(|verification| verification.kind == "test")
                .count() as f64,
            "count",
            "behavior_trace.verification",
        ),
    );
    metrics.insert(
        EngineeringMetric::TestRuntimeMs,
        MetricReceipt::missing(
            "milliseconds",
            "behavior_trace.verification",
            "verification durations were not emitted",
        ),
    );
    metrics.insert(
        EngineeringMetric::ProcessStartups,
        count_label_metric(trace, "process_start", "process_manager"),
    );
    metrics.insert(
        EngineeringMetric::LspColdStarts,
        MetricReceipt::missing(
            "count",
            "language_intelligence.telemetry",
            "LSP startup telemetry was not attached",
        ),
    );
    metrics.insert(
        EngineeringMetric::BrowserVerificationSuccess,
        explicit_boolean_metric(
            labels,
            "browser_verification_success",
            "browser_verification_failure",
            "browser_trace",
            "browser verification outcome was not observed",
        ),
    );
    metrics.insert(
        EngineeringMetric::RollbackCorrectness,
        explicit_boolean_metric(
            labels,
            "rollback_correctness",
            "rollback_failure",
            "transaction_trace",
            "rollback verification outcome was not observed",
        ),
    );
    metrics.insert(
        EngineeringMetric::CiSuccess,
        explicit_boolean_metric(
            labels,
            "ci_success",
            "ci_failure",
            "ci_trace",
            "CI outcome was not observed",
        ),
    );
    metrics
}

fn token_metric(value: u64, name: &str) -> MetricReceipt {
    if value == 0 {
        MetricReceipt::missing(
            "tokens",
            "behavior_trace.stats",
            format!("{name} usage was not emitted"),
        )
    } else {
        MetricReceipt::observed(value as f64, "tokens", "behavior_trace.stats")
    }
}

fn explicit_boolean_metric(
    labels: &BTreeSet<String>,
    success: &str,
    failure: &str,
    source: &str,
    missing_reason: &str,
) -> MetricReceipt {
    if labels.contains(success) {
        MetricReceipt::observed(1.0, "boolean", source)
    } else if labels.contains(failure) {
        MetricReceipt::observed(0.0, "boolean", source)
    } else {
        MetricReceipt::missing("boolean", source, missing_reason)
    }
}

fn count_label_metric(trace: &BehaviorTrace, operation: &str, source: &str) -> MetricReceipt {
    let count = trace
        .events
        .iter()
        .filter(|event| {
            matches!(
                event,
                BehaviorEvent::CapabilityIdentity { capability: observed }
                    if observed == operation
            )
        })
        .count();
    if count == 0 {
        MetricReceipt::missing("count", source, "process startup count was not observed")
    } else {
        MetricReceipt::observed(count as f64, "count", source)
    }
}

pub const REQUIRED_FEATURE_FLAGS: &[&str] = &[
    "repo_intelligence",
    "language_intelligence",
    "package_intelligence",
    "build_intelligence",
    "git_intelligence",
    "test_impact",
    "change_impact",
    "verification_planner",
    "workspace_snapshots",
    "browser_verification",
    "process_manager",
    "hook_policy",
];

pub fn empty_metrics() -> BTreeMap<EngineeringMetric, MetricReceipt> {
    EngineeringMetric::ALL
        .into_iter()
        .map(|metric| {
            (
                metric,
                MetricReceipt::missing("unknown", "not_recorded", "measurement not collected"),
            )
        })
        .collect()
}
