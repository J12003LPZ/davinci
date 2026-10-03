use davinci_coding_agent::design::{acceptance::acceptance_gaps, records::*, types::*};

fn report() -> QualityReport {
    let current = CheckResult {
        state: CheckState::Current,
        coverage: vec!["fixture evidence".into()],
        failures: vec![],
    };
    let pending = CheckResult {
        state: CheckState::Pending,
        coverage: vec![],
        failures: vec![],
    };
    QualityReport {
        revision: RevisionReference {
            artifact_id: ArtifactId::new(),
            revision: RevisionId(1),
            source_hash: "a".repeat(64),
        },
        source: current.clone(),
        render: current.clone(),
        interaction: pending.clone(),
        accessibility: current.clone(),
        visual: pending.clone(),
        assets: current,
        implementation: pending,
        evidence: vec![],
    }
}
#[test]
fn accepting_requires_current_captures_no_failed_gates_and_explicit_incomplete_dimensions() {
    let good = report();
    assert_eq!(
        acceptance_gaps(&good).unwrap(),
        vec!["interaction", "visual"]
    );
    for state in [
        CheckState::Failed,
        CheckState::Stale,
        CheckState::Unavailable,
        CheckState::Pending,
    ] {
        let mut missing = good.clone();
        missing.render.state = state;
        assert!(acceptance_gaps(&missing).is_err());
    }
    let mut bad = good;
    bad.accessibility.state = CheckState::Failed;
    assert!(acceptance_gaps(&bad).is_err());
}
