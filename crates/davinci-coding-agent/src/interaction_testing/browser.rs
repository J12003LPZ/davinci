//! Managed browser adapter, trusted bridge process, and JSONL protocol.

use serde::{Deserialize, Serialize};
use url::Url;

use super::{validate_receipt_provenance, BackendKind, InteractionReceipt};

pub const MAX_BROWSER_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_BROWSER_EVENTS: usize = 128;

/// Compares parsed canonical origins in production; raw URL prefix matching is unsafe.
pub fn browser_origin_allowed(request_origin: &str, fixture_origin: &str) -> bool {
    let Ok(req_url) = Url::parse(request_origin) else {
        return false;
    };
    let Ok(fix_url) = Url::parse(fixture_origin) else {
        return false;
    };
    [&req_url, &fix_url].iter().all(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some()
    }) && req_url.origin() == fix_url.origin()
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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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

// Serde's internally tagged unit variants ignore extra fields even with
// deny_unknown_fields on the enum. Validate those messages as a strict object.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitBridgeEvent {
    #[serde(rename = "type")]
    _kind: String,
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
    retained_events: usize,
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
            retained_events: 0,
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
            self.handle_bridge_event(BrowserBridgeEvent::NetworkFailure {
                url: url.to_owned(),
                status: 403,
                failure_text: "BLOCKED by origin policy".into(),
            });
            false
        } else {
            true
        }
    }

    /// Feeds an event received from the browser bridge.
    pub fn handle_bridge_event(&mut self, event: BrowserBridgeEvent) {
        // Closing must update lifecycle even when the evidence budget is exhausted.
        if matches!(event, BrowserBridgeEvent::Closed) {
            self.is_running = false;
        }
        if self.retained_events >= MAX_BROWSER_EVENTS
            || serde_json::to_vec(&event)
                .map_or(true, |bytes| bytes.len() > MAX_BROWSER_MESSAGE_BYTES)
        {
            self.assertions_passed = false;
            return;
        }
        self.retained_events += 1;
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
                self.assertions_passed = false;
                self.console_errors.push(message);
            }
            BrowserBridgeEvent::NetworkFailure {
                url,
                status,
                failure_text,
            } => {
                self.assertions_passed = false;
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
        if line.len() > MAX_BROWSER_MESSAGE_BYTES {
            self.assertions_passed = false;
            return Err("Browser bridge message exceeds 64 KiB".into());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(());
        }

        let event = serde_json::from_str::<BrowserBridgeEvent>(trimmed).and_then(|event| {
            if matches!(
                event,
                BrowserBridgeEvent::Ready
                    | BrowserBridgeEvent::ActionDone
                    | BrowserBridgeEvent::Closed
            ) {
                serde_json::from_str::<UnitBridgeEvent>(trimmed)?;
            }
            Ok(event)
        });
        match event {
            Ok(event) => {
                self.handle_bridge_event(event);
                Ok(())
            }
            Err(e) => {
                self.assertions_passed = false;
                let err_msg = format!("Malformed bridge JSON: {}", e);
                self.handle_bridge_event(BrowserBridgeEvent::Error {
                    message: err_msg.clone(),
                });
                Err(err_msg)
            }
        }
    }

    /// Produces a verified interaction receipt.
    pub fn build_receipt(&self, scenario_id: &str) -> Result<InteractionReceipt, String> {
        let passed = self.assertions_passed
            && !self.assertions.is_empty()
            && self.console_errors.is_empty()
            && self.network_failures.is_empty();
        let receipt = InteractionReceipt {
            scenario_id: scenario_id.to_string(),
            backend_kind: BackendKind::FixtureOnly,
            backend_identity: "fake_browser_fixture".to_string(),
            source_manifest: None,
            assertions_passed: passed,
            assertions: self.assertions.clone(),
            frames_count: self.dom_snapshots.len() + self.screenshots.len(),
            event_log: self.event_log.clone(),
            console_errors: self.console_errors.clone(),
            network_failures: self.network_failures.clone(),
            trace_refs: Vec::new(),
            exit_outcome: Some(if passed { 0 } else { 1 }),
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
    fn browser_policy_refuses_credentials_and_non_http_urls() {
        let origin = "http://127.0.0.1:43123";
        assert!(!browser_origin_allowed(
            "http://user:secret@127.0.0.1:43123/",
            origin
        ));
        assert!(!browser_origin_allowed(
            "http://user@127.0.0.1:43123/",
            origin
        ));
        assert!(!browser_origin_allowed(
            "file:///private.txt",
            "file:///private.txt"
        ));
        assert!(!browser_origin_allowed("data:text/html,private", origin));
        assert!(browser_origin_allowed(
            "http://127.0.0.1:43123/login",
            origin
        ));
    }

    #[test]
    fn browser_errors_prevent_passing_fixture_receipt() {
        for event in [
            BrowserBridgeEvent::ConsoleError {
                message: "frontend bug".into(),
            },
            BrowserBridgeEvent::NetworkFailure {
                url: "http://127.0.0.1:43123/api/login".into(),
                status: 500,
                failure_text: "server bug".into(),
            },
        ] {
            let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
            session.handle_bridge_event(BrowserBridgeEvent::AssertionResult {
                selector: "#login".into(),
                passed: true,
                actual: "Welcome".into(),
            });
            session.handle_bridge_event(event);
            assert!(!session.build_receipt("login").unwrap().assertions_passed);
        }
    }

    #[test]
    fn browser_jsonl_is_bounded_before_deserialization() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        let line = serde_json::to_string(&BrowserBridgeEvent::DomSnapshot {
            html: "x".repeat(1024 * 1024),
        })
        .unwrap();
        assert!(session.process_jsonl_line(&line).is_err());
        assert!(session.dom_snapshots.is_empty());
        assert!(!session.assertions_passed);
    }

    #[test]
    fn browser_event_overflow_is_bounded_and_not_success() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        session.handle_bridge_event(BrowserBridgeEvent::AssertionResult {
            selector: "#login".into(),
            passed: true,
            actual: "Welcome".into(),
        });
        for _ in 0..1000 {
            session.handle_bridge_event(BrowserBridgeEvent::ActionDone);
        }
        assert!(session.event_log.len() <= 128);
        assert!(!session.build_receipt("login").unwrap().assertions_passed);
    }

    #[test]
    fn browser_snapshot_count_and_payload_are_bounded() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        for _ in 0..1000 {
            session.handle_bridge_event(BrowserBridgeEvent::DomSnapshot {
                html: "x".repeat(8192),
            });
        }
        assert!(session.dom_snapshots.len() <= 128);
        assert!(!session.assertions_passed);
        let mut oversized = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        oversized.handle_bridge_event(BrowserBridgeEvent::DomSnapshot {
            html: "x".repeat(1024 * 1024),
        });
        assert!(oversized.dom_snapshots.is_empty());
        assert!(!oversized.assertions_passed);
    }

    #[test]
    fn browser_close_survives_overflow_and_policy_failures_are_bounded() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        for _ in 0..1000 {
            assert!(!session.check_network_request("https://external.example/"));
        }
        assert!(session.network_failures.len() <= MAX_BROWSER_EVENTS);
        session.handle_bridge_event(BrowserBridgeEvent::Closed);
        assert!(!session.is_running);
        assert!(!session.assertions_passed);
    }

    #[test]
    fn browser_unknown_fields_and_repeated_malformed_lines_are_not_trusted() {
        let mut session = ManagedBrowserSession::new("http://127.0.0.1:43123", Vec::new());
        assert!(session
            .process_jsonl_line(r#"{"type":"ready","evaluate":"steal()"}"#)
            .is_err());
        assert!(session
            .process_jsonl_line(r#"{"type":"console_error","message":"bug","evaluate":"steal()"}"#)
            .is_err());
        for _ in 0..1000 {
            assert!(session.process_jsonl_line("{broken").is_err());
        }
        session.handle_bridge_event(BrowserBridgeEvent::Ready);
        assert!(session.event_log.len() <= MAX_BROWSER_EVENTS);
        assert!(!session.assertions_passed);
    }

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
