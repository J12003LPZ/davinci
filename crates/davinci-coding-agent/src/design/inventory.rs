//! Reference closure for cumulative quotas and explicit orphan cleanup.
use super::{admission::*, error::*, events::*, store::*, types::*};
use davinci_session::JsonlSession;
use serde_json::Value;
use std::collections::BTreeMap;

fn references(value: &Value, pending: &mut Vec<ArtifactRef>) {
    match value {
        Value::Object(fields)
            if fields.contains_key("relative_store_path") && fields.contains_key("sha256") =>
        {
            if let Ok(reference) = serde_json::from_value(value.clone()) {
                pending.push(reference);
            }
        }
        Value::Object(fields) => {
            for value in fields.values() {
                references(value, pending);
            }
        }
        Value::Array(values) => {
            for value in values {
                references(value, pending);
            }
        }
        _ => {}
    }
}
pub(crate) fn closure(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    values: &[Value],
    limit: u64,
) -> DesignResult<BTreeMap<String, ArtifactRef>> {
    let mut pending = Vec::new();
    for value in values {
        references(value, &mut pending);
    }
    let mut found = BTreeMap::new();
    let mut bytes = 0u64;
    while let Some(reference) = pending.pop() {
        validate_hash(&reference.sha256)?;
        if let Some(previous) = found.get(&reference.sha256) {
            let previous: &ArtifactRef = previous;
            if previous.size != reference.size {
                return Err(DesignError::CorruptArtifact(
                    "inconsistent blob size".into(),
                ));
            }
            continue;
        }
        bytes = bytes
            .checked_add(reference.size)
            .ok_or_else(|| DesignError::BudgetExceeded("artifact size overflow".into()))?;
        if bytes > limit || found.len() >= 8192 {
            return Err(DesignError::BudgetExceeded(
                "cumulative artifact storage limit".into(),
            ));
        }
        let data = store.read_blob(ctx, &reference)?;
        if reference.media_type == "application/json" {
            let value: Value = serde_json::from_slice(&data)
                .map_err(|_| DesignError::CorruptArtifact("referenced JSON is corrupt".into()))?;
            references(&value, &mut pending);
        }
        found.insert(reference.sha256.clone(), reference);
    }
    Ok(found)
}
pub(crate) fn all_changes(
    ctx: &AuthorizedDesignContext,
    session: &JsonlSession,
) -> DesignResult<Vec<DesignChange>> {
    ctx.check_session(session)?;
    session
        .entries
        .iter()
        .filter(|entry| entry.custom_type.as_deref() == Some(CUSTOM_TYPE))
        .map(|entry| {
            let event: DesignEvent =
                serde_json::from_value(entry.extra.get("data").cloned().ok_or_else(|| {
                    DesignError::CorruptArtifact("design event data missing".into())
                })?)?;
            if event.owner_session != ctx.session_id() || event.workspace != ctx.workspace_id() {
                return Err(DesignError::Denied("design event owner mismatch".into()));
            }
            Ok(event.change)
        })
        .collect()
}
fn artifact(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    change: &DesignChange,
) -> DesignResult<Option<ArtifactId>> {
    Ok(match change {
        DesignChange::Created { manifest } | DesignChange::Forked { manifest, .. } => {
            Some(manifest.id)
        }
        DesignChange::Revision { artifact_id, .. }
        | DesignChange::Cancelled { artifact_id }
        | DesignChange::HandoffChecked { artifact_id, .. } => Some(*artifact_id),
        DesignChange::Comment { comment } => Some(comment.artifact_id),
        DesignChange::Rendered { receipt } => Some(receipt.request.artifact_id),
        DesignChange::Accepted { revision, .. } => Some(revision.artifact_id),
        DesignChange::RunCheckpoint { state, .. }
        | DesignChange::HandoffDraft { state, .. }
        | DesignChange::HandoffProposed { proposal: state } => {
            let value: Value = serde_json::from_slice(&store.read_blob(ctx, state)?)?;
            Some(serde_json::from_value(
                value["request"]["artifact_id"].clone(),
            )?)
        }
        DesignChange::Interacted { receipt } => {
            let value: Value = serde_json::from_slice(&store.read_blob(ctx, receipt)?)?;
            Some(serde_json::from_value(
                value["request"]["render"]["artifact_id"].clone(),
            )?)
        }
        _ => None,
    })
}
pub(crate) fn check_artifact(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &JsonlSession,
    change: &DesignChange,
    limit: u64,
) -> DesignResult<()> {
    let Some(id) = artifact(store, ctx, change)? else {
        return Ok(());
    };
    let mut values = vec![serde_json::to_value(change)?];
    for previous in all_changes(ctx, session)? {
        if artifact(store, ctx, &previous)? == Some(id) {
            values.push(serde_json::to_value(previous)?);
        }
    }
    closure(store, ctx, &values, limit)?;
    Ok(())
}
