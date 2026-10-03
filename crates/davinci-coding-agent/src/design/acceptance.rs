//! Human acceptance is source/evidence-bound and never implementation evidence.
use super::{
    admission::*, error::*, events::DesignChange, records::*, runtime::TrustedDesignRuntime,
    store::*, types::*,
};
use davinci_session::JsonlSession;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptRequest {
    pub artifact_id: ArtifactId,
    pub revision: RevisionId,
    pub evidence_hash: String,
    pub acknowledged_incomplete: Vec<String>,
    pub operation_id: OperationId,
}

pub fn acceptance_gaps(report: &QualityReport) -> DesignResult<Vec<String>> {
    if report.source.state != CheckState::Current || report.render.state != CheckState::Current {
        return Err(DesignError::Denied(
            "acceptance requires current source and every required viewport capture".into(),
        ));
    }
    let mut incomplete = Vec::new();
    for (name, result) in [
        ("source", &report.source),
        ("render", &report.render),
        ("interaction", &report.interaction),
        ("accessibility", &report.accessibility),
        ("visual", &report.visual),
        ("assets", &report.assets),
    ] {
        if result.state == CheckState::Failed || result.state == CheckState::Stale {
            return Err(DesignError::Denied(format!(
                "{name} has failed or stale evidence"
            )));
        }
        if result.state != CheckState::Current {
            incomplete.push(name.into());
        }
    }
    Ok(incomplete)
}

pub fn accept_revision(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &mut JsonlSession,
    runtime: &TrustedDesignRuntime,
    request: AcceptRequest,
) -> DesignResult<RevisionReference> {
    store.prepare(ctx, session, "design_accept", &request)?;
    validate_hash(&request.evidence_hash)?;
    let input_hash = digest(&request)?;
    if let Some(change) = store.retry(ctx, session, request.operation_id, &input_hash)? {
        return match change {
            DesignChange::Accepted { revision, .. } => Ok(revision),
            _ => Err(DesignError::Conflict(
                "acceptance operation type changed".into(),
            )),
        };
    }
    let head = store.manifest(ctx, session, request.artifact_id)?;
    if head.revision != request.revision {
        return Err(DesignError::Conflict("acceptance revision changed".into()));
    }
    let revision = store.read_revision(ctx, session, head.id, head.revision)?;
    let receipts = store.receipts(ctx, session, head.id, head.revision)?;
    runtime.verify()?;
    let report = super::quality::quality_report(
        &revision,
        &receipts,
        runtime.fingerprint(),
        &super::render::policy_hash(),
    )?;
    if digest(&report)? != request.evidence_hash {
        return Err(DesignError::Conflict(
            "acceptance evidence changed; review again".into(),
        ));
    }
    let gaps = acceptance_gaps(&report)?;
    if gaps != request.acknowledged_incomplete {
        return Err(DesignError::Denied(
            "acknowledge the exact incomplete dimensions before accepting".into(),
        ));
    }
    ctx.check("design_accept", &serde_json::to_value(&request)?)?;
    store.publish(
        ctx,
        session,
        request.operation_id,
        input_hash,
        DesignChange::Accepted {
            revision: report.revision.clone(),
            evidence_hash: request.evidence_hash,
            acknowledged_incomplete: gaps,
        },
    )?;
    Ok(report.revision)
}

pub fn accepted_revision(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &JsonlSession,
    artifact_id: ArtifactId,
    revision: RevisionId,
) -> DesignResult<RevisionReference> {
    let head = store.manifest(ctx, session, artifact_id)?;
    if head.revision != revision {
        return Err(DesignError::Conflict("accepted source has changed".into()));
    }
    let current = store.read_revision(ctx, session, artifact_id, revision)?;
    store
        .events(ctx, session)?
        .into_iter()
        .rev()
        .find_map(|event| match event.change {
            DesignChange::Accepted {
                revision: reference,
                ..
            } if reference.artifact_id == artifact_id
                && reference.revision == revision
                && reference.source_hash == current.source_hash =>
            {
                Some(reference)
            }
            _ => None,
        })
        .ok_or_else(|| DesignError::Denied("this exact revision has not been accepted".into()))
}
