//! Durable admission for network, MCP, and opaque extension actions.
//!
//! External systems do not share the operation journal's storage or ownership
//! boundary.  This adapter therefore records a redacted invocation binding,
//! latches the possible effect before host I/O, and refuses to replay an
//! unresolved response-loss window without reconciliation evidence.

use super::tools::{ToolOperationDispatchError, ToolOperationRuntime};
use crate::runtime::operations::{
    AdmittedOperation, CallerType, EffectClass, EffectProfile, IdempotencyScope, JournalError,
    OperationAdmission, OperationEvent, OperationId, OperationKind, OperationState, PayloadDigest,
    PlannedToolOperation, Timestamp, ToolOperationPlanError, ToolOperationPlanner,
};
use crate::runtime::RuntimeHandle;
use crate::tools::ToolResult;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub const EXTERNAL_OPERATION_SCHEMA_VERSION: u32 = 1;
const MAX_ENDPOINT_BYTES: usize = 2048;
const MAX_REFERENCE_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalOperationDisposition {
    New,
    ExistingInFlight,
    ExistingResult,
}

#[derive(Debug, thiserror::Error)]
pub enum ExternalOperationError {
    #[error(transparent)]
    Dispatch(#[from] ToolOperationDispatchError),
    #[error(transparent)]
    Plan(#[from] ToolOperationPlanError),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error("external operation call identity must not be empty")]
    EmptyCallId,
    #[error("external operation kind must not be empty")]
    EmptyOperationKind,
    #[error("external endpoint identity must not be empty")]
    EmptyEndpoint,
    #[error("external endpoint identity is too large")]
    EndpointTooLarge,
    #[error("external endpoint identity contains credentials")]
    CredentialsInEndpoint,
    #[error("external endpoint identity contains control characters")]
    InvalidEndpoint,
    #[error("external authorization context must be an opaque label or digest")]
    SensitiveAuthorizationContext,
    #[error("remote idempotency keys require a verified endpoint contract")]
    RemoteKeyRequiresVerifiedContract,
    #[error("remote idempotency key must not be empty")]
    EmptyRemoteKey,
    #[error("external receipt reference is invalid")]
    InvalidReceiptReference,
    #[error("external operation is still owned by its original caller")]
    ExistingInFlight,
    #[error("external operation has no replayable durable result")]
    NotReplayable,
    #[error("response loss requires reconciliation or an authenticated human decision")]
    ResponseLossRequiresReconciliation,
    #[error("external operation binding could not be encoded: {0}")]
    Payload(String),
}

/// Stable endpoint identity.  Query strings, fragments, user names, and
/// passwords are removed so correlation metadata cannot become a credential
/// store.  Callers should pass a digest or short label for authorization_context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalEndpointIdentity {
    pub endpoint: String,
    pub server: Option<String>,
    pub transport: String,
    pub schema_version: Option<String>,
    pub capability_version: Option<String>,
    pub authorization_context: Option<String>,
}

impl ExternalEndpointIdentity {
    pub fn new(
        endpoint: impl Into<String>,
        server: Option<String>,
        transport: impl Into<String>,
        schema_version: Option<String>,
        capability_version: Option<String>,
        authorization_context: Option<String>,
    ) -> Result<Self, ExternalOperationError> {
        let endpoint = sanitize_endpoint(&endpoint.into())?;
        let transport = bounded_label(transport.into(), "transport")?;
        let server = server
            .map(|value| bounded_label(value, "server"))
            .transpose()?;
        let schema_version = schema_version
            .map(|value| bounded_label(value, "schema_version"))
            .transpose()?;
        let capability_version = capability_version
            .map(|value| bounded_label(value, "capability_version"))
            .transpose()?;
        let authorization_context = authorization_context
            .map(sanitize_authorization_context)
            .transpose()?;
        Ok(Self {
            endpoint,
            server,
            transport,
            schema_version,
            capability_version,
            authorization_context,
        })
    }

    pub fn digest(&self) -> Result<PayloadDigest, ExternalOperationError> {
        PayloadDigest::of_json(
            &serde_json::to_value(self)
                .map_err(|error| ExternalOperationError::Payload(error.to_string()))?,
        )
        .map_err(|error| ExternalOperationError::Payload(error.to_string()))
    }
}

/// Endpoint-specific evidence required before an external idempotency key is
/// sent.  An unverified contract intentionally cannot authorize replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalEndpointContract {
    pub verified: bool,
    pub remote_idempotency_key: Option<String>,
    pub supports_receipts: bool,
    pub receipt_refs: Vec<String>,
}

impl ExternalEndpointContract {
    pub fn unverified() -> Self {
        Self {
            verified: false,
            remote_idempotency_key: None,
            supports_receipts: false,
            receipt_refs: Vec::new(),
        }
    }

    pub fn verified_with_remote_key(
        key: impl Into<String>,
    ) -> Result<Self, ExternalOperationError> {
        let key = key.into();
        if key.trim().is_empty() {
            return Err(ExternalOperationError::EmptyRemoteKey);
        }
        validate_reference(&key)?;
        Ok(Self {
            verified: true,
            remote_idempotency_key: Some(key),
            supports_receipts: false,
            receipt_refs: Vec::new(),
        })
    }

    pub fn with_receipt_refs(
        mut self,
        receipt_refs: Vec<String>,
    ) -> Result<Self, ExternalOperationError> {
        for reference in &receipt_refs {
            validate_reference(reference)?;
        }
        self.supports_receipts = !receipt_refs.is_empty();
        self.receipt_refs = receipt_refs;
        Ok(self)
    }

    fn validate(&self) -> Result<(), ExternalOperationError> {
        if self.remote_idempotency_key.is_some() && !self.verified {
            return Err(ExternalOperationError::RemoteKeyRequiresVerifiedContract);
        }
        if let Some(key) = &self.remote_idempotency_key {
            validate_reference(key)?;
        }
        for reference in &self.receipt_refs {
            validate_reference(reference)?;
        }
        Ok(())
    }
}

/// Redacted, replay-safe metadata persisted in the operation payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalInvocationBinding {
    pub schema_version: u32,
    pub operation_kind: String,
    pub endpoint: ExternalEndpointIdentity,
    pub endpoint_digest: PayloadDigest,
    pub request_digest: PayloadDigest,
    pub request_bytes: usize,
    pub authorization_context: Option<String>,
    pub remote_idempotency_key: Option<String>,
    pub supported_receipt_refs: Vec<String>,
    pub opaque_extension: bool,
}

#[derive(Clone)]
pub struct ExternalOperationAdapter {
    runtime: ToolOperationRuntime,
}

impl std::fmt::Debug for ExternalOperationAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExternalOperationAdapter")
    }
}

#[derive(Clone)]
pub struct ExternalOperationHandle {
    adapter: ExternalOperationAdapter,
    pub admitted: AdmittedOperation,
    plan: PlannedToolOperation,
    binding: ExternalInvocationBinding,
    disposition: ExternalOperationDisposition,
}

impl std::fmt::Debug for ExternalOperationHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalOperationHandle")
            .field("operation_id", &self.operation_id())
            .field("attempt_id", &self.attempt_id())
            .field("binding", &self.binding)
            .field("disposition", &self.disposition)
            .finish()
    }
}

impl ExternalOperationHandle {
    pub fn operation_id(&self) -> OperationId {
        self.admitted.spec.operation_id()
    }

    pub fn attempt_id(&self) -> crate::runtime::operations::AttemptId {
        self.admitted.attempt.attempt_id()
    }

    pub fn disposition(&self) -> ExternalOperationDisposition {
        self.disposition
    }

    pub fn should_execute(&self) -> bool {
        self.disposition == ExternalOperationDisposition::New
    }

    pub fn binding(&self) -> &ExternalInvocationBinding {
        &self.binding
    }

    pub fn metadata(&self) -> Value {
        json!({
            "operation_id": self.operation_id().to_string(),
            "attempt_id": self.attempt_id().to_string(),
            "binding": &self.binding,
        })
    }

    pub fn replay_result(&self) -> Result<ToolResult, ExternalOperationError> {
        if self.disposition != ExternalOperationDisposition::ExistingResult {
            return Err(ExternalOperationError::NotReplayable);
        }
        self.adapter
            .runtime
            .dispatcher()
            .replay_result(&self.admitted)
            .map_err(Into::into)
    }

    /// Claims and latches the operation before the external adapter performs
    /// any network, MCP, hook, or extension-host I/O.
    pub fn begin<T>(
        &self,
        cancelled: bool,
        revalidate: impl FnOnce() -> Result<(), String>,
        latch: impl FnOnce() -> T,
    ) -> Result<T, ExternalOperationError> {
        if !self.should_execute() {
            return match self.disposition {
                ExternalOperationDisposition::ExistingInFlight => {
                    Err(ExternalOperationError::ExistingInFlight)
                }
                ExternalOperationDisposition::ExistingResult => {
                    Err(ExternalOperationError::NotReplayable)
                }
                ExternalOperationDisposition::New => unreachable!(),
            };
        }
        let permit = self.adapter.runtime.dispatcher().begin_dispatch(
            &self.admitted,
            &self.plan,
            self.admitted
                .attempt
                .authorization()
                .and_then(|receipt| receipt.policy_revision.parse::<u64>().ok()),
            cancelled,
            revalidate,
        )?;
        permit
            .dispatch(self.adapter.runtime.dispatcher().journal(), latch)
            .map_err(ExternalOperationError::Journal)
    }

    pub fn complete(&self, result: &ToolResult) -> Result<(), ExternalOperationError> {
        if !self.should_execute() {
            return Err(ExternalOperationError::ExistingInFlight);
        }
        let safe = redacted_external_tool_result(result);
        self.adapter
            .runtime
            .dispatcher()
            .complete_for_session(&self.admitted, &safe, false)
            .map_err(Into::into)
    }

    /// Mark a response-loss window as requiring reconciliation.  No external
    /// callback is invoked here and no automatic replay is permitted.
    pub fn response_lost(&self) -> Result<(), ExternalOperationError> {
        if !self.should_execute() {
            return Err(ExternalOperationError::ResponseLossRequiresReconciliation);
        }
        let journal = self.adapter.runtime.dispatcher().journal();
        let attempt = journal.load_attempt(self.attempt_id())?;
        if !matches!(
            attempt.state(),
            OperationState::Running | OperationState::EffectPossible
        ) {
            return Err(ExternalOperationError::ResponseLossRequiresReconciliation);
        }
        let interrupted = journal.transition(
            self.adapter.runtime.dispatcher().owner(),
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Interrupt { at: now() },
            None,
            Vec::new(),
        )?;
        journal.transition(
            self.adapter.runtime.dispatcher().owner(),
            interrupted.attempt_id(),
            interrupted.revision(),
            OperationEvent::RequireRecovery,
            None,
            Vec::new(),
        )?;
        Ok(())
    }
}

impl ExternalOperationAdapter {
    pub fn new(runtime: ToolOperationRuntime) -> Self {
        Self { runtime }
    }

    pub fn from_runtime(runtime: &RuntimeHandle) -> Option<Self> {
        runtime.operations.clone().map(Self::new)
    }

    pub fn runtime(&self) -> &ToolOperationRuntime {
        &self.runtime
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &self,
        call_id: &str,
        operation_kind: &str,
        args: &Value,
        endpoint: ExternalEndpointIdentity,
        contract: ExternalEndpointContract,
        opaque_extension: bool,
        permission_revision: u64,
    ) -> Result<ExternalOperationHandle, ExternalOperationError> {
        if call_id.trim().is_empty() {
            return Err(ExternalOperationError::EmptyCallId);
        }
        if operation_kind.trim().is_empty() {
            return Err(ExternalOperationError::EmptyOperationKind);
        }
        contract.validate()?;
        let request_bytes = serde_json::to_vec(args)
            .map_err(|error| ExternalOperationError::Payload(error.to_string()))?
            .len();
        let binding = ExternalInvocationBinding {
            schema_version: EXTERNAL_OPERATION_SCHEMA_VERSION,
            operation_kind: operation_kind.to_owned(),
            endpoint_digest: endpoint.digest()?,
            authorization_context: endpoint.authorization_context.clone(),
            remote_idempotency_key: contract.remote_idempotency_key.clone(),
            supported_receipt_refs: contract.receipt_refs.clone(),
            endpoint,
            request_digest: PayloadDigest::of_json(args)
                .map_err(|error| ExternalOperationError::Payload(error.to_string()))?,
            request_bytes,
            opaque_extension,
        };
        let payload = json!({
            "schema_version": EXTERNAL_OPERATION_SCHEMA_VERSION,
            "binding": &binding,
        });
        let effects = EffectProfile {
            classification: EffectClass::ExternalMutation,
            supports_idempotency_key: contract.verified
                && contract.remote_idempotency_key.is_some(),
            supports_postcondition_probe: contract.supports_receipts,
            supports_compensation: false,
            requires_live_owner: true,
        };
        let caller = if binding.endpoint.transport.eq_ignore_ascii_case("mcp") {
            CallerType::Mcp
        } else {
            CallerType::Custom
        };
        let kind = if caller == CallerType::Mcp {
            OperationKind::McpCall
        } else if binding.endpoint.transport.eq_ignore_ascii_case("http")
            || binding.endpoint.transport.eq_ignore_ascii_case("https")
            || binding.endpoint.transport.eq_ignore_ascii_case("network")
        {
            OperationKind::NetworkAction
        } else {
            OperationKind::CustomExternalAction
        };
        let plan = ToolOperationPlanner::managed_execution(
            self.runtime.operation_context().clone(),
            caller,
            IdempotencyScope::CallerDefined,
            &format!("external:{call_id}"),
            kind,
            effects,
            payload,
        )?;
        let admission = self
            .runtime
            .dispatcher()
            .admit(plan.clone(), permission_revision)?;
        let (admitted, disposition) = match admission {
            OperationAdmission::New(admitted) => (admitted, ExternalOperationDisposition::New),
            OperationAdmission::ExistingInFlight(admitted) => {
                (admitted, ExternalOperationDisposition::ExistingInFlight)
            }
            OperationAdmission::ExistingResult(admitted) => {
                (admitted, ExternalOperationDisposition::ExistingResult)
            }
            OperationAdmission::Collision => {
                return Err(ExternalOperationError::Journal(
                    JournalError::IdempotencyCollision,
                ));
            }
        };
        Ok(ExternalOperationHandle {
            adapter: self.clone(),
            admitted,
            plan,
            binding,
            disposition,
        })
    }
}

fn sanitize_endpoint(value: &str) -> Result<String, ExternalOperationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ExternalOperationError::EmptyEndpoint);
    }
    if value.len() > MAX_ENDPOINT_BYTES || value.chars().any(|c| c.is_control()) {
        return Err(ExternalOperationError::InvalidEndpoint);
    }
    if let Ok(mut url) = url::Url::parse(value) {
        if !url.username().is_empty() || url.password().is_some() {
            return Err(ExternalOperationError::CredentialsInEndpoint);
        }
        url.set_query(None);
        url.set_fragment(None);
        return Ok(url.to_string());
    }
    if value.contains('@') {
        return Err(ExternalOperationError::CredentialsInEndpoint);
    }
    Ok(value.split(['?', '#']).next().unwrap_or(value).to_owned())
}

fn bounded_label(value: String, _field: &str) -> Result<String, ExternalOperationError> {
    if value.trim().is_empty()
        || value.len() > MAX_REFERENCE_BYTES
        || value.chars().any(|c| c.is_control())
    {
        return Err(ExternalOperationError::InvalidEndpoint);
    }
    Ok(value)
}

fn sanitize_authorization_context(value: String) -> Result<String, ExternalOperationError> {
    let value = bounded_label(value, "authorization_context")?;
    let lower = value.to_ascii_lowercase();
    if [
        "password",
        "secret",
        "token",
        "cookie",
        "authorization",
        "api_key",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return Err(ExternalOperationError::SensitiveAuthorizationContext);
    }
    Ok(value)
}

fn validate_reference(value: &str) -> Result<(), ExternalOperationError> {
    if value.trim().is_empty()
        || value.len() > MAX_REFERENCE_BYTES
        || value.chars().any(|c| c.is_control())
        || value.to_ascii_lowercase().contains("secret")
        || value.to_ascii_lowercase().contains("token")
    {
        return Err(ExternalOperationError::InvalidReceiptReference);
    }
    Ok(())
}

fn now() -> Timestamp {
    Timestamp::from_unix_millis(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default(),
    )
}

const SENSITIVE_KEYS: &[&str] = &[
    "password",
    "passcode",
    "token",
    "secret",
    "authorization",
    "cookie",
    "credential",
    "api_key",
    "access_token",
    "refresh_token",
    "client_secret",
    "private_key",
    "headers",
];

fn sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    SENSITIVE_KEYS.iter().any(|candidate| {
        candidate
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
            == normalized
    })
}

fn redacted_scalar(value: &Value) -> Value {
    let bytes = match value {
        Value::String(text) => text.as_bytes().to_vec(),
        _ => serde_json::to_vec(value).unwrap_or_default(),
    };
    let digest = Sha256::digest(&bytes);
    json!({
        "redacted": true,
        "sha256": format!("{digest:x}"),
        "length": bytes.len(),
    })
}

fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut redacted = Map::new();
            for (key, child) in map {
                redacted.insert(
                    key.clone(),
                    if sensitive_key(key) {
                        redacted_scalar(child)
                    } else {
                        redact_value(child)
                    },
                );
            }
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        other => other.clone(),
    }
}

pub fn redacted_external_tool_result(result: &ToolResult) -> ToolResult {
    ToolResult {
        content: redacted_scalar(&Value::String(result.content.clone()))
            .get("sha256")
            .and_then(Value::as_str)
            .map(|digest| format!("[external result redacted; sha256={digest}]"))
            .unwrap_or_else(|| "[external result redacted]".to_owned()),
        is_error: result.is_error,
        details: result.details.as_ref().map(redact_value),
    }
}
