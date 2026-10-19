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
fn a_brief_may_start_with_a_subcommand_word_through_new_or_double_dash() {
    for (line, brief) in [
        (
            "/design new \"open a cafe landing page\"",
            "open a cafe landing page",
        ),
        ("/design -- list of tasks app", "list of tasks app"),
        (
            "/design new revise the hero section",
            "revise the hero section",
        ),
        (
            "/design new status board for my team",
            "status board for my team",
        ),
        ("/design -- export settings page", "export settings page"),
    ] {
        assert_eq!(create(line).brief, brief, "{line}");
    }
    // Without `new`, a reserved word is a verb, and a bad argument is an error.
    for line in [
        "/design open a cafe landing page",
        "/design list of tasks app",
        "/design status board for my team",
    ] {
        assert!(parse_design_command(line).is_err(), "{line}");
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

mod design_support;
use davinci_ai::ContentBlock;
use davinci_coding_agent::design::{error::*, generation::*, types::*};
use design_support::*;
use serde_json::json;

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

#[test]
fn resume_is_a_command_and_finds_the_run_by_its_checkpoint() {
    let (dir, mut agent, store, request) = setup();
    let mut m = model(vec![text("nope"), text("nope"), text("nope")]);
    let mut r = renderer();
    let blocked = run_it(&store, &mut agent, dir.path(), &request, &mut m, &mut r);
    let checkpoint = blocked.failure.expect("receipt").checkpoint;

    let line = format!("/design resume {} {checkpoint}", request.artifact_id);
    match parse_design_command(&line).unwrap() {
        DesignCommand::Resume {
            artifact_id,
            operation_id,
        } => {
            assert_eq!(artifact_id, request.artifact_id);
            assert_eq!(operation_id, checkpoint);
        }
        other => panic!("resume expected, got {other:?}"),
    }
    assert!(parse_design_command(&format!("/design resume {}", request.artifact_id)).is_err());
    // A brief that starts with the word needs `new`; bare it is a usage error.
    assert_eq!(
        create("/design new resume page for a nurse").brief,
        "resume page for a nurse"
    );
    assert!(parse_design_command("/design resume page for a nurse").is_err());

    let mut m = model(vec![concepts(), source("design_create"), review()]);
    m.calls = 3;
    let state = resume_operation(
        &store,
        &mut agent,
        dir.path(),
        checkpoint,
        &mut m,
        &mut r,
        None,
    )
    .unwrap();
    assert_eq!(state.progress.phase, RunPhase::Complete);
    assert!(matches!(
        resume_operation(
            &store,
            &mut agent,
            dir.path(),
            OperationId::new(),
            &mut m,
            &mut r,
            None
        ),
        Err(DesignError::NotFound(_))
    ));
}

#[test]
fn select_is_a_command_and_none_clears_it() {
    let artifact = ArtifactId::new();
    let variant = VariantId::new();
    match parse_design_command(&format!("/design select {artifact} {variant}")).unwrap() {
        DesignCommand::Select {
            artifact_id,
            variant_id,
        } => {
            assert_eq!(artifact_id, artifact);
            assert_eq!(variant_id, Some(variant));
        }
        other => panic!("select expected, got {other:?}"),
    }
    assert!(matches!(
        parse_design_command(&format!("/design select {artifact} none")).unwrap(),
        DesignCommand::Select {
            variant_id: None,
            ..
        }
    ));
    assert!(parse_design_command(&format!("/design select {artifact}")).is_err());
    assert!(parse_design_command(&format!("/design select {artifact} not-a-variant")).is_err());
    assert_eq!(
        create("/design new select a color palette for a florist").brief,
        "select a color palette for a florist"
    );
    assert!(parse_design_command("/design select a color palette for a florist").is_err());
}

#[test]
fn a_snippet_never_carries_the_credential_after_a_scheme() {
    let snippet = sanitize_snippet(
        "Authorization: Bearer abc.def.ghi and Basic dXNlcjpwYXNz",
        200,
    );
    assert!(!snippet.contains("abc.def.ghi") && !snippet.contains("dXNlcjpwYXNz"));
    assert_eq!(
        snippet,
        "Authorization: [redacted] [redacted] and [redacted] [redacted]"
    );
}

#[test]
fn resume_with_another_designs_checkpoint_stops_before_any_provider_call() {
    use davinci_coding_agent::design::admission::AuthorizedDesignContext;
    use davinci_coding_agent::design::commands::DesignCommand;
    use davinci_coding_agent::design::controller::DesignController;

    let (dir, mut agent, store, request) = setup();
    let mut m = model(vec![text("nope"), text("nope"), text("nope")]);
    let mut r = renderer();
    let blocked = run_it(&store, &mut agent, dir.path(), &request, &mut m, &mut r);
    let checkpoint = blocked.failure.expect("receipt").checkpoint;
    let ctx = AuthorizedDesignContext::from_agent(&mut agent, dir.path()).unwrap();
    let session = agent.session.as_ref().unwrap();
    let before = latest_run(&store, &ctx, session, checkpoint)
        .unwrap()
        .unwrap();

    let controller = DesignController::new(store, true);
    let other = ArtifactId::new();
    let outcome = controller.command(
        &mut agent,
        dir.path(),
        DesignCommand::Resume {
            artifact_id: other,
            operation_id: checkpoint,
        },
    );
    // The mismatch is reported before a model or renderer is even built
    // (building the real subscription model would fail differently), and the
    // run is exactly as it was: nothing was spent and nothing was committed.
    match outcome {
        Err(DesignError::Conflict(message)) => assert!(message.contains("different design")),
        other => panic!("conflict expected, got {other:?}"),
    }
    let ctx = AuthorizedDesignContext::from_agent(&mut agent, dir.path()).unwrap();
    let session = agent.session.as_ref().unwrap();
    let after = latest_run(&controller.store, &ctx, session, checkpoint)
        .unwrap()
        .unwrap();
    assert_eq!(before.progress.repairs, after.progress.repairs);
    assert_eq!(before.last_revision, after.last_revision);
    assert_eq!(before.progress.phase, after.progress.phase);
}
