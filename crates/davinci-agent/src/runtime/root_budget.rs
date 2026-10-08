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
    pub max_output_tokens: Option<u64>,
    pub max_cost_microusd: Option<u64>,
    #[serde(default)]
    pub codex_subscription: Option<davinci_ai::subscription_policy::CodexSubscriptionPolicy>,
    /// Absolute, persisted deadline; resuming never resets elapsed allowance.
    pub deadline_unix_ms: u64,
}

impl BudgetLimits {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_requests == 0
            || self.deadline_unix_ms == 0
            || self.max_output_tokens == Some(0)
            || self.max_cost_microusd == Some(0)
        {
            return Err("root budget requires positive finite limits".into());
        }
        match &self.codex_subscription {
            Some(policy) => {
                policy.validate()?;
                if self.max_output_tokens.is_some() || self.max_cost_microusd.is_some() {
                    return Err("subscription-only budget uses requests and time, not API USD or unverified output caps".into());
                }
            }
            None if self.max_output_tokens.is_none() => {
                return Err("strict root budget requires an output cap".into())
            }
            None => {}
        }
        Ok(())
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operation: Option<String>,
    output_ceiling: Option<u64>,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    operations: BTreeMap<String, OperationLimit>,
}

/// A narrower host operation on the same ledger. All children and retries
/// inherit it at admission, including processes holding older root handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationLimit {
    max_requests: u64,
    deadline_unix_ms: u64,
    closed: bool,
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
            } else if let Some(ceiling) = reservation.output_ceiling {
                add(&mut snapshot.reserved_output_tokens, ceiling)?;
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
        limits.validate()?;
        if root.is_empty()
            || root.len() > 256
            || limits.max_requests == 0
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
                    operations: BTreeMap::new(),
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
        format!("{:x}", davinci_sys::hex::Lower(&Sha256::digest(bytes)))
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
        if ledger.operations.len() > 4096
            || ledger
                .operations
                .values()
                .filter(|limit| !limit.closed)
                .count()
                > 1
            || ledger.operations.iter().any(|(id, limit)| {
                id.is_empty()
                    || id.len() > 256
                    || limit.max_requests == 0
                    || limit.deadline_unix_ms == 0
            })
        {
            return Err("root budget contains invalid operation limits".into());
        }
        for (attempt, reservation) in &ledger.reservations {
            if attempt.is_empty()
                || attempt.len() > 512
                || reservation.actor.is_empty()
                || reservation.actor.len() > 256
                || reservation.output_ceiling == Some(0)
                || (self.limits.max_output_tokens.is_some() && reservation.output_ceiling.is_none())
                || reservation
                    .operation
                    .as_ref()
                    .is_some_and(|id| !ledger.operations.contains_key(id))
                || reservation
                    .output
                    .zip(reservation.output_ceiling)
                    .is_some_and(|(n, ceiling)| n > ceiling)
                || reservation
                    .cost_microusd
                    .zip(reservation.cost_ceiling)
                    .is_some_and(|(n, bound)| n > bound)
                || (self.limits.max_cost_microusd.is_some() && reservation.cost_ceiling.is_none())
                || (reservation.disposition == Disposition::Committed
                    && ((self.limits.max_output_tokens.is_some() && reservation.output.is_none())
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

    /// Install a persisted narrower limit without replacing the root identity
    /// or refunding any prior request. Reopening the same active operation is
    /// idempotent; its approved deadline and allowance cannot be changed.
    pub fn begin_operation(
        &self,
        id: &str,
        max_requests: u64,
        deadline_unix_ms: u64,
    ) -> Result<(), String> {
        if id.is_empty() || id.len() > 256 || max_requests == 0 || deadline_unix_ms == 0 {
            return Err("operation needs a bounded identity and positive finite limits".into());
        }
        let _lock = self.lock()?;
        let mut ledger = self.read()?;
        let requested = OperationLimit {
            max_requests,
            deadline_unix_ms,
            closed: false,
        };
        if let Some(existing) = ledger.operations.get(id) {
            return if existing == &requested {
                Ok(())
            } else {
                Err("operation limits changed or operation already closed".into())
            };
        }
        if ledger.operations.len() >= 4096 || ledger.operations.values().any(|limit| !limit.closed)
        {
            return Err("another operation owns this root or operation capacity reached".into());
        }
        ledger.operations.insert(id.into(), requested);
        self.write(&ledger)
    }

    /// Host-owned completion releases only the narrower scope. Unsettled
    /// reservations remain charged to the root and still block future sends.
    pub fn finish_operation(&self, id: &str) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut ledger = self.read()?;
        ledger
            .operations
            .get_mut(id)
            .ok_or("unknown root operation")?
            .closed = true;
        self.write(&ledger)
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
        self.reserve_attempt(actor, attempt, Some(output), cost_ceiling)
    }

    fn reserve_attempt(
        &self,
        actor: &str,
        attempt: &str,
        output: Option<u64>,
        cost_ceiling: Option<u64>,
    ) -> Result<(), String> {
        if self.faulted.load(Ordering::SeqCst) {
            return Err("root budget denied: persistence failed".into());
        }
        if actor.is_empty()
            || actor.len() > 256
            || attempt.is_empty()
            || attempt.len() > 512
            || output == Some(0)
            || (self.limits.max_output_tokens.is_some() && output.is_none())
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
        let operation = ledger.operations.iter().find(|(_, limit)| !limit.closed);
        if let Some((id, limit)) = operation {
            let count = ledger
                .reservations
                .values()
                .filter(|reservation| {
                    reservation.operation.as_ref() == Some(id)
                        && reservation.disposition != Disposition::Released
                })
                .count();
            if count as u64 >= limit.max_requests || now >= u128::from(limit.deadline_unix_ms) {
                return Err("root operation request allowance or deadline exhausted".into());
            }
        }
        let operation = operation.map(|(id, _)| id.clone());
        let used = snapshot
            .committed_output_tokens
            .checked_add(snapshot.reserved_output_tokens);
        if snapshot.requests >= self.limits.max_requests
            || self.limits.max_output_tokens.is_some_and(|limit| {
                used.and_then(|n| n.checked_add(output.unwrap_or(0)))
                    .is_none_or(|n| n > limit)
            })
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
                operation,
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
            || output
                .zip(reservation.output_ceiling)
                .is_some_and(|(n, ceiling)| n > ceiling)
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
        reservation.disposition = if (self.limits.max_output_tokens.is_none()
            || reservation.output.is_some())
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
    fn validate_request(
        &self,
        model: &davinci_ai::Model,
        auth: &davinci_ai::ResolvedAuth,
        options: &davinci_ai::StreamOptions,
        body: &serde_json::Value,
        url: &str,
    ) -> Result<(), String> {
        if let Some(policy) = &self.limits.codex_subscription {
            policy.validate_request(model, auth, options, body, url)?;
        }
        Ok(())
    }

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
        if let Some(policy) = &self.limits.codex_subscription {
            if event.model != format!("openai-codex/{}", policy.model)
                || event.selected_effort.as_deref() != Some(policy.effort.as_str())
                || event.transport.as_deref() != Some("http")
            {
                return Err("subscription-only admission denied: request baseline changed".into());
            }
        }
        self.reserve_attempt(
            actor,
            &format!("{}:{id}", event.logical_request_id),
            if self.limits.codex_subscription.is_some() {
                None
            } else {
                Some(output.ok_or("root budget denied: provider output ceiling is unknown")?)
            },
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
        if let Some(policy) = &self.limits.codex_subscription {
            if event.status != "completed"
                || event.returned_model.as_deref() != Some(policy.model.as_str())
            {
                let _lock = self.lock()?;
                let mut ledger = self.read()?;
                ledger.halted = true;
                if let Some(reservation) = ledger
                    .reservations
                    .get_mut(&format!("{}:{id}", event.logical_request_id))
                {
                    reservation.disposition = Disposition::Unknown;
                }
                self.write(&ledger)?;
                return Err(format!(
                    "subscription-only receipt rejected: response {}{}, model {}, expected {}; campaign halted",
                    event.status,
                    event
                        .http_status
                        .map(|code| format!(" (HTTP {code})"))
                        .unwrap_or_default(),
                    event.returned_model.as_deref().unwrap_or("not reported"),
                    policy.model,
                ));
            }
        }
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
                max_output_tokens: Some(u64::MAX),
                max_cost_microusd: None,
                codex_subscription: None,
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
                    operation: None,
                    output_ceiling: Some(1),
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

    #[test]
    fn a_rejected_subscription_receipt_names_what_the_provider_returned() {
        use davinci_ai::provider_observation::{AttemptBudget, ProviderAttemptObservation};
        let directory = tempfile::tempdir().unwrap();
        let budget = RootBudget::open(
            directory.path().join("budget.json"),
            "root",
            BudgetLimits {
                max_requests: 4,
                max_output_tokens: None,
                max_cost_microusd: None,
                codex_subscription: Some(
                    davinci_ai::subscription_policy::CodexSubscriptionPolicy {
                        model: "gpt-5.6-luna".into(),
                        effort: "medium".into(),
                    },
                ),
                deadline_unix_ms: u64::MAX,
            },
        )
        .unwrap();
        let receipt = |status: &str, http: Option<u16>, model: Option<&str>| {
            serde_json::from_value::<ProviderAttemptObservation>(serde_json::json!({
                "schema_version": 1, "kind": "attempt_end", "logical_request_id": "r",
                "attempt_id": 1, "purpose": "design", "model": "openai-codex/gpt-5.6-luna",
                "returned_model": model, "selected_effort": "medium", "schema_hash": "",
                "transport": "http", "elapsed_ms": 0.0, "duration_ms": null,
                "status": status, "http_status": http, "usage": null,
            }))
            .unwrap()
        };
        let error =
            AttemptBudget::reconcile(&budget, &receipt("failed", Some(429), None)).unwrap_err();
        assert_eq!(
            error,
            "subscription-only receipt rejected: response failed (HTTP 429), model not reported, expected gpt-5.6-luna; campaign halted"
        );
        let error = AttemptBudget::reconcile(&budget, &receipt("completed", None, Some("gpt-5.6")))
            .unwrap_err();
        assert!(
            error.contains("response completed, model gpt-5.6, expected gpt-5.6-luna"),
            "{error}"
        );
        assert!(budget.read().unwrap().halted);
    }
}
