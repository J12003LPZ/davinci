//! Offline design fixture and evidence validation. This module never calls a model.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignManifest {
    pub schema_version: u32,
    pub references: Vec<DesignReference>,
    pub briefs: Vec<DesignBrief>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignReference {
    pub id: String,
    pub path: String,
    pub sha256: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignBrief {
    pub id: String,
    pub target_class: String,
    pub brief: String,
    pub content_constraints: Vec<String>,
    pub expected_states: Vec<String>,
    pub viewports: Vec<[u32; 2]>,
    pub reference_ids: Vec<String>,
    pub expected_outcome: String,
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}
pub fn load_manifest(root: &Path) -> Result<DesignManifest, String> {
    let bytes = fs::read(root.join("manifest.json")).map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err("fixture manifest exceeds limit".into());
    }
    let manifest: DesignManifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if manifest.schema_version != 1
        || !(12..=64).contains(&manifest.briefs.len())
        || manifest.references.is_empty()
    {
        return Err("manifest coverage is incomplete".into());
    }
    let mut references = BTreeSet::new();
    for reference in &manifest.references {
        davinci_coding_agent::design::types::validate_path(&reference.path)
            .map_err(|e| e.to_string())?;
        if !references.insert(&reference.id) || !hash(&reference.sha256) {
            return Err("invalid reference identity".into());
        }
        let path = root.join(&reference.path);
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        if !canonical.starts_with(root.canonicalize().map_err(|e| e.to_string())?)
            || fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
        {
            return Err("reference escapes fixture root".into());
        }
        let content = fs::read(path).map_err(|e| e.to_string())?;
        if content.len() > 256 * 1024
            || format!("{:x}", Sha256::digest(&content)) != reference.sha256
        {
            return Err("reference content changed".into());
        }
    }
    let mut ids = BTreeSet::new();
    for brief in &manifest.briefs {
        if !ids.insert(&brief.id)
            || !text(&brief.id, 64)
            || !text(&brief.brief, 4096)
            || !["landing", "product", "document", "slides"].contains(&brief.target_class.as_str())
            || !["eligible", "incomplete", "denied"].contains(&brief.expected_outcome.as_str())
            || brief.content_constraints.len() < 2
            || brief.expected_states.len() < 2
            || brief.reference_ids.is_empty()
            || brief
                .reference_ids
                .iter()
                .any(|id| !references.contains(id))
            || brief
                .content_constraints
                .iter()
                .chain(&brief.expected_states)
                .any(|value| !text(value, 1024))
            || ![[390, 844], [768, 1024], [1440, 900]]
                .iter()
                .all(|viewport| brief.viewports.contains(viewport))
        {
            return Err(format!("incomplete brief: {}", brief.id));
        }
    }
    Ok(manifest)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationIdentity {
    pub source_hash: String,
    pub profile_hash: String,
    pub runtime_hash: String,
    pub evidence_hash: String,
    pub model: String,
    pub effort: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignEvaluationRecord {
    pub schema_version: u32,
    pub brief_id: String,
    pub task_hash: Option<String>,
    pub baseline: Option<EvaluationIdentity>,
    pub candidate: Option<EvaluationIdentity>,
    pub deterministic_gates: std::collections::BTreeMap<String, String>,
    pub preference: Option<String>,
    pub model_requests: u32,
    pub unknown_requests: u32,
    pub repairs: u32,
    pub latency_ms: Option<u64>,
    pub unknowns: Vec<String>,
}
impl DesignEvaluationRecord {
    pub fn unrun(brief_id: &str) -> Self {
        Self {
            schema_version: 1,
            brief_id: brief_id.into(),
            task_hash: None,
            baseline: None,
            candidate: None,
            deterministic_gates: Default::default(),
            preference: None,
            model_requests: 0,
            unknown_requests: 0,
            repairs: 0,
            latency_ms: None,
            unknowns: vec![
                "Evaluation not run; no quality preference or live measurements recorded".into(),
            ],
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || !text(&self.brief_id, 64)
            || self.model_requests.saturating_add(self.unknown_requests) > 12
            || self.repairs > 2
            || self.unknowns.len() > 32
            || self.unknowns.iter().any(|v| !text(v, 2048))
            || (self.unknown_requests > 0 && self.unknowns.is_empty())
        {
            return Err("invalid evaluation accounting".into());
        }
        if self.task_hash.as_deref().is_some_and(|v| !hash(v)) {
            return Err("invalid task hash".into());
        }
        for identity in [&self.baseline, &self.candidate].into_iter().flatten() {
            if [
                &identity.source_hash,
                &identity.profile_hash,
                &identity.runtime_hash,
                &identity.evidence_hash,
            ]
            .iter()
            .any(|v| !hash(v))
                || !text(&identity.model, 256)
                || !text(&identity.effort, 64)
            {
                return Err("incomplete run identity".into());
            }
        }
        if self.deterministic_gates.len() > 32
            || self.deterministic_gates.iter().any(|(name, state)| {
                !text(name, 128)
                    || !["passed", "failed", "incomplete", "denied"].contains(&state.as_str())
            })
        {
            return Err("invalid deterministic gates".into());
        }
        if let Some(preference) = &self.preference {
            if !["baseline", "candidate", "tie"].contains(&preference.as_str())
                || self.baseline.is_none()
                || self.candidate.is_none()
                || self.task_hash.is_none()
                || self.latency_ms.is_none()
                || self.deterministic_gates.is_empty()
                || self
                    .deterministic_gates
                    .values()
                    .any(|value| value != "passed")
                || self.unknown_requests > 0
            {
                return Err("reviewer preference requires complete paired evidence".into());
            }
        }
        Ok(())
    }
}
