//! WOR-67: a lifecycle transition with no evidence references must never
//! change state. `apply_transition` reaches its `all(is_new_authority)` check
//! with an empty list, which passes vacuously, so the later `validate_value`
//! rejection is the only thing that stops it. This pins that end to end.
use davinci_agent::runtime::context_manifest::ProvenanceKind;
use davinci_agent::runtime::context_vm::{
    parse_checkpoint_proposal, CheckpointProposal, ContextEvent, ContextEventKind,
    ContextStateReducer, ProposedStateValue,
};

fn event(seq: u64, kind: ContextEventKind, provenance_kind: ProvenanceKind) -> ContextEvent {
    ContextEvent {
        source_ref: format!("e:{seq}"),
        seq,
        kind,
        provenance_kind,
        content_hash: format!("hash-{seq}"),
        visible_text: format!("text {seq}"),
        artifact_refs: Vec::new(),
        images: Vec::new(),
    }
}

#[test]
fn wor67_empty_evidence_or_replacement_refs_change_nothing() {
    let events = vec![
        event(1, ContextEventKind::User, ProvenanceKind::UserDecision),
        event(2, ContextEventKind::User, ProvenanceKind::UserDecision),
        event(
            3,
            ContextEventKind::ToolResult,
            ProvenanceKind::ToolEvidence,
        ),
    ];
    let seed = CheckpointProposal {
        goals: vec![ProposedStateValue {
            value: "ship it".into(),
            source_refs: vec!["e:1".into()],
            provenance_kind: ProvenanceKind::UserDecision,
        }],
        constraints: vec![ProposedStateValue {
            value: "keep the API".into(),
            source_refs: vec!["e:1".into()],
            provenance_kind: ProvenanceKind::UserDecision,
        }],
        blockers: vec![ProposedStateValue {
            value: "tests fail".into(),
            source_refs: vec!["e:1".into()],
            provenance_kind: ProvenanceKind::UserDecision,
        }],
        ..CheckpointProposal::default()
    };
    let parent = ContextStateReducer::validate_proposal(&Default::default(), &events, seed);
    assert_eq!(parent.constraints.len(), 1);
    assert_eq!(parent.goals.len(), 1);
    assert_eq!(parent.blockers.len(), 1);

    let empty = serde_json::json!({"value":"x","source_refs":[],"provenance_kind":"user_decision"});
    let valid =
        serde_json::json!({"value":"x","source_refs":["e:2"],"provenance_kind":"user_decision"});
    let cases = [
        ("constraint", "keep the API"),
        ("goal", "ship it"),
        ("blocker", "tests fail"),
    ];
    for (slot, previous) in cases {
        for kind in ["resolve", "supersede", "reject"] {
            for (evidence, replacement) in [
                (&empty, serde_json::Value::Null),
                (&empty, valid.clone()),
                (&valid, empty.clone()),
            ] {
                let text = serde_json::json!({"transitions":[{
                    "slot": slot, "kind": kind, "previous": previous,
                    "evidence": evidence, "replacement": replacement,
                }]})
                .to_string();
                let proposal = parse_checkpoint_proposal(&text).unwrap();
                let next = ContextStateReducer::validate_proposal(&parent, &events, proposal);
                assert_eq!(next, parent, "{slot}/{kind} changed state");
            }
        }
    }

    // Control: the same shape with real evidence does apply, so the loop above
    // would notice a transition leaking through.
    let text = serde_json::json!({"transitions":[{
        "slot": "constraint", "kind": "supersede", "previous": "keep the API",
        "evidence": valid, "replacement": valid,
    }]})
    .to_string();
    let proposal = parse_checkpoint_proposal(&text).unwrap();
    let next = ContextStateReducer::validate_proposal(&parent, &events, proposal);
    assert_ne!(next, parent);
}
