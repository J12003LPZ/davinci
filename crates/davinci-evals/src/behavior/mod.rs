//! Behavioral reliability evaluation module.

pub mod runner;
pub mod scenario;
pub mod scorer;
pub mod trace;

pub use runner::{aggregate_suite_scores, evaluate_scenario_trace, BehaviorSuiteSummary};
pub use scenario::{
    BehaviorCategory, BehaviorLimits, BehaviorRequirement, BehaviorScenario,
};
pub use scorer::{score_trace, ScoreCard};
pub use trace::{
    classify_shell_command, detect_verification_claims, BehaviorEvent, BehaviorStats, BehaviorTrace,
    VerificationEvent,
};

