use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LearningConfig {
    pub enabled: bool,
    pub background_review: bool,
    pub shadow_mode: bool,
    pub auto_apply_project: bool,
    pub auto_apply_global: bool,
    pub max_candidates_per_review: usize,
    pub max_review_input_tokens: usize,
    pub max_review_iterations: usize,
    pub auto_promote_verified_uses: u64,
    pub review_timeout_ms: u64,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            background_review: true,
            shadow_mode: true,
            auto_apply_project: false,
            auto_apply_global: false,
            max_candidates_per_review: 3,
            max_review_input_tokens: 12_000,
            max_review_iterations: 6,
            auto_promote_verified_uses: 2,
            review_timeout_ms: 30_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_defaults_are_safe() {
        let config = LearningConfig::default();
        assert!(config.enabled);
        assert!(config.background_review);
        assert!(config.shadow_mode);
        assert!(!config.auto_apply_project);
        assert!(!config.auto_apply_global);
        assert_eq!(config.max_candidates_per_review, 3);
        assert_eq!(config.max_review_input_tokens, 12_000);
        assert_eq!(config.max_review_iterations, 6);
        assert_eq!(config.auto_promote_verified_uses, 2);
        assert_eq!(config.review_timeout_ms, 30_000);
    }
}
