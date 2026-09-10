//! Deterministic completion evaluator and dimension verification.

use serde::{Deserialize, Serialize};

use super::contracts::TaskContract;
use super::evidence::DimensionState;
use super::evidence_store::ExecutionReceipt;
use super::ids::{EvidenceId, TaskId};
use super::source_manifest::SourceManifest;
use super::tasks::TaskRecord;

pub fn completion_allowed(dimensions: &[(bool, &str)]) -> bool {
    !dimensions.is_empty()
        && dimensions
            .iter()
            .all(|(required, state)| !required || *state == "passed_current")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionKind {
    Implementation,
    TargetedTests,
    Build,
    InstalledApp,
    LiveUiCheck,
}

impl DimensionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Implementation => "implementation",
            Self::TargetedTests => "targeted_tests",
            Self::Build => "build",
            Self::InstalledApp => "installed_app",
            Self::LiveUiCheck => "live_ui_check",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationClass {
    Implementation,
    Verification,
    Handoff,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetReservation {
    pub token_ceiling: Option<u64>,
    pub tokens_spent: u64,
    pub verification_reserve: u64,
    pub handoff_reserve: u64,
    pub pricing_known: bool,
    pub estimated_cost_usd: Option<f64>,
}

impl Default for BudgetReservation {
    fn default() -> Self {
        Self {
            token_ceiling: None,
            tokens_spent: 0,
            verification_reserve: 0,
            handoff_reserve: 0,
            pricing_known: true,
            estimated_cost_usd: None,
        }
    }
}

impl BudgetReservation {
    pub fn new(
        token_ceiling: Option<u64>,
        tokens_spent: u64,
        verification_reserve: u64,
        handoff_reserve: u64,
        pricing_known: bool,
        estimated_cost_usd: Option<f64>,
    ) -> Self {
        Self {
            token_ceiling,
            tokens_spent,
            verification_reserve,
            handoff_reserve,
            pricing_known,
            estimated_cost_usd,
        }
    }

    /// Checks whether an implementation step can proceed without consuming
    /// the protected verification and handoff reserves.
    pub fn can_reserve_implementation(&self, requested_tokens: u64) -> bool {
        if let Some(ceiling) = self.token_ceiling {
            let protected_reserve = self
                .verification_reserve
                .saturating_add(self.handoff_reserve);
            let available_for_impl = ceiling.saturating_sub(protected_reserve);
            self.tokens_spent.saturating_add(requested_tokens) <= available_for_impl
        } else {
            true
        }
    }

    /// Checks whether verification capacity is preserved.
    pub fn has_verification_reserve(&self) -> bool {
        if let Some(ceiling) = self.token_ceiling {
            let remaining = ceiling.saturating_sub(self.tokens_spent);
            remaining >= self.verification_reserve
        } else {
            true
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DimensionCheck {
    pub dimension: DimensionKind,
    pub requirement_id: Option<String>,
    pub required: bool,
    pub state: DimensionState,
    pub evidence_id: Option<EvidenceId>,
    pub details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionEvaluation {
    pub task_id: TaskId,
    /// Contract digest bound to the evidence evaluation, when the task is
    /// contract-scoped. Completion rejects an evaluation for a different
    /// contract binding.
    #[serde(default)]
    pub contract_digest: Option<String>,
    /// Revision and attempt observed while evidence was evaluated. The task
    /// commit must still match these values before marking it complete.
    #[serde(default)]
    pub evaluated_revision: u64,
    #[serde(default)]
    pub evaluated_attempt: u32,
    pub allowed: bool,
    pub dimensions: Vec<DimensionCheck>,
    pub remaining_gaps: Vec<String>,
    pub source_fingerprint: String,
    pub evaluated_at_ms: i64,
}

/// Evaluates completion dimensions against contract and evidence receipts.
pub fn evaluate_task_completion(
    task: &TaskRecord,
    contract: Option<&TaskContract>,
    receipts: &[ExecutionReceipt],
    current_manifest: &SourceManifest,
    expected_attempt: u32,
) -> CompletionEvaluation {
    evaluate_task_completion_with_budget(
        task,
        contract,
        receipts,
        current_manifest,
        expected_attempt,
        None,
    )
}

/// Evaluates completion dimensions against contract, evidence receipts, and budget reserves.
pub fn evaluate_task_completion_with_budget(
    task: &TaskRecord,
    contract: Option<&TaskContract>,
    receipts: &[ExecutionReceipt],
    current_manifest: &SourceManifest,
    expected_attempt: u32,
    budget: Option<&BudgetReservation>,
) -> CompletionEvaluation {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let mut checks = Vec::new();
    let mut gaps = Vec::new();

    // 1. Implementation: Always required
    let mut impl_passed = false;
    let mut impl_ev = None;
    for r in receipts {
        if r.attempt == expected_attempt
            && r.is_passed()
            && (r.tool_name == "write"
                || r.tool_name == "edit"
                || r.tool_name == "apply_patch"
                || r.tool_name == "implementation")
        {
            impl_passed = true;
            impl_ev = Some(r.receipt_id);
            break;
        }
    }
    // If no explicit mutation tool was used, but task has result or is docs-only, check if any passed receipt exists
    if !impl_passed {
        for r in receipts {
            if r.attempt == expected_attempt && r.is_passed() {
                impl_passed = true;
                impl_ev = Some(r.receipt_id);
                break;
            }
        }
    }
    checks.push(DimensionCheck {
        dimension: DimensionKind::Implementation,
        requirement_id: None,
        required: true,
        state: if impl_passed {
            DimensionState::PassedCurrent
        } else {
            gaps.push("Implementation not verified".into());
            DimensionState::Failed
        },
        evidence_id: impl_ev,
        details: None,
    });

    // 2. Targeted Tests: Check contract verification_requirements
    let test_reqs: Vec<String> = contract
        .map(|c| {
            c.verification_requirements
                .iter()
                .filter(|r| *r != "security" && *r != "security_scan")
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let tests_required = !test_reqs.is_empty();
    if tests_required {
        for req in test_reqs {
            let mut req_passed = false;
            let mut req_ev = None;
            for r in receipts {
                if r.attempt == expected_attempt && r.is_passed() {
                    // Match exact requirement ID if present
                    if r.requirement_id.as_deref() == Some(&req)
                        || (r.tool_name == "cargo test" && req == "tests")
                    {
                        req_passed = true;
                        req_ev = Some(r.receipt_id);
                        break;
                    }
                }
            }
            checks.push(DimensionCheck {
                dimension: DimensionKind::TargetedTests,
                requirement_id: Some(req.clone()),
                required: true,
                state: if req_passed {
                    DimensionState::PassedCurrent
                } else {
                    gaps.push(format!("Required check '{req}' not satisfied"));
                    DimensionState::Failed
                },
                evidence_id: req_ev,
                details: None,
            });
        }
    } else {
        checks.push(DimensionCheck {
            dimension: DimensionKind::TargetedTests,
            requirement_id: None,
            required: false,
            state: DimensionState::NotRequired,
            evidence_id: None,
            details: None,
        });
    }

    // 3. Build dimension: Not required by default unless specified
    let build_required = contract.map_or(false, |c| {
        c.verification_requirements.contains(&"build".to_string())
    });
    if build_required {
        let mut build_passed = false;
        let mut build_ev = None;
        for r in receipts {
            if r.attempt == expected_attempt
                && r.is_passed()
                && (r.tool_name == "cargo build" || r.requirement_id.as_deref() == Some("build"))
            {
                build_passed = true;
                build_ev = Some(r.receipt_id);
                break;
            }
        }
        checks.push(DimensionCheck {
            dimension: DimensionKind::Build,
            requirement_id: Some("build".into()),
            required: true,
            state: if build_passed {
                DimensionState::PassedCurrent
            } else {
                gaps.push("Build not passed".into());
                DimensionState::Failed
            },
            evidence_id: build_ev,
            details: None,
        });
    } else {
        checks.push(DimensionCheck {
            dimension: DimensionKind::Build,
            requirement_id: None,
            required: false,
            state: DimensionState::NotRequired,
            evidence_id: None,
            details: None,
        });
    }

    // 4. Installed App dimension
    let install_required = contract.map_or(false, |c| {
        c.verification_requirements
            .iter()
            .any(|r| r == "install" || r == "installed_app")
    });
    if install_required {
        let mut install_passed = false;
        let mut install_ev = None;
        for r in receipts {
            if r.attempt == expected_attempt
                && r.is_passed()
                && (r.tool_name == "installed_app"
                    || r.requirement_id.as_deref() == Some("installed_app")
                    || r.requirement_id.as_deref() == Some("install"))
            {
                install_passed = true;
                install_ev = Some(r.receipt_id);
                break;
            }
        }
        checks.push(DimensionCheck {
            dimension: DimensionKind::InstalledApp,
            requirement_id: Some("installed_app".into()),
            required: true,
            state: if install_passed {
                DimensionState::PassedCurrent
            } else {
                gaps.push("Installed app verification not satisfied".into());
                DimensionState::Failed
            },
            evidence_id: install_ev,
            details: None,
        });
    } else {
        checks.push(DimensionCheck {
            dimension: DimensionKind::InstalledApp,
            requirement_id: None,
            required: false,
            state: DimensionState::NotRequired,
            evidence_id: None,
            details: None,
        });
    }

    // 5. Live UI Check dimension
    checks.push(DimensionCheck {
        dimension: DimensionKind::LiveUiCheck,
        requirement_id: None,
        required: false,
        state: DimensionState::NotPerformed,
        evidence_id: None,
        details: Some("Physical keyboard interaction not automated".into()),
    });

    // 6. Budget reserve check
    let mut budget_gap = false;
    if let Some(budget) = budget {
        if !budget.has_verification_reserve() {
            gaps.push(
                "Verification reserve breached: insufficient budget remaining for verification"
                    .into(),
            );
            budget_gap = true;
        }
    }

    let allowed = !checks.is_empty()
        && !budget_gap
        && checks
            .iter()
            .all(|c| !c.required || c.state == DimensionState::PassedCurrent);

    CompletionEvaluation {
        task_id: task.id,
        contract_digest: contract.map(|c| c.digest.clone()),
        evaluated_revision: task.revision,
        evaluated_attempt: expected_attempt,
        allowed,
        dimensions: checks,
        remaining_gaps: gaps,
        source_fingerprint: current_manifest.digest.clone(),
        evaluated_at_ms: now,
    }
}

#[cfg(test)]
mod tests {
    use super::super::ids::RunId;
    use super::super::source_manifest::SourceManifestBuilder;
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn f06_required_dimensions() {
        assert!(completion_allowed(&[
            (true, "passed_current"),
            (false, "not_performed")
        ]));
        assert!(!completion_allowed(&[(true, "stale")]));
        assert!(!completion_allowed(&[(true, "unavailable")]));
        assert!(!completion_allowed(&[]));
    }

    #[test]
    fn test_all_skipped_graph_verification() {
        // Receipt from all-skipped verification has started: false, exit_code: None
        let receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "graph_skip".into(),
            tool_name: "graph_verify".into(),
            started: false,
            exit_code: None,
            ..Default::default()
        };

        assert!(!receipt.is_passed());
        assert!(!completion_allowed(&[(true, "not_performed")]));
    }

    #[test]
    fn test_required_security_unavailable() {
        let receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "sec_unavail".into(),
            tool_name: "security_scanner".into(),
            started: false,
            exit_code: None,
            ..Default::default()
        };

        assert!(!receipt.is_passed());
        assert!(!completion_allowed(&[(true, "unavailable")]));
    }

    #[test]
    fn test_pass_from_old_task_attempt() {
        let dir = tempdir().unwrap();
        let manifest = SourceManifestBuilder::new(dir.path()).build().unwrap();
        let task = TaskRecord::new(RunId::new(), "Task with attempt 2");
        let old_attempt_receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "attempt1_receipt".into(),
            task_id: Some(task.id),
            attempt: 1, // Prior attempt
            tool_name: "implementation".into(),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };
        // expected_attempt is 2, old receipt is attempt 1
        let eval = evaluate_task_completion(&task, None, &[old_attempt_receipt], &manifest, 2);
        assert!(!eval.allowed);
    }

    #[test]
    fn test_wrong_requirement_id() {
        let dir = tempdir().unwrap();
        let manifest = SourceManifestBuilder::new(dir.path()).build().unwrap();
        let task = TaskRecord::new(RunId::new(), "Task with contract");
        let contract = TaskContract::new(
            "c_test",
            1,
            task.id,
            1,
            vec!["src/**".into()],
            vec![],
            false,
            vec![],
            vec!["tests_integration".into()],
            vec![],
        )
        .unwrap();

        let wrong_receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "wrong_id".into(),
            task_id: Some(task.id),
            tool_name: "test".into(),
            started: true,
            exit_code: Some(0),
            requirement_id: Some("tests_unit".into()), // Wrong ID!
            ..Default::default()
        };

        let eval = evaluate_task_completion(&task, Some(&contract), &[wrong_receipt], &manifest, 0);
        assert!(!eval.allowed);
        assert!(eval
            .remaining_gaps
            .iter()
            .any(|g| g.contains("tests_integration")));
    }

    #[test]
    fn test_no_dimensions() {
        assert!(!completion_allowed(&[]));
    }

    #[test]
    fn test_explicitly_not_required_build_is_displayed_as_such() {
        let dir = tempdir().unwrap();
        let manifest = SourceManifestBuilder::new(dir.path()).build().unwrap();
        let task = TaskRecord::new(RunId::new(), "Docs only task");
        let receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "doc_edit".into(),
            task_id: Some(task.id),
            tool_name: "edit".into(),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };

        let eval = evaluate_task_completion(&task, None, &[receipt], &manifest, 0);
        assert!(eval.allowed);
        let build_check = eval
            .dimensions
            .iter()
            .find(|d| d.dimension == DimensionKind::Build)
            .unwrap();
        assert!(!build_check.required);
        assert_eq!(build_check.state, DimensionState::NotRequired);
    }

    #[test]
    fn test_budget_reservation_unknown_pricing() {
        let budget = BudgetReservation::new(
            Some(100_000),
            40_000,
            20_000,
            10_000,
            false, // Pricing unknown
            None,
        );

        // Even with unknown pricing, token ceiling and reservation checks are strictly enforced
        assert!(budget.can_reserve_implementation(30_000)); // 40k + 30k = 70k <= 100k - 30k reserve
        assert!(!budget.can_reserve_implementation(35_000)); // 40k + 35k = 75k > 70k available
        assert!(budget.has_verification_reserve()); // 100k - 40k = 60k >= 20k
        assert!(!budget.pricing_known);
        assert_eq!(budget.estimated_cost_usd, None);
    }

    #[test]
    fn test_no_verification_reserve() {
        let budget = BudgetReservation::new(
            Some(100_000),
            85_000, // 85k spent out of 100k
            20_000, // 20k required verification reserve
            5_000,
            true,
            Some(1.50),
        );

        // Only 15k remaining, but 20k is needed for verification reserve
        assert!(!budget.has_verification_reserve());
        assert!(!budget.can_reserve_implementation(1_000));

        // Evaluate task completion with breached verification reserve
        let dir = tempdir().unwrap();
        let manifest = SourceManifestBuilder::new(dir.path()).build().unwrap();
        let task = TaskRecord::new(RunId::new(), "Exhausted budget task");
        let receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "impl_1".into(),
            task_id: Some(task.id),
            tool_name: "implementation".into(),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };

        let eval = evaluate_task_completion_with_budget(
            &task,
            None,
            &[receipt],
            &manifest,
            0,
            Some(&budget),
        );
        assert!(!eval.allowed);
        assert!(eval
            .remaining_gaps
            .iter()
            .any(|g| g.contains("Verification reserve breached")));
    }

    #[test]
    fn test_installed_app_dimension_required_and_satisfied() {
        let dir = tempdir().unwrap();
        let manifest = SourceManifestBuilder::new(dir.path()).build().unwrap();
        let task = TaskRecord::new(RunId::new(), "Release installation task");

        let contract = TaskContract::new(
            "release-install",
            1,
            task.id,
            1,
            vec!["src/main.rs".into()],
            vec![],
            false,
            vec![],
            vec!["installed_app".into()],
            vec![],
        )
        .unwrap();

        // 1. Missing install evidence -> fails
        let impl_receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "impl".into(),
            task_id: Some(task.id),
            tool_name: "implementation".into(),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };
        let eval = evaluate_task_completion(
            &task,
            Some(&contract),
            &[impl_receipt.clone()],
            &manifest,
            0,
        );
        assert!(!eval.allowed);
        let install_check = eval
            .dimensions
            .iter()
            .find(|d| d.dimension == DimensionKind::InstalledApp)
            .unwrap();
        assert!(install_check.required);
        assert_eq!(install_check.state, DimensionState::Failed);

        // 2. With passing installed_app receipt -> passes
        let install_receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "verify_installed".into(),
            task_id: Some(task.id),
            tool_name: "installed_app".into(),
            requirement_id: Some("installed_app".into()),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };
        let eval = evaluate_task_completion(
            &task,
            Some(&contract),
            &[impl_receipt, install_receipt],
            &manifest,
            0,
        );
        assert!(eval.allowed);
        let install_check = eval
            .dimensions
            .iter()
            .find(|d| d.dimension == DimensionKind::InstalledApp)
            .unwrap();
        assert_eq!(install_check.state, DimensionState::PassedCurrent);
    }
}
