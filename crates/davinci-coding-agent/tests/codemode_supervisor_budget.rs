//! WOR-196 / WOR-200: truncated output is not `Completed`, and setup time is
//! charged against the request timeout. Both need the disposable admitted
//! Node and fixture bundle, like `codemode_supervisor.rs`.
use davinci_agent::codemode::*;
use davinci_coding_agent::codemode_host::{
    assets::{AssetManifest, HostAssets},
    NodeCodeModeHost,
};
use std::path::Path;

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

struct Catalog {
    delay: std::time::Duration,
}
impl CodeModeBroker for Catalog {
    fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
        std::thread::sleep(self.delay);
        Ok(ToolPage {
            tools: vec![],
            total: 0,
            cursor: None,
        })
    }
    fn describe(&self, _: &str) -> Result<serde_json::Value, CodeModeError> {
        panic!("no metadata expected")
    }
    fn call(&self, _: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
        panic!("no calls expected")
    }
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn native_truncated_output_is_partial_not_completed() {
    let outcome = fixture_host().execute(
        &CodeModeRequest {
            code: "return 'abcdefghijklmnopqrstuvwxyz'".into(),
            timeout_ms: Some(1000),
            max_output_bytes: Some(8),
        },
        &fixture_context("truncated-fixture"),
        std::sync::Arc::new(Catalog {
            delay: Default::default(),
        }),
    );
    assert!(!outcome.output_complete, "{outcome:?}");
    assert!(
        matches!(outcome.status, CodeModeStatus::Partial),
        "{outcome:?}"
    );
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn native_slow_catalog_discovery_counts_against_the_timeout() {
    let outcome = fixture_host().execute(
        &CodeModeRequest {
            code: "return 1".into(),
            timeout_ms: Some(100),
            max_output_bytes: None,
        },
        &fixture_context("slow-catalog-fixture"),
        std::sync::Arc::new(Catalog {
            delay: std::time::Duration::from_millis(400),
        }),
    );
    assert!(
        matches!(outcome.status, CodeModeStatus::Failed),
        "{outcome:?}"
    );
    assert_eq!(
        outcome.error.as_ref().map(|error| error.code.as_str()),
        Some("TIMEOUT")
    );
}
