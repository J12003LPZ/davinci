//! Cross-subsystem verification evidence bundle connecting tests, security,
//! and graph execution outcomes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityPolicyMode {
    Off,
    Risk,
    Always,
}

impl Default for SecurityPolicyMode {
    fn default() -> Self {
        Self::Risk
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SecurityVerification {
    NotRequired,
    Passed { scan_id: String },
    Failed { scan_id: String, blockers: usize },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationBundle {
    pub commands_ran: usize,
    pub commands_failed: usize,
    pub deterministic_passed: bool,
    pub security: SecurityVerification,
    pub changed_files: Vec<String>,
    pub graph_run_id: Option<String>,
}

impl VerificationBundle {
    /// Pure, deterministic judgment of whether this verification bundle is
    /// eligible for approval. Never invokes a model or inspects subjective review.
    pub fn approval_eligible(&self, mode: SecurityPolicyMode) -> bool {
        // Deterministic commands must run and succeed without failures
        if !self.deterministic_passed || self.commands_ran == 0 || self.commands_failed > 0 {
            return false;
        }

        match mode {
            SecurityPolicyMode::Off => true,
            SecurityPolicyMode::Risk => match &self.security {
                SecurityVerification::NotRequired => true,
                SecurityVerification::Passed { .. } => true,
                SecurityVerification::Failed { .. } => false,
                SecurityVerification::Unavailable { .. } => true, // fail-open with warning in risk mode
            },
            SecurityPolicyMode::Always => match &self.security {
                SecurityVerification::Passed { .. } => true,
                SecurityVerification::NotRequired => false, // scan is mandatory in always mode
                SecurityVerification::Failed { .. } => false,
                SecurityVerification::Unavailable { .. } => false, // fail-closed in always mode
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_failure_never_passes_bundle() {
        let bundle = VerificationBundle {
            commands_ran: 1,
            commands_failed: 1,
            deterministic_passed: false,
            security: SecurityVerification::NotRequired,
            changed_files: vec![],
            graph_run_id: None,
        };
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Risk));
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Off));
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Always));
    }

    #[test]
    fn nothing_ran_never_passes_bundle() {
        let bundle = VerificationBundle {
            commands_ran: 0,
            commands_failed: 0,
            deterministic_passed: false,
            security: SecurityVerification::NotRequired,
            changed_files: vec![],
            graph_run_id: None,
        };
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Risk));
    }

    #[test]
    fn security_failure_blocks_approval_in_risk_and_always_modes() {
        let bundle = VerificationBundle {
            commands_ran: 2,
            commands_failed: 0,
            deterministic_passed: true,
            security: SecurityVerification::Failed {
                scan_id: "scan-1".into(),
                blockers: 1,
            },
            changed_files: vec!["src/auth.rs".into()],
            graph_run_id: Some("run-1".into()),
        };
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Risk));
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Always));
        // Off mode excuses security failures
        assert!(bundle.approval_eligible(SecurityPolicyMode::Off));
    }

    #[test]
    fn security_unavailable_policy_behavior() {
        let bundle = VerificationBundle {
            commands_ran: 1,
            commands_failed: 0,
            deterministic_passed: true,
            security: SecurityVerification::Unavailable {
                reason: "no scanner installed".into(),
            },
            changed_files: vec!["src/auth.rs".into()],
            graph_run_id: Some("run-1".into()),
        };
        // Risk mode fails open on unavailable scanner
        assert!(bundle.approval_eligible(SecurityPolicyMode::Risk));
        // Always mode fails closed on unavailable scanner
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Always));
        // Off mode is eligible
        assert!(bundle.approval_eligible(SecurityPolicyMode::Off));
    }

    #[test]
    fn passed_verification_and_security_is_approval_eligible() {
        let bundle = VerificationBundle {
            commands_ran: 3,
            commands_failed: 0,
            deterministic_passed: true,
            security: SecurityVerification::Passed {
                scan_id: "scan-ok".into(),
            },
            changed_files: vec!["src/main.rs".into()],
            graph_run_id: Some("run-1".into()),
        };
        assert!(bundle.approval_eligible(SecurityPolicyMode::Risk));
        assert!(bundle.approval_eligible(SecurityPolicyMode::Always));
        assert!(bundle.approval_eligible(SecurityPolicyMode::Off));
    }
}
