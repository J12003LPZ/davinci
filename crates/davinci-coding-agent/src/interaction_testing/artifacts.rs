//! Artifact hashing, redaction, retention limits, and evidence integration.

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::{validate_receipt_provenance, BackendKind, InteractionReceipt};

/// 50 MiB maximum artifact budget per interaction test run.
pub const MAX_RUN_BYTES: usize = 50 * 1024 * 1024;
/// 500 MiB maximum aggregate artifact budget per task.
pub const MAX_TASK_BYTES: usize = 500 * 1024 * 1024;

/// Verifies whether an input environment proves a physical keyboard.
pub fn proves_physical_keyboard(environment: &str) -> bool {
    environment == "manual_physical_keyboard"
}

/// Computes SHA-256 hash of arbitrary byte content.
pub fn compute_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Redacts API keys, bearer tokens, and secrets from logs and text.
pub fn redact_secrets(input: &str) -> String {
    let re_bearer = Regex::new(r"(?i)bearer\s+[A-Za-z0-9_\-\.]{8,}").unwrap();
    let re_sk = Regex::new(r"sk-[A-Za-z0-9_\-]{16,}").unwrap();
    let re_auth =
        Regex::new(r"(?i)(authorization|password|secret|api_key)\s*[:=]\s*[^\s,]+").unwrap();

    let step1 = re_bearer.replace_all(input, "Bearer [REDACTED_SECRET]");
    let step2 = re_sk.replace_all(&step1, "[REDACTED_SECRET]");
    let step3 = re_auth.replace_all(&step2, "$1: [REDACTED_SECRET]");
    step3.to_string()
}

/// Metadata bound to an interaction evidence receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundEvidenceMetadata {
    pub binary_hash: String,
    pub scenario_hash: String,
    pub source_manifest: String,
    pub backend_version: String,
    pub environment: String,
    pub assertions_hash: String,
    pub assertions_count: usize,
    pub passed_assertions: usize,
    pub artifacts_retained_bytes: usize,
    pub artifacts_omitted: bool,
}

/// Bound artifact payload tracking size budgets and retention.
#[derive(Debug, Clone, Default)]
pub struct ArtifactBudgetTracker {
    pub total_bytes: usize,
    pub max_run_bytes: usize,
    pub max_task_bytes: usize,
    pub task_bytes_before_run: usize,
    pub items: HashMap<String, Vec<u8>>,
    pub omitted_labels: Vec<String>,
}

impl ArtifactBudgetTracker {
    pub fn new(max_run_bytes: usize) -> Self {
        Self::with_task_usage(max_run_bytes, MAX_TASK_BYTES, 0)
    }

    pub fn with_task_usage(
        max_run_bytes: usize,
        max_task_bytes: usize,
        task_bytes_before_run: usize,
    ) -> Self {
        Self {
            total_bytes: 0,
            max_run_bytes,
            max_task_bytes,
            task_bytes_before_run,
            items: HashMap::new(),
            omitted_labels: Vec::new(),
        }
    }

    /// Stores an artifact if under both the run and aggregate task budgets.
    /// If over budget, retains omission metadata and never drops assertion records.
    pub fn store_artifact(&mut self, label: &str, data: Vec<u8>) -> bool {
        let size = data.len();
        let Some(run_after) = self.total_bytes.checked_add(size) else {
            self.omitted_labels
                .push(format!("omitted: {label} (run budget arithmetic overflow)"));
            return false;
        };
        if run_after > self.max_run_bytes {
            self.omitted_labels.push(format!(
                "omitted: {} (size {} exceeds run budget {}/{})",
                label, size, self.total_bytes, self.max_run_bytes
            ));
            return false;
        }
        let Some(task_after) = self.task_bytes_before_run.checked_add(run_after) else {
            self.omitted_labels.push(format!(
                "omitted: {label} (task budget arithmetic overflow)"
            ));
            return false;
        };
        if task_after > self.max_task_bytes {
            self.omitted_labels.push(format!(
                "omitted: {} (size {} exceeds task budget {}/{})",
                label,
                size,
                self.task_bytes_before_run.saturating_add(self.total_bytes),
                self.max_task_bytes
            ));
            return false;
        }

        self.total_bytes = run_after;
        self.items.insert(label.to_string(), data);
        true
    }
}

/// Binds an automated interaction receipt to verifiable execution provenance.
pub fn bind_receipt_provenance(
    receipt: &InteractionReceipt,
    binary_hash: &str,
    expected_binary_hash: &str,
    scenario_content: &str,
    environment: &str,
    tracker: &ArtifactBudgetTracker,
) -> Result<BoundEvidenceMetadata, String> {
    // 1. Validate binary hash matches expected installed binary
    if binary_hash != expected_binary_hash {
        return Err(format!(
            "Binary hash mismatch: expected {}, found {}",
            expected_binary_hash, binary_hash
        ));
    }

    // 2. Require a host-supplied current-source identity for automated evidence.
    let source_manifest = receipt
        .source_manifest
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "Receipt rejected: source manifest is required for automated evidence".to_string()
        })?;

    // 3. Reject zero assertions
    if receipt.assertions.is_empty() {
        return Err("Receipt rejected: zero assertions recorded".into());
    }

    // 4. Prevent simulated/browser adapter from claiming manual physical keyboard
    if proves_physical_keyboard(environment) && receipt.backend_kind != BackendKind::PhysicalManual
    {
        return Err(format!(
            "Provenance violation: backend {:?} cannot produce manual_physical_keyboard evidence",
            receipt.backend_kind
        ));
    }

    validate_receipt_provenance(receipt)?;

    let passed_count = receipt
        .assertions
        .iter()
        .filter(|a| a.starts_with("PASS"))
        .count();
    let assertion_bytes = serde_json::to_vec(&receipt.assertions)
        .map_err(|error| format!("Cannot encode assertion identity: {error}"))?;

    Ok(BoundEvidenceMetadata {
        binary_hash: binary_hash.to_string(),
        scenario_hash: compute_sha256(scenario_content.as_bytes()),
        source_manifest: source_manifest.to_string(),
        backend_version: receipt.backend_identity.clone(),
        environment: environment.to_string(),
        assertions_hash: compute_sha256(&assertion_bytes),
        assertions_count: receipt.assertions.len(),
        passed_assertions: passed_count,
        artifacts_retained_bytes: tracker.total_bytes,
        artifacts_omitted: !tracker.omitted_labels.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f11_evidence_environment() {
        assert!(!proves_physical_keyboard("pty"));
        assert!(!proves_physical_keyboard("renderer_fixture"));
        assert!(!proves_physical_keyboard("browser"));
        assert!(proves_physical_keyboard("manual_physical_keyboard"));
    }

    #[test]
    fn f11_missing_source_manifest_is_unproven() {
        let mut receipt = sample_receipt(
            true,
            vec!["PASS: source-bound".into()],
            BackendKind::FixtureOnly,
        );
        receipt.source_manifest = None;
        let tracker = ArtifactBudgetTracker::new(MAX_RUN_BYTES);
        let err = bind_receipt_provenance(
            &receipt,
            "hash_123",
            "hash_123",
            "scenario content",
            "pty",
            &tracker,
        )
        .unwrap_err();
        assert!(err.contains("source manifest"));
    }

    #[test]
    fn f11_assertion_identity_is_bound() {
        let mut receipt =
            sample_receipt(true, vec!["PASS: alpha".into()], BackendKind::FixtureOnly);
        receipt.source_manifest = Some("source-digest-123".into());
        let tracker = ArtifactBudgetTracker::new(MAX_RUN_BYTES);
        let alpha = bind_receipt_provenance(
            &receipt,
            "hash_123",
            "hash_123",
            "scenario content",
            "pty",
            &tracker,
        )
        .unwrap();
        receipt.assertions = vec!["PASS: beta".into()];
        let beta = bind_receipt_provenance(
            &receipt,
            "hash_123",
            "hash_123",
            "scenario content",
            "pty",
            &tracker,
        )
        .unwrap();
        assert_ne!(alpha.assertions_hash, beta.assertions_hash);
    }

    #[test]
    fn f11_wrong_installed_binary() {
        let receipt = sample_receipt(true, vec!["PASS: test".into()], BackendKind::FixtureOnly);
        let tracker = ArtifactBudgetTracker::new(MAX_RUN_BYTES);

        let err = bind_receipt_provenance(
            &receipt,
            "hash_actual_123",
            "hash_expected_456",
            "scenario content",
            "pty",
            &tracker,
        )
        .unwrap_err();

        assert!(err.contains("Binary hash mismatch"));
    }

    #[test]
    fn f11_zero_assertions() {
        let receipt = sample_receipt(true, Vec::new(), BackendKind::FixtureOnly);
        let tracker = ArtifactBudgetTracker::new(MAX_RUN_BYTES);

        let err = bind_receipt_provenance(
            &receipt,
            "hash_123",
            "hash_123",
            "scenario content",
            "pty",
            &tracker,
        )
        .unwrap_err();

        assert!(err.contains("zero assertions"));
    }

    #[test]
    fn f11_trace_cap() {
        let mut tracker = ArtifactBudgetTracker::new(1024); // 1 KiB cap for test
        let small_blob = vec![0u8; 500];
        let large_blob = vec![0u8; 2000];

        assert!(tracker.store_artifact("small_frame.png", small_blob));
        assert_eq!(tracker.total_bytes, 500);

        // Large blob exceeds cap; never stored, marked omitted
        assert!(!tracker.store_artifact("large_trace.zip", large_blob));
        assert_eq!(tracker.total_bytes, 500);
        assert_eq!(tracker.omitted_labels.len(), 1);
        assert!(tracker.omitted_labels[0].contains("omitted"));
    }

    #[test]
    fn f11_task_artifact_cap() {
        let mut tracker = ArtifactBudgetTracker::with_task_usage(1024, 1200, 900);
        assert!(tracker.store_artifact("first.bin", vec![0u8; 200]));
        assert!(!tracker.store_artifact("over_task.bin", vec![0u8; 101]));
        assert_eq!(tracker.total_bytes, 200);
        assert!(tracker
            .omitted_labels
            .iter()
            .any(|label| label.contains("task budget")));
    }

    #[test]
    fn f11_missing_screenshot() {
        let tracker = ArtifactBudgetTracker::new(MAX_RUN_BYTES);
        assert!(!tracker.items.contains_key("missing_screenshot.png"));
        assert_eq!(tracker.total_bytes, 0);
    }

    #[test]
    fn f11_secret_redaction() {
        let raw = "Authorization: Bearer sk-proj1234567890abcdef12345\nPassword: mysecretpassword123\nDone.";
        let redacted = redact_secrets(raw);

        assert!(!redacted.contains("sk-proj"));
        assert!(!redacted.contains("mysecretpassword"));
        assert!(redacted.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn f11_cancelled_run() {
        let mut receipt = sample_receipt(
            false,
            vec!["FAIL: cancelled".into()],
            BackendKind::FixtureOnly,
        );
        receipt.exit_outcome = Some(130); // SIGINT / Cancelled
        receipt.event_log.push("Run cancelled by user".into());

        assert!(!receipt.assertions_passed);
        assert_eq!(receipt.exit_outcome, Some(130));
        assert!(receipt.event_log.iter().any(|l| l.contains("cancelled")));
    }

    #[test]
    fn f11_fixture_pass_vs_real_pty_pass() {
        let fake_receipt = InteractionReceipt {
            scenario_id: "scen-01".into(),
            backend_kind: BackendKind::RealPty, // Claimed RealPty
            backend_identity: "fake_fixture_simulated".into(), // But simulated identity
            source_manifest: None,
            assertions_passed: true,
            assertions: vec!["PASS: check".into()],
            frames_count: 1,
            event_log: Vec::new(),
            console_errors: Vec::new(),
            network_failures: Vec::new(),
            trace_refs: Vec::new(),
            exit_outcome: Some(0),
        };

        // Provenance enforcement must reject fake backend claiming real PTY status
        let err = validate_receipt_provenance(&fake_receipt).unwrap_err();
        assert!(err.contains("Provenance violation"));
    }

    #[test]
    fn f11_manual_claim_cannot_be_generated_by_browser() {
        let receipt = sample_receipt(true, vec!["PASS: ok".into()], BackendKind::RealBrowser);
        let tracker = ArtifactBudgetTracker::new(MAX_RUN_BYTES);

        let err = bind_receipt_provenance(
            &receipt,
            "hash_123",
            "hash_123",
            "scenario content",
            "manual_physical_keyboard", // Falsely claimed manual physical keyboard
            &tracker,
        )
        .unwrap_err();

        assert!(err.contains("cannot produce manual_physical_keyboard"));
    }

    fn sample_receipt(
        passed: bool,
        assertions: Vec<String>,
        backend_kind: BackendKind,
    ) -> InteractionReceipt {
        InteractionReceipt {
            scenario_id: "sample_scenario".into(),
            backend_kind,
            backend_identity: "test_backend".into(),
            source_manifest: Some("source-digest-123".into()),
            assertions_passed: passed,
            assertions,
            frames_count: 1,
            event_log: vec!["started".into()],
            console_errors: Vec::new(),
            network_failures: Vec::new(),
            trace_refs: Vec::new(),
            exit_outcome: Some(0),
        }
    }
}
