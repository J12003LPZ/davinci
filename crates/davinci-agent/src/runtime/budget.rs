//! Root resource budget and ledger for whole-task resource bounding.
//! Matching spec: docs/superpowers/plans/2026-09-07-davinci-feature-specs/09-whole-task-budgets-loop-detection.md

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::runtime::ids::{AgentId, RunId};

/// Contract helper: returns remaining implementation allowance after reserves.
pub fn implementation_available(
    total: u64,
    charged: u64,
    verify: u64,
    handoff: u64,
) -> Option<u64> {
    total
        .checked_sub(charged)?
        .checked_sub(verify)?
        .checked_sub(handoff)
}

/// Settle an attempt once idempotently.
pub fn settle_once(seen: &mut std::collections::BTreeSet<String>, attempt: &str) -> bool {
    seen.insert(attempt.to_owned())
}

/// Contract helper: evaluates budget decisions and transitions.
pub fn budget_decision(remaining: u64, paused: bool, choice: &str) -> &'static str {
    if remaining == 0 {
        return "hard_stop";
    }
    if !paused {
        return "running";
    }
    match choice {
        "continue" => "continue",
        "plan" => "return_to_plan",
        "stop" => "stop_checkpoint",
        _ => "paused",
    }
}

/// Purpose classification for resource reservations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationPurpose {
    Implementation,
    Verification,
    Handoff,
}

/// Monetary cost with currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub amount_minor_units: u64,
    pub currency: String,
}

/// Cost amount that can be explicitly known or marked unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostAmount {
    Known(u64),
    Unknown,
}

/// Whole-task resource budget configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudget {
    pub root_run_id: RunId,
    pub revision: u64,
    pub token_ceiling: u64,
    pub deadline: Duration,
    pub max_concurrency: usize,
    pub retry_ceiling: u32,
    pub cost_ceiling: Option<Money>,
    pub verification_reserve: u64,
    pub handoff_reserve: u64,
}

impl ResourceBudget {
    /// Create a budget with explicit parameters and reserve validation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root_run_id: RunId,
        revision: u64,
        token_ceiling: u64,
        deadline: Duration,
        max_concurrency: usize,
        retry_ceiling: u32,
        cost_ceiling: Option<Money>,
        verification_reserve: u64,
        handoff_reserve: u64,
    ) -> Result<Self, BudgetError> {
        let required_reserves = verification_reserve
            .checked_add(handoff_reserve)
            .ok_or(BudgetError::IntegerOverflow)?;

        if token_ceiling < required_reserves {
            return Err(BudgetError::CeilingBelowRequiredReserves {
                ceiling: token_ceiling,
                required: required_reserves,
            });
        }

        Ok(Self {
            root_run_id,
            revision,
            token_ceiling,
            deadline,
            max_concurrency: max_concurrency.max(1),
            retry_ceiling,
            cost_ceiling,
            verification_reserve,
            handoff_reserve,
        })
    }

    /// Create standard default budget for a run with 15% verification and 5% handoff reserve.
    pub fn default_for_run(root_run_id: RunId, token_ceiling: u64) -> Result<Self, BudgetError> {
        let verification_reserve = token_ceiling.saturating_mul(15) / 100;
        let handoff_reserve = token_ceiling.saturating_mul(5) / 100;
        Self::new(
            root_run_id,
            1,
            token_ceiling,
            Duration::from_secs(15 * 60),
            4,
            5,
            None,
            verification_reserve,
            handoff_reserve,
        )
    }
}

/// Active resource reservation before dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reservation {
    pub id: String,
    pub attempt_id: String,
    pub purpose: ReservationPurpose,
    pub max_input: u64,
    pub max_output: u64,
    pub remaining_time: Duration,
    pub worker_slot: Option<usize>,
}

/// Terminal receipt reconciling an attempt's usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageReceipt {
    pub attempt_id: String,
    pub provider_usage: u64,
    pub estimated_usage: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub cost: CostAmount,
    pub finished_at: u64,
}

/// Bounded authenticated lease granted to child tasks/workers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildBudgetLease {
    pub root_run_id: RunId,
    pub lease_id: String,
    pub agent_id: AgentId,
    pub token_limit: u64,
    pub granted_at_ms: u64,
}

/// Snapshot of whole-task resource consumption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    pub root_run_id: RunId,
    pub revision: u64,
    pub token_ceiling: u64,
    pub tokens_charged: u64,
    pub tokens_reserved: u64,
    pub verification_reserve: u64,
    pub handoff_reserve: u64,
    pub implementation_remaining: Option<u64>,
    pub elapsed: Duration,
    pub deadline: Duration,
    pub active_workers: usize,
    pub max_concurrency: usize,
    pub retries_used: u32,
    pub retry_ceiling: u32,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub cost_minor_units: Option<u64>,
    pub has_unknown_cost: bool,
}

/// Budget errors enforcing bounds and fail-closed security.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BudgetError {
    #[error("Integer overflow calculating budget")]
    IntegerOverflow,
    #[error("Budget ceiling {ceiling} cannot satisfy required reserves {required}")]
    CeilingBelowRequiredReserves { ceiling: u64, required: u64 },
    #[error("Token budget exhausted: requested {requested}, available {available}")]
    BudgetExhausted { requested: u64, available: u64 },
    #[error("Retry ceiling {ceiling} reached")]
    RetryCeilingReached { ceiling: u32 },
    #[error("Concurrency ceiling {ceiling} reached")]
    ConcurrencyCeilingReached { ceiling: usize },
    #[error("Deadline of {deadline:?} expired")]
    DeadlineExpired { deadline: Duration },
    #[error("Verification cannot spend handoff reserve (limit: {limit}, requested: {requested})")]
    CannotSpendHandoffReserve { limit: u64, requested: u64 },
    #[error("Attempt {0} already settled")]
    AlreadySettled(String),
    #[error("Reservation {0} not found")]
    ReservationNotFound(String),
    #[error("Child attempted to create root ledger for existing task without lease")]
    RecursionWithoutRootLease,
    #[error("Revision mismatch: expected {expected}, actual {actual}")]
    RevisionMismatch { expected: u64, actual: u64 },
    #[error("Cost ceiling exceeded: {reason}")]
    CostCeilingExceeded { reason: String },
    #[error("Unauthorized ceiling update: host authorization required")]
    UnauthorizedCeilingUpdate,
}

#[derive(Debug)]
struct LedgerState {
    budget: ResourceBudget,
    active_reservations: HashMap<String, Reservation>,
    settled_attempts: BTreeSet<String>,
    receipts: Vec<UsageReceipt>,
    charged_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    tokens_by_purpose: HashMap<ReservationPurpose, u64>,
    cost_minor_units: u64,
    has_unknown_cost: bool,
    retries_used: u32,
    active_workers: usize,
    child_leases: HashMap<String, ChildBudgetLease>,
}

/// Durable root resource ledger tracking usage across parent, descendants, retries, and reviews.
pub struct ResourceLedger {
    root_run_id: RunId,
    start_instant: Instant,
    state: Mutex<LedgerState>,
    worker_changed: Condvar,
}

impl ResourceLedger {
    pub fn new(budget: ResourceBudget) -> Arc<Self> {
        let root_run_id = budget.root_run_id;
        Arc::new(Self {
            root_run_id,
            start_instant: Instant::now(),
            state: Mutex::new(LedgerState {
                budget,
                active_reservations: HashMap::new(),
                settled_attempts: BTreeSet::new(),
                receipts: Vec::new(),
                charged_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                tokens_by_purpose: HashMap::new(),
                cost_minor_units: 0,
                has_unknown_cost: false,
                retries_used: 0,
                active_workers: 0,
                child_leases: HashMap::new(),
            }),
            worker_changed: Condvar::new(),
        })
    }

    pub fn root_run_id(&self) -> RunId {
        self.root_run_id
    }

    pub fn current_budget(&self) -> ResourceBudget {
        self.state.lock().unwrap().budget.clone()
    }

    pub fn snapshot(&self) -> BudgetSnapshot {
        let state = self.state.lock().unwrap();
        let elapsed = self.start_instant.elapsed();
        let tokens_reserved: u64 = state
            .active_reservations
            .values()
            .map(|r| r.max_input.saturating_add(r.max_output))
            .sum();

        let committed = state.charged_tokens.saturating_add(tokens_reserved);
        let impl_rem = implementation_available(
            state.budget.token_ceiling,
            committed,
            state.budget.verification_reserve,
            state.budget.handoff_reserve,
        );

        BudgetSnapshot {
            root_run_id: self.root_run_id,
            revision: state.budget.revision,
            token_ceiling: state.budget.token_ceiling,
            tokens_charged: state.charged_tokens,
            tokens_reserved,
            verification_reserve: state.budget.verification_reserve,
            handoff_reserve: state.budget.handoff_reserve,
            implementation_remaining: impl_rem,
            elapsed,
            deadline: state.budget.deadline,
            active_workers: state.active_workers,
            max_concurrency: state.budget.max_concurrency,
            retries_used: state.retries_used,
            retry_ceiling: state.budget.retry_ceiling,
            cache_read_tokens: state.cache_read_tokens,
            cache_write_tokens: state.cache_write_tokens,
            cost_minor_units: if state.has_unknown_cost {
                None
            } else {
                Some(state.cost_minor_units)
            },
            has_unknown_cost: state.has_unknown_cost,
        }
    }

    /// Atomically reserve capacity before worker or provider dispatch.
    pub fn reserve(
        &self,
        attempt_id: &str,
        purpose: ReservationPurpose,
        max_input: u64,
        max_output: u64,
    ) -> Result<Reservation, BudgetError> {
        let requested = max_input
            .checked_add(max_output)
            .ok_or(BudgetError::IntegerOverflow)?;

        let mut state = self.state.lock().unwrap();

        if self.start_instant.elapsed() > state.budget.deadline {
            return Err(BudgetError::DeadlineExpired {
                deadline: state.budget.deadline,
            });
        }

        if let Some(ceiling) = &state.budget.cost_ceiling {
            if state.has_unknown_cost {
                return Err(BudgetError::CostCeilingExceeded {
                    reason:
                        "Conservative refusal: cost is unknown with active hard monetary ceiling"
                            .into(),
                });
            }
            if state.cost_minor_units >= ceiling.amount_minor_units {
                return Err(BudgetError::CostCeilingExceeded {
                    reason: format!(
                        "Hard monetary ceiling reached: {}/{}",
                        state.cost_minor_units, ceiling.amount_minor_units
                    ),
                });
            }
        }

        let currently_reserved: u64 = state
            .active_reservations
            .values()
            .map(|r| r.max_input.saturating_add(r.max_output))
            .sum();

        let committed = state
            .charged_tokens
            .checked_add(currently_reserved)
            .ok_or(BudgetError::IntegerOverflow)?;

        match purpose {
            ReservationPurpose::Implementation => {
                let available = implementation_available(
                    state.budget.token_ceiling,
                    committed,
                    state.budget.verification_reserve,
                    state.budget.handoff_reserve,
                )
                .unwrap_or(0);

                if requested > available {
                    return Err(BudgetError::BudgetExhausted {
                        requested,
                        available,
                    });
                }
            }
            ReservationPurpose::Verification => {
                // Verification can spend its own reserve and unused implementation,
                // but cannot spend handoff reserve.
                let max_spendable = state
                    .budget
                    .token_ceiling
                    .saturating_sub(state.budget.handoff_reserve);
                let available = max_spendable.saturating_sub(committed);

                if requested > available {
                    return Err(BudgetError::CannotSpendHandoffReserve {
                        limit: available,
                        requested,
                    });
                }
            }
            ReservationPurpose::Handoff => {
                let available = state.budget.token_ceiling.saturating_sub(committed);
                if requested > available {
                    return Err(BudgetError::BudgetExhausted {
                        requested,
                        available,
                    });
                }
            }
        }

        let reservation_id = format!("res_{}", uuid::Uuid::new_v4());
        let remaining_time = state
            .budget
            .deadline
            .checked_sub(self.start_instant.elapsed())
            .unwrap_or(Duration::ZERO);

        let reservation = Reservation {
            id: reservation_id.clone(),
            attempt_id: attempt_id.to_string(),
            purpose,
            max_input,
            max_output,
            remaining_time,
            worker_slot: None,
        };

        state
            .active_reservations
            .insert(reservation_id, reservation.clone());
        Ok(reservation)
    }

    /// Cancel a reservation if an attempt is aborted before dispatch.
    pub fn cancel_reservation(&self, reservation_id: &str) {
        let mut state = self.state.lock().unwrap();
        state.active_reservations.remove(reservation_id);
    }

    /// Reconcile actual usage idempotently by attempt ID.
    pub fn settle_receipt(
        &self,
        reservation_id: Option<&str>,
        receipt: UsageReceipt,
    ) -> Result<bool, BudgetError> {
        let mut state = self.state.lock().unwrap();

        if !state.settled_attempts.insert(receipt.attempt_id.clone()) {
            // Already settled idempotently
            return Ok(false);
        }

        let purpose = if let Some(res_id) = reservation_id {
            state
                .active_reservations
                .remove(res_id)
                .map(|r| r.purpose)
                .unwrap_or(ReservationPurpose::Implementation)
        } else {
            ReservationPurpose::Implementation
        };

        state.charged_tokens = state
            .charged_tokens
            .checked_add(receipt.provider_usage)
            .unwrap_or(u64::MAX);

        state.cache_read_tokens = state
            .cache_read_tokens
            .saturating_add(receipt.cache_read_tokens);
        state.cache_write_tokens = state
            .cache_write_tokens
            .saturating_add(receipt.cache_write_tokens);

        *state.tokens_by_purpose.entry(purpose).or_insert(0) = state
            .tokens_by_purpose
            .get(&purpose)
            .copied()
            .unwrap_or(0)
            .saturating_add(receipt.provider_usage);

        match receipt.cost {
            CostAmount::Known(minor) => {
                state.cost_minor_units = state.cost_minor_units.saturating_add(minor);
            }
            CostAmount::Unknown => {
                state.has_unknown_cost = true;
            }
        }

        state.receipts.push(receipt);
        Ok(true)
    }

    /// Reconcile an attempt that crashed, aborted, or timed out after dispatch before receipt.
    /// Retains conservative reserved exposure and marks cost unknown.
    pub fn reconcile_failed_attempt(
        &self,
        reservation_id: Option<&str>,
        attempt_id: &str,
        conservative_tokens: Option<u64>,
    ) -> Result<bool, BudgetError> {
        let mut state = self.state.lock().unwrap();
        if !state.settled_attempts.insert(attempt_id.to_string()) {
            return Ok(false);
        }

        let tokens = if let Some(res_id) = reservation_id {
            if let Some(res) = state.active_reservations.remove(res_id) {
                conservative_tokens.unwrap_or_else(|| res.max_input.saturating_add(res.max_output))
            } else {
                conservative_tokens.unwrap_or(0)
            }
        } else {
            conservative_tokens.unwrap_or(0)
        };

        state.charged_tokens = state.charged_tokens.saturating_add(tokens);
        state.has_unknown_cost = true;
        Ok(true)
    }

    /// Record a provider or worker retry attempt against the retry ceiling.
    pub fn record_retry(&self) -> Result<u32, BudgetError> {
        let mut state = self.state.lock().unwrap();
        if state.retries_used >= state.budget.retry_ceiling {
            return Err(BudgetError::RetryCeilingReached {
                ceiling: state.budget.retry_ceiling,
            });
        }
        state.retries_used += 1;
        Ok(state.retries_used)
    }

    /// Acquire an atomic worker concurrency slot.
    pub fn acquire_worker_slot(self: &Arc<Self>) -> Result<WorkerSlotGuard, BudgetError> {
        let mut state = self.state.lock().unwrap();
        if state.active_workers >= state.budget.max_concurrency {
            return Err(BudgetError::ConcurrencyCeilingReached {
                ceiling: state.budget.max_concurrency,
            });
        }
        state.active_workers += 1;
        Ok(WorkerSlotGuard {
            ledger: Arc::clone(self),
        })
    }

    /// Create a bounded child lease for a delegated subagent/child task.
    pub fn create_child_lease(
        &self,
        agent_id: AgentId,
        token_limit: u64,
    ) -> Result<ChildBudgetLease, BudgetError> {
        let mut state = self.state.lock().unwrap();
        let currently_reserved: u64 = state
            .active_reservations
            .values()
            .map(|r| r.max_input.saturating_add(r.max_output))
            .sum();

        let committed = state.charged_tokens.saturating_add(currently_reserved);

        let available = implementation_available(
            state.budget.token_ceiling,
            committed,
            state.budget.verification_reserve,
            state.budget.handoff_reserve,
        )
        .unwrap_or(0);

        if token_limit > available {
            return Err(BudgetError::BudgetExhausted {
                requested: token_limit,
                available,
            });
        }

        let lease_id = format!("lease_{}", uuid::Uuid::new_v4());
        let lease = ChildBudgetLease {
            root_run_id: self.root_run_id,
            lease_id: lease_id.clone(),
            agent_id,
            token_limit,
            granted_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };

        state.child_leases.insert(lease_id, lease.clone());
        Ok(lease)
    }

    /// Update ceilings with revision checking and host authorization.
    pub fn update_ceilings(
        &self,
        expected_revision: u64,
        new_token_ceiling: Option<u64>,
        new_deadline: Option<Duration>,
        authorized_by_host: bool,
    ) -> Result<BudgetSnapshot, BudgetError> {
        if !authorized_by_host {
            return Err(BudgetError::UnauthorizedCeilingUpdate);
        }
        let mut state = self.state.lock().unwrap();
        if state.budget.revision != expected_revision {
            return Err(BudgetError::RevisionMismatch {
                expected: expected_revision,
                actual: state.budget.revision,
            });
        }
        if let Some(new_tokens) = new_token_ceiling {
            let required_reserves = state
                .budget
                .verification_reserve
                .saturating_add(state.budget.handoff_reserve);
            if new_tokens < required_reserves {
                return Err(BudgetError::CeilingBelowRequiredReserves {
                    ceiling: new_tokens,
                    required: required_reserves,
                });
            }
            state.budget.token_ceiling = new_tokens;
        }
        if let Some(new_dur) = new_deadline {
            state.budget.deadline = new_dur;
        }
        state.budget.revision += 1;
        drop(state);
        Ok(self.snapshot())
    }

    /// Internal slot release invoked when a WorkerSlotGuard drops.
    fn release_worker_slot(&self) {
        let mut state = self.state.lock().unwrap();
        state.active_workers = state.active_workers.saturating_sub(1);
        self.worker_changed.notify_all();
    }
}

/// RAII guard releasing a worker concurrency slot on drop, even during panics or cancellations.
pub struct WorkerSlotGuard {
    ledger: Arc<ResourceLedger>,
}

impl Drop for WorkerSlotGuard {
    fn drop(&mut self) {
        self.ledger.release_worker_slot();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f09_implementation_preserves_reserve() {
        assert_eq!(implementation_available(100, 50, 20, 10), Some(20));
        assert_eq!(implementation_available(100, 90, 20, 10), None);
        assert_eq!(implementation_available(100, u64::MAX, 20, 10), None);
    }

    #[test]
    fn test_huge_integer_overflow() {
        assert_eq!(implementation_available(100, u64::MAX, 20, 10), None);
        assert_eq!(implementation_available(100, 50, u64::MAX, 10), None);
        assert_eq!(implementation_available(100, 50, 20, u64::MAX), None);

        let err = ResourceBudget::new(
            RunId::new(),
            1,
            100,
            Duration::from_secs(60),
            4,
            5,
            None,
            u64::MAX,
            10,
        );
        assert_eq!(err, Err(BudgetError::IntegerOverflow));
    }

    #[test]
    fn test_zero_budget_and_ceiling_below_reserves() {
        // Zero budget with non-zero reserves fails
        let err = ResourceBudget::new(
            RunId::new(),
            1,
            0,
            Duration::from_secs(60),
            4,
            5,
            None,
            10,
            5,
        );
        assert_eq!(
            err,
            Err(BudgetError::CeilingBelowRequiredReserves {
                ceiling: 0,
                required: 15
            })
        );

        // Verification larger than ceiling fails
        let err2 = ResourceBudget::new(
            RunId::new(),
            1,
            100,
            Duration::from_secs(60),
            4,
            5,
            None,
            120,
            10,
        );
        assert_eq!(
            err2,
            Err(BudgetError::CeilingBelowRequiredReserves {
                ceiling: 100,
                required: 130
            })
        );
    }

    #[test]
    fn f09_charge_once() {
        let mut seen = std::collections::BTreeSet::new();
        assert!(settle_once(&mut seen, "attempt-1"));
        assert!(!settle_once(&mut seen, "attempt-1"));
        assert!(settle_once(&mut seen, "attempt-2"));
    }

    #[test]
    fn test_concurrent_reservation_race() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            10,
            10,
            None,
            200,
            100,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        let mut handles = Vec::new();
        for i in 0..10 {
            let l = Arc::clone(&ledger);
            handles.push(std::thread::spawn(move || {
                let mut granted = 0;
                for j in 0..20 {
                    let attempt = format!("thread_{i}_attempt_{j}");
                    if l.reserve(&attempt, ReservationPurpose::Implementation, 50, 50)
                        .is_ok()
                    {
                        granted += 1;
                    }
                }
                granted
            }));
        }

        let total_granted: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        // Implementation capacity is 1000 - 200 - 100 = 700.
        // Each reservation requests 50 + 50 = 100 tokens.
        // Exactly 7 reservations should succeed!
        assert_eq!(total_granted, 7);
    }

    #[test]
    fn test_verification_cannot_spend_handoff_reserve_silently() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            100,
            Duration::from_secs(60),
            4,
            5,
            None,
            20,
            10, // Handoff reserve is 10
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        // Implementation spends 70 tokens (all allowed implementation)
        let res1 = ledger
            .reserve("impl", ReservationPurpose::Implementation, 35, 35)
            .unwrap();
        assert_eq!(res1.max_input + res1.max_output, 70);

        // Verification attempts to reserve 25 tokens (available is 100 - 10 - 70 = 20)
        let err = ledger.reserve("verif", ReservationPurpose::Verification, 15, 10);
        assert!(matches!(
            err,
            Err(BudgetError::CannotSpendHandoffReserve {
                limit: 20,
                requested: 25
            })
        ));

        // Verification reserving exactly 20 tokens succeeds
        let res2 = ledger.reserve("verif", ReservationPurpose::Verification, 10, 10);
        assert!(res2.is_ok());

        // Now even 1 token for verification must fail because handoff reserve of 10 cannot be spent
        let err2 = ledger.reserve("verif_overflow", ReservationPurpose::Verification, 1, 0);
        assert!(matches!(
            err2,
            Err(BudgetError::CannotSpendHandoffReserve {
                limit: 0,
                requested: 1
            })
        ));
    }

    #[test]
    fn test_slot_release_after_failure() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            2, // Max concurrency 2
            5,
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        let slot1 = ledger.acquire_worker_slot().unwrap();
        let slot2 = ledger.acquire_worker_slot().unwrap();

        // Third slot fails
        assert_eq!(
            ledger.acquire_worker_slot().err(),
            Some(BudgetError::ConcurrencyCeilingReached { ceiling: 2 })
        );

        // Drop one slot (simulating worker termination or panic)
        drop(slot1);

        // Now acquiring a slot succeeds again
        let slot3 = ledger.acquire_worker_slot();
        assert!(slot3.is_ok());

        drop(slot2);
        drop(slot3);
        assert_eq!(ledger.snapshot().active_workers, 0);
    }

    #[test]
    fn test_recursion_child_lease_and_root_binding() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            4,
            5,
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        let child_id = AgentId::new();
        let lease = ledger.create_child_lease(child_id, 300).unwrap();
        assert_eq!(lease.root_run_id, ledger.root_run_id());
        assert_eq!(lease.token_limit, 300);

        // Requesting more than available implementation fails
        let err = ledger.create_child_lease(AgentId::new(), 900);
        assert!(matches!(err, Err(BudgetError::BudgetExhausted { .. })));
    }

    #[test]
    fn test_duplicate_callback_idempotency() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            4,
            5,
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);
        let receipt = UsageReceipt {
            attempt_id: "attempt_cb_1".to_string(),
            provider_usage: 100,
            estimated_usage: 100,
            cache_read_tokens: 20,
            cache_write_tokens: 10,
            cost: CostAmount::Known(5),
            finished_at: 1000,
        };

        assert!(ledger.settle_receipt(None, receipt.clone()).unwrap());
        // Duplicate callback must be ignored idempotently without double-counting
        assert!(!ledger.settle_receipt(None, receipt).unwrap());
        let snap = ledger.snapshot();
        assert_eq!(snap.tokens_charged, 100);
        assert_eq!(snap.cache_read_tokens, 20);
        assert_eq!(snap.cache_write_tokens, 10);
        assert_eq!(snap.cost_minor_units, Some(5));
    }

    #[test]
    fn test_retry_with_no_usage() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            4,
            2, // retry ceiling 2
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        assert_eq!(ledger.record_retry().unwrap(), 1);
        assert_eq!(ledger.record_retry().unwrap(), 2);
        assert_eq!(
            ledger.record_retry().err(),
            Some(BudgetError::RetryCeilingReached { ceiling: 2 })
        );
        // Retries with 0 usage don't inflate charged tokens
        let snap = ledger.snapshot();
        assert_eq!(snap.tokens_charged, 0);
        assert_eq!(snap.retries_used, 2);
    }

    #[test]
    fn test_cached_tokens_without_double_counting() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            4,
            5,
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);
        let receipt = UsageReceipt {
            attempt_id: "attempt_cached".to_string(),
            provider_usage: 300,
            estimated_usage: 300,
            cache_read_tokens: 150,
            cache_write_tokens: 50,
            cost: CostAmount::Known(10),
            finished_at: 2000,
        };
        ledger.settle_receipt(None, receipt).unwrap();

        let snap = ledger.snapshot();
        assert_eq!(snap.tokens_charged, 300);
        assert_eq!(snap.cache_read_tokens, 150);
        assert_eq!(snap.cache_write_tokens, 50);
    }

    #[test]
    fn test_review_child_and_background_learning_charges_root() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            4,
            5,
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        // Verification / review child reservation
        let res = ledger
            .reserve("review_node_1", ReservationPurpose::Verification, 50, 50)
            .unwrap();
        assert_eq!(res.purpose, ReservationPurpose::Verification);

        let receipt = UsageReceipt {
            attempt_id: "review_node_1".to_string(),
            provider_usage: 80,
            estimated_usage: 100,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost: CostAmount::Known(2),
            finished_at: 3000,
        };
        ledger.settle_receipt(Some(&res.id), receipt).unwrap();

        let snap = ledger.snapshot();
        assert_eq!(snap.tokens_charged, 80);
        assert_eq!(snap.tokens_reserved, 0);
    }

    #[test]
    fn test_missing_pricing_retains_unknown_cost() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            4,
            5,
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        let receipt = UsageReceipt {
            attempt_id: "unknown_pricing_call".to_string(),
            provider_usage: 200,
            estimated_usage: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost: CostAmount::Unknown,
            finished_at: 4000,
        };
        ledger.settle_receipt(None, receipt).unwrap();

        let snap = ledger.snapshot();
        assert_eq!(snap.tokens_charged, 200);
        assert!(snap.has_unknown_cost);
        assert_eq!(snap.cost_minor_units, None);
    }

    #[test]
    fn test_provider_overrun_beyond_reservation_blocks_further_work() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            500,
            Duration::from_secs(60),
            4,
            5,
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        // Implementation capacity is 500 - 100 - 50 = 350.
        // Reserved 200 tokens
        let res = ledger
            .reserve("overrun_call", ReservationPurpose::Implementation, 100, 100)
            .unwrap();

        // Model actually returned 400 tokens (overrun beyond reservation and available implementation!)
        let receipt = UsageReceipt {
            attempt_id: "overrun_call".to_string(),
            provider_usage: 400,
            estimated_usage: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost: CostAmount::Known(15),
            finished_at: 5000,
        };
        ledger.settle_receipt(Some(&res.id), receipt).unwrap();

        let snap = ledger.snapshot();
        assert_eq!(snap.tokens_charged, 400);
        assert_eq!(snap.implementation_remaining, None);

        // Next implementation reservation must fail closed with BudgetExhausted
        let err = ledger.reserve("next_impl", ReservationPurpose::Implementation, 10, 10);
        assert!(matches!(err, Err(BudgetError::BudgetExhausted { .. })));
    }

    #[test]
    fn test_crash_after_dispatch_before_receipt_retains_conservative_exposure() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            4,
            5,
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        let res = ledger
            .reserve("crashed_call", ReservationPurpose::Implementation, 150, 150)
            .unwrap();

        // Crash / abort occurs: reconcile_failed_attempt keeps conservative reserved exposure (300) and unknown cost
        assert!(ledger
            .reconcile_failed_attempt(Some(&res.id), "crashed_call", None)
            .unwrap());

        let snap = ledger.snapshot();
        assert_eq!(snap.tokens_charged, 300);
        assert!(snap.has_unknown_cost);
    }

    #[test]
    fn f09_continue_cannot_bypass_ceiling() {
        assert_eq!(budget_decision(0, true, "continue"), "hard_stop");
        assert_eq!(budget_decision(10, true, "continue"), "continue");
        assert_eq!(budget_decision(10, true, "plan"), "return_to_plan");
        assert_eq!(budget_decision(10, true, "stop"), "stop_checkpoint");
    }

    #[test]
    fn test_deadline_during_provider_backoff_blocks_reservation() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_millis(5),
            4,
            5,
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);
        std::thread::sleep(Duration::from_millis(15));
        let res = ledger.reserve(
            "backoff_attempt",
            ReservationPurpose::Implementation,
            10,
            10,
        );
        assert!(matches!(res, Err(BudgetError::DeadlineExpired { .. })));
    }

    #[test]
    fn test_no_checkpoint_budget_fails_when_all_tokens_spent() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            200,
            Duration::from_secs(60),
            4,
            5,
            None,
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        let res = ledger
            .reserve("impl_1", ReservationPurpose::Implementation, 25, 25)
            .unwrap();
        let receipt = UsageReceipt {
            attempt_id: "impl_1".to_string(),
            provider_usage: 200, // consumes entire token ceiling
            estimated_usage: 50,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost: CostAmount::Known(10),
            finished_at: 1000,
        };
        ledger.settle_receipt(Some(&res.id), receipt).unwrap();

        // Even handoff checkpoint reservation must fail when 0 tokens remain
        let err = ledger.reserve("stop_checkpoint", ReservationPurpose::Handoff, 5, 5);
        assert!(matches!(
            err,
            Err(BudgetError::BudgetExhausted {
                requested: 10,
                available: 0
            })
        ));
    }

    #[test]
    fn test_user_raises_only_tokens_not_time_leaves_deadline_enforced() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            200,
            Duration::from_millis(5),
            4,
            5,
            None,
            50,
            25,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);
        std::thread::sleep(Duration::from_millis(15));

        // User raises token ceiling to 500, but does not extend deadline
        let snap = ledger.update_ceilings(1, Some(500), None, true).unwrap();
        assert_eq!(snap.token_ceiling, 500);

        // Reservation still fails on expired deadline
        let res = ledger.reserve("work", ReservationPurpose::Implementation, 10, 10);
        assert!(matches!(res, Err(BudgetError::DeadlineExpired { .. })));
    }

    #[test]
    fn test_unauthorized_and_revision_mismatch_ceiling_update() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            200,
            Duration::from_secs(60),
            4,
            5,
            None,
            50,
            25,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        // Unauthorized
        let err = ledger.update_ceilings(1, Some(500), None, false);
        assert!(matches!(err, Err(BudgetError::UnauthorizedCeilingUpdate)));

        // Revision mismatch
        let err = ledger.update_ceilings(99, Some(500), None, true);
        assert!(matches!(
            err,
            Err(BudgetError::RevisionMismatch {
                expected: 99,
                actual: 1
            })
        ));

        // Ceiling below required reserves fails
        let err = ledger.update_ceilings(1, Some(30), None, true);
        assert!(matches!(
            err,
            Err(BudgetError::CeilingBelowRequiredReserves {
                ceiling: 30,
                required: 75
            })
        ));
    }

    #[test]
    fn test_unknown_cost_with_hard_monetary_cap_refuses_conservatively() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            4,
            5,
            Some(Money {
                amount_minor_units: 500,
                currency: "USD".into(),
            }),
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        // Normal reservation works
        let res = ledger
            .reserve("call_1", ReservationPurpose::Implementation, 10, 10)
            .unwrap();

        // Settle with unknown cost
        let receipt = UsageReceipt {
            attempt_id: "call_1".to_string(),
            provider_usage: 20,
            estimated_usage: 20,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost: CostAmount::Unknown,
            finished_at: 1000,
        };
        ledger.settle_receipt(Some(&res.id), receipt).unwrap();

        // Next reservation must fail closed with CostCeilingExceeded
        let res = ledger.reserve("call_2", ReservationPurpose::Implementation, 10, 10);
        assert!(matches!(res, Err(BudgetError::CostCeilingExceeded { .. })));
    }

    #[test]
    fn test_hard_monetary_ceiling_exceeded_blocks_reservation() {
        let budget = ResourceBudget::new(
            RunId::new(),
            1,
            1000,
            Duration::from_secs(60),
            4,
            5,
            Some(Money {
                amount_minor_units: 100,
                currency: "USD".into(),
            }),
            100,
            50,
        )
        .unwrap();
        let ledger = ResourceLedger::new(budget);

        let res = ledger
            .reserve("call_1", ReservationPurpose::Implementation, 10, 10)
            .unwrap();

        // Settle exceeding monetary ceiling
        let receipt = UsageReceipt {
            attempt_id: "call_1".to_string(),
            provider_usage: 20,
            estimated_usage: 20,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost: CostAmount::Known(120),
            finished_at: 1000,
        };
        ledger.settle_receipt(Some(&res.id), receipt).unwrap();

        let res = ledger.reserve("call_2", ReservationPurpose::Implementation, 10, 10);
        assert!(matches!(res, Err(BudgetError::CostCeilingExceeded { .. })));
    }
}
