//! Which concept a person chose.
//!
//! A selection is a durable fact about the session, kept as its own session
//! entry type rather than a `DesignChange`: an older build reads design events
//! strictly and would refuse a whole session over an event it does not know,
//! while it skips entry types it does not own. Choosing a concept changes no
//! source and no file in the workspace; it only narrows what a natural-language
//! revision touches and what a later implementation proposal starts from.

use super::{admission::*, error::*, store::*, types::*};
use davinci_session::{branch_entries, custom_entry, JsonlSession};
use serde::{Deserialize, Serialize};

pub const SELECTION_TYPE: &str = "davinci.design.selection.v1";

/// Choose a concept of the current revision, or clear the choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectVariant {
    pub artifact_id: ArtifactId,
    pub revision: RevisionId,
    pub variant_id: Option<VariantId>,
    pub operation_id: OperationId,
}

/// What was chosen and on which source. It is evidence of a choice, not of
/// acceptance, and it authorizes nothing outside the design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignSelectionReceipt {
    pub artifact_id: ArtifactId,
    /// The revision that was on screen when the choice was made.
    pub revision: RevisionId,
    pub source_hash: String,
    /// `None` clears the choice.
    pub variant_id: Option<VariantId>,
    pub operation_id: OperationId,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionEntry {
    schema_version: SchemaVersion,
    owner_session: String,
    workspace: String,
    payload_digest: String,
    receipt: DesignSelectionReceipt,
}

fn entries(
    ctx: &AuthorizedDesignContext,
    session: &JsonlSession,
) -> DesignResult<Vec<SelectionEntry>> {
    ctx.check_session(session)?;
    let mut found = Vec::new();
    for entry in branch_entries(&session.entries, session.leaf_id.as_deref()) {
        if entry.custom_type.as_deref() != Some(SELECTION_TYPE) {
            continue;
        }
        let parsed: SelectionEntry = serde_json::from_value(
            entry
                .extra
                .get("data")
                .cloned()
                .ok_or_else(|| DesignError::CorruptArtifact("missing selection data".into()))?,
        )
        .map_err(|_| DesignError::CorruptArtifact("invalid design selection".into()))?;
        if parsed.owner_session != ctx.session_id() || parsed.workspace != ctx.workspace_id() {
            return Err(DesignError::Denied(
                "design selection owner mismatch; fork explicitly before reuse".into(),
            ));
        }
        found.push(parsed);
    }
    Ok(found)
}

/// Record a choice. Repeating the same operation returns the same receipt; the
/// same operation id with another payload is a conflict.
pub fn select_variant(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &mut JsonlSession,
    request: SelectVariant,
) -> DesignResult<DesignSelectionReceipt> {
    store.prepare(ctx, session, "design_patch", &request)?;
    let input_digest = digest(&request)?;
    for earlier in entries(ctx, session)? {
        if earlier.receipt.operation_id == request.operation_id {
            return if earlier.payload_digest == input_digest {
                Ok(earlier.receipt)
            } else {
                Err(DesignError::Conflict(
                    "operation ID reused with a different payload".into(),
                ))
            };
        }
    }
    let head = store.manifest(ctx, session, request.artifact_id)?;
    if head.revision != request.revision {
        return Err(DesignError::Conflict(
            "the design changed since it was shown; reload before choosing".into(),
        ));
    }
    let revision = store.read_revision(ctx, session, head.id, head.revision)?;
    if let Some(chosen) = request.variant_id {
        if !revision.variants.iter().any(|variant| variant.id == chosen) {
            return Err(DesignError::NotFound(
                "that concept is not in this revision".into(),
            ));
        }
    }
    let receipt = DesignSelectionReceipt {
        artifact_id: head.id,
        revision: head.revision,
        source_hash: revision.source_hash,
        variant_id: request.variant_id,
        operation_id: request.operation_id,
    };
    let entry = SelectionEntry {
        schema_version: SchemaVersion,
        owner_session: ctx.session_id().into(),
        workspace: ctx.workspace_id().into(),
        payload_digest: input_digest,
        receipt: receipt.clone(),
    };
    let id = request.operation_id.to_string();
    let parent = session.leaf_id.clone();
    session
        .append_entry_once(
            &id,
            parent.as_deref(),
            custom_entry(&id, SELECTION_TYPE, serde_json::to_value(entry)?),
        )
        .map_err(|error| DesignError::IoFailure(error.to_string()))?;
    Ok(receipt)
}

/// The concept currently chosen for an artifact on this session branch. A
/// choice outlives revisions as long as the concept is still in the head
/// revision; one that is gone, or cleared, reads as no choice.
pub fn current_selection(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &JsonlSession,
    artifact_id: ArtifactId,
) -> DesignResult<Option<DesignSelectionReceipt>> {
    let latest = entries(ctx, session)?
        .into_iter()
        .rev()
        .find(|entry| entry.receipt.artifact_id == artifact_id)
        .map(|entry| entry.receipt);
    let Some(receipt) = latest else {
        return Ok(None);
    };
    let Some(chosen) = receipt.variant_id else {
        return Ok(None);
    };
    let head = store.manifest(ctx, session, artifact_id)?;
    if head.revision.0 == 0 {
        return Ok(None);
    }
    let revision = store.read_revision(ctx, session, artifact_id, head.revision)?;
    Ok(revision
        .variants
        .iter()
        .any(|variant| variant.id == chosen)
        .then_some(receipt))
}
