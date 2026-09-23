use super::{AttemptId, CallerType, OperationId, OperationSpec, OutboxDraft, PayloadDigest};
use serde::{Deserialize, Serialize};

pub const SESSION_RESULT_CONSUMER: &str = "session_result_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationResultReady {
    pub version: u32,
    pub event_id: String,
    pub session_id: String,
    pub tool_call_id: String,
    pub operation_id: OperationId,
    pub attempt_id: AttemptId,
    pub parent_operation_id: Option<OperationId>,
    pub caller: CallerType,
    pub result_digest: PayloadDigest,
}

impl OperationResultReady {
    pub fn new(
        spec: &OperationSpec,
        attempt_id: AttemptId,
        result_digest: PayloadDigest,
    ) -> Result<Self, String> {
        let context = spec.context();
        let tool_call_id = context
            .wire_tool_call_id
            .clone()
            .ok_or_else(|| "operation result has no wire tool call ID".to_owned())?;
        Ok(Self {
            version: 1,
            event_id: Self::event_id(attempt_id),
            session_id: context.session_id.clone(),
            tool_call_id,
            operation_id: spec.operation_id(),
            attempt_id,
            parent_operation_id: context.parent_operation_id,
            caller: context.caller,
            result_digest,
        })
    }

    pub fn event_id(attempt_id: AttemptId) -> String {
        format!("operation-result-{attempt_id}")
    }

    pub fn into_draft(self) -> Result<OutboxDraft, super::JournalError> {
        OutboxDraft::new(
            SESSION_RESULT_CONSUMER,
            serde_json::to_value(self)
                .map_err(|error| super::JournalError::Serialization(error.to_string()))?,
        )
    }
}
