//! Managed browser adapter, trusted bridge process, and JSONL protocol.

use serde::{Deserialize, Serialize};
use url::Url;

use super::{validate_receipt_provenance, BackendKind, InteractionReceipt};

/// Compares parsed canonical origins in production; raw URL prefix matching is unsafe.
pub fn browser_origin_allowed(request_origin: &str, fixture_origin: &str) -> bool {
    let Ok(req_url) = Url::parse(request_origin) else {
        return false;
    };
    let Ok(fix_url) = Url::parse(fixture_origin) else {
        return false;
    };
    req_url.origin() == fix_url.origin()
}

/// Commands sent from host to the browser bridge over JSONL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserCommand {
    Launch { fixture_origin: String },
    Navigate { url: String },
    Click { selector: String },
    Type { selector: String, text: String },
    AssertDom { selector: String, expected: String },
    CaptureSnapshot,
    CaptureScreenshot,
    Close,
}

/// Events received from the browser bridge over JSONL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserBridgeEvent {
    Ready,
    Navigated {
        url: String,
        status: u16,
    },
    ActionDone,
    AssertionResult {
        selector: String,
        passed: bool,
        actual: String,
    },
    DomSnapshot {
        html: String,
    },
    Screenshot {
        base64: String,
    },
    ConsoleError {
        message: String,
    },
    NetworkFailure {
        url: String,
        status: u16,
        failure_text: String,
    },
    Error {
        message: String,
    },
    Closed,
}

/// Managed browser session with origin sandboxing and telemetry capture.
#[derive(Debug, Clone)]
pub struct ManagedBrowserSession {
    pub fixture_origin: String,
    pub allowed_origins: Vec<String>,
    pub is_running: bool,
    pub current_url: Option<String>,
    pub console_errors: Vec<String>,
    pub network_failures: Vec<String>,
    pub dom_snapshots: Vec<String>,
    pub screenshots: Vec<String>,
    pub assertions: Vec<String>,
    pub event_log: Vec<String>,
    pub assertions_passed: bool,
}

impl ManagedBrowserSession {
    pub fn new(fixture_origin: impl Into<String>, allowed_origins: Vec<String>) -> Self {
        Self {
            fixture_origin: fixture_origin.into(),
            allowed_origins,
            is_running: true,
            current_url: None,
            console_errors: Vec::new(),
            network_failures: Vec::new(),
            dom_snapshots: Vec::new(),
            screenshots: Vec::new(),
            assertions: Vec::new(),
            event_log: Vec::new(),
            assertions_passed: true,
        }
    }

    /// Verifies if a network request is allowed by origin sandbox policy.
    pub fn check_network_request(&mut self, url: &str) -> bool {
        let allowed = browser_origin_allowed(url, &self.fixture_origin)
            || self
                .allowed_origins
                .iter()
                .any(|orig| browser_origin_allowed(url, orig));

        if !allowed {
            self.network_failures.push(format!("BLOCKED: {}", url));
            self.event_log
                .push(format!("Network request blocked by policy: {}", url));
            false
        } else {
            true
        }
    }

    /// Feeds an event received from the browser bridge.
    pub fn handle_bridge_event(&mut self, event: BrowserBridgeEvent) {
        match event {
            BrowserBridgeEvent::Ready => {
                self.event_log.push("Browser bridge ready".into());
            }
            BrowserBridgeEvent::Navigated { url, status } => {
                self.current_url = Some(url.clone());
                self.event_log
                    .push(format!("Navigated to {} (status {})", url, status));
            }
            BrowserBridgeEvent::ActionDone => {
                self.event_log.push("Action completed".into());
            }
            BrowserBridgeEvent::AssertionResult {
                selector,
                passed,
                actual,
            } => {
                if !passed {
                    self.assertions_passed = false;
                }
                self.assertions.push(format!(
                    "{}: selector `{}` actual: `{}`",
                    if passed { "PASS" } else { "FAIL" },
                    selector,
                    actual
                ));
            }
            BrowserBridgeEvent::DomSnapshot { html } => {
                self.dom_snapshots.push(html);
            }
            BrowserBridgeEvent::Screenshot { base64 } => {
                self.screenshots.push(base64);
            }
            BrowserBridgeEvent::ConsoleError { message } => {
                self.console_errors.push(message);
            }
            BrowserBridgeEvent::NetworkFailure {
                url,
                status,
                failure_text,
            } => {
                self.network_failures
                    .push(format!("{} (status {}): {}", url, status, failure_text));
            }
            BrowserBridgeEvent::Error { message } => {
                self.assertions_passed = false;
                self.event_log.push(format!("Bridge error: {}", message));
            }
            BrowserBridgeEvent::Closed => {
                self.is_running = false;
                self.event_log.push("Browser closed".into());
            }
        }
    }

    /// Parses a JSONL line from the browser bridge, handling malformed input safely.
    pub fn process_jsonl_line(&mut self, line: &str) -> Result<(), String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(());
        }

        match serde_json::from_str::<BrowserBridgeEvent>(trimmed) {
            Ok(event) => {
                self.handle_bridge_event(event);
                Ok(())
            }
            Err(e) => {
                self.assertions_passed = false;
                let err_msg = format!("Malformed bridge JSON: {}", e);
                self.event_log.push(err_msg.clone());
                Err(err_msg)
            }
        }
    }

    /// Produces a verified interaction receipt.
    pub fn build_receipt(&self, scenario_id: &str) -> Result<InteractionReceipt, String> {
        let receipt = InteractionReceipt {
            scenario_id: scenario_id.to_string(),
            backend_kind: BackendKind::FixtureOnly,
            backend_identity: "fake_browser_fixture".to_string(),
            source_manifest: None,
            assertions_passed: self.assertions_passed && !self.assertions.is_empty(),
            assertions: self.assertions.clone(),
            frames_count: self.dom_snapshots.len() + self.screenshots.len(),
            event_log: self.event_log.clone(),
            console_errors: self.console_errors.clone(),
            network_failures: self.network_failures.clone(),
            trace_refs: Vec::new(),
            exit_outcome: Some(if self.assertions_passed { 0 } else { 1 }),
        };

        validate_receipt_provenance(&receipt)?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn f11_network_fixture_only() {
        assert!(browser_origin_allowed(
            "http://127.0.0.1:43123",
            "http://127.0.0.1:43123"
        ));
        assert!(!browser_origin_allowed(
            "https://example.com",
            "http://127.0.0.1:43123"
        ));
        assert!(!browser_origin_allowed(
            "http://127.0.0.1:43124",
            "http://127.0.0.1:43123"
        ));
    }

    #[test]
    fn f11_same_host_different_port() {
        assert!(!browser_origin_allowed(
            "http://127.0.0.1:8080",
            "http://127.0.0.1:8081"
        ));
    }

    #[test]
    fn f11_forbidden_redirect() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        // Internal page navigation
        assert!(session.check_network_request("http://127.0.0.1:43123/login"));
        // Redirect to external domain
        assert!(!session.check_network_request("https://evil.com/phish"));
        assert!(session
            .network_failures
            .iter()
            .any(|f| f.contains("evil.com")));
    }

    #[test]
    fn f11_external_image_request() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        assert!(!session.check_network_request("https://cdn.thirdparty.com/tracker.png"));
        assert_eq!(session.network_failures.len(), 1);
    }

    #[test]
    fn f11_service_worker() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        // SW requests must be gated by origin policy
        assert!(!session.check_network_request("https://sw.external.org/sync"));
    }

    #[test]
    fn f11_failed_fetch() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        let event = BrowserBridgeEvent::NetworkFailure {
            url: "http://127.0.0.1:43123/api/missing".into(),
            status: 404,
            failure_text: "Not Found".into(),
        };
        session.handle_bridge_event(event);
        assert_eq!(session.network_failures.len(), 1);
        assert!(session.network_failures[0].contains("404"));
    }

    #[test]
    fn f11_console_error() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        let event = BrowserBridgeEvent::ConsoleError {
            message: "Uncaught TypeError: cannot read properties of undefined".into(),
        };
        session.handle_bridge_event(event);
        assert_eq!(session.console_errors.len(), 1);
        assert!(session.console_errors[0].contains("TypeError"));
    }

    #[test]
    fn f11_dom_assertion_fails_despite_screenshot() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        // A screenshot was taken
        session.handle_bridge_event(BrowserBridgeEvent::Screenshot {
            base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
        });
        // But DOM assertion failed
        session.handle_bridge_event(BrowserBridgeEvent::AssertionResult {
            selector: "#title".into(),
            passed: false,
            actual: "Wrong Title".into(),
        });

        let receipt = session.build_receipt("test_scenario").unwrap();
        assert!(!receipt.assertions_passed);
        assert_eq!(receipt.frames_count, 1);
        assert!(receipt.assertions[0].starts_with("FAIL:"));
    }

    #[test]
    fn f11_browser_crash() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        session.handle_bridge_event(BrowserBridgeEvent::Error {
            message: "Browser process crashed unexpectedly".into(),
        });
        assert!(!session.assertions_passed);
        assert!(session.event_log.iter().any(|l| l.contains("crashed")));
    }

    #[test]
    fn f11_malformed_bridge_message() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        let result = session.process_jsonl_line("NOT_VALID_JSON{{{");
        assert!(result.is_err());
        assert!(!session.assertions_passed);
    }

    #[test]
    fn f11_missing_runtime() {
        let missing = Path::new("non_existent_path_to_browser_binary");
        let res = super::super::check_browser_runtime(Some(missing), None, None);
        assert!(res.is_err());
    }
}
