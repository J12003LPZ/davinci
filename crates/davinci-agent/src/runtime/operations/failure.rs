use serde::{Deserialize, Serialize};

pub const CAUSAL_FAILURE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureSubsystem {
    Persistence,
    Authorization,
    Admission,
    Execution,
    Process,
    Verification,
    Publication,
    Recovery,
    Schema,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReasonCode {
    Persistence,
    AuthorizationDenied,
    IdempotencyCollision,
    OwnerLoss,
    ProcessIdentityMismatch,
    UnknownEffect,
    VerificationFailed,
    PublicationFailed,
    Corruption,
    UnsupportedSchema,
    InvalidTransition,
    RevisionConflict,
    ResourceConflict,
    UnsupportedOperation,
    Cancellation,
    Timeout,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(deny_unknown_fields)]
#[error("{subsystem:?} operation failure ({reason:?})")]
pub struct OperationFailure {
    pub reason: FailureReasonCode,
    pub subsystem: FailureSubsystem,
    #[source]
    pub cause: Option<Box<OperationFailure>>,
}

impl OperationFailure {
    pub fn new(reason: FailureReasonCode, subsystem: FailureSubsystem) -> Self {
        Self {
            reason,
            subsystem,
            cause: None,
        }
    }

    pub fn caused_by(mut self, cause: OperationFailure) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }
}

/// Certainty of an external or host-side effect at the time a failure was
/// reported.  `Unknown` is intentionally distinct from `Possible`: it means
/// there is no evidence strong enough to claim that the adapter reached its
/// effect boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectCertainty {
    KnownNoEffect,
    KnownEffect,
    PossibleEffect,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableResultStatus {
    NotCommitted,
    Committed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    NotRequired,
    Pending,
    Verified,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationStatus {
    NotRequired,
    Pending,
    Published,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowedRecoveryAction {
    None,
    ReplayDurableResult,
    ReconcileBeforeReplay,
    AuthenticatedHumanDecision,
    Block,
}

/// A bounded, redacted causal explanation that can be shown by inspectors and
/// host UIs without reducing every failure to a generic worker error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CausalFailureReport {
    pub schema_version: u32,
    pub operation_id: Option<String>,
    pub attempt_id: Option<String>,
    pub root_namespace_id: Option<String>,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub worker_id: Option<String>,
    pub causal_parent: Option<String>,
    pub attempt: Option<u32>,
    pub revision: Option<u64>,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub subsystem: FailureSubsystem,
    pub reason: FailureReasonCode,
    pub message: String,
    pub occurred_at_ms: u64,
    pub artifact_refs: Vec<String>,
    pub effect_certainty: EffectCertainty,
    pub durable_result: DurableResultStatus,
    pub verification: VerificationStatus,
    pub publication: PublicationStatus,
    pub allowed_recovery: AllowedRecoveryAction,
}

impl CausalFailureReport {
    pub fn new(
        subsystem: FailureSubsystem,
        reason: FailureReasonCode,
        message: impl AsRef<str>,
        occurred_at_ms: u64,
    ) -> Self {
        Self {
            schema_version: CAUSAL_FAILURE_SCHEMA_VERSION,
            operation_id: None,
            attempt_id: None,
            root_namespace_id: None,
            run_id: None,
            session_id: None,
            agent_id: None,
            worker_id: None,
            causal_parent: None,
            attempt: None,
            revision: None,
            from_state: None,
            to_state: None,
            subsystem,
            reason,
            message: crate::runtime::contracts::redact_secrets(message.as_ref()),
            occurred_at_ms,
            artifact_refs: Vec::new(),
            effect_certainty: EffectCertainty::Unknown,
            durable_result: DurableResultStatus::Unknown,
            verification: VerificationStatus::Unknown,
            publication: PublicationStatus::Unknown,
            allowed_recovery: AllowedRecoveryAction::Block,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_identity(
        mut self,
        operation_id: Option<String>,
        attempt_id: Option<String>,
        root_namespace_id: Option<String>,
        run_id: Option<String>,
        session_id: Option<String>,
        agent_id: Option<String>,
        worker_id: Option<String>,
    ) -> Self {
        self.operation_id = operation_id;
        self.attempt_id = attempt_id;
        self.root_namespace_id = root_namespace_id;
        self.run_id = run_id;
        self.session_id = session_id;
        self.agent_id = agent_id;
        self.worker_id = worker_id;
        self
    }

    pub fn with_transition(
        mut self,
        causal_parent: Option<String>,
        attempt: Option<u32>,
        revision: Option<u64>,
        from_state: Option<String>,
        to_state: Option<String>,
    ) -> Self {
        self.causal_parent = causal_parent;
        self.attempt = attempt;
        self.revision = revision;
        self.from_state = from_state;
        self.to_state = to_state;
        self
    }

    pub fn with_outcome(
        mut self,
        effect_certainty: EffectCertainty,
        durable_result: DurableResultStatus,
        verification: VerificationStatus,
        publication: PublicationStatus,
        allowed_recovery: AllowedRecoveryAction,
    ) -> Self {
        self.effect_certainty = effect_certainty;
        self.durable_result = durable_result;
        self.verification = verification;
        self.publication = publication;
        self.allowed_recovery = allowed_recovery;
        self
    }

    pub fn with_artifact_refs(mut self, refs: impl IntoIterator<Item = String>) -> Self {
        self.artifact_refs = refs
            .into_iter()
            .filter(|reference| !reference.trim().is_empty())
            .map(|reference| crate::runtime::contracts::redact_secrets(&reference))
            .take(64)
            .collect();
        self
    }

    pub fn failed_subsystem(&self) -> FailureSubsystem {
        self.subsystem
    }

    pub fn is_recoverable_without_replay(&self) -> bool {
        matches!(
            self.allowed_recovery,
            AllowedRecoveryAction::ReplayDurableResult | AllowedRecoveryAction::None
        ) && !matches!(
            self.effect_certainty,
            EffectCertainty::PossibleEffect | EffectCertainty::Unknown
        )
    }
}
