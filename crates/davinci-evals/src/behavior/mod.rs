//! Behavioral reliability evaluation module.

pub mod trace;

pub use trace::{
    classify_shell_command, detect_verification_claims, BehaviorEvent, BehaviorStats, BehaviorTrace,
    VerificationEvent,
};
