//! Immutable, task-selected design guidance. The generic skill loader is unchanged.
use super::{error::*, store::digest, types::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SOURCE: &str = include_str!("../../design-resources/taste-skill/SKILL.md");
const LICENSE: &str = include_str!("../../design-resources/taste-skill/LICENSE");
const PROVENANCE: &str = include_str!("../../design-resources/taste-skill/SOURCE.json");
pub const COMMIT: &str = "ce26fc25c0e5e8cab638f883de62d9a86ee5e45b";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSection {
    pub number: u32,
    pub name: String,
    pub sha256: String,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSelection {
    pub kind: DesignKind,
    pub name: String,
    pub source_commit: String,
    pub source_sha256: String,
    pub license_sha256: String,
    pub local_sha256: String,
    pub local_guidance: String,
    pub sections: Vec<ProfileSection>,
    pub dials: [u32; 3],
    pub overrides: Vec<String>,
}
pub fn byte_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn select_profile(kind: DesignKind, overrides: &[String]) -> DesignResult<ProfileSelection> {
    let metadata: serde_json::Value = serde_json::from_str(PROVENANCE)?;
    let source_hash = byte_hash(SOURCE.as_bytes());
    let license_hash = byte_hash(LICENSE.as_bytes());
    if metadata["commit"] != COMMIT
        || metadata["sha256"] != source_hash
        || metadata["license_sha256"] != license_hash
        || !LICENSE.starts_with("MIT License")
    {
        return Err(DesignError::CorruptArtifact(
            "pinned Taste provenance mismatch".into(),
        ));
    }
    let (name, local, dials) = match kind {
        DesignKind::Landing => (
            "marketing-v1",
            include_str!("../../design-resources/profiles/marketing.md"),
            [7, 4, 4],
        ),
        DesignKind::Product => (
            "product-v1",
            include_str!("../../design-resources/profiles/product.md"),
            [4, 2, 7],
        ),
        DesignKind::Document => (
            "document-v1",
            include_str!("../../design-resources/profiles/document.md"),
            [5, 1, 4],
        ),
    };
    let mut sections = Vec::new();
    if kind == DesignKind::Landing {
        for number in [0, 1, 11, 13, 14] {
            let section = metadata["sections"]
                .as_array()
                .and_then(|s| s.iter().find(|s| s["number"] == number))
                .ok_or_else(|| {
                    DesignError::CorruptArtifact("required Taste section missing".into())
                })?;
            let range = section["start"]
                .as_u64()
                .zip(section["end"].as_u64())
                .ok_or_else(|| DesignError::CorruptArtifact("invalid section range".into()))?;
            let text = SOURCE
                .get(range.0 as usize..range.1 as usize)
                .ok_or_else(|| DesignError::CorruptArtifact("invalid section boundaries".into()))?;
            let sha256 = byte_hash(text.as_bytes());
            if section["sha256"] != sha256 {
                return Err(DesignError::CorruptArtifact("section hash mismatch".into()));
            }
            sections.push(ProfileSection {
                number,
                name: section["name"].as_str().unwrap_or_default().into(),
                sha256,
                text: text.into(),
            });
        }
    }
    if overrides.len() > 16 {
        return Err(DesignError::BudgetExceeded("profile overrides".into()));
    }
    for value in overrides {
        validate_text(value, 2048, "profile override")?;
    }
    Ok(ProfileSelection {
        kind,
        name: name.into(),
        source_commit: COMMIT.into(),
        source_sha256: source_hash,
        license_sha256: license_hash,
        local_sha256: byte_hash(local.as_bytes()),
        local_guidance: local.into(),
        sections,
        dials,
        overrides: overrides.to_vec(),
    })
}
impl ProfileSelection {
    pub fn validate(&self) -> DesignResult<()> {
        if self.dials.iter().any(|n| !(1..=10).contains(n)) {
            return Err(DesignError::InvalidInput(
                "design dials must be 1..10".into(),
            ));
        }
        let expected = select_profile(self.kind, &self.overrides)?;
        if digest(self)? != digest(&expected)? {
            return Err(DesignError::CorruptArtifact(
                "profile selection is not the pinned version".into(),
            ));
        }
        Ok(())
    }
}
