use davinci_coding_agent::design::generation::{
    validate_directions, ConceptDirection, RunPhase, RunProgress,
};

fn direction(layout: &str, typography: &str) -> ConceptDirection {
    ConceptDirection {
        title: layout.into(),
        layout: layout.into(),
        typography: typography.into(),
        content_strategy: "Use the supplied factual content and a working primary action".into(),
    }
}
#[test]
fn directions_require_structural_and_typographic_difference() {
    assert!(validate_directions(
        &[direction("editorial", "serif"), direction("split", "sans")],
        2
    )
    .is_ok());
    assert!(validate_directions(
        &[
            direction("editorial", "serif"),
            direction("editorial", "sans")
        ],
        2
    )
    .is_err());
    assert!(validate_directions(
        &[direction("editorial", "serif"), direction("split", "serif")],
        2
    )
    .is_err());
    assert!(validate_directions(&[direction("editorial", "serif")], 2).is_err());
}
#[test]
fn persisted_run_rejects_unknown_requests_and_a_third_repair() {
    let mut state = RunProgress {
        phase: RunPhase::Generate,
        in_flight: true,
        repairs: 0,
        deadline_ms: u64::MAX,
    };
    assert!(state.can_dispatch(1).is_err());
    state.in_flight = false;
    state.repairs = 2;
    assert!(state.begin_repair().is_err());
    state.repairs = 0;
    state.begin_repair().unwrap();
    state.begin_repair().unwrap();
    assert_eq!(state.repairs, 2);
    assert!(state.begin_repair().is_err());
    state.deadline_ms = 1;
    assert!(state.can_dispatch(2).is_err());
}

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

struct RecordedModel {
    replies: VecDeque<ContentBlock>,
    calls: usize,
    unknown: bool,
    cancel: Option<Arc<AtomicBool>>,
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
struct RecordedRenderer {
    calls: usize,
    failures: usize,
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
fn setup() -> (tempfile::TempDir, Agent, DesignStore, GenerationRequest) {
    setup_with_variants(2)
}
fn setup_with_variants(
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
    };
    (dir, agent, store, request)
}
fn concepts() -> ContentBlock {
    ContentBlock::Text {
        text: serde_json::to_string(&vec![
            direction("editorial", "serif"),
            direction("split", "sans"),
        ])
        .unwrap(),
    }
}
fn source(name: &str) -> ContentBlock {
    ContentBlock::ToolCall {
        id: OperationId::new().to_string(),
        name: name.into(),
        arguments: json!({"files":{"a.html":"<!doctype html><html lang='en'><title>A</title><h1>Editorial fixture</h1></html>","b.html":"<!doctype html><html lang='en'><title>B</title><main>Split fixture</main></html>"},"entry_points":["a.html","b.html"],"variants":[{"id":VariantId::new(),"title":"Editorial","artboards":[{"id":ArtboardId::new(),"title":"Desktop","entry_point":"a.html"}]},{"id":VariantId::new(),"title":"Split","artboards":[{"id":ArtboardId::new(),"title":"Desktop","entry_point":"b.html"}]}],"bindings":[]}),
    }
}
fn review() -> ContentBlock {
    ContentBlock::Text {
        text: "{\"findings\":[]}".into(),
    }
}
#[test]
fn rejected_source_bundle_spends_a_bounded_repair_instead_of_blocking() {
    let (dir, mut agent, store, request) = setup();
    let mut malformed = source("design_create");
    if let ContentBlock::ToolCall { arguments, .. } = &mut malformed {
        arguments["entry_points"] = json!(["missing.html"]);
    }
    let mut model = RecordedModel {
        replies: VecDeque::from([concepts(), malformed, source("design_create"), review()]),
        calls: 0,
        unknown: false,
        cancel: None,
    };
    let mut renderer = RecordedRenderer {
        calls: 0,
        failures: 0,
    };
    let state = run(
        &store,
        &mut agent,
        dir.path(),
        request,
        &mut model,
        &mut renderer,
        None,
    )
    .unwrap();
    assert_eq!(
        state.progress.phase,
        RunPhase::Complete,
        "{:?}",
        state.findings
    );
    assert_eq!(state.last_revision, RevisionId(1));
    assert_eq!(state.progress.repairs, 1);
    assert_eq!(model.calls, 4);
}

#[test]
fn single_concept_patch_object_advances_retained_revision_without_extra_request() {
    let (dir, mut agent, store, request) = setup_with_variants(1);
    let variant = json!({"id":VariantId::new(),"title":"Editorial","artboards":[{"id":ArtboardId::new(),"title":"Desktop","entry_point":"a.html"}]});
    let source_call = |name: &str, variants: serde_json::Value| ContentBlock::ToolCall {
        id: OperationId::new().to_string(),
        name: name.into(),
        arguments: json!({"files":{"a.html":"<!doctype html><html lang='en'><title>A</title><h1>Editorial fixture</h1></html>"},"entry_points":["a.html"],"variants":variants,"bindings":[]}),
    };
    let mut model = RecordedModel {
        replies: VecDeque::from([
            ContentBlock::Text {
                text: serde_json::to_string(&vec![direction("editorial", "serif")]).unwrap(),
            },
            source_call("design_create", json!([variant.clone()])),
            review(),
            source_call("design_patch", variant),
            review(),
        ]),
        calls: 0,
        unknown: false,
        cancel: None,
    };
    let mut renderer = RecordedRenderer {
        calls: 0,
        failures: 1,
    };
    let state = run(
        &store,
        &mut agent,
        dir.path(),
        request,
        &mut model,
        &mut renderer,
        None,
    )
    .unwrap();
    assert_eq!(
        state.progress.phase,
        RunPhase::Complete,
        "{:?}",
        state.findings
    );
    assert_eq!(state.last_revision, RevisionId(2));
    assert_eq!(state.progress.repairs, 1);
    assert_eq!(model.calls, 5);
    assert_eq!(agent.root_budget().unwrap().snapshot().unwrap().requests, 5);
}

#[test]
fn recorded_generation_repairs_real_finding_and_reopen_does_not_repeat_requests() {
    let (dir, mut agent, store, request) = setup();
    let mut model = RecordedModel {
        replies: VecDeque::from([
            concepts(),
            source("design_create"),
            review(),
            source("design_patch"),
            review(),
        ]),
        calls: 0,
        unknown: false,
        cancel: None,
    };
    let mut renderer = RecordedRenderer {
        calls: 0,
        failures: 1,
    };
    let state = run(
        &store,
        &mut agent,
        dir.path(),
        request.clone(),
        &mut model,
        &mut renderer,
        None,
    )
    .unwrap();
    assert_eq!(
        state.progress.phase,
        RunPhase::Complete,
        "{:?}",
        state.findings
    );
    assert_eq!(state.last_revision, RevisionId(2));
    assert_eq!(state.progress.repairs, 1);
    assert_eq!(model.calls, 5);
    assert_eq!(agent.root_budget().unwrap().snapshot().unwrap().requests, 5);
    let path = agent.session.as_ref().unwrap().path.clone();
    agent.session = Some(JsonlSession::open(&path).unwrap());
    let resumed = run(
        &store,
        &mut agent,
        dir.path(),
        request,
        &mut model,
        &mut renderer,
        None,
    )
    .unwrap();
    assert_eq!(resumed.last_revision, RevisionId(2));
    assert_eq!(model.calls, 5);
    assert_eq!(resumed.quality.unwrap().visual.state, CheckState::Pending);
}
#[test]
fn third_repair_and_unknown_outcomes_leave_committed_source_intact() {
    let (dir, mut agent, store, request) = setup();
    let mut model = RecordedModel {
        replies: VecDeque::from([
            concepts(),
            source("design_create"),
            review(),
            source("design_patch"),
            review(),
            source("design_patch"),
            review(),
        ]),
        calls: 0,
        unknown: false,
        cancel: None,
    };
    let mut renderer = RecordedRenderer {
        calls: 0,
        failures: 10,
    };
    let state = run(
        &store,
        &mut agent,
        dir.path(),
        request,
        &mut model,
        &mut renderer,
        None,
    )
    .unwrap();
    assert_eq!(state.progress.phase, RunPhase::Blocked);
    assert_eq!(state.progress.repairs, 2);
    assert_eq!(state.last_revision, RevisionId(3));
    assert_eq!(model.calls, 7);
    let (dir, mut agent, store, request) = setup();
    let mut model = RecordedModel {
        replies: VecDeque::from([concepts()]),
        calls: 0,
        unknown: true,
        cancel: None,
    };
    let state = run(
        &store,
        &mut agent,
        dir.path(),
        request.clone(),
        &mut model,
        &mut renderer,
        None,
    )
    .unwrap();
    assert_eq!(state.progress.phase, RunPhase::Blocked);
    assert_eq!(state.last_revision, RevisionId(0));
    run(
        &store,
        &mut agent,
        dir.path(),
        request,
        &mut model,
        &mut renderer,
        None,
    )
    .unwrap();
    assert_eq!(model.calls, 1);
    assert_eq!(agent.root_budget().unwrap().snapshot().unwrap().unknown, 1);
}
