//! Design wire references wrap the existing evidence owner without changing its ABI.
use super::{error::*, types::decimal_u64};
use davinci_agent::runtime::{
    evidence::ArtifactRef as EvidenceRef, evidence_store::VerificationEvidenceStore,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub id: String,
    pub sha256: String,
    pub media_type: String,
    #[serde(with = "decimal_u64")]
    pub size: u64,
    pub relative_store_path: String,
    pub redaction: Option<String>,
}
impl From<EvidenceRef> for ArtifactRef {
    fn from(r: EvidenceRef) -> Self {
        Self {
            id: r.id,
            sha256: r.sha256,
            media_type: r.media_type,
            size: r.size,
            relative_store_path: r.relative_store_path,
            redaction: r.redaction,
        }
    }
}
impl From<&ArtifactRef> for EvidenceRef {
    fn from(r: &ArtifactRef) -> Self {
        Self {
            id: r.id.clone(),
            sha256: r.sha256.clone(),
            media_type: r.media_type.clone(),
            size: r.size,
            relative_store_path: r.relative_store_path.clone(),
            redaction: r.redaction.clone(),
        }
    }
}
pub(crate) struct Blobs(PathBuf);
impl Blobs {
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }
    fn owner(&self) -> Result<VerificationEvidenceStore, String> {
        // Check every existing ancestor before the evidence owner creates directories.
        let existing = self
            .0
            .ancestors()
            .find(|p| p.exists())
            .ok_or("blob parent missing")?;
        super::runtime::no_links(existing).map_err(|e| e.to_string())?;
        Ok(VerificationEvidenceStore::new(self.0.clone()))
    }
    pub fn store_artifact(&self, media: &str, bytes: &[u8]) -> Result<ArtifactRef, String> {
        self.owner()?.store_artifact(media, bytes).map(Into::into)
    }
    pub fn get_artifact(&self, reference: &ArtifactRef) -> Result<Vec<u8>, String> {
        super::types::validate_hash(&reference.sha256).map_err(|e| e.to_string())?;
        if reference.relative_store_path != format!("{}.bin", reference.sha256) {
            return Err(DesignError::CorruptArtifact("invalid blob path".into()).to_string());
        }
        super::runtime::no_links(&self.0.join(&reference.relative_store_path))
            .map_err(|e| e.to_string())?;
        self.owner()?.get_artifact(&reference.into())
    }
}
