//! Native interaction-test controller with injectable terminal/browser backends.

pub mod artifacts;
pub mod browser;
pub mod browser_fixtures;
pub mod browser_process;
pub mod runner;
pub mod scenarios;
pub mod screen;
pub mod terminal;

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Determines active interaction backend category from availability flags.
pub fn interaction_backend(fixture_supported: bool, real_supported: bool) -> &'static str {
    if real_supported {
        "real_backend"
    } else if fixture_supported {
        "fixture_only"
    } else {
        "unavailable"
    }
}

/// Target surface for an interaction scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionTarget {
    Terminal,
    Browser,
}

/// Bounded scenario step enumeration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionStep {
    Launch,
    TypeText(String),
    Key(String),
    Resize {
        cols: u16,
        rows: u16,
    },
    WaitForCondition {
        description: String,
        timeout_ms: u64,
    },
    AssertScreen {
        pattern: String,
    },
    AssertDom {
        selector: String,
        expected: String,
    },
    Capture,
    Shutdown,
}

/// Declarative interaction scenario configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionScenario {
    pub id: String,
    pub target: InteractionTarget,
    pub executable_or_url: String,
    #[serde(default)]
    pub source_manifest: Option<String>,
    pub steps: Vec<InteractionStep>,
    pub time_budget_ms: u64,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    pub artifact_policy: String,
}

/// Categorization of the backend used to produce an interaction receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    FixtureOnly,
    RealPty,
    RealBrowser,
    PhysicalManual,
}

/// Immutable receipt capturing interaction test results and evidence provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionReceipt {
    pub scenario_id: String,
    pub backend_kind: BackendKind,
    pub backend_identity: String,
    #[serde(default)]
    pub source_manifest: Option<String>,
    pub assertions_passed: bool,
    pub assertions: Vec<String>,
    pub frames_count: usize,
    pub event_log: Vec<String>,
    pub console_errors: Vec<String>,
    pub network_failures: Vec<String>,
    pub trace_refs: Vec<String>,
    pub exit_outcome: Option<i32>,
}

/// Terminal backend trait for injectable PTY / fixture runners.
pub trait TerminalBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn is_real(&self) -> bool;
}

/// Browser backend trait for injectable Playwright / loopback fixture runners.
pub trait BrowserBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn is_real(&self) -> bool;
}

/// Fake terminal backend for offline deterministic testing.
#[derive(Debug, Default)]
pub struct FakeTerminalBackend;

impl TerminalBackend for FakeTerminalBackend {
    fn name(&self) -> &'static str {
        "fake_pty_fixture"
    }

    fn is_real(&self) -> bool {
        false
    }
}

/// Fake browser backend for offline deterministic testing.
#[derive(Debug, Default)]
pub struct FakeBrowserBackend;

impl BrowserBackend for FakeBrowserBackend {
    fn name(&self) -> &'static str {
        "fake_browser_fixture"
    }

    fn is_real(&self) -> bool {
        false
    }
}

/// Enforces evidence provenance: a fixture or fake backend must never claim real PTY or real browser status.
pub fn validate_receipt_provenance(receipt: &InteractionReceipt) -> Result<(), String> {
    let lower = receipt.backend_identity.to_ascii_lowercase();
    let is_simulated =
        lower.contains("fake") || lower.contains("fixture") || lower.contains("mock");

    if is_simulated
        && (receipt.backend_kind == BackendKind::RealPty
            || receipt.backend_kind == BackendKind::RealBrowser)
    {
        return Err(format!(
            "Provenance violation: simulated backend `{}` falsely claimed {:?} status",
            receipt.backend_identity, receipt.backend_kind
        ));
    }
    Ok(())
}

/// Checks the local availability of an external browser runtime without attempting automatic downloads.
pub fn check_browser_runtime(
    runtime_path: Option<&Path>,
    expected_version: Option<&str>,
    actual_version: Option<&str>,
) -> Result<String, String> {
    let path = runtime_path
        .ok_or_else(|| "Browser runtime unavailable: no executable path configured".to_string())?;
    if !path.exists() {
        return Err(format!(
            "Browser runtime executable not found at {}",
            path.display()
        ));
    }

    if let (Some(expected), Some(actual)) = (expected_version, actual_version) {
        if actual != expected {
            return Err(format!(
                "Wrong browser binary version: expected {}, found {}",
                expected, actual
            ));
        }
    }

    Ok(format!("Browser runtime verified at {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f11_backend_capabilities() {
        assert_eq!(interaction_backend(false, false), "unavailable");
        assert_eq!(interaction_backend(true, false), "fixture_only");
        assert_eq!(interaction_backend(true, true), "real_backend");
    }

    #[test]
    fn test_feature_disabled() {
        // When real backend is disabled or unsupported, fallback to fixture or unavailable
        let backend_status = interaction_backend(true, false);
        assert_eq!(backend_status, "fixture_only");

        let backend = FakeTerminalBackend;
        assert_eq!(backend.name(), "fake_pty_fixture");
        assert!(!backend.is_real());
    }

    #[test]
    fn test_missing_cache_runtime() {
        // Missing runtime returns error without attempting npx or downloading packages
        let res = check_browser_runtime(None, Some("1.40.0"), None);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("unavailable"));

        let non_existent = Path::new("/opt/missing/playwright.exe");
        let res_path = check_browser_runtime(Some(non_existent), Some("1.40.0"), None);
        assert!(res_path.is_err());
        assert!(res_path.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_wrong_binary_version() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("browser.exe");
        std::fs::write(&bin, b"binary").unwrap();

        let res = check_browser_runtime(Some(&bin), Some("1.40.0"), Some("1.39.0"));
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Wrong browser binary version"));

        let ok_res = check_browser_runtime(Some(&bin), Some("1.40.0"), Some("1.40.0"));
        assert!(ok_res.is_ok());
    }

    #[test]
    fn test_fake_receipt_cannot_claim_real_pty() {
        let fraudulent_receipt = InteractionReceipt {
            scenario_id: "scen_1".into(),
            backend_kind: BackendKind::RealPty,
            backend_identity: "fake_pty_fixture".into(),
            source_manifest: None,
            assertions_passed: true,
            assertions: vec!["screen_matches".into()],
            frames_count: 5,
            event_log: vec![],
            console_errors: vec![],
            network_failures: vec![],
            trace_refs: vec![],
            exit_outcome: Some(0),
        };

        let res = validate_receipt_provenance(&fraudulent_receipt);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Provenance violation"));

        let valid_receipt = InteractionReceipt {
            scenario_id: "scen_1".into(),
            backend_kind: BackendKind::FixtureOnly,
            backend_identity: "fake_pty_fixture".into(),
            source_manifest: None,
            assertions_passed: true,
            assertions: vec!["screen_matches".into()],
            frames_count: 5,
            event_log: vec![],
            console_errors: vec![],
            network_failures: vec![],
            trace_refs: vec![],
            exit_outcome: Some(0),
        };
        assert!(validate_receipt_provenance(&valid_receipt).is_ok());
    }

    #[test]
    fn test_offline_build_both_targets() {
        // Fake backends initialize and report correct properties offline
        let term = FakeTerminalBackend;
        let browser = FakeBrowserBackend;
        assert_eq!(term.name(), "fake_pty_fixture");
        assert_eq!(browser.name(), "fake_browser_fixture");
        assert!(!term.is_real());
        assert!(!browser.is_real());
    }
}
