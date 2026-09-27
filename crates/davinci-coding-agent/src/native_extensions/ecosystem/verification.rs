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
    /// Digest of the host-owned source inputs used by this receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_manifest_digest: Option<String>,
}

impl VerificationBundle {
    /// Pure, deterministic judgment of whether this verification bundle is
    /// eligible for approval. Never invokes a model or inspects subjective review.
    pub fn approval_eligible(&self, mode: SecurityPolicyMode) -> bool {
        self.approval_eligible_for_contract(None, mode, &[])
    }

    /// Evaluates whether this verification bundle is eligible for approval under an
    /// optional TaskContract, bridging mandatory contract requirements.
    ///
    /// Invariants:
    /// - Deterministic commands must have run and passed without failures.
    /// - If a contract requires security (or mode is Always), an unavailable or missing
    ///   scanner fails closed and blocks completion, even in Risk mode.
    /// - Each named verification requirement in the contract must explicitly match a
    ///   passed check. Extra unrelated passing checks cannot satisfy a named requirement.
    /// - When no contract is active (legacy / uncontracted), standard policy mode rules apply.
    pub fn approval_eligible_for_contract(
        &self,
        contract: Option<&davinci_agent::runtime::contracts::TaskContract>,
        mode: SecurityPolicyMode,
        passed_named_checks: &[&str],
    ) -> bool {
        // Deterministic commands must run and succeed without failures
        if !self.deterministic_passed || self.commands_ran == 0 || self.commands_failed > 0 {
            return false;
        }

        let security_required = contract.map_or(false, |c| {
            c.verification_requirements
                .iter()
                .any(|r| r == "security" || r == "security_scan")
        });

        // Evaluate security verification
        if security_required {
            // A contract-required security verification fails closed on unavailable, missing, or failed.
            // It MUST NOT inherit Risk mode's lenient fail-open behavior for optional evidence.
            let security_state = match &self.security {
                SecurityVerification::Passed { .. } => "passed",
                SecurityVerification::Failed { .. } => "failed",
                SecurityVerification::Unavailable { .. } => "unavailable",
                SecurityVerification::NotRequired => "not_required",
            };
            if !davinci_agent::runtime::contracts::required_check_satisfied(true, security_state) {
                return false;
            }
        } else {
            // Fall back to standard mode behavior for optional security
            let mode_eligible = match mode {
                SecurityPolicyMode::Off => true,
                SecurityPolicyMode::Risk => match &self.security {
                    SecurityVerification::NotRequired => true,
                    SecurityVerification::Passed { .. } => true,
                    SecurityVerification::Failed { .. } => false,
                    SecurityVerification::Unavailable { .. } => true, // fail-open with warning in risk mode
                },
                SecurityPolicyMode::Always => match &self.security {
                    SecurityVerification::Passed { .. } => true,
                    SecurityVerification::NotRequired => false,
                    SecurityVerification::Failed { .. } => false,
                    SecurityVerification::Unavailable { .. } => false, // fail-closed in always mode
                },
            };
            if !mode_eligible {
                return false;
            }
        }

        // Evaluate any non-security named verification requirements in the contract
        if let Some(c) = contract {
            for req in &c.verification_requirements {
                if req == "security" || req == "security_scan" {
                    continue;
                }
                let passed = passed_named_checks.iter().any(|check| *check == req);
                let check_state = if passed { "passed" } else { "missing" };
                if !davinci_agent::runtime::contracts::required_check_satisfied(true, check_state) {
                    return false;
                }
            }
        }

        true
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
            source_manifest_digest: None,
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
            source_manifest_digest: None,
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
            source_manifest_digest: None,
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
            source_manifest_digest: None,
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
            source_manifest_digest: None,
        };
        assert!(bundle.approval_eligible(SecurityPolicyMode::Risk));
        assert!(bundle.approval_eligible(SecurityPolicyMode::Always));
        assert!(bundle.approval_eligible(SecurityPolicyMode::Off));
    }

    #[test]
    fn f05_required_unavailable_security_blocks_in_risk_mode() {
        let bundle = VerificationBundle {
            commands_ran: 1,
            commands_failed: 0,
            deterministic_passed: true,
            security: SecurityVerification::Unavailable {
                reason: "scanner missing".into(),
            },
            changed_files: vec!["src/auth.rs".into()],
            graph_run_id: Some("run-1".into()),
            source_manifest_digest: None,
        };
        // Standard risk mode without contract requirement permits unavailable security
        assert!(bundle.approval_eligible(SecurityPolicyMode::Risk));

        // When a contract explicitly requires security, unavailable security MUST fail closed
        let contract = davinci_agent::runtime::contracts::TaskContract::new(
            "contract-1",
            1,
            davinci_agent::TaskId::new(),
            1,
            vec!["src/auth.rs".into()],
            vec![],
            false,
            vec![],
            vec!["security".into()],
            vec![],
        )
        .unwrap();

        assert!(!bundle.approval_eligible_for_contract(
            Some(&contract),
            SecurityPolicyMode::Risk,
            &[]
        ));
    }

    #[test]
    fn f05_extra_unrelated_passed_tests_cannot_satisfy_named_requirement() {
        let bundle = VerificationBundle {
            commands_ran: 2,
            commands_failed: 0,
            deterministic_passed: true,
            security: SecurityVerification::NotRequired,
            changed_files: vec!["src/lib.rs".into()],
            graph_run_id: Some("run-1".into()),
            source_manifest_digest: None,
        };

        let contract = davinci_agent::runtime::contracts::TaskContract::new(
            "contract-2",
            1,
            davinci_agent::TaskId::new(),
            1,
            vec!["src/lib.rs".into()],
            vec![],
            false,
            vec![],
            vec!["cargo_test_auth".into()],
            vec![],
        )
        .unwrap();

        // Extra unrelated passing test ("cargo_test_unrelated") must not satisfy "cargo_test_auth"
        assert!(!bundle.approval_eligible_for_contract(
            Some(&contract),
            SecurityPolicyMode::Risk,
            &["cargo_test_unrelated"]
        ));

        // When the exact required test is provided as passed, approval succeeds
        assert!(bundle.approval_eligible_for_contract(
            Some(&contract),
            SecurityPolicyMode::Risk,
            &["cargo_test_auth", "cargo_test_unrelated"]
        ));
    }

    #[test]
    fn f05_missing_required_security_scanner_blocks() {
        let bundle = VerificationBundle {
            commands_ran: 1,
            commands_failed: 0,
            deterministic_passed: true,
            security: SecurityVerification::Unavailable {
                reason: "no security scanner binary installed on system".into(),
            },
            changed_files: vec!["src/token.rs".into()],
            graph_run_id: None,
            source_manifest_digest: None,
        };

        let contract = davinci_agent::runtime::contracts::TaskContract::new(
            "contract-sec",
            1,
            davinci_agent::TaskId::new(),
            1,
            vec!["src/token.rs".into()],
            vec![],
            false,
            vec![],
            vec!["security_scan".into()],
            vec![],
        )
        .unwrap();

        assert!(!bundle.approval_eligible_for_contract(
            Some(&contract),
            SecurityPolicyMode::Off,
            &[]
        ));
        assert!(!bundle.approval_eligible_for_contract(
            Some(&contract),
            SecurityPolicyMode::Risk,
            &[]
        ));
        assert!(!bundle.approval_eligible_for_contract(
            Some(&contract),
            SecurityPolicyMode::Always,
            &[]
        ));
    }
}
