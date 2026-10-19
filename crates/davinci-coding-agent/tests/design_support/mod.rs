//! Recorded replies and a synthetic renderer shared by the design tests: no provider,
//! no browser. Geometry from the renderer exercises orchestration only and is not
//! browser evidence.
#![allow(dead_code)]

use davinci_agent::{
    runtime::capacity::{BudgetLimits, RootBudget},
    Agent, CompleteOutput, PermissionMode,
};
use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
use davinci_coding_agent::design::{
    admission::*, error::*, generation::*, model::*, records::*, store::*, types::*,
};
use davinci_session::JsonlSession;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub struct RecordedModel {
    pub replies: VecDeque<ContentBlock>,
    pub calls: usize,
    pub unknown: bool,
    pub cancel: Option<Arc<AtomicBool>>,
}
impl DesignModel for RecordedModel {
    fn validate(&self, _: &Agent) -> DesignResult<()> {
        Ok(())
    }
    fn supports_images(&self) -> bool {
        false
    }
    fn complete(
        &mut self,
        owner: &Agent,
        _: &DesignModelRequest,
    ) -> Result<CompleteOutput, String> {
        self.calls += 1;
        let budget = owner.root_budget().unwrap();
        let attempt = format!("recorded-{}", self.calls);
        budget.reserve("fixture", &attempt, 100, None)?;
        budget.reconcile(&attempt, if self.unknown { None } else { Some(10) }, None)?;
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
        let block = self
            .replies
            .pop_front()
            .ok_or("recorded response missing")?;
        let stop = if matches!(block, ContentBlock::ToolCall { .. }) {
            StopReason::ToolUse
        } else {
            StopReason::Stop
        };
        Ok(CompleteOutput {
            message: AssistantMessage {
                id: attempt,
                role: "assistant".into(),
                content: vec![block],
                model: owner.model_id.clone(),
                usage: None,
                stop_reason: Some(stop),
                error_message: None,
                extra: Default::default(),
            },
            stream_events: None,
            native_responses_resume: None,
            streamed_live: false,
        })
    }
}
/// Synthetic geometry only exercises orchestration; this is not browser evidence.
pub struct RecordedRenderer {
    pub calls: usize,
    pub failures: usize,
}
impl DesignRenderer for RecordedRenderer {
    fn render(
        &mut self,
        _: &DesignStore,
        _: &AuthorizedDesignContext,
        _: &mut JsonlSession,
        revision: &DesignRevision,
    ) -> DesignResult<QualityReport> {
        self.calls += 1;
        let pending = CheckResult {
            state: CheckState::Pending,
            coverage: vec![],
            failures: vec![],
        };
        let measured = CheckResult {
            state: if self.calls <= self.failures {
                CheckState::Failed
            } else {
                CheckState::Current
            },
            coverage: vec!["synthetic fixture only".into()],
            failures: if self.calls <= self.failures {
                vec!["fixture overflow".into()]
            } else {
                vec![]
            },
        };
        Ok(QualityReport {
            revision: RevisionReference {
                artifact_id: revision.artifact_id,
                revision: revision.revision,
                source_hash: revision.source_hash.clone(),
            },
            source: measured.clone(),
            render: measured,
            interaction: pending.clone(),
            accessibility: pending.clone(),
            visual: pending.clone(),
            assets: pending.clone(),
            implementation: pending,
            evidence: vec![],
        })
    }
}
pub fn setup() -> (tempfile::TempDir, Agent, DesignStore, GenerationRequest) {
    setup_with_variants(2)
}
pub fn setup_with_variants(
    variants: u32,
) -> (tempfile::TempDir, Agent, DesignStore, GenerationRequest) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let mut agent = Agent::new("fixture");
    agent.cwd = root.clone();
    agent.set_permission_mode(PermissionMode::AlwaysApprove);
    agent.session = Some(
        JsonlSession::create_in_directory(&root.join("sessions"), &root.to_string_lossy(), None)
            .unwrap(),
    );
    agent
        .bind_root_budget(
            RootBudget::open(
                root.join("ledger.json"),
                "test-root",
                BudgetLimits {
                    max_requests: 100,
                    max_output_tokens: Some(100_000),
                    max_cost_microusd: None,
                    codex_subscription: None,
                    deadline_unix_ms: u64::MAX,
                },
            )
            .unwrap(),
        )
        .unwrap();
    let store = DesignStore::new(root.join("design"));
    let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
    let artifact = store
        .create(
            &ctx,
            agent.session.as_mut().unwrap(),
            CreateDesign {
                title: "Fixture".into(),
                brief: "Two honest offline concepts".into(),
                kind: DesignKind::Product,
                variants,
                operation_id: OperationId::new(),
            },
        )
        .unwrap();
    let request = GenerationRequest {
        artifact_id: artifact.id,
        expected_revision: RevisionId(0),
        brief: "Create the design".into(),
        operation_id: OperationId::new(),
        target_variant: None,
    };
    (dir, agent, store, request)
}
pub fn concepts() -> ContentBlock {
    ContentBlock::Text {
        text: serde_json::to_string(&vec![
            direction("editorial", "serif"),
            direction("split", "sans"),
        ])
        .unwrap(),
    }
}
pub fn source(name: &str) -> ContentBlock {
    ContentBlock::ToolCall {
        id: OperationId::new().to_string(),
        name: name.into(),
        arguments: json!({"files":{"a.html":"<!doctype html><html lang='en'><title>A</title><h1>Editorial fixture</h1></html>","b.html":"<!doctype html><html lang='en'><title>B</title><main>Split fixture</main></html>"},"entry_points":["a.html","b.html"],"variants":[{"id":VariantId::new(),"title":"Editorial","artboards":[{"id":ArtboardId::new(),"title":"Desktop","entry_point":"a.html"}]},{"id":VariantId::new(),"title":"Split","artboards":[{"id":ArtboardId::new(),"title":"Desktop","entry_point":"b.html"}]}],"bindings":[]}),
    }
}
pub fn review() -> ContentBlock {
    ContentBlock::Text {
        text: "{\"findings\":[]}".into(),
    }
}
pub fn direction(layout: &str, typography: &str) -> ConceptDirection {
    ConceptDirection {
        title: layout.into(),
        layout: layout.into(),
        typography: typography.into(),
        content_strategy: format!(
            "Lead with {layout}: supplied factual content and a working primary action"
        ),
    }
}

pub fn text(body: &str) -> ContentBlock {
    ContentBlock::Text { text: body.into() }
}
pub fn model(replies: Vec<ContentBlock>) -> RecordedModel {
    RecordedModel {
        replies: VecDeque::from(replies),
        calls: 0,
        unknown: false,
        cancel: None,
    }
}
pub fn renderer() -> RecordedRenderer {
    RecordedRenderer {
        calls: 0,
        failures: 0,
    }
}
pub fn source_with(name: &str, variants: serde_json::Value) -> ContentBlock {
    let count = variants.as_array().map_or(1, |list| list.len());
    let mut files = serde_json::Map::new();
    let mut entries = vec![];
    for index in 0..count {
        let file = format!("v{index}.html");
        files.insert(
            file.clone(),
            json!(format!(
                "<!doctype html><html lang='en'><title>V{index}</title><main>Concept {index}</main></html>"
            )),
        );
        entries.push(file);
    }
    ContentBlock::ToolCall {
        id: OperationId::new().to_string(),
        name: name.into(),
        arguments: json!({"files": files, "entry_points": entries, "variants": variants, "bindings": []}),
    }
}
pub fn variant(index: usize) -> serde_json::Value {
    json!({"id": VariantId::new(), "title": format!("Concept {index}"), "artboards": [{"id": ArtboardId::new(), "title": "Desktop", "entry_point": format!("v{index}.html")}]})
}
pub fn three_concepts() -> ContentBlock {
    text(
        &serde_json::to_string(&vec![
            direction("editorial", "serif"),
            direction("split", "sans"),
            direction("modular", "mono"),
        ])
        .unwrap(),
    )
}
pub fn run_it(
    store: &DesignStore,
    agent: &mut Agent,
    dir: &std::path::Path,
    request: &GenerationRequest,
    model: &mut RecordedModel,
    renderer: &mut RecordedRenderer,
) -> DesignRunState {
    run(store, agent, dir, request.clone(), model, renderer, None).unwrap()
}
