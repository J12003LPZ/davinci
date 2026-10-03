//! Durable allocation of validation and reconciliation capacity; native-only.
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Allocation {
    schema_version: u32,
    scan_id: String,
    candidate_count: usize,
    tokens: u64,
    turns: usize,
}

impl Allocation {
    pub fn open(
        store: Option<&super::store::Store>,
        scan_id: &str,
        count: usize,
        remaining: u64,
        max_tokens: u64,
        turns: usize,
    ) -> Result<Self, String> {
        let saved = store
            .map(|store| store.read_optional("validation-allocation.json", 4096))
            .transpose()?
            .flatten();
        let allocation = match &saved {
            Some(bytes) => {
                serde_json::from_slice(bytes).map_err(|_| "invalid saved validation allocation")?
            }
            None => Self {
                schema_version: 1,
                scan_id: scan_id.into(),
                candidate_count: count,
                tokens: remaining,
                turns,
            },
        };
        if allocation.schema_version != 1
            || allocation.scan_id != scan_id
            || allocation.candidate_count != count
            || allocation.tokens > max_tokens
            || allocation.turns != turns
        {
            return Err("validation allocation differs from current run".into());
        }
        if saved.is_none() {
            if let Some(store) = store {
                store.publish(
                    "validation-allocation.json",
                    &serde_json::to_vec(&allocation)
                        .map_err(|_| "cannot encode validation allocation")?,
                )?;
            }
        }
        Ok(allocation)
    }

    pub fn reconciliation_tokens(&self) -> u64 {
        if self.candidate_count > 1 {
            self.tokens / 3
        } else {
            0
        }
    }
    pub fn reconciliation_turns(&self) -> usize {
        if self.candidate_count > 1 {
            self.turns / 3
        } else {
            0
        }
    }
    pub fn candidate_tokens(&self) -> u64 {
        (self.tokens - self.reconciliation_tokens()) / self.candidate_count.max(1) as u64
    }
    pub fn candidate_turns(&self) -> usize {
        self.turns - self.reconciliation_turns()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_validation_allocation_survives_consumed_budget_on_resume() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = super::super::store::Store::open(agent.path(), root.path(), &id, true).unwrap();
        let first = Allocation::open(Some(&store), &id, 2, 90_000, 96_000, 30).unwrap();
        let resumed = Allocation::open(Some(&store), &id, 2, 40_000, 96_000, 30).unwrap();
        assert_eq!(first, resumed);
        assert_eq!(
            first.candidate_tokens() * 2 + first.reconciliation_tokens(),
            90_000
        );
        assert_eq!(first.candidate_turns() + first.reconciliation_turns(), 30);
        assert!(Allocation::open(Some(&store), &id, 3, 40_000, 96_000, 30).is_err());
        assert!(Allocation::open(Some(&store), &id, 2, 40_000, 24_000, 30).is_err());
    }
}
