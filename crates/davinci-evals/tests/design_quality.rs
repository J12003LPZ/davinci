use davinci_evals::design_quality::{load_manifest, DesignEvaluationRecord};
use std::path::Path;

#[test]
fn frozen_design_briefs_cover_classes_states_viewports_and_negative_cases() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/evals/design");
    let manifest = load_manifest(&root).unwrap();
    assert!(manifest.briefs.len() >= 12);
    for class in ["landing", "product", "document", "slides"] {
        assert!(manifest
            .briefs
            .iter()
            .any(|brief| brief.target_class == class));
    }
    for outcome in ["denied", "incomplete", "eligible"] {
        assert!(manifest
            .briefs
            .iter()
            .any(|brief| brief.expected_outcome == outcome));
    }
}

#[test]
fn evaluation_cannot_launder_unknown_usage_or_missing_visual_evidence() {
    let mut record = DesignEvaluationRecord::unrun("cafe");
    record.validate().unwrap();
    record.unknown_requests = 1;
    record.unknowns.clear();
    assert!(record.validate().is_err());
    record.unknowns.push("Provider outcome unknown".into());
    record.validate().unwrap();
    record.preference = Some("candidate".into());
    assert!(record.validate().is_err());
    record.preference = None;
    record.model_requests = 13;
    assert!(record.validate().is_err());
}
