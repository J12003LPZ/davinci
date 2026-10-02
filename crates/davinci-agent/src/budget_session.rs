//! A budgeted conversation cannot silently resume with a fresh allowance.
use crate::runtime::capacity::{BudgetLimits, RootBudget};
use crate::{Agent, JsonlSession};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const ENTRY: &str = "davinci_root_budget_v1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    root: String,
    ledger: PathBuf,
    limits: BudgetLimits,
    output_limit: Option<u64>,
}

impl Agent {
    pub(crate) fn persist_harness_checkpoint(&mut self) -> Result<(), String> {
        if let Some(budget) = self.root_budget.clone() {
            self.persist_root_budget(&budget)?;
        }
        let (Some(runtime), Some(session)) = (&self.runtime, &mut self.session) else {
            return Ok(());
        };
        let watchdog = runtime
            .progress_watchdog
            .lock()
            .map_err(|_| "repair watchdog unavailable")?;
        let data = serde_json::to_value(&*watchdog).map_err(|e| e.to_string())?;
        if session
            .entries
            .iter()
            .rev()
            .find(|e| e.custom_type.as_deref() == Some("davinci_repair_v1"))
            .and_then(|e| e.extra.get("data"))
            == Some(&data)
        {
            return Ok(());
        }
        let mut extra = serde_json::Map::new();
        extra.insert("data".into(), data);
        session
            .append_entry(davinci_session::SessionEntry {
                id: String::new(),
                entry_type: "custom".into(),
                parent_id: None,
                seq: 0,
                timestamp: 0,
                message: None,
                custom_type: Some("davinci_repair_v1".into()),
                extra,
            })
            .map_err(|e| format!("cannot persist repair checkpoint: {e}"))
    }

    pub(crate) fn restore_repair_checkpoint(
        session: &JsonlSession,
    ) -> Result<Option<crate::runtime::progress_watchdog::ProgressWatchdog>, String> {
        davinci_session::branch_entries(&session.entries, session.leaf_id.as_deref())
            .into_iter()
            .rev()
            .find(|entry| entry.custom_type.as_deref() == Some("davinci_repair_v1"))
            .map(|entry| {
                crate::runtime::progress_watchdog::ProgressWatchdog::restore(
                    entry
                        .extra
                        .get("data")
                        .cloned()
                        .ok_or("missing repair checkpoint")?,
                )
            })
            .transpose()
    }

    pub(crate) fn restore_root_budget(
        &self,
        session: &JsonlSession,
    ) -> Result<Option<RootBudget>, String> {
        let mut restored: Option<RootBudget> = None;
        for entry in session
            .entries
            .iter()
            .filter(|entry| entry.custom_type.as_deref() == Some(ENTRY))
        {
            let binding: Binding = serde_json::from_value(
                entry
                    .extra
                    .get("data")
                    .cloned()
                    .ok_or("session root budget binding is missing")?,
            )
            .map_err(|e| format!("session root budget binding is invalid: {e}"))?;
            let budget = RootBudget::reopen(binding.ledger, &binding.root, binding.limits)?;
            let identity = budget.binding_identity();
            let process = davinci_ai::provider_observation::process_budget_binding();
            let local_matches = self
                .root_budget
                .as_ref()
                .is_some_and(|current| current.binding_identity() == identity);
            let process_matches = process
                .as_ref()
                .is_some_and(|(current, _)| current == &identity);
            if !(local_matches || process_matches)
                || binding.output_limit.is_some_and(|limit| {
                    process.as_ref().is_none_or(|(_, current)| *current > limit)
                })
                || restored
                    .as_ref()
                    .is_some_and(|current| current.binding_identity() != identity)
            {
                return Err("budgeted session requires its original root binding; pass --root-budget with the original configuration".into());
            }
            restored = Some(budget);
        }
        Ok(restored)
    }

    pub(crate) fn persist_root_budget(&mut self, budget: &RootBudget) -> Result<(), String> {
        let Some(session) = &self.session else {
            return Ok(());
        };
        if session
            .entries
            .iter()
            .any(|entry| entry.custom_type.as_deref() == Some(ENTRY))
        {
            let existing = self
                .restore_root_budget(session)?
                .ok_or("missing session root budget")?;
            if existing.binding_identity() != budget.binding_identity() {
                return Err("cannot replace a session's root budget".into());
            }
            return Ok(());
        }
        let output_limit = davinci_ai::provider_observation::process_budget_binding()
            .filter(|(identity, _)| identity == &budget.binding_identity())
            .map(|(_, limit)| limit);
        let data = serde_json::to_value(Binding {
            root: budget.root_id().into(),
            ledger: budget.path().into(),
            limits: budget.limits().clone(),
            output_limit,
        })
        .map_err(|e| e.to_string())?;
        let mut extra = serde_json::Map::new();
        extra.insert("data".into(), data);
        self.session
            .as_mut()
            .expect("session checked")
            .append_entry(davinci_session::SessionEntry {
                id: String::new(),
                entry_type: "custom".into(),
                parent_id: None,
                seq: 0,
                timestamp: 0,
                message: None,
                custom_type: Some(ENTRY.into()),
                extra,
            })
            .map_err(|e| format!("cannot persist root budget binding: {e}"))
    }
}
