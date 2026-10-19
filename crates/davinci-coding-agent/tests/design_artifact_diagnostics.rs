//! `/design` from a plain sentence: what the command keeps, what it infers,
//! and how a failed generation reports itself.

use davinci_coding_agent::design::commands::{parse_design_command, DesignCommand};
use davinci_coding_agent::design::types::DesignKind;

fn create(line: &str) -> davinci_coding_agent::design::records::CreateDesign {
    match parse_design_command(line).unwrap_or_else(|error| panic!("{line}: {error:?}")) {
        DesignCommand::Create { input } => input,
        other => panic!("{line}: create expected, got {other:?}"),
    }
}

#[test]
fn design_brief_without_flags_keeps_full_intent() {
    let input = create("/design Create a modern dashboard for AI agents");
    assert_eq!(input.brief, "Create a modern dashboard for AI agents");
    assert_eq!(
        input.kind,
        DesignKind::Product,
        "inferred, not implicit landing"
    );
    assert_eq!(input.variants, 3);

    // Legacy form: explicit flags override inference and parse as before.
    let legacy = create("/design new \"a dashboard\" --kind document --variants 2");
    assert_eq!(legacy.kind, DesignKind::Document);
    assert_eq!(legacy.variants, 2);
    assert_eq!(legacy.brief, "a dashboard");
}

#[test]
fn an_apostrophe_inside_prose_is_not_a_quote() {
    let input = create("/design Create a dashboard for Julien's team");
    assert_eq!(input.brief, "Create a dashboard for Julien's team");
    let inch = create("/design A 5\" screen kiosk app");
    assert_eq!(inch.brief, "A 5\" screen kiosk app");
    // A quote at the start of a word still quotes.
    let quoted = create("/design new 'Julien\\'s  team' --kind product");
    assert_eq!(quoted.brief, "Julien's  team");
    assert!(parse_design_command("/design new 'unterminated").is_err());
}

#[test]
fn a_brief_may_start_with_a_subcommand_word() {
    for (line, brief) in [
        (
            "/design open a cafe landing page",
            "open a cafe landing page",
        ),
        ("/design list of tasks app", "list of tasks app"),
        ("/design revise the hero section", "revise the hero section"),
        (
            "/design status board for my team",
            "status board for my team",
        ),
        ("/design export settings page", "export settings page"),
    ] {
        assert_eq!(create(line).brief, brief, "{line}");
    }
    // A subcommand with its real arguments is still a subcommand.
    let id = "2f6f2c2e-52c0-4d3a-8a4e-5b2f3c9a1d10";
    assert!(matches!(
        parse_design_command(&format!("/design open {id}")).unwrap(),
        DesignCommand::Open { .. }
    ));
    assert!(matches!(
        parse_design_command("/design list").unwrap(),
        DesignCommand::List {}
    ));
    // Bare verbs that need an argument are errors, not briefs.
    for bare in [
        "/design open",
        "/design status",
        "/design revise",
        "/design apply",
    ] {
        assert!(parse_design_command(bare).is_err(), "{bare}");
    }
}

#[test]
fn the_kind_comes_from_the_words_and_falls_back_to_the_documented_default() {
    assert_eq!(create("/design a signup form").kind, DesignKind::Product);
    assert_eq!(
        create("/design a quarterly report").kind,
        DesignKind::Document
    );
    assert_eq!(
        create("/design a landing page for a bakery").kind,
        DesignKind::Landing
    );
    assert_eq!(
        create("/design something fresh").kind,
        davinci_coding_agent::design::intent::FALLBACK_KIND
    );
    assert_eq!(create("/design a dashboard").variants, 3);
}

// ---- generation fixtures (recorded replies, synthetic renderer; no provider, no browser)

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
fn direction(layout: &str, typography: &str) -> ConceptDirection {
    ConceptDirection {
        title: layout.into(),
        layout: layout.into(),
        typography: typography.into(),
        content_strategy: format!(
            "Lead with {layout}: supplied factual content and a working primary action"
        ),
    }
}

fn text(body: &str) -> ContentBlock {
    ContentBlock::Text { text: body.into() }
}
fn model(replies: Vec<ContentBlock>) -> RecordedModel {
    RecordedModel {
        replies: VecDeque::from(replies),
        calls: 0,
        unknown: false,
        cancel: None,
    }
}
fn renderer() -> RecordedRenderer {
    RecordedRenderer {
        calls: 0,
        failures: 0,
    }
}
fn source_with(name: &str, variants: serde_json::Value) -> ContentBlock {
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
fn variant(index: usize) -> serde_json::Value {
    json!({"id": VariantId::new(), "title": format!("Concept {index}"), "artboards": [{"id": ArtboardId::new(), "title": "Desktop", "entry_point": format!("v{index}.html")}]})
}
fn three_concepts() -> ContentBlock {
    text(
        &serde_json::to_string(&vec![
            direction("editorial", "serif"),
            direction("split", "sans"),
            direction("modular", "mono"),
        ])
        .unwrap(),
    )
}
fn run_it(
    store: &DesignStore,
    agent: &mut Agent,
    dir: &std::path::Path,
    request: &GenerationRequest,
    model: &mut RecordedModel,
    renderer: &mut RecordedRenderer,
) -> DesignRunState {
    run(store, agent, dir, request.clone(), model, renderer, None).unwrap()
}

#[test]
fn design_error_reports_exact_stage_and_recoverable_revision() {
    let (dir, mut agent, store, request) = setup();
    let secret = "sk-live0123456789abcdef0123456789abcdef";
    let bad = format!("Sure! here are ideas {secret} {}", "x ".repeat(2_000));
    let mut m = model(vec![text(&bad), text(&bad), text(&bad)]);
    let mut r = renderer();
    let state = run_it(&store, &mut agent, dir.path(), &request, &mut m, &mut r);
    assert_eq!(state.progress.phase, RunPhase::Blocked);
    assert_eq!(m.calls, 3, "one ask plus two bounded retries");
    let failure = state.failure.expect("a blocked run says why");
    assert_eq!(failure.phase, RunPhase::Directions);
    assert_eq!(failure.repairs_remaining, 0);
    assert_eq!(failure.repair_count, 2);
    assert!(failure.expected_schema.contains("exactly 2"));
    assert!(!failure.snippet.contains(secret), "snippet is redacted");
    assert!(failure.snippet.chars().count() <= 400, "snippet is bounded");
    assert_eq!(failure.artifact, request.artifact_id);
    assert_eq!(failure.checkpoint, request.operation_id);
    assert!(!failure.model_route.is_empty());
}

#[test]
fn directions_are_repaired_in_place_and_a_fenced_plan_is_accepted() {
    let (dir, mut agent, store, request) = setup();
    let plan = match concepts() {
        ContentBlock::Text { text } => text,
        _ => unreachable!(),
    };
    let fenced = format!("```json\n{plan}\n```");
    let mut m = model(vec![
        text("here you go: not json"),
        text(&fenced),
        source("design_create"),
        review(),
    ]);
    let mut r = renderer();
    let state = run_it(&store, &mut agent, dir.path(), &request, &mut m, &mut r);
    assert_eq!(state.progress.phase, RunPhase::Complete);
    assert_eq!(state.progress.repairs, 1);
    assert!(state.failure.is_none());
}

#[test]
fn one_variant_normalization_and_bad_shape_checkpoint() {
    // A single object where an array of one was meant is accepted only when
    // it is otherwise schema-valid.
    let (dir, mut agent, store, request) = setup_with_variants(1);
    let one = json!({"title":"Solo","layout":"split","typography":"sans","content_strategy":"A single clear idea"});
    let mut m = model(vec![
        text(&one.to_string()),
        source_with("design_create", json!([variant(0)])),
        review(),
    ]);
    let mut r = renderer();
    let state = run_it(&store, &mut agent, dir.path(), &request, &mut m, &mut r);
    assert_eq!(
        state.progress.phase,
        RunPhase::Complete,
        "{:?}",
        state.failure
    );

    // Three concepts keep all three.
    let (dir, mut agent, store, request) = setup_with_variants(3);
    let mut m = model(vec![
        three_concepts(),
        source_with("design_create", json!([variant(0), variant(1), variant(2)])),
        review(),
    ]);
    let mut r = renderer();
    let state = run_it(&store, &mut agent, dir.path(), &request, &mut m, &mut r);
    assert_eq!(state.progress.phase, RunPhase::Complete);
    assert_eq!(state.directions.len(), 3);

    // Malformed `variants` is repaired a bounded number of times, then the
    // run stops at Generate with the exact error.
    let (dir, mut agent, store, request) = setup();
    let bad = || source_with("design_create", json!("not an array"));
    let mut m = model(vec![concepts(), bad(), bad(), bad()]);
    let mut r = renderer();
    let state = run_it(&store, &mut agent, dir.path(), &request, &mut m, &mut r);
    assert_eq!(state.progress.phase, RunPhase::Blocked);
    let failure = state.failure.expect("receipt");
    assert_eq!(failure.phase, RunPhase::Generate);
    assert!(failure
        .expected_schema
        .contains("variants is always an array"));
    assert_eq!(failure.revision, RevisionId(0), "nothing was committed");
    assert!(!failure.message.is_empty());
}

#[test]
fn a_blocked_run_resumes_from_its_checkpoint_inside_the_same_allowance() {
    let (dir, mut agent, store, request) = setup();
    let mut m = model(vec![text("nope"), text("nope"), text("nope")]);
    let mut r = renderer();
    let blocked = run_it(&store, &mut agent, dir.path(), &request, &mut m, &mut r);
    assert_eq!(blocked.progress.phase, RunPhase::Blocked);
    assert_eq!(blocked.progress.repairs, 2);

    // Attempt ids continue where the first provider session stopped.
    let mut m = model(vec![concepts(), source("design_create"), review()]);
    m.calls = 3;
    let resumed = resume(
        &store,
        &mut agent,
        dir.path(),
        request.clone(),
        &mut m,
        &mut r,
        None,
    )
    .unwrap();
    assert_eq!(
        resumed.progress.phase,
        RunPhase::Complete,
        "{:?}",
        resumed.failure
    );
    assert_eq!(resumed.progress.repairs, 2, "repairs are not reset");
    assert!(resumed.failure.is_none());
}

#[test]
fn an_unparseable_critique_is_asked_again() {
    let (dir, mut agent, store, request) = setup();
    let mut m = model(vec![
        concepts(),
        source("design_create"),
        text("looks great to me"),
        review(),
    ]);
    let mut r = renderer();
    let state = run_it(&store, &mut agent, dir.path(), &request, &mut m, &mut r);
    assert_eq!(state.progress.phase, RunPhase::Complete);
    assert_eq!(state.progress.repairs, 1);
}

#[test]
fn directions_that_repeat_an_idea_are_rejected() {
    let same = |layout: &str, typography: &str| ConceptDirection {
        title: layout.into(),
        layout: layout.into(),
        typography: typography.into(),
        content_strategy: "Same idea restyled".into(),
    };
    assert!(validate_directions(&[same("editorial", "serif"), same("split", "sans")], 2).is_err());
    assert!(validate_directions(
        &[direction("editorial", "serif"), direction("split", "sans")],
        2
    )
    .is_ok());
    assert_eq!(
        parse_directions(
            r#"{"directions":[{"title":"a","layout":"split","typography":"sans","content_strategy":"x"}]}"#,
            1
        )
        .unwrap()
        .len(),
        1
    );
    assert!(parse_directions("[1,2]", 2).is_err());
}

#[test]
fn infer_product_dashboard_and_generate_three_distinct_directions() {
    let input = create("/design Create a modern dashboard for AI agents");
    assert_eq!(input.kind, DesignKind::Product);
    assert_eq!(input.variants, 3);

    let (dir, mut agent, store, mut request) = setup_with_variants(input.variants);
    request.brief = input.brief.clone();
    let mut m = model(vec![
        three_concepts(),
        source_with("design_create", json!([variant(0), variant(1), variant(2)])),
        review(),
    ]);
    let mut r = renderer();
    let state = run_it(&store, &mut agent, dir.path(), &request, &mut m, &mut r);
    assert_eq!(state.progress.phase, RunPhase::Complete);
    let strategies: std::collections::BTreeSet<_> = state
        .directions
        .iter()
        .map(|d| d.content_strategy.clone())
        .collect();
    assert_eq!(strategies.len(), 3, "each concept is a different idea");
}
