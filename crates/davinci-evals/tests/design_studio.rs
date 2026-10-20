//! The design studio's offline evaluation, as a test target `scripts/test-design.py` runs.

use davinci_evals::behavior::design_studio::{run_design_studio_evals, THRESHOLD};

#[test]
fn natural_language_design_routing_clears_every_bar() {
    let report = run_design_studio_evals();
    for (name, score) in [
        ("brief fidelity", report.brief_fidelity),
        ("kind accuracy", report.kind_accuracy),
        ("plan decisions", report.plan_decisions),
        ("redaction", report.redaction),
    ] {
        assert!(score >= THRESHOLD, "{name} scored {score}");
    }
    assert!(
        report.results.iter().all(|result| result.passed),
        "{:#?}",
        report
            .results
            .iter()
            .filter(|r| !r.passed)
            .collect::<Vec<_>>()
    );
    assert!(
        report.results.len() >= 50,
        "{} fixtures",
        report.results.len()
    );
}
