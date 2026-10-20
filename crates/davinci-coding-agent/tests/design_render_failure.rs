//! When rendering or compiling fails and the repairs run out, and when the
//! review's findings outlast the repairs: what stays the head, what is kept,
//! and what the receipt offers. Offline: a scripted model and renderer.

mod design_support;

use davinci_ai::ContentBlock;
use davinci_coding_agent::design::{
    admission::AuthorizedDesignContext, controller::with_next_step, error::*, generation::*,
    types::*,
};
use davinci_coding_agent::design::{records::*, store::DesignStore};
use design_support::*;
use serde_json::json;

/// Renders like the recorded one, but fails with `InvalidInput` (the compile
/// path) for every call after the first `good_calls`.
struct FailingRenderer {
    calls: usize,
    good_calls: usize,
    inner: RecordedRenderer,
}

impl FailingRenderer {
    fn new(good_calls: usize) -> Self {
        Self {
            calls: 0,
            good_calls,
            inner: renderer(),
        }
    }
}

impl DesignRenderer for FailingRenderer {
    fn render(
        &mut self,
        store: &DesignStore,
        ctx: &AuthorizedDesignContext,
        session: &mut davinci_session::JsonlSession,
        revision: &DesignRevision,
    ) -> DesignResult<QualityReport> {
        self.calls += 1;
        if self.calls > self.good_calls {
            return Err(DesignError::InvalidInput(
                "virtual source compilation failed".into(),
            ));
        }
        self.inner.render(store, ctx, session, revision)
    }
}

struct Ids {
    variants: [VariantId; 2],
    artboards: [ArtboardId; 2],
}

fn ids() -> Ids {
    Ids {
        variants: [VariantId::new(), VariantId::new()],
        artboards: [ArtboardId::new(), ArtboardId::new()],
    }
}

/// The same two concepts every time (so a patch is a patch); `a_text` is the
/// only thing that changes between rounds.
fn two_concepts(name: &str, ids: &Ids, a_text: &str) -> ContentBlock {
    ContentBlock::ToolCall {
        id: OperationId::new().to_string(),
        name: name.into(),
        arguments: json!({
            "files": {
                "a.html": format!("<!doctype html><html lang='en'><title>A</title><h1>{a_text}</h1></html>"),
                "b.html": "<!doctype html><html lang='en'><title>B</title><main>Split fixture</main></html>",
            },
            "entry_points": ["a.html", "b.html"],
            "variants": [
                {"id": ids.variants[0], "title": "Editorial", "artboards": [{"id": ids.artboards[0], "title": "Desktop", "entry_point": "a.html"}]},
                {"id": ids.variants[1], "title": "Split", "artboards": [{"id": ids.artboards[1], "title": "Desktop", "entry_point": "b.html"}]},
            ],
            "bindings": [],
        }),
    }
}

fn findings() -> ContentBlock {
    text("{\"findings\":[\"the headline is too small\"]}")
}

fn head_files(
    store: &DesignStore,
    agent: &mut davinci_agent::Agent,
    dir: &std::path::Path,
    artifact: ArtifactId,
) -> (RevisionId, std::collections::BTreeMap<String, String>) {
    let ctx = AuthorizedDesignContext::from_agent(agent, dir).unwrap();
    let session = agent.session.as_mut().unwrap();
    let manifest = store.manifest(&ctx, session, artifact).unwrap();
    let files = store
        .read_sources(&ctx, session, artifact, manifest.revision)
        .unwrap();
    (manifest.revision, files)
}

#[test]
fn a_revision_that_never_renders_is_taken_off_the_head_and_kept() {
    let (dir, mut agent, store, request) = setup();
    let ids = ids();
    // Round one renders; the revision after it never does.
    let mut m = model(vec![
        concepts(),
        two_concepts("design_create", &ids, "Good headline"),
        review(),
    ]);
    let first = run(
        &store,
        &mut agent,
        dir.path(),
        request.clone(),
        &mut m,
        &mut FailingRenderer::new(1),
        None,
    )
    .unwrap();
    assert_eq!(first.progress.phase, RunPhase::Complete);
    let artifact = request.artifact_id;
    let (good_revision, good_files) = head_files(&store, &mut agent, dir.path(), artifact);
    assert_eq!(good_revision, RevisionId(1));

    let revise = GenerationRequest {
        artifact_id: artifact,
        expected_revision: good_revision,
        brief: "make the headline bolder".into(),
        operation_id: OperationId::new(),
        target_variant: Some(ids.variants[0]),
    };
    let mut m = model(vec![
        two_concepts("design_patch", &ids, "Broken one"),
        two_concepts("design_patch", &ids, "Broken two"),
        two_concepts("design_patch", &ids, "Broken three"),
    ]);
    m.calls = 10;
    // The recorded renderer's first call was spent on round one.
    let mut failing = FailingRenderer::new(0);
    let state = run(
        &store,
        &mut agent,
        dir.path(),
        revise,
        &mut m,
        &mut failing,
        None,
    )
    .unwrap();

    assert_eq!(state.progress.phase, RunPhase::Blocked);
    assert_eq!(
        failing.calls, 3,
        "one render per attempt, then the repairs ran out"
    );
    let failure = state.failure.clone().expect("a blocked run says why");
    assert_eq!(failure.phase, RunPhase::Render);
    assert_eq!(failure.repairs_remaining, 0);
    assert!(
        !failure.retryable,
        "resuming cannot mend a withdrawn revision"
    );
    assert!(!failure.head_unverified);

    // The head is the last good content again (as a new revision); the broken
    // revision is staged in history and still readable.
    let staged = failure
        .staged_revision
        .expect("the broken revision is named");
    assert_eq!(
        staged,
        RevisionId(4),
        "rev 1 good, 2 and 3 repaired, 4 withdrawn"
    );
    let (head, head_files_now) = head_files(&store, &mut agent, dir.path(), artifact);
    assert_eq!(head, RevisionId(5));
    assert_eq!(failure.revision, head);
    assert_eq!(
        head_files_now, good_files,
        "the head is the last good source"
    );
    let ctx = AuthorizedDesignContext::from_agent(&agent, dir.path()).unwrap();
    let session = agent.session.as_mut().unwrap();
    let broken = store.read_sources(&ctx, session, artifact, staged).unwrap();
    assert!(broken["a.html"].contains("Broken three"), "{broken:?}");
    let restored = store.read_revision(&ctx, session, artifact, head).unwrap();
    assert_eq!(restored.restored_from, Some(RevisionId(1)));

    // The message and the next step say what happened.
    assert!(
        failure.message.contains("taken off the head"),
        "{}",
        failure.message
    );
    assert!(
        failure.message.contains("revision 4"),
        "{}",
        failure.message
    );
    let value = with_next_step(serde_json::to_value(&state).unwrap(), artifact);
    let why = value["next"]["why"].as_str().unwrap();
    assert!(why.contains("kept in history"), "{why}");
    assert!(value["next"]["action"]
        .as_str()
        .unwrap()
        .starts_with("/design revise"));
}

#[test]
fn a_first_revision_that_never_renders_is_labelled_unverified() {
    let (dir, mut agent, store, request) = setup();
    let ids = ids();
    let mut m = model(vec![
        concepts(),
        two_concepts("design_create", &ids, "One"),
        two_concepts("design_patch", &ids, "Two"),
        two_concepts("design_patch", &ids, "Three"),
    ]);
    let mut failing = FailingRenderer::new(0);
    let state = run(
        &store,
        &mut agent,
        dir.path(),
        request.clone(),
        &mut m,
        &mut failing,
        None,
    )
    .unwrap();
    assert_eq!(state.progress.phase, RunPhase::Blocked);
    let failure = state.failure.clone().unwrap();
    assert_eq!(failure.phase, RunPhase::Render);
    // Nothing earlier rendered, so nothing could be put back.
    assert!(failure.head_unverified);
    assert_eq!(failure.staged_revision, None);
    assert!(
        failure.message.contains("never rendered"),
        "{}",
        failure.message
    );
    let value = with_next_step(serde_json::to_value(&state).unwrap(), request.artifact_id);
    // Nothing was withdrawn, so trying the render again is still on offer.
    assert!(failure.retryable);
    assert!(value["next"]["action"]
        .as_str()
        .unwrap()
        .starts_with("/design resume"));
}

#[test]
fn review_findings_that_outlast_the_repairs_are_not_offered_for_resume() {
    let (dir, mut agent, store, request) = setup();
    let ids = ids();
    let mut m = model(vec![
        concepts(),
        two_concepts("design_create", &ids, "One"),
        findings(),
        two_concepts("design_patch", &ids, "Two"),
        findings(),
        two_concepts("design_patch", &ids, "Three"),
        findings(),
    ]);
    let state = run(
        &store,
        &mut agent,
        dir.path(),
        request.clone(),
        &mut m,
        &mut renderer(),
        None,
    )
    .unwrap();
    assert_eq!(state.progress.phase, RunPhase::Blocked);
    let failure = state.failure.clone().unwrap();
    assert_eq!(failure.phase, RunPhase::Review);
    assert!(!failure.retryable, "{failure:?}");
    assert!(
        failure.message.contains("after two repair rounds"),
        "{}",
        failure.message
    );
    // The revision under review rendered, so it stays the head.
    assert_eq!(failure.staged_revision, None);
    assert!(!failure.head_unverified);
    let (head, files) = head_files(&store, &mut agent, dir.path(), request.artifact_id);
    assert_eq!(head, failure.revision);
    assert!(files["a.html"].contains("Three"));
    let value = with_next_step(serde_json::to_value(&state).unwrap(), request.artifact_id);
    let action = value["next"]["action"].as_str().unwrap();
    assert!(action.starts_with("/design revise"), "{action}");
    assert!(!action.contains("resume"));
}
