//! Root-owned, durable admission. Reservations are ceilings, never usage.
//! Children share this handle or reopen the identical root and limits. Every
//! mutation locks the whole read/validate/write transaction across processes.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_LEDGER_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetLimits {
    pub max_requests: u64,
    pub max_output_tokens: u64,
    pub max_cost_microusd: Option<u64>,
    /// Absolute, persisted deadline; resuming never resets elapsed allowance.
    pub deadline_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Disposition {
    Pending,
    Unknown,
    Committed,
    Released,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    actor: String,
    output_ceiling: u64,
    cost_ceiling: Option<u64>,
    output: Option<u64>,
    cost_microusd: Option<u64>,
    disposition: Disposition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    schema_version: u32,
    root: String,
    limits: BudgetLimits,
    reservations: BTreeMap<String, Reservation>,
    halted: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BudgetSnapshot {
    pub requests: u64,
    pub pending: u64,
    pub unknown: u64,
    pub committed_output_tokens: u64,
    pub reserved_output_tokens: u64,
    pub committed_cost_microusd: u64,
    pub reserved_cost_microusd: u64,
    pub halted: bool,
}

impl Ledger {
    fn snapshot(&self) -> Result<BudgetSnapshot, String> {
        let mut snapshot = BudgetSnapshot {
            halted: self.halted,
            ..Default::default()
        };
        fn add(total: &mut u64, value: u64) -> Result<(), String> {
            *total = total
                .checked_add(value)
                .ok_or("root budget counter overflow")?;
            Ok(())
        }
        for reservation in self.reservations.values() {
            if reservation.disposition == Disposition::Released {
                continue;
            }
            add(&mut snapshot.requests, 1)?;
            match reservation.disposition {
                Disposition::Pending => add(&mut snapshot.pending, 1)?,
                Disposition::Unknown => add(&mut snapshot.unknown, 1)?,
                _ => {}
            }
            if let Some(output) = reservation.output {
                add(&mut snapshot.committed_output_tokens, output)?;
            } else {
                add(
                    &mut snapshot.reserved_output_tokens,
                    reservation.output_ceiling,
                )?;
            }
            if let Some(cost) = reservation.cost_microusd {
                add(&mut snapshot.committed_cost_microusd, cost)?;
            } else if let Some(ceiling) = reservation.cost_ceiling {
                add(&mut snapshot.reserved_cost_microusd, ceiling)?;
            }
        }
        Ok(snapshot)
    }
}

#[derive(Debug, Clone)]
pub struct RootBudget {
    path: PathBuf,
    root: String,
    limits: BudgetLimits,
    faulted: Arc<AtomicBool>,
}

impl RootBudget {
    pub fn open(path: PathBuf, root: &str, limits: BudgetLimits) -> Result<Self, String> {
        Self::load(path, root, limits, true)
    }

    /// Child processes and resumed sessions cannot create a replacement ledger.
    pub fn reopen(path: PathBuf, root: &str, limits: BudgetLimits) -> Result<Self, String> {
        Self::load(path, root, limits, false)
    }

    fn load(path: PathBuf, root: &str, limits: BudgetLimits, create: bool) -> Result<Self, String> {
        if root.is_empty()
            || root.len() > 256
            || limits.max_requests == 0
            || limits.max_output_tokens == 0
            || limits.max_cost_microusd == Some(0)
            || limits.deadline_unix_ms == 0
        {
            return Err("root budget needs an identity and positive finite limits".into());
        }
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        let budget = Self {
            path,
            root: root.into(),
            limits,
            faulted: Arc::new(AtomicBool::new(false)),
        };
        let _lock = budget.lock()?;
        match std::fs::symlink_metadata(&budget.path) {
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                budget.write(&Ledger {
                    schema_version: 1,
                    root: budget.root.clone(),
                    limits: budget.limits.clone(),
                    reservations: BTreeMap::new(),
                    halted: false,
                })?
            }
            Err(error) => return Err(error.to_string()),
            Ok(_) => {
                budget.read()?;
            }
        }
        Ok(budget)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn root_id(&self) -> &str {
        &self.root
    }
    pub fn limits(&self) -> &BudgetLimits {
        &self.limits
    }
    pub fn binding_identity(&self) -> String {
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(&(&self.path, &self.root, &self.limits))
            .expect("budget identity contains only serializable fields");
        format!("{:x}", Sha256::digest(bytes))
    }

    fn lock(&self) -> Result<davinci_sys::lock::ExclusiveFileLock, String> {
        davinci_sys::lock::ExclusiveFileLock::acquire(
            &davinci_sys::lock::lock_path_for(&self.path),
            Duration::from_secs(5),
        )
        .map_err(|error| format!("root budget lock unavailable: {error}"))
    }

    fn read(&self) -> Result<Ledger, String> {
        let metadata = std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_LEDGER_BYTES as u64
        {
            return Err("root budget must be an ordinary bounded ledger".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("root budget reparse point refused".into());
            }
        }
        let ledger: Ledger =
            serde_json::from_slice(&std::fs::read(&self.path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("root budget cannot be reconciled: {e}"))?;
        if ledger.schema_version != 1 || ledger.root != self.root || ledger.limits != self.limits {
            return Err("root budget identity or approved limits changed".into());
        }
        for (attempt, reservation) in &ledger.reservations {
            if attempt.is_empty()
                || attempt.len() > 512
                || reservation.actor.is_empty()
                || reservation.actor.len() > 256
                || reservation.output_ceiling == 0
                || reservation
                    .output
                    .is_some_and(|n| n > reservation.output_ceiling)
                || reservation
                    .cost_microusd
                    .zip(reservation.cost_ceiling)
                    .is_some_and(|(n, bound)| n > bound)
                || (self.limits.max_cost_microusd.is_some() && reservation.cost_ceiling.is_none())
                || (reservation.disposition == Disposition::Committed
                    && (reservation.output.is_none()
                        || (self.limits.max_cost_microusd.is_some()
                            && reservation.cost_microusd.is_none())))
            {
                return Err("root budget contains inconsistent reservation evidence".into());
            }
        }
        ledger.snapshot()?;
        Ok(ledger)
    }

    fn write(&self, ledger: &Ledger) -> Result<(), String> {
        let bytes = serde_json::to_vec(ledger).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_LEDGER_BYTES {
            return Err(
                "root budget ledger capacity reached; existing reservations retained".into(),
            );
        }
        davinci_sys::fs::atomic_write_private(&self.path, &bytes).map_err(|e| {
            self.faulted.store(true, Ordering::SeqCst);
            e.to_string()
        })
    }

    pub fn snapshot(&self) -> Result<BudgetSnapshot, String> {
        let _lock = self.lock()?;
        self.read()?.snapshot()
    }

    /// A unique actual-send identity must be reserved before dispatch. A
    /// duplicate reservation is rejected, since it could otherwise send twice.
    pub fn reserve(
        &self,
        actor: &str,
        attempt: &str,
        output: u64,
        cost_ceiling: Option<u64>,
    ) -> Result<(), String> {
        if self.faulted.load(Ordering::SeqCst) {
            return Err("root budget denied: persistence failed".into());
        }
        if actor.is_empty()
            || actor.len() > 256
            || attempt.is_empty()
            || attempt.len() > 512
            || output == 0
        {
            return Err(
                "root budget reservation needs bounded owner, attempt and output ceiling".into(),
            );
        }
        let _lock = self.lock()?;
        let mut ledger = self.read()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis();
        let snapshot = ledger.snapshot()?;
        if snapshot.halted
            || snapshot.unknown > 0
            || now >= u128::from(self.limits.deadline_unix_ms)
        {
            return Err("root budget denied: unresolved accounting or deadline reached".into());
        }
        if ledger.reservations.contains_key(attempt) || ledger.reservations.len() >= 65_536 {
            return Err("root budget denied: duplicate attempt or ledger capacity reached".into());
        }
        let used = snapshot
            .committed_output_tokens
            .checked_add(snapshot.reserved_output_tokens);
        if snapshot.requests >= self.limits.max_requests
            || used
                .and_then(|n| n.checked_add(output))
                .is_none_or(|n| n > self.limits.max_output_tokens)
        {
            return Err("root budget denied: request or output allowance exhausted".into());
        }
        if let Some(limit) = self.limits.max_cost_microusd {
            let cost = cost_ceiling
                .ok_or("root budget denied: unknown pricing cannot guarantee strict spend")?;
            if snapshot
                .committed_cost_microusd
                .checked_add(snapshot.reserved_cost_microusd)
                .and_then(|n| n.checked_add(cost))
                .is_none_or(|n| n > limit)
            {
                return Err("root budget denied: spend allowance exhausted".into());
            }
        }
        ledger.reservations.insert(
            attempt.into(),
            Reservation {
                actor: actor.into(),
                output_ceiling: output,
                cost_ceiling,
                output: None,
                cost_microusd: None,
                disposition: Disposition::Pending,
            },
        );
        self.write(&ledger)
    }

    /// Only trusted usage can settle a reservation. Unknown completion keeps
    /// the ceiling charged and blocks admission; delayed evidence can settle it.
    pub fn reconcile(
        &self,
        attempt: &str,
        output: Option<u64>,
        cost: Option<u64>,
    ) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut ledger = self.read()?;
        let reservation = ledger
            .reservations
            .get_mut(attempt)
            .ok_or("unknown root reservation")?;
        let conflict = reservation.disposition == Disposition::Released
            || output.is_some_and(|n| n > reservation.output_ceiling)
            || cost
                .zip(reservation.cost_ceiling)
                .is_some_and(|(n, ceiling)| n > ceiling)
            || reservation.output.zip(output).is_some_and(|(a, b)| a != b)
            || reservation
                .cost_microusd
                .zip(cost)
                .is_some_and(|(a, b)| a != b);
        if conflict {
            ledger.halted = true;
            self.write(&ledger)?;
            return Err("conflicting or over-limit root receipt; admission halted".into());
        }
        reservation.output = output.or(reservation.output);
        reservation.cost_microusd = cost.or(reservation.cost_microusd);
        reservation.disposition = if reservation.output.is_some()
            && (self.limits.max_cost_microusd.is_none() || reservation.cost_microusd.is_some())
        {
            Disposition::Committed
        } else {
            Disposition::Unknown
        };
        self.write(&ledger)
    }

    /// Host-only release for work positively known not to have been sent.
    /// A transport timeout or cancellation after sending is not such evidence.
    pub fn release_unsent(&self, attempt: &str) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut ledger = self.read()?;
        let reservation = ledger
            .reservations
            .get_mut(attempt)
            .ok_or("unknown root reservation")?;
        if !matches!(
            reservation.disposition,
            Disposition::Pending | Disposition::Released
        ) {
            return Err("sent or uncertain reservation cannot be released".into());
        }
        reservation.disposition = Disposition::Released;
        self.write(&ledger)
    }

    /// Cold restore after the previous owner is known dead. No fresh allowance.
    pub fn recover_pending(&self) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut ledger = self.read()?;
        for reservation in ledger.reservations.values_mut() {
            if reservation.disposition == Disposition::Pending {
                reservation.disposition = Disposition::Unknown;
            }
        }
        self.write(&ledger)
    }
}

impl davinci_ai::provider_observation::AttemptBudget for RootBudget {
    fn binding_identity(&self) -> Option<String> {
        Some(self.binding_identity())
    }

    fn reserve(
        &self,
        event: &davinci_ai::provider_observation::ProviderAttemptObservation,
        output: Option<u64>,
    ) -> Result<(), String> {
        if event.root_id.as_deref() != Some(self.root_id()) {
            return Err("root budget denied: provider root identity mismatch".into());
        }
        let actor = event
            .actor_id
            .as_deref()
            .ok_or("root budget denied: missing actor")?;
        let id = event
            .attempt_id
            .ok_or("root budget denied: missing attempt")?;
        // Catalog estimates do not establish a route/tier monetary guarantee.
        // Until a trusted bound is supplied, strict monetary admission refuses.
        self.reserve(
            actor,
            &format!("{}:{id}", event.logical_request_id),
            output.ok_or("root budget denied: provider output ceiling is unknown")?,
            None,
        )
    }

    fn reconcile(
        &self,
        event: &davinci_ai::provider_observation::ProviderAttemptObservation,
    ) -> Result<(), String> {
        let id = event
            .attempt_id
            .ok_or("root budget receipt lacks attempt identity")?;
        // Output can be measured even when input cache-write provenance is
        // absent. Settle only this known dimension; monetary uncertainty is
        // independently retained by reconcile.
        let output = event
            .raw_usage
            .as_ref()
            .filter(|raw| !raw.anomalous)
            .and_then(|raw| raw.output)
            .filter(|output| {
                event
                    .usage
                    .as_ref()
                    .is_some_and(|usage| usage.output == *output)
            });
        self.reconcile(&format!("{}:{id}", event.logical_request_id), output, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_oversized_budget_write_preserves_last_readable_ledger() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("budget.json");
        let budget = RootBudget::open(
            path.clone(),
            "root",
            BudgetLimits {
                max_requests: 65_536,
                max_output_tokens: u64::MAX,
                max_cost_microusd: None,
                deadline_unix_ms: u64::MAX,
            },
        )
        .unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut ledger = budget.read().unwrap();
        for index in 0..24_000 {
            ledger.reservations.insert(
                format!("{index:08}{}", "x".repeat(500)),
                Reservation {
                    actor: "a".repeat(256),
                    output_ceiling: 1,
                    cost_ceiling: None,
                    output: None,
                    cost_microusd: None,
                    disposition: Disposition::Pending,
                },
            );
        }
        assert!(budget.write(&ledger).is_err());
        assert_eq!(std::fs::read(path).unwrap(), original);
        assert_eq!(budget.snapshot().unwrap().requests, 0);
    }
}
