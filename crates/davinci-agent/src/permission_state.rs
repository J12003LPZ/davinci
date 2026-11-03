//! Revisioned ownership for the native permission policy (no upstream TS equivalent).
//! Policy values remain in permission.rs; this is the existing agent lock boundary.

use crate::PermissionPolicy;
use std::ops::{Deref, DerefMut};
use std::sync::{LockResult, Mutex, MutexGuard, PoisonError};

#[derive(Debug)]
struct State {
    policy: PermissionPolicy,
    revision: Option<u64>,
}

/// Shared policy with a monotonic revision for approval freshness checks.
/// Mutable access invalidates prior approvals even when the value is restored.
/// Replacing the policy through a guard retains this state's revision history.
#[derive(Debug)]
pub struct PermissionState(Mutex<State>);

impl PermissionState {
    pub fn new(policy: PermissionPolicy) -> Self {
        Self(Mutex::new(State {
            policy,
            revision: Some(0),
        }))
    }

    pub fn lock(&self) -> LockResult<PermissionGuard<'_>> {
        match self.0.lock() {
            Ok(inner) => Ok(PermissionGuard { inner }),
            Err(err) => Err(PoisonError::new(PermissionGuard {
                inner: err.into_inner(),
            })),
        }
    }

    /// Point the read-boundary exception at the agent's own evidence store.
    /// This is a trusted path pointer, not a user grant, so it must not
    /// invalidate pending approvals by advancing the revision.
    pub fn set_evidence_access(
        &self,
        root: Option<std::path::PathBuf>,
        session_token: Option<String>,
    ) {
        match self.0.lock() {
            Ok(mut state) => state.policy.set_evidence_access(root, session_token),
            Err(err) => err
                .into_inner()
                .policy
                .set_evidence_access(root, session_token),
        }
    }
}

#[derive(Debug)]
pub struct PermissionGuard<'a> {
    inner: MutexGuard<'a, State>,
}

impl PermissionGuard<'_> {
    /// None means the revision counter was exhausted; pending grants must fail closed.
    pub fn revision(&self) -> Option<u64> {
        self.inner.revision
    }
}

impl Deref for PermissionGuard<'_> {
    type Target = PermissionPolicy;

    fn deref(&self) -> &Self::Target {
        &self.inner.policy
    }
}

impl DerefMut for PermissionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.revision = self.inner.revision.and_then(|rev| rev.checked_add(1));
        &mut self.inner.policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PermissionMode;

    #[test]
    fn evidence_root_does_not_advance_the_revision() {
        let state = PermissionState::new(PermissionPolicy::new(PermissionMode::Ask));
        state.set_evidence_access(
            Some(std::env::temp_dir().join("evidence")),
            Some("token".into()),
        );
        assert_eq!(state.lock().unwrap().revision(), Some(0));
    }

    #[test]
    fn reads_preserve_revision_and_replacement_cannot_reset_it() {
        let state = PermissionState::new(PermissionPolicy::new(PermissionMode::Ask));
        let mut guard = state.lock().unwrap();
        let original = guard.clone();
        assert_eq!(guard.revision(), Some(0));
        guard.mode = PermissionMode::Auto;
        *guard = original;
        assert_eq!(guard.mode, PermissionMode::Ask);
        assert_eq!(guard.revision(), Some(2));
    }

    #[test]
    fn revision_exhaustion_never_wraps_back_to_valid() {
        let state = PermissionState::new(PermissionPolicy::default());
        state.0.lock().unwrap().revision = Some(u64::MAX);
        let mut guard = state.lock().unwrap();
        guard.mode = PermissionMode::Ask;
        assert_eq!(guard.revision(), None);
        guard.mode = PermissionMode::Auto;
        assert_eq!(guard.revision(), None);
    }
}
