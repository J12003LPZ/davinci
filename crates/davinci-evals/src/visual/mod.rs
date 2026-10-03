//! Deterministic source-level visual design evaluation.

pub mod fingerprint;
pub mod scoring;

pub use fingerprint::{fingerprint_frontend_source, DesignFingerprint};
pub use scoring::{
    score_visual_suite, suite_diversity_score, visual_verification_rate, VisualDiversityReport,
};
