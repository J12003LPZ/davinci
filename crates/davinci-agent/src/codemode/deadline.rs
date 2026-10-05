use super::{CodeModeError, CodeModeLimits, CodeModeRequest, CodeModeRunContext};
use crate::runtime::capacity::RootBudget;
use std::time::{SystemTime, UNIX_EPOCH};

impl CodeModeRunContext {
    /// Bound a host launch by the unchanged, persisted root deadline.
    /// Call again immediately before starting the host watchdog after setup.
    pub fn limits_for_request(
        &self,
        request: &CodeModeRequest,
    ) -> Result<CodeModeLimits, CodeModeError> {
        let mut limits = self.limits.for_request(request)?;
        if let Some(remaining) = remaining_root_wall_ms(self.root_budget.as_ref())? {
            limits.wall_ms = limits.wall_ms.min(remaining);
        }
        Ok(limits)
    }
}

/// Read the original root allowance; a script never creates or renews it.
pub(crate) fn remaining_root_wall_ms(
    budget: Option<&RootBudget>,
) -> Result<Option<u64>, CodeModeError> {
    let Some(budget) = budget else {
        return Ok(None);
    };
    let snapshot = budget
        .snapshot()
        .map_err(|_| CodeModeError::new("UNAVAILABLE", "root budget evidence unavailable"))?;
    if snapshot.halted {
        return Err(CodeModeError::new("LIMIT_EXCEEDED", "root budget halted"));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .ok_or_else(|| CodeModeError::new("UNAVAILABLE", "root deadline clock unavailable"))?;
    match budget.limits().deadline_unix_ms.checked_sub(now) {
        Some(remaining) if remaining > 0 => Ok(Some(remaining)),
        _ => Err(CodeModeError::new("CANCELLED", "root deadline exhausted")),
    }
}
