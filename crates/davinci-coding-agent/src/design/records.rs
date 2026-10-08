use super::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateDesign {
    pub title: String,
    pub brief: String,
    pub kind: DesignKind,
    pub operation_id: OperationId,
    pub variants: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactManifest {
    pub schema_version: SchemaVersion,
    pub id: ArtifactId,
    pub owner_session: String,
    pub workspace: String,
    pub branch_entry: Option<String>,
    pub title: String,
    pub kind: DesignKind,
    pub brief: ArtifactRef,
    pub requested_variants: u32,
    #[serde(with = "decimal_u64")]
    pub created_at_ms: u64,
    pub revision: RevisionId,
    pub ancestry: Option<RevisionReference>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionReference {
    pub artifact_id: ArtifactId,
    pub revision: RevisionId,
    pub source_hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignRevision {
    pub schema_version: SchemaVersion,
    pub artifact_id: ArtifactId,
    pub revision: RevisionId,
    pub parent: RevisionId,
    pub operation_id: OperationId,
    pub source_hash: String,
    pub sources: SourceBundle,
    pub variants: Vec<Variant>,
    pub bindings: Vec<EditableBinding>,
    pub assets: Vec<Asset>,
    pub profile_refs: Vec<ArtifactRef>,
    pub system_snapshot: Option<ArtifactRef>,
    pub restored_from: Option<RevisionId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionWrite {
    pub artifact_id: ArtifactId,
    pub expected_revision: RevisionId,
    pub operation_id: OperationId,
    pub sources: SourceBundle,
    pub variants: Vec<Variant>,
    pub bindings: Vec<EditableBinding>,
    pub assets: Vec<Asset>,
    pub profile_refs: Vec<ArtifactRef>,
    pub system_snapshot: Option<ArtifactRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditableBinding {
    pub node_id: NodeId,
    pub artboard_id: ArtboardId,
    pub source_file: String,
    /// The host fills this in from the file's bytes; a model may leave it out.
    #[serde(default)]
    pub source_hash: String,
    pub pointer: String,
    pub constraint: BindingConstraint,
    pub affected_nodes: Vec<NodeId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BindingConstraint {
    Text { max_bytes: u32 },
    Enum { values: Vec<String> },
    Number { min: i64, max: i64 },
    Color {},
    LocalAsset { paths: Vec<String> },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub path: String,
    pub source: ArtifactRef,
    pub provenance: String,
    pub rights: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignEdit {
    pub artifact_id: ArtifactId,
    pub expected_revision: RevisionId,
    pub node_id: NodeId,
    pub expected_binding_hash: String,
    pub operation_id: OperationId,
    pub value: serde_json::Value,
    pub confirmed_affected_nodes: Vec<NodeId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignComment {
    pub id: CommentId,
    pub artifact_id: ArtifactId,
    pub revision: RevisionId,
    pub artboard_id: ArtboardId,
    pub node_id: Option<NodeId>,
    pub x_milli: u32,
    pub y_milli: u32,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}
impl Viewport {
    pub fn defaults() -> [Self; 3] {
        [
            Self {
                width: 390,
                height: 844,
            },
            Self {
                width: 768,
                height: 1024,
            },
            Self {
                width: 1440,
                height: 900,
            },
        ]
    }
    pub fn validate(&self) -> super::error::DesignResult<()> {
        if self.width < 128
            || self.height < 128
            || self.width > 4096
            || self.height > 4096
            || u64::from(self.width) * u64::from(self.height) > 8_000_000
        {
            return Err(super::error::DesignError::BudgetExceeded(
                "viewport pixel budget".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderRequest {
    pub artifact_id: ArtifactId,
    pub revision: RevisionId,
    pub artboard_id: ArtboardId,
    pub viewport: Viewport,
    pub theme: Theme,
    pub fixture: String,
    pub reduced_motion: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Light,
    Dark,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Pending,
    Current,
    Stale,
    Failed,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    pub state: CheckState,
    pub coverage: Vec<String>,
    pub failures: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderReceipt {
    pub request: RenderRequest,
    pub source_hash: String,
    pub runtime_hash: String,
    pub policy_hash: String,
    pub screenshot: ArtifactRef,
    pub geometry: ArtifactRef,
    pub checks: BTreeMap<String, CheckResult>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityReport {
    pub revision: RevisionReference,
    pub source: CheckResult,
    pub render: CheckResult,
    pub interaction: CheckResult,
    pub accessibility: CheckResult,
    pub visual: CheckResult,
    pub assets: CheckResult,
    pub implementation: CheckResult,
    pub evidence: Vec<ArtifactRef>,
}
