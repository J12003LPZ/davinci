use super::{error::*, records::*, types::*};

fn state(value: CheckState, coverage: &[&str], failures: Vec<String>) -> CheckResult {
    CheckResult {
        state: value,
        coverage: coverage.iter().map(|s| (*s).into()).collect(),
        failures,
    }
}
/// Each dimension stands alone. Subjective review never clears deterministic failures.
pub fn quality_report(
    revision: &DesignRevision,
    receipts: &[RenderReceipt],
    runtime_hash: &str,
    policy_hash: &str,
) -> DesignResult<QualityReport> {
    validate_bundle(&revision.sources, &DesignLimits::default())?;
    if super::assets::source_digest(&revision.sources, &revision.assets)? != revision.source_hash {
        return Err(DesignError::CorruptArtifact(
            "source manifest digest mismatch".into(),
        ));
    }
    let reference = RevisionReference {
        artifact_id: revision.artifact_id,
        revision: revision.revision,
        source_hash: revision.source_hash.clone(),
    };
    let mut report = QualityReport {
        revision: reference,
        source: state(
            CheckState::Current,
            &["bounded source bundle", "digest verified"],
            vec![],
        ),
        render: state(
            CheckState::Unavailable,
            &[],
            vec!["No source-bound capture".into()],
        ),
        interaction: state(CheckState::Pending, &[], vec![]),
        accessibility: state(CheckState::Pending, &[], vec![]),
        visual: state(
            CheckState::Pending,
            &[],
            vec!["Awaiting model or human review".into()],
        ),
        assets: state(
            if revision.assets.iter().all(|a| !a.rights.trim().is_empty()) {
                CheckState::Current
            } else {
                CheckState::Failed
            },
            &["declared provenance and rights"],
            vec![],
        ),
        implementation: state(
            CheckState::Pending,
            &[],
            vec!["Repository build and tests have not run".into()],
        ),
        evidence: vec![],
    };
    if receipts.is_empty() {
        return Ok(report);
    }
    let boards: Vec<_> = revision
        .variants
        .iter()
        .flat_map(|v| &v.artboards)
        .map(|b| b.id)
        .collect();
    for receipt in receipts {
        if receipt.request.artifact_id != revision.artifact_id
            || receipt.request.revision != revision.revision
            || receipt.source_hash != revision.source_hash
            || receipt.runtime_hash != runtime_hash
            || receipt.policy_hash != policy_hash
        {
            report.render = state(
                CheckState::Stale,
                &[],
                vec!["Capture fingerprints do not match this revision and runtime".into()],
            );
            return Ok(report);
        }
        receipt.request.viewport.validate()?;
        if !boards.contains(&receipt.request.artboard_id) {
            return Err(DesignError::CorruptArtifact(
                "capture references unknown artboard".into(),
            ));
        }
    }
    let complete = boards.iter().all(|id| {
        Viewport::defaults().iter().all(|vp| {
            receipts
                .iter()
                .any(|r| r.request.artboard_id == *id && &r.request.viewport == vp)
        })
    });
    report.render = state(
        if complete {
            CheckState::Current
        } else {
            CheckState::Pending
        },
        &["source-bound viewport capture"],
        if complete {
            vec![]
        } else {
            vec!["Required artboard/viewports missing".into()]
        },
    );
    for receipt in receipts {
        if let Some(check) = receipt.checks.get("render") {
            report
                .render
                .coverage
                .extend(check.coverage.iter().cloned());
            report
                .render
                .failures
                .extend(check.failures.iter().cloned());
            if check.state == CheckState::Failed || !check.failures.is_empty() {
                report.render.state = CheckState::Failed;
            }
        }
    }
    for receipt in receipts {
        report
            .evidence
            .extend([receipt.screenshot.clone(), receipt.geometry.clone()]);
    }
    for (name, dimension) in [
        ("interaction", &mut report.interaction),
        ("accessibility", &mut report.accessibility),
    ] {
        let checks: Vec<_> = receipts
            .iter()
            .filter_map(|receipt| receipt.checks.get(name))
            .collect();
        for check in &checks {
            dimension.coverage.extend(check.coverage.iter().cloned());
            dimension.failures.extend(check.failures.iter().cloned());
        }
        dimension.coverage.sort();
        dimension.coverage.dedup();
        dimension.state = if !dimension.failures.is_empty()
            || checks.iter().any(|c| c.state == CheckState::Failed)
        {
            CheckState::Failed
        } else if checks.iter().any(|c| c.state == CheckState::Stale) {
            CheckState::Stale
        } else if checks.iter().any(|c| c.state == CheckState::Unavailable) {
            CheckState::Unavailable
        } else if complete
            && checks.len() == receipts.len()
            && checks.iter().all(|c| c.state == CheckState::Current)
        {
            CheckState::Current
        } else {
            CheckState::Pending
        };
    }
    Ok(report)
}
