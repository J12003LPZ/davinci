//! Crash-conservative request accounting; no upstream TypeScript counterpart.
use super::store::Store;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct RequestBudget {
    store: Option<Store>,
    scan_id: String,
    max_turns: usize,
    max_tokens: u64,
    max_discovery_tokens: u64,
    turns: usize,
    tokens: u64,
    requests: Vec<(u64, bool, bool)>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reservation {
    schema_version: u32,
    scan_id: String,
    turn: usize,
    max_turns: usize,
    max_tokens: u64,
    reserved_tokens: u64,
    discovery: bool,
    max_discovery_tokens: u64,
}

impl RequestBudget {
    fn discovery_usage(&self) -> (usize, u64) {
        self.requests
            .iter()
            .filter(|(_, _, discovery)| *discovery)
            .fold((0, 0u64), |(turns, tokens), (charged, _, _)| {
                (turns + 1, tokens.saturating_add(*charged))
            })
    }
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"accountedTokens":self.tokens,"requests":self.turns,
            "remainingTokens":self.max_tokens.saturating_sub(self.tokens),
            "remainingRequests":self.max_turns.saturating_sub(self.turns),
            "unsettledRequests":self.requests.iter().filter(|(_, settled, _)| !settled).count(),
            "discoveryRequests":self.discovery_usage().0,"discoveryTokens":self.discovery_usage().1,
            "maxDiscoveryTokens":self.max_discovery_tokens,
            "remainingDiscoveryTokens":self.max_discovery_tokens.saturating_sub(self.discovery_usage().1),
            "remainingDiscoveryRequests":(self.max_turns / 2).saturating_sub(self.discovery_usage().0),
            "persistent":self.store.is_some()})
    }
    #[cfg(test)]
    pub fn open(
        store: Option<Store>,
        scan_id: String,
        max_turns: usize,
        max_tokens: u64,
    ) -> Result<Self, String> {
        Self::open_scoped(store, scan_id, max_turns, max_tokens, max_tokens * 3 / 4)
    }

    #[allow(dead_code)]
    pub fn open_scoped_with_root_lease(
        store: Option<Store>,
        scan_id: String,
        max_turns: usize,
        max_tokens: u64,
        max_discovery_tokens: u64,
        root_lease_tokens: Option<u64>,
    ) -> Result<Self, String> {
        if let Some(lease) = root_lease_tokens {
            if max_tokens > lease {
                return Err(format!(
                    "subsystem security scan budget ({max_tokens}) exceeds root lease ({lease})"
                ));
            }
        }
        Self::open_scoped(store, scan_id, max_turns, max_tokens, max_discovery_tokens)
    }

    /// Enforce that this subsystem budget cannot exceed an authorized root lease.
    #[allow(dead_code)]
    pub fn enforce_root_lease(&self, root_lease_tokens: u64) -> Result<(), String> {
        if self.max_tokens > root_lease_tokens {
            return Err(format!(
                "subsystem budget max_tokens ({}) cannot exceed root lease ({})",
                self.max_tokens, root_lease_tokens
            ));
        }
        Ok(())
    }

    pub fn open_scoped(
        store: Option<Store>,
        scan_id: String,
        max_turns: usize,
        max_tokens: u64,
        max_discovery_tokens: u64,
    ) -> Result<Self, String> {
        if max_turns == 0 || max_turns > 140 || max_tokens == 0 || max_discovery_tokens > max_tokens
        {
            return Err("invalid durable scan budget limits".into());
        }
        let mut budget = Self {
            store,
            scan_id,
            max_turns,
            max_tokens,
            max_discovery_tokens,
            turns: 0,
            tokens: 0,
            requests: Vec::new(),
        };
        if let Some(store) = &budget.store {
            let mut gap = false;
            for turn in 1..=140 {
                let settlement =
                    store.read_optional(&format!("request-usage-{turn}.json"), 4096)?;
                let Some(bytes) =
                    store.read_optional(&format!("request-budget-{turn}.json"), 4096)?
                else {
                    if settlement.is_some() {
                        return Err("orphan request settlement".into());
                    }
                    gap = true;
                    continue;
                };
                let record: Reservation =
                    serde_json::from_slice(&bytes).map_err(|_| "invalid request budget record")?;
                if gap
                    || record.schema_version != 1
                    || record.scan_id != budget.scan_id
                    || record.turn != turn
                    || record.max_turns != max_turns
                    || record.max_tokens != max_tokens
                    || record.max_discovery_tokens != max_discovery_tokens
                    || turn > max_turns
                    || record.reserved_tokens == 0
                    || record.reserved_tokens > max_tokens
                {
                    return Err("inconsistent durable scan budget".into());
                }
                budget.turns = turn;
                let charged = if let Some(bytes) = &settlement {
                    let usage: Reservation =
                        serde_json::from_slice(bytes).map_err(|_| "invalid request settlement")?;
                    if usage.schema_version != 1
                        || usage.scan_id != budget.scan_id
                        || usage.turn != turn
                        || usage.max_tokens != max_tokens
                        || usage.max_turns != max_turns
                        || usage.max_discovery_tokens != max_discovery_tokens
                        || usage.discovery != record.discovery
                        || usage.reserved_tokens == 0
                    {
                        return Err("inconsistent request settlement".into());
                    }
                    usage.reserved_tokens
                } else {
                    record.reserved_tokens
                };
                budget.tokens = budget
                    .tokens
                    .checked_add(charged)
                    .ok_or("scan budget overflow")?;
                budget
                    .requests
                    .push((charged, settlement.is_some(), record.discovery));
            }
        }
        Ok(budget)
    }

    #[cfg(test)]
    pub fn reserve(&mut self, tokens: u64) -> Result<usize, String> {
        self.reserve_scoped(tokens, false)
    }

    pub fn reserve_scoped(&mut self, tokens: u64, discovery: bool) -> Result<usize, String> {
        let total = self
            .tokens
            .checked_add(tokens)
            .ok_or("scan budget overflow")?;
        if tokens == 0 || self.turns >= self.max_turns || total > self.max_tokens {
            return Err("durable scan request budget exhausted".into());
        }
        let (discovery_turns, discovery_tokens) = self.discovery_usage();
        if discovery
            && (discovery_turns >= self.max_turns / 2
                || discovery_tokens.saturating_add(tokens) > self.max_discovery_tokens)
        {
            return Err("durable discovery budget exhausted; validation reserve protected".into());
        }
        let turn = self.turns + 1;
        let reservation = Reservation {
            schema_version: 1,
            scan_id: self.scan_id.clone(),
            turn,
            max_turns: self.max_turns,
            max_tokens: self.max_tokens,
            reserved_tokens: tokens,
            discovery,
            max_discovery_tokens: self.max_discovery_tokens,
        };
        if let Some(store) = &self.store {
            store.publish(
                &format!("request-budget-{turn}.json"),
                &serde_json::to_vec(&reservation).map_err(|_| "cannot encode request budget")?,
            )?;
        }
        self.turns = turn;
        self.tokens = total;
        self.requests.push((tokens, false, discovery));
        Ok(turn)
    }

    pub fn settle(&mut self, turn: usize, tokens: u64) -> Result<(), String> {
        let (reserved, settled, discovery) = *self
            .requests
            .get(turn.checked_sub(1).ok_or("invalid request ticket")?)
            .ok_or("unknown request ticket")?;
        if settled || tokens == 0 {
            return Err("invalid repeated request settlement".into());
        }
        let total = self
            .tokens
            .checked_sub(reserved)
            .and_then(|n| n.checked_add(tokens))
            .ok_or("scan budget overflow")?;
        let usage = Reservation {
            schema_version: 1,
            scan_id: self.scan_id.clone(),
            turn,
            max_turns: self.max_turns,
            max_tokens: self.max_tokens,
            reserved_tokens: tokens,
            discovery,
            max_discovery_tokens: self.max_discovery_tokens,
        };
        if let Some(store) = &self.store {
            store.publish(
                &format!("request-usage-{turn}.json"),
                &serde_json::to_vec(&usage).map_err(|_| "cannot encode request settlement")?,
            )?;
        }
        self.tokens = total;
        self.requests[turn - 1] = (tokens, true, discovery);
        if total > self.max_tokens {
            return Err("provider usage exceeded durable scan budget".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn security_resume_preserves_validation_reserve() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        let mut budget = RequestBudget::open(Some(store.clone()), id.clone(), 4, 100).unwrap();
        budget.reserve_scoped(75, true).unwrap();
        let mut resumed = RequestBudget::open(Some(store), id, 4, 100).unwrap();
        assert!(resumed.reserve_scoped(1, true).is_err());
        resumed.reserve_scoped(25, false).unwrap();
    }

    #[test]
    fn security_scan_budget_reserves_validation() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        let mut budget = RequestBudget::open_scoped(Some(store), id, 8, 100, 75).unwrap();
        budget.reserve_scoped(75, true).unwrap();
        assert!(budget.reserve_scoped(1, true).is_err());
        budget.reserve_scoped(25, false).unwrap();
        let summary = budget.summary();
        assert_eq!(summary["remainingDiscoveryTokens"], 0);
        assert_eq!(summary["remainingTokens"], 0);
    }

    #[test]
    fn security_discovery_turn_reserve_survives_settlement_and_resume() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        let mut budget = RequestBudget::open(Some(store.clone()), id.clone(), 4, 100).unwrap();
        let first = budget.reserve_scoped(30, true).unwrap();
        budget.settle(first, 1).unwrap();
        budget.reserve_scoped(1, true).unwrap();
        let mut resumed = RequestBudget::open(Some(store.clone()), id.clone(), 4, 100).unwrap();
        assert_eq!(resumed.discovery_usage(), (2, 2));
        assert!(resumed.reserve_scoped(1, true).is_err());
        resumed.reserve_scoped(50, false).unwrap();
        assert!(RequestBudget::open_scoped(Some(store), id, 4, 100, 74).is_err());
    }

    #[test]
    fn security_request_budget_settlement_and_overage_survive_resume() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        let mut budget = RequestBudget::open(Some(store.clone()), id.clone(), 3, 100).unwrap();
        let first = budget.reserve(70).unwrap();
        budget.settle(first, 10).unwrap();
        assert!(budget.settle(first, 10).is_err());
        let mut resumed = RequestBudget::open(Some(store.clone()), id.clone(), 3, 100).unwrap();
        assert_eq!(resumed.tokens, 10);
        let second = resumed.reserve(90).unwrap();
        assert!(resumed.settle(second, 110).is_err());
        let mut exhausted = RequestBudget::open(Some(store), id, 3, 100).unwrap();
        assert_eq!(exhausted.tokens, 120);
        assert!(exhausted.reserve(1).is_err());
    }

    #[test]
    fn security_request_budget_rejects_corruption_gaps_and_identity_drift() {
        for (turn, record) in [(1, b"{}".to_vec()), (2, b"{}".to_vec())] {
            let root = tempfile::tempdir().unwrap();
            let agent = tempfile::tempdir().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
            store
                .publish(&format!("request-budget-{turn}.json"), &record)
                .unwrap();
            assert!(RequestBudget::open(Some(store), id, 2, 100).is_err());
        }
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        RequestBudget::open(Some(store.clone()), id.clone(), 2, 100)
            .unwrap()
            .reserve(10)
            .unwrap();
        assert!(RequestBudget::open(Some(store.clone()), id.clone(), 2, 101).is_err());
        assert!(
            RequestBudget::open(Some(store), uuid::Uuid::new_v4().to_string(), 2, 100).is_err()
        );
    }

    #[test]
    fn security_request_budget_failed_publication_does_not_admit_request() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        let mut budget = RequestBudget::open(Some(store.clone()), id, 2, 100).unwrap();
        store.publish("request-budget-1.json", b"occupied").unwrap();
        assert!(budget.reserve(10).is_err());
        assert_eq!((budget.turns, budget.tokens), (0, 0));
    }

    #[test]
    fn security_request_budget_survives_interruption_before_response() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        let mut first = RequestBudget::open(Some(store.clone()), id.clone(), 2, 100).unwrap();
        first.reserve(70).unwrap();
        drop(first);
        let mut resumed = RequestBudget::open(Some(store), id, 2, 100).unwrap();
        assert_eq!(resumed.tokens, 70);
        assert!(resumed.reserve(31).is_err());
        resumed.reserve(30).unwrap();
        assert!(resumed.reserve(1).is_err());
    }

    #[test]
    fn test_subsystem_budget_never_exceeds_root_lease() {
        // Tighter local cap than root lease succeeds
        let b =
            RequestBudget::open_scoped_with_root_lease(None, "scan1".into(), 5, 100, 50, Some(200));
        assert!(b.is_ok());

        // Local cap exceeding root lease fails
        let b =
            RequestBudget::open_scoped_with_root_lease(None, "scan2".into(), 5, 300, 50, Some(200));
        assert!(b.is_err());

        // enforce_root_lease validation
        let budget = RequestBudget::open_scoped(None, "scan3".into(), 5, 150, 50).unwrap();
        assert!(budget.enforce_root_lease(200).is_ok());
        assert!(budget.enforce_root_lease(100).is_err());
    }
}
