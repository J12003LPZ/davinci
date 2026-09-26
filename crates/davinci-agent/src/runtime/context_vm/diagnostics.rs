//! Host-visible Context VM diagnostics: recorded failures and fallbacks,
//! one-shot notices, and whether exact recovery has been offered to the model.
//! No TypeScript counterpart; the Context VM is a Davinci runtime feature.

use super::{ContextVmRuntime, ShadowComparison};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Failures kept for `/status`; the total count is kept separately.
const MAX_RECENT_FAILURES: usize = 8;
/// Notices wait for a host to drain them; a host that never drains keeps a bound.
const MAX_PENDING_NOTICES: usize = 16;

/// One Context VM failure. `stage` names where it happened (`compile`,
/// `append_delta`, `fold`, `fold_proposal`) and `reason` what went wrong,
/// including the fallback that was taken instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextVmFailure {
    pub stage: String,
    pub reason: String,
}

impl ContextVmFailure {
    pub fn render(&self) -> String {
        format!("{}: {}", self.stage, self.reason)
    }
}

#[derive(Debug, Default)]
pub(crate) struct ContextVmDiagnostics {
    recent_failures: VecDeque<ContextVmFailure>,
    failure_count: u64,
    /// Failures not yet written into a prepared context manifest.
    unreported_failures: Vec<ContextVmFailure>,
    notices: Vec<String>,
    failure_notified: bool,
    shadow_mismatch: bool,
    /// Sticky: once the VM has folded or paged events out of the image, the
    /// model keeps `retrieve_context` so the tool catalog does not flap.
    retrieval_offered: bool,
}

impl ContextVmRuntime {
    /// Record a failure or fallback. The first one in this runtime (one per
    /// session) also queues a notice; later ones are counted for `/status`
    /// and the prepared context manifest. A repeat of the immediately
    /// preceding failure is the same failure observed again and is not recounted.
    pub fn record_failure(&self, stage: &str, reason: impl Into<String>) {
        let failure = ContextVmFailure {
            stage: stage.into(),
            reason: reason.into(),
        };
        let mut diagnostics = self.diagnostics.write().unwrap_or_else(|e| e.into_inner());
        if diagnostics.recent_failures.back() == Some(&failure) {
            return;
        }
        diagnostics.failure_count = diagnostics.failure_count.saturating_add(1);
        if diagnostics.recent_failures.len() == MAX_RECENT_FAILURES {
            diagnostics.recent_failures.pop_front();
        }
        diagnostics.recent_failures.push_back(failure.clone());
        if diagnostics.unreported_failures.len() < MAX_RECENT_FAILURES {
            diagnostics.unreported_failures.push(failure.clone());
        }
        if !diagnostics.failure_notified {
            diagnostics.failure_notified = true;
            push_notice(
                &mut diagnostics,
                format!(
                    "Context VM {} failed: {}. Later failures are listed in /status.",
                    failure.stage, failure.reason
                ),
            );
        }
    }

    pub fn failure_count(&self) -> u64 {
        self.diagnostics
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .failure_count
    }

    /// Most recent failures, oldest first.
    pub fn recent_failures(&self) -> Vec<ContextVmFailure> {
        self.diagnostics
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .recent_failures
            .iter()
            .cloned()
            .collect()
    }

    pub(crate) fn take_unreported_failures(&self) -> Vec<ContextVmFailure> {
        std::mem::take(
            &mut self
                .diagnostics
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .unreported_failures,
        )
    }

    pub fn push_notice(&self, text: impl Into<String>) {
        push_notice(
            &mut self.diagnostics.write().unwrap_or_else(|e| e.into_inner()),
            text.into(),
        );
    }

    /// Notices queued since the last call, for the host to show once.
    pub fn take_notices(&self) -> Vec<String> {
        std::mem::take(
            &mut self
                .diagnostics
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .notices,
        )
    }

    /// Queue a notice when shadow comparison starts disagreeing. Quiet while
    /// the views match and while an already reported mismatch persists; the
    /// running counts stay in the metrics shown by `/status`.
    pub(crate) fn note_shadow_comparison(&self, comparison: &ShadowComparison) {
        let users = comparison.missing_user_refs.len();
        let tools = comparison.missing_tool_refs.len();
        let mismatch = users > 0 || tools > 0;
        let mut diagnostics = self.diagnostics.write().unwrap_or_else(|e| e.into_inner());
        if mismatch && !diagnostics.shadow_mismatch {
            push_notice(
                &mut diagnostics,
                format!(
                    "Context VM shadow: the VM image omits {users} user and {tools} tool \
                     source(s) the legacy view sends (legacy ~{} tokens, VM ~{} tokens). \
                     Provider input is unchanged.",
                    comparison.legacy_estimated_tokens, comparison.vm_estimated_tokens
                ),
            );
        }
        diagnostics.shadow_mismatch = mismatch;
    }

    pub(crate) fn mark_retrieval_offered(&self) {
        self.diagnostics
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .retrieval_offered = true;
    }

    /// True once this VM has folded or paged events out of its image.
    pub fn retrieval_offered(&self) -> bool {
        self.diagnostics
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .retrieval_offered
    }
}

fn push_notice(diagnostics: &mut ContextVmDiagnostics, text: String) {
    if diagnostics.notices.len() == MAX_PENDING_NOTICES {
        diagnostics.notices.remove(0);
    }
    diagnostics.notices.push(text);
}
