//! Carry validated one-call consent to an asynchronous execution boundary.
use super::{ApprovalRegistry, APPROVAL_TTL, MAX_PENDING};
use crate::PermissionState;
use serde_json::Value;
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
    time::Instant,
};

#[derive(Debug, Default)]
pub(super) struct DispatchState {
    epoch: AtomicU64,
    permits: Mutex<HashMap<String, Arc<DispatchPermit>>>,
}

impl DispatchState {
    pub(super) fn revoke_all(&self) {
        let _ = self
            .epoch
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |epoch| {
                epoch.checked_add(1)
            });
        self.permits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

/// Opaque, non-serializable consent issued only after the host approval check.
/// Cloning the handle does not duplicate its single-use authority.
#[derive(Debug)]
pub struct DispatchPermit {
    registry: Weak<ApprovalRegistry>,
    policy: Weak<PermissionState>,
    epoch: u64,
    revision: u64,
    digest: String,
    issued: Instant,
    consumed: AtomicBool,
}

fn identity(cwd: &Path, name: &str, args: &Value) -> String {
    crate::tool_ledger::canonical_arguments_digest(&serde_json::json!([
        cwd.as_os_str().as_encoded_bytes(),
        name,
        args
    ]))
}

impl ApprovalRegistry {
    pub(crate) fn retain_dispatch(
        self: &Arc<Self>,
        request: &crate::ToolApprovalRequest,
        cwd: &Path,
        policy: &Arc<PermissionState>,
        revision: u64,
    ) -> Result<(), &'static str> {
        let epoch = self.dispatch.epoch.load(Ordering::SeqCst);
        if epoch == u64::MAX {
            return Err("approval generation is exhausted");
        }
        let mut permits = self
            .dispatch
            .permits
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        permits.retain(|_, permit| permit.issued.elapsed() < APPROVAL_TTL);
        if permits.len() >= MAX_PENDING {
            return Err("approved dispatch queue is full");
        }
        permits.insert(
            request.tool_call_id.clone(),
            Arc::new(DispatchPermit {
                registry: Arc::downgrade(self),
                policy: Arc::downgrade(policy),
                epoch,
                revision,
                digest: identity(cwd, &request.tool, &request.args),
                issued: Instant::now(),
                consumed: AtomicBool::new(false),
            }),
        );
        Ok(())
    }

    pub(crate) fn take_dispatch(&self, call_id: &str) -> Option<Arc<DispatchPermit>> {
        self.dispatch
            .permits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(call_id)
    }
}

impl DispatchPermit {
    /// Caller holds the current policy lock and must independently reject Deny.
    pub(crate) fn consume(
        &self,
        policy: &Arc<PermissionState>,
        revision: Option<u64>,
        cwd: &Path,
        name: &str,
        args: &Value,
    ) -> Result<(), &'static str> {
        self.recheck(policy, revision, cwd, name, args)?;
        self.consumed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| "one-call approval was already consumed")
    }

    pub(crate) fn recheck(
        &self,
        policy: &Arc<PermissionState>,
        revision: Option<u64>,
        cwd: &Path,
        name: &str,
        args: &Value,
    ) -> Result<(), &'static str> {
        self.recheck_at(policy, revision, cwd, name, args, Instant::now())
    }

    fn recheck_at(
        &self,
        policy: &Arc<PermissionState>,
        revision: Option<u64>,
        cwd: &Path,
        name: &str,
        args: &Value,
        now: Instant,
    ) -> Result<(), &'static str> {
        let registry = self.registry.upgrade().ok_or("approval owner ended")?;
        let issued_policy = self.policy.upgrade().ok_or("approval policy ended")?;
        let fresh = now
            .checked_duration_since(self.issued)
            .is_some_and(|age| age < APPROVAL_TTL);
        if !Arc::ptr_eq(&issued_policy, policy)
            || revision != Some(self.revision)
            || self.epoch == u64::MAX
            || registry.dispatch.epoch.load(Ordering::SeqCst) != self.epoch
            || !fresh
            || self.digest != identity(cwd, name, args)
        {
            return Err("approval identity or freshness changed; request approval again");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PermissionMode, PermissionPolicy, PermissionVerdict};

    fn fixture() -> (
        Arc<ApprovalRegistry>,
        Arc<PermissionState>,
        Value,
        Arc<DispatchPermit>,
    ) {
        let registry = Arc::new(ApprovalRegistry::default());
        let state = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::Ask,
        )));
        let args = serde_json::json!({"executable":"node", "argv":["server.js"]});
        let PermissionVerdict::Ask(request) =
            state
                .lock()
                .unwrap()
                .decide("call", "process_start", &args, Path::new("."))
        else {
            panic!("expected approval")
        };
        registry
            .retain_dispatch(&request, Path::new("."), &state, 0)
            .unwrap();
        let permit = registry.take_dispatch("call").unwrap();
        assert!(registry.take_dispatch("call").is_none());
        (registry, state, args, permit)
    }

    #[test]
    fn dispatch_consent_runs_once_without_a_session_grant() {
        let (_registry, state, args, permit) = fixture();
        assert!(permit
            .consume(&state, Some(0), Path::new("."), "process_start", &args)
            .is_ok());
        assert!(permit
            .consume(&state, Some(0), Path::new("."), "process_start", &args)
            .is_err());
        assert!(state.lock().unwrap().session_allow.is_empty());
    }

    #[test]
    fn dispatch_consent_rejects_changed_request_owner_policy_and_revocation() {
        for changed in [
            "args", "cwd", "name", "owner", "revision", "revoked", "expired",
        ] {
            let (registry, state, mut args, mut permit) = fixture();
            let other = Arc::new(PermissionState::new(PermissionPolicy::new(
                PermissionMode::Ask,
            )));
            if changed == "args" {
                args["argv"] = serde_json::json!(["different.js"]);
            }
            if changed == "revoked" {
                registry.revoke_all();
            }
            let cwd = Path::new(if changed == "cwd" { "different" } else { "." });
            let name = if changed == "name" {
                "process_stop"
            } else {
                "process_start"
            };
            let revision = Some(if changed == "revision" { 2 } else { 0 });
            let owner = if changed == "owner" { &other } else { &state };
            let rejected = if changed == "expired" {
                let expired_at = permit
                    .issued
                    .checked_add(APPROVAL_TTL)
                    .expect("approval TTL fits in Instant");
                permit
                    .recheck_at(owner, revision, cwd, name, &args, expired_at)
                    .is_err()
            } else {
                permit.consume(owner, revision, cwd, name, &args).is_err()
            };
            assert!(rejected, "{changed}");
        }
    }

    #[test]
    fn dispatch_consent_rejects_policy_aba_and_registry_drop() {
        let (registry, state, args, permit) = fixture();
        let revision = {
            let mut guard = state.lock().unwrap();
            let original = guard.clone();
            guard.mode = PermissionMode::ReadOnly;
            *guard = original;
            guard.revision()
        };
        assert!(permit
            .consume(&state, revision, Path::new("."), "process_start", &args)
            .is_err());
        drop(registry);
        assert!(permit
            .consume(&state, Some(0), Path::new("."), "process_start", &args)
            .is_err());
    }
}
