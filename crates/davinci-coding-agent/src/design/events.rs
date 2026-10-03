use super::{records::*, types::*};
use serde::{Deserialize, Serialize};

pub const CUSTOM_TYPE: &str = "davinci.design.v1";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignEvent {
    pub schema_version: SchemaVersion,
    pub owner_session: String,
    pub workspace: String,
    pub operation_id: OperationId,
    pub payload_digest: String,
    pub change: DesignChange,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesignChange {
    Created {
        manifest: ArtifactManifest,
    },
    Forked {
        manifest: ArtifactManifest,
        revision_manifest: ArtifactRef,
    },
    Revision {
        artifact_id: ArtifactId,
        revision: RevisionId,
        manifest: ArtifactRef,
    },
    Comment {
        comment: DesignComment,
    },
    Rendered {
        receipt: RenderReceipt,
    },
    Interacted {
        receipt: ArtifactRef,
    },
    Accepted {
        revision: RevisionReference,
        evidence_hash: String,
        acknowledged_incomplete: Vec<String>,
    },
    SystemSynced {
        snapshot: ArtifactRef,
        path: String,
    },
    RunCheckpoint {
        run_id: OperationId,
        state: ArtifactRef,
    },
    HandoffProposed {
        proposal: ArtifactRef,
    },
    HandoffDraft {
        run_id: OperationId,
        state: ArtifactRef,
    },
    HandoffApplied {
        proposal_hash: String,
        transaction_id: String,
    },
    HandoffChecked {
        artifact_id: ArtifactId,
        run_id: OperationId,
        report: ArtifactRef,
    },
    Cancelled {
        artifact_id: ArtifactId,
    },
}
