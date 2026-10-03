//! Behavioral reliability evaluation module.

pub mod artifacts;
pub mod compare;
pub mod context_vm;
pub mod executor;
pub mod gate;
pub mod json_events;
pub mod mutation;
pub mod process;
pub mod runner;
pub mod scenario;
pub mod scorer;
pub mod statistics;
pub mod trace;
pub mod verification;

pub use artifacts::{persist_eval_run, ArtifactHashes, ArtifactRoot, EvalRunManifest};
pub use compare::{
    compare_eval_runs, compare_repeated_runs, format_comparison_markdown,
    format_repeated_comparison_markdown, EvalVariant, PairedComparison, Regression,
    RepeatedPairedComparison,
};
pub use context_vm::{context_vm_fixture_names, run_context_vm_evals, ContextVmEvalResult};
pub use executor::{
    execute_multiturn_synthetic_scenario, execute_scenario, MultiTurnSoakResult, ScenarioRunResult,
    WorkspaceDiff,
};
pub use gate::{evaluate_gate, GateResult, RegressionBudget};
pub use json_events::trace_from_json_lines;
pub use mutation::{audit_dead_prompt_modules, evaluate_ablation, AblationResult, DeadPromptAudit};
pub use process::{run_davinci_process, DavinciProcessConfig, DavinciProcessRun};
pub use runner::{
    aggregate_scenario_results, aggregate_suite_scores, classify_failure_signal,
    classify_process_result, evaluate_scenario_trace, require_scored_runs,
    scheduled_infrastructure_gate, summarize_dispositions, BehaviorSuiteSummary,
    DispositionedSuiteSummary, RunDisposition, RunDispositionSummary,
    MAX_SCHEDULED_INFRASTRUCTURE_FAILURE_RATE,
};
pub use scenario::{
    load_core_200_corpus, load_debugging_hard_corpus, load_long_horizon_corpus,
    load_regression_corpus, load_regression_suite, BehaviorCategory, BehaviorLimits,
    BehaviorRequirement, BehaviorScenario, BehaviorTurn, MultiTurnBehaviorScenario, RegressionCase,
    RegressionOwner, VerificationCommand,
};
pub use scorer::{score_trace, ScoreCard};
pub use statistics::{
    bootstrap_pass_delta_ci95, deterministic_bootstrap_pass_delta_ci, pair_by_scenario_repetition,
    paired_scenario_deltas, scenario_win_tie_loss, PairedObservation, PairedScenarioDelta,
    PairedScenarioObservation, RepeatedMetric, ScenarioObservation, ScenarioRunSample,
    ScenarioWinLoss, ScenarioWinTieLoss, DEFAULT_PROMOTION_REPEATS,
};
pub use trace::{
    classify_shell_command, detect_verification_claims, BehaviorEvent, BehaviorStats,
    BehaviorTrace, FileDiff, VerificationEvent,
};
pub use verification::{
    run_verification_command, run_verification_commands, validate_fixture_command,
    validate_scenario_verification, VerificationResult,
};
