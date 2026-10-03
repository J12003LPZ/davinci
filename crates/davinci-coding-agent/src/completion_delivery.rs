//! Delivery inspector and installed binary verification for proof-backed completion.
//!
//! Bridges built executable artifacts to installed disk binaries, PATH resolutions,
//! wrapper/shim targets, and running process memory images.

/// Evaluates whether the installed binary hash matches the verified build artifact hash.
pub fn installed_matches(
    build: Option<&str>,
    installed: Option<&str>,
    resolution_verified: bool,
) -> bool {
    resolution_verified
        && match (build, installed) {
            (Some(a), Some(b)) => !a.is_empty() && a == b,
            _ => false,
        }
}

use davinci_agent::verification::acceptance::{acceptance_pack, AcceptanceCheck, ChangeRisk};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceLevel {
    Implemented,
    LocallyVerified,
    IntegrationVerified,
    ReleaseCandidate,
    DeployedOperationallyVerified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryIdentity {
    pub source_digest: String,
    pub configuration_digest: String,
    pub artifact_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryRequirement {
    pub id: String,
    pub risk: ChangeRisk,
}

/// Host-owned evidence only. Worker assertions, prose and screenshots do not
/// instantiate receipts. The caller must authenticate execution and coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceReceipt {
    pub receipt_id: String,
    pub requirement_id: String,
    pub check: AcceptanceCheck,
    pub identity: DeliveryIdentity,
    pub passed: bool,
}

#[derive(Debug, Clone)]
pub struct DeliveryInput {
    pub identity: DeliveryIdentity,
    pub requirements: Vec<DeliveryRequirement>,
    pub receipts: Vec<AcceptanceReceipt>,
    pub operational_references: Vec<String>,
    pub rollback_references: Vec<String>,
    /// Observed explicit authorization; this evaluator never grants authority.
    pub deployment_authorized: bool,
    pub installed_sha256: Option<String>,
    pub installed_resolution_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryRecord {
    pub schema_version: u32,
    pub level: AcceptanceLevel,
    pub identity: DeliveryIdentity,
    pub requirements: Vec<DeliveryRequirement>,
    pub receipts: Vec<AcceptanceReceipt>,
    pub gaps: Vec<String>,
    pub operational_references: Vec<String>,
    pub rollback_references: Vec<String>,
}

fn hash_known(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn public_identity(identity: &DeliveryIdentity) -> DeliveryIdentity {
    DeliveryIdentity {
        source_digest: if hash_known(&identity.source_digest) {
            identity.source_digest.clone()
        } else {
            "unknown".into()
        },
        configuration_digest: if hash_known(&identity.configuration_digest) {
            identity.configuration_digest.clone()
        } else {
            "unknown".into()
        },
        artifact_sha256: identity.artifact_sha256.clone().filter(|s| hash_known(s)),
    }
}

/// Versioned, deterministic evidence evaluation. No command is executed and
/// no publishing/deployment authorization is implied by any acceptance level.
pub fn delivery_record(input: &DeliveryInput) -> DeliveryRecord {
    let mut gaps = Vec::new();
    let identity_known = hash_known(&input.identity.source_digest)
        && hash_known(&input.identity.configuration_digest);
    if !identity_known {
        gaps.push("Current source/configuration identity is unavailable.".into());
    }
    let mut by_id = BTreeMap::new();
    let mut conflict = false;
    for receipt in &input.receipts {
        if receipt.receipt_id.is_empty()
            || by_id
                .insert(&receipt.receipt_id, receipt)
                .is_some_and(|previous| previous != receipt)
        {
            conflict = true;
        }
    }
    if conflict {
        gaps.push("Conflicting or unidentified execution receipts.".into());
    }
    let mut requirement_ids = BTreeSet::new();
    let requirements_known = !input.requirements.is_empty()
        && input.requirements.iter().all(|requirement| {
            !requirement.id.is_empty() && requirement_ids.insert(&requirement.id)
        });
    if !requirements_known {
        gaps.push("Requirements are missing or have duplicate identities.".into());
    }
    let passed = |id: &str, check: AcceptanceCheck| {
        let current = input
            .receipts
            .iter()
            .filter(|receipt| {
                receipt.requirement_id == id
                    && receipt.check == check
                    && receipt.identity == input.identity
            })
            .collect::<Vec<_>>();
        !current.is_empty() && current.iter().all(|receipt| receipt.passed)
    };
    for requirement in &input.requirements {
        for check in acceptance_pack(requirement.risk).required {
            if !passed(&requirement.id, check) {
                gaps.push(format!(
                    "{}: {check:?} has no passing current evidence.",
                    requirement.id
                ));
            }
        }
    }
    let local = identity_known && requirements_known && !conflict && gaps.is_empty();
    let every =
        |check| requirements_known && input.requirements.iter().all(|r| passed(&r.id, check));
    let artifact_known = input
        .identity
        .artifact_sha256
        .as_deref()
        .is_some_and(hash_known);
    let integration = local && artifact_known && every(AcceptanceCheck::Integration);
    let release = integration
        && every(AcceptanceCheck::ReleaseProvenance)
        && every(AcceptanceCheck::Rollback)
        && !input.operational_references.is_empty()
        && !input.rollback_references.is_empty();
    let installed = artifact_known
        && installed_matches(
            input.identity.artifact_sha256.as_deref(),
            input.installed_sha256.as_deref(),
            input.installed_resolution_verified,
        );
    let deployed = release
        && input.deployment_authorized
        && installed
        && every(AcceptanceCheck::DeploymentHealth);
    if !integration {
        gaps.push("Integration verification against the identified build is unavailable.".into());
    }
    if !release {
        gaps.push(
            "Release provenance, rollback proof or operational references are incomplete.".into(),
        );
    }
    if !installed {
        gaps.push(
            "The installed/running artifact has not been matched to the tested build.".into(),
        );
    }
    if !input.deployment_authorized {
        gaps.push("Deployment requires separate explicit authority.".into());
    }
    if !every(AcceptanceCheck::DeploymentHealth) {
        gaps.push("Operational health evidence is unavailable.".into());
    }
    let level = if deployed {
        AcceptanceLevel::DeployedOperationallyVerified
    } else if release {
        AcceptanceLevel::ReleaseCandidate
    } else if integration {
        AcceptanceLevel::IntegrationVerified
    } else if local {
        AcceptanceLevel::LocallyVerified
    } else {
        AcceptanceLevel::Implemented
    };
    let clean = davinci_agent::runtime::contracts::redact_secrets;
    let mut receipts = input.receipts.clone();
    for receipt in &mut receipts {
        receipt.receipt_id = clean(&receipt.receipt_id);
        receipt.requirement_id = clean(&receipt.requirement_id);
        receipt.identity = public_identity(&receipt.identity);
    }
    DeliveryRecord {
        schema_version: 1,
        level,
        identity: public_identity(&input.identity),
        requirements: input
            .requirements
            .iter()
            .map(|r| DeliveryRequirement {
                id: clean(&r.id),
                risk: r.risk,
            })
            .collect(),
        receipts,
        gaps: gaps.iter().map(|s| clean(s)).collect(),
        operational_references: input
            .operational_references
            .iter()
            .map(|s| clean(s))
            .collect(),
        rollback_references: input.rollback_references.iter().map(|s| clean(s)).collect(),
    }
}
