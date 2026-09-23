//! Durable admission for native browser actions.
//!
//! Browser calls are native extensions, so they do not pass through the
//! built-in provider operation path.  This adapter gives them the same
//! journaled intent, single-use effect latch, and conservative result
//! publication while keeping the live browser lease host-owned.

use super::tools::{ToolOperationDispatchError, ToolOperationRuntime};
use crate::process_manager::BrowserLeaseIdentity;
use crate::runtime::operations::{
    AdmittedOperation, CallerType, EffectClass, EffectProfile, IdempotencyScope, JournalError,
    OperationAdmission, OperationId, OperationKind, PayloadDigest, PlannedToolOperation,
    ToolOperationPlanError, ToolOperationPlanner,
};
use crate::runtime::RuntimeHandle;
use crate::tools::ToolResult;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const BROWSER_OPERATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserOperationDisposition {
    New,
    ExistingInFlight,
    ExistingResult,
}

#[derive(Debug, thiserror::Error)]
pub enum BrowserOperationError {
    #[error(transparent)]
    Dispatch(#[from] ToolOperationDispatchError),
    #[error(transparent)]
    Plan(#[from] ToolOperationPlanError),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error("browser action name must not be empty")]
    EmptyAction,
    #[error("browser tool call identity must not be empty")]
    EmptyCallId,
    #[error("browser operation is still owned by its original caller")]
    ExistingInFlight,
    #[error("browser operation has no replayable durable result")]
    NotReplayable,
    #[error("browser operation payload could not be encoded: {0}")]
    Payload(String),
}

/// The persisted browser binding contains process and page identity only.
/// It intentionally excludes URLs, form values, headers, cookies, and other
/// credential-bearing browser data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserActionBinding {
    pub schema_version: u32,
    pub action: String,
    pub browser_id: Option<String>,
    pub page_id: Option<String>,
    pub lease: BrowserLeaseIdentity,
    pub arguments_digest: PayloadDigest,
    pub arguments_bytes: usize,
}

#[derive(Clone)]
pub struct BrowserOperationAdapter {
    runtime: ToolOperationRuntime,
}

impl std::fmt::Debug for BrowserOperationAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserOperationAdapter")
    }
}

#[derive(Clone)]
pub struct BrowserOperationHandle {
    adapter: BrowserOperationAdapter,
    pub admitted: AdmittedOperation,
    plan: PlannedToolOperation,
    binding: BrowserActionBinding,
    disposition: BrowserOperationDisposition,
}

impl std::fmt::Debug for BrowserOperationHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserOperationHandle")
            .field("operation_id", &self.operation_id())
            .field("attempt_id", &self.attempt_id())
            .field("binding", &self.binding)
            .field("disposition", &self.disposition)
            .finish()
    }
}

impl BrowserOperationHandle {
    pub fn operation_id(&self) -> OperationId {
        self.admitted.spec.operation_id()
    }

    pub fn attempt_id(&self) -> crate::runtime::operations::AttemptId {
        self.admitted.attempt.attempt_id()
    }

    pub fn disposition(&self) -> BrowserOperationDisposition {
        self.disposition
    }

    pub fn should_execute(&self) -> bool {
        self.disposition == BrowserOperationDisposition::New
    }

    pub fn binding(&self) -> &BrowserActionBinding {
        &self.binding
    }

    pub fn metadata(&self) -> Value {
        json!({
            "operation_id": self.operation_id().to_string(),
            "attempt_id": self.attempt_id().to_string(),
            "binding": &self.binding,
        })
    }

    pub fn replay_result(&self) -> Result<ToolResult, BrowserOperationError> {
        if self.disposition != BrowserOperationDisposition::ExistingResult {
            return Err(BrowserOperationError::NotReplayable);
        }
        self.adapter
            .runtime
            .dispatcher()
            .replay_result(&self.admitted)
            .map_err(Into::into)
    }

    /// Claim the operation and latch a possible browser effect before any
    /// engine or page I/O.  The closure is deliberately side-effect free.
    pub fn begin<T>(
        &self,
        cancelled: bool,
        revalidate: impl FnOnce() -> Result<(), String>,
        latch: impl FnOnce() -> T,
    ) -> Result<T, BrowserOperationError> {
        if !self.should_execute() {
            return match self.disposition {
                BrowserOperationDisposition::ExistingInFlight => {
                    Err(BrowserOperationError::ExistingInFlight)
                }
                BrowserOperationDisposition::ExistingResult => {
                    Err(BrowserOperationError::NotReplayable)
                }
                BrowserOperationDisposition::New => unreachable!(),
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
            .map_err(BrowserOperationError::Journal)
    }

    /// Persist a sanitized result.  Browser result content is treated as
    /// untrusted page data and is never written to the journal verbatim.
    pub fn complete(&self, result: &ToolResult) -> Result<(), BrowserOperationError> {
        if !self.should_execute() {
            return Err(BrowserOperationError::ExistingInFlight);
        }
        let safe = redacted_browser_tool_result(&self.binding.action, result);
        self.adapter
            .runtime
            .dispatcher()
            .complete_for_session(&self.admitted, &safe, false)
            .map_err(Into::into)
    }
}

impl BrowserOperationAdapter {
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
        action: &str,
        args: &Value,
        browser_id: Option<String>,
        page_id: Option<String>,
        lease: BrowserLeaseIdentity,
        permission_revision: u64,
    ) -> Result<BrowserOperationHandle, BrowserOperationError> {
        if call_id.trim().is_empty() {
            return Err(BrowserOperationError::EmptyCallId);
        }
        if action.trim().is_empty() {
            return Err(BrowserOperationError::EmptyAction);
        }
        let arguments_bytes = serde_json::to_vec(args)
            .map_err(|error| BrowserOperationError::Payload(error.to_string()))?
            .len();
        let arguments_digest = PayloadDigest::of_json(args)
            .map_err(|error| BrowserOperationError::Payload(error.to_string()))?;
        let binding = BrowserActionBinding {
            schema_version: BROWSER_OPERATION_SCHEMA_VERSION,
            action: action.to_owned(),
            browser_id,
            page_id,
            lease,
            arguments_digest,
            arguments_bytes,
        };
        let payload = json!({
            "schema_version": BROWSER_OPERATION_SCHEMA_VERSION,
            "binding": &binding,
        });
        let effects = EffectProfile {
            classification: EffectClass::ExternalMutation,
            supports_idempotency_key: false,
            supports_postcondition_probe: false,
            supports_compensation: false,
            requires_live_owner: true,
        };
        let plan = ToolOperationPlanner::managed_execution(
            self.runtime.operation_context().clone(),
            CallerType::Browser,
            IdempotencyScope::CallerDefined,
            &format!("browser:{call_id}"),
            OperationKind::BrowserAction,
            effects,
            payload,
        )?;
        let admission = self
            .runtime
            .dispatcher()
            .admit(plan.clone(), permission_revision)?;
        let (admitted, disposition) = match admission {
            OperationAdmission::New(admitted) => (admitted, BrowserOperationDisposition::New),
            OperationAdmission::ExistingInFlight(admitted) => {
                (admitted, BrowserOperationDisposition::ExistingInFlight)
            }
            OperationAdmission::ExistingResult(admitted) => {
                (admitted, BrowserOperationDisposition::ExistingResult)
            }
            OperationAdmission::Collision => {
                return Err(BrowserOperationError::Dispatch(
                    ToolOperationDispatchError::Journal(JournalError::IdempotencyCollision),
                ));
            }
        };
        Ok(BrowserOperationHandle {
            adapter: self.clone(),
            admitted,
            plan,
            binding,
            disposition,
        })
    }
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
];

fn sensitive_key(key: &str, action: &str) -> bool {
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
    }) || (action == "browser_type" && normalized == "text")
}

fn redacted_scalar(value: &Value) -> Value {
    let bytes = match value {
        Value::String(text) => text.as_bytes().to_vec(),
        _ => serde_json::to_vec(value).unwrap_or_default(),
    };
    let digest = Sha256::digest(&bytes);
    let hash = format!("{digest:x}");
    json!({"redacted": true, "sha256": hash, "length": bytes.len()})
}

fn redact_value(value: &Value, action: &str) -> Value {
    match value {
        Value::Object(object) => {
            let mut output = Map::new();
            for (key, value) in object {
                output.insert(
                    key.clone(),
                    if sensitive_key(key, action) {
                        redacted_scalar(value)
                    } else {
                        redact_value(value, action)
                    },
                );
            }
            Value::Object(output)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_value(item, action))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// Redact credential-bearing values in a browser payload or evidence object.
pub fn redact_browser_value(value: &Value, action: &str) -> Value {
    redact_value(value, action)
}

/// Return a journal-safe browser result.  A non-JSON `browser_type` response
/// is replaced wholesale because it cannot be inspected for echoed form data.
pub fn redacted_browser_tool_result(action: &str, result: &ToolResult) -> ToolResult {
    let content = if let Ok(value) = serde_json::from_str::<Value>(&result.content) {
        redact_value(&value, action).to_string()
    } else if action == "browser_type" {
        "[redacted browser_type result]".to_owned()
    } else {
        result.content.clone()
    };
    ToolResult {
        content,
        is_error: result.is_error,
        details: result
            .details
            .as_ref()
            .map(|details| redact_value(details, action)),
    }
}
