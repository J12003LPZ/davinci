//! Behavioral reliability evaluation module.

pub mod compare;
pub mod gate;
pub mod mutation;
pub mod runner;
pub mod scenario;
pub mod scorer;
pub mod trace;

pub use compare::{
    compare_eval_runs, format_comparison_markdown, EvalVariant, PairedComparison, Regression,
};
pub use gate::{evaluate_gate, GateResult, RegressionBudget};
pub use mutation::{audit_dead_prompt_modules, evaluate_ablation, AblationResult, DeadPromptAudit};
pub use runner::{aggregate_suite_scores, evaluate_scenario_trace, BehaviorSuiteSummary};
pub use scenario::{
    load_core_200_corpus, BehaviorCategory, BehaviorLimits, BehaviorRequirement, BehaviorScenario,
};
pub use scorer::{score_trace, ScoreCard};
pub use trace::{
    classify_shell_command, detect_verification_claims, BehaviorEvent, BehaviorStats,
    BehaviorTrace, VerificationEvent,
};
