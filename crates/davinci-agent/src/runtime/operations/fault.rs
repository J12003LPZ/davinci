use serde::{Deserialize, Serialize};

/// Explicit crash-test boundaries. Production execution does not read an
/// environment variable or install a default fault injector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultPoint {
    BeforeIntentCommit,
    AfterIntentCommit,
    AfterAttemptClaim,
    AfterEffectLatch,
    AfterSideEffect,
    BeforeResultCommit,
    AfterResultCommit,
    BeforePublication,
    AfterSinkCommit,
    BeforeOutboxAcknowledgement,
}

/// A typed hook for crash-test harnesses. Runtime code never constructs an
/// implementation from ambient process state.
pub trait FaultInjector {
    fn checkpoint(&mut self, point: FaultPoint);
}
