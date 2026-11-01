//! Droid reference fixtures 05 (syntax error) and 09 (long job), driven
//! through the real admitted Node host. Both were previously labelled
//! UNTESTED because no admitted runtime was available offline.
use davinci_agent::codemode::*;
use davinci_coding_agent::codemode_host::{
    assets::{AssetManifest, HostAssets},
    NodeCodeModeHost,
};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn fixture_host() -> NodeCodeModeHost {
    let root = std::env::var("DAVINCI_CODEMODE_FIXTURE_BUNDLE").unwrap();
    let node = std::env::var("DAVINCI_CODEMODE_FIXTURE_NODE").unwrap();
    let manifest: AssetManifest = serde_json::from_slice(
        &std::fs::read(std::env::var("DAVINCI_CODEMODE_FIXTURE_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    NodeCodeModeHost::new(
        Path::new(&node),
        HostAssets::validate(Path::new(&root), &manifest).unwrap(),
    )
    .unwrap()
}

fn fixture_context(id: &str) -> CodeModeRunContext {
    CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: id.into(),
            session_id: None,
            branch_leaf: None,
            workspace_binding: "fixture-workspace".into(),
            runtime_run_id: format!("{id}-run"),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::ReadOnly,
        limits: CodeModeLimits::default(),
        capability_revision: "fixture-revision".into(),
        cancellation: Default::default(),
        root_budget: None,
    }
}

/// Never reached by the syntax-error fixture; a call here is a failure.
struct NoCalls;
impl CodeModeBroker for NoCalls {
    fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
        Ok(ToolPage {
            tools: vec![],
            total: 0,
            cursor: None,
        })
    }
    fn describe(&self, _: &str) -> Result<serde_json::Value, CodeModeError> {
        panic!("no metadata expected")
    }
    fn call(&self, call: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
        panic!("a syntax error must not dispatch a child call: {call:?}")
    }
}

/// Stands in for a long-running command: one call, a large tail, an exit status.
struct LongJobBroker {
    calls: Arc<AtomicUsize>,
    delay: std::time::Duration,
}

impl CodeModeBroker for LongJobBroker {
    fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
        Ok(ToolPage {
            tools: vec![],
            total: 0,
            cursor: None,
        })
    }
    /// The host's launch catalog: the guest only sees what the run authorizes.
    fn catalog(&self, _: usize) -> Result<Vec<String>, CodeModeError> {
        Ok(vec!["exec_command".into()])
    }
    fn describe(&self, _: &str) -> Result<serde_json::Value, CodeModeError> {
        panic!("no metadata expected")
    }
    fn call(&self, _: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        let mut text = String::new();
        for line in 1..=500 {
            text.push_str(&format!("line {line}\n"));
        }
        Ok(CodeModeToolValue {
            text,
            structured_content: Some(serde_json::json!({"exitCode": 0})),
            complete: true,
            artifact: None,
            operation_ref: "op-long-job".into(),
        })
    }
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn droid_reference_syntax_error_is_surfaced() {
    let outcome = fixture_host().execute(
        &CodeModeRequest {
            code: "return {{{".into(),
            timeout_ms: Some(2_000),
            max_output_bytes: None,
        },
        &fixture_context("syntax-error-fixture"),
        Arc::new(NoCalls),
    );
    // A syntax error is an explicit script failure with the parser message,
    // never a completed run and never a dispatched child.
    assert!(
        matches!(outcome.status, CodeModeStatus::Failed),
        "{outcome:?}"
    );
    assert!(!outcome.script_completed, "{outcome:?}");
    assert!(outcome.children.is_empty(), "{outcome:?}");
    let error = outcome.error.as_ref().expect("syntax error is reported");
    assert!(
        error.message.contains("parse") || error.message.contains("Syntax"),
        "{error:?}"
    );
    // A parse failure is the model's own input error, not a sandbox fault.
    assert_eq!(error.code, "INVALID_INPUT", "{error:?}");
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn droid_reference_long_job_returns_tail_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let broker = LongJobBroker {
        calls: calls.clone(),
        delay: std::time::Duration::from_millis(250),
    };
    let outcome = fixture_host().execute(
        &CodeModeRequest {
            code: r#"const result = await tools.exec_command({command:"fixture-long-job"});
const lines = result.text.trimEnd().split("\n");
return {exitCode: result.structuredContent.exitCode, tail: lines.slice(-3), lines: result.text.trimEnd().split("\n").length};"#
                .into(),
            timeout_ms: Some(5_000),
            max_output_bytes: None,
        },
        &fixture_context("long-job-fixture"),
        Arc::new(broker),
    );
    // The model gets the retained tail and the exit status from one call.
    assert!(
        matches!(outcome.status, CodeModeStatus::Completed),
        "{outcome:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the command ran once");
    assert!(outcome.output_text.contains("line 500"), "{outcome:?}");
    assert!(outcome.output_text.contains("line 498"), "{outcome:?}");
    assert!(outcome.output_text.contains("exitCode"), "{outcome:?}");
    assert!(outcome.output_text.contains("500"), "{outcome:?}");
    assert!(
        outcome.children.is_empty(),
        "host evidence is authoritative"
    );
}
