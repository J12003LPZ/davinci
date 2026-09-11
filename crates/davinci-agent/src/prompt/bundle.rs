//! Versioned prompt bundles and candidate evaluation descriptors.

use crate::prompt::composer::{
    compose_modules, legacy_default_module, stable_v2_modules, ComposedPrompt, PromptCacheClass,
    PromptContext, PromptModule,
};
use crate::prompt::version::{
    PromptProfile, LEGACY_PROMPT_VERSION, PREVIEW_PROMPT_VERSION, STABLE_PROMPT_VERSION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A frozen bundle of prompt modules defining a prompt profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptBundle {
    pub id: &'static str,
    pub version: u32,
    pub modules: Vec<PromptModule>,
}

impl PromptBundle {
    /// Concatenates all stable cache-class modules in definition order.
    pub fn stable_text(&self) -> String {
        self.modules
            .iter()
            .filter(|m| m.cache_class == PromptCacheClass::Stable)
            .map(|m| m.body.trim())
            .filter(|body| !body.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Computes the SHA256 hex digest of the stable module text.
    pub fn stable_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.stable_text().as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Composes this bundle into a full prompt with provider adapter and runtime state.
    pub fn compose(&self, ctx: &PromptContext<'_>) -> ComposedPrompt {
        let mut modules = self.modules.clone();
        let family = crate::prompt::provider::prompt_model_family(ctx.provider, ctx.model_id);
        if let Some(adapter) = crate::prompt::provider::provider_adapter(family) {
            modules.push(adapter);
        }
        modules.push(crate::prompt::runtime_state::runtime_state_module(
            &crate::prompt::runtime_state::RuntimePromptState {
                permission_mode: ctx.permission_mode,
                plan_revision: None,
                plan_approved: false,
                active_contract: false,
            },
        ));
        let mut composed = compose_modules(&modules);
        composed.manifest.profile = self.id.to_string();
        composed.manifest.profile_version = self.version;
        composed
    }

    /// Checks if this bundle is byte-identical in its stable text to another bundle.
    pub fn is_byte_identical(&self, other: &PromptBundle) -> bool {
        self.stable_sha256() == other.stable_sha256()
    }
}

/// The legacy v1 prompt bundle.
pub fn legacy_bundle() -> PromptBundle {
    PromptBundle {
        id: "legacy-v1",
        version: LEGACY_PROMPT_VERSION,
        modules: vec![legacy_default_module()],
    }
}

/// The production Stable prompt bundle (v2).
pub fn stable_bundle() -> PromptBundle {
    PromptBundle {
        id: "stable",
        version: STABLE_PROMPT_VERSION,
        modules: stable_v2_modules(),
    }
}

/// Candidate preview verification module enhancing verification rigor.
pub fn preview_verification_module() -> PromptModule {
    PromptModule {
        id: "verification.completion".to_string(),
        version: 2,
        cache_class: PromptCacheClass::Stable,
        body: "\
After changing code, run the smallest meaningful verification first, then broader checks \
when risk warrants them. If a check fails, investigate the failure. Do not explain it away. \
Do not claim a check passed unless you ran it. Do not claim a test, build, lint, or behavior \
passed unless fresh evidence from this run supports it. Stop when the requested outcome is \
complete and verified. Do not continue polishing unrelated areas merely because tools and \
context remain available. When reproducing a failure, ensure the reproducer fails before the \
fix and passes after the fix."
            .to_string(),
    }
}

/// The candidate modules for Preview v3.
pub fn preview_v3_modules() -> Vec<PromptModule> {
    vec![
        crate::prompt::core::core_identity_module(),
        crate::prompt::core::core_autonomy_module(),
        crate::prompt::coding::coding_exploration_module(),
        crate::prompt::coding::coding_scope_discipline_module(),
        crate::prompt::coding::coding_change_quality_module(),
        crate::prompt::collaboration::collaboration_user_intent_module(),
        preview_verification_module(),
    ]
}

/// The candidate Preview prompt bundle (v3), behaviorally distinct from Stable.
pub fn preview_bundle() -> PromptBundle {
    PromptBundle {
        id: "preview",
        version: PREVIEW_PROMPT_VERSION,
        modules: preview_v3_modules(),
    }
}

/// Validates that an A/B comparison between two prompt bundles is valid and not byte-identical.
pub fn validate_ab_comparison(base: &PromptBundle, candidate: &PromptBundle) -> Result<(), String> {
    if base.stable_sha256() == candidate.stable_sha256() {
        return Err(format!(
            "A/B comparison refused: candidate bundle '{}' is byte-identical to base '{}' (sha256: {})",
            candidate.id,
            base.id,
            base.stable_sha256()
        ));
    }
    Ok(())
}

/// Descriptor identifying an active prompt candidate for evaluation and promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCandidateDescriptor {
    pub candidate_id: String,
    pub name: String,
    pub description: String,
    pub base_profile: PromptProfile,
    pub candidate_profile: PromptProfile,
    pub base_bundle_hash: String,
    pub candidate_bundle_hash: String,
}

impl PromptCandidateDescriptor {
    /// Creates a new candidate descriptor, verifying that the candidate is not byte-identical to base.
    pub fn new(
        candidate_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        base: &PromptBundle,
        candidate: &PromptBundle,
    ) -> Result<Self, String> {
        validate_ab_comparison(base, candidate)?;
        let base_profile = PromptProfile::parse(base.id)
            .ok_or_else(|| format!("Unknown base profile id '{}'", base.id))?;
        let candidate_profile = PromptProfile::parse(candidate.id)
            .ok_or_else(|| format!("Unknown candidate profile id '{}'", candidate.id))?;

        Ok(Self {
            candidate_id: candidate_id.into(),
            name: name.into(),
            description: description.into(),
            base_profile,
            candidate_profile,
            base_bundle_hash: base.stable_sha256(),
            candidate_bundle_hash: candidate.stable_sha256(),
        })
    }

    /// Validates that the provided bundles match this descriptor's recorded hashes and remain non-identical.
    pub fn validate(&self, base: &PromptBundle, candidate: &PromptBundle) -> Result<(), String> {
        validate_ab_comparison(base, candidate)?;
        if self.base_bundle_hash != base.stable_sha256() {
            return Err(format!(
                "Base bundle hash mismatch: descriptor expected {}, got {}",
                self.base_bundle_hash,
                base.stable_sha256()
            ));
        }
        if self.candidate_bundle_hash != candidate.stable_sha256() {
            return Err(format!(
                "Candidate bundle hash mismatch: descriptor expected {}, got {}",
                self.candidate_bundle_hash,
                candidate.stable_sha256()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_and_preview_bundles_have_distinct_hashes() {
        let stable = stable_bundle();
        let preview = preview_bundle();

        assert_eq!(stable.id, "stable");
        assert_eq!(stable.version, STABLE_PROMPT_VERSION);
        assert_eq!(preview.id, "preview");
        assert_eq!(preview.version, PREVIEW_PROMPT_VERSION);

        let stable_hash = stable.stable_sha256();
        let preview_hash = preview.stable_sha256();

        assert!(!stable_hash.is_empty());
        assert!(!preview_hash.is_empty());
        assert_ne!(
            stable_hash, preview_hash,
            "Preview candidate must change prompt hash from stable"
        );
    }

    #[test]
    fn candidate_descriptor_creation_and_validation() {
        let stable = stable_bundle();
        let preview = preview_bundle();

        let desc = PromptCandidateDescriptor::new(
            "preview-v3-eval",
            "Preview V3 Evaluation",
            "Candidate with reproducer evidence requirement",
            &stable,
            &preview,
        )
        .expect("candidate descriptor must be created when bundles differ");

        assert_eq!(desc.candidate_id, "preview-v3-eval");
        assert_eq!(desc.base_profile, PromptProfile::Stable);
        assert_eq!(desc.candidate_profile, PromptProfile::Preview);
        assert_eq!(desc.base_bundle_hash, stable.stable_sha256());
        assert_eq!(desc.candidate_bundle_hash, preview.stable_sha256());

        assert!(desc.validate(&stable, &preview).is_ok());
    }

    #[test]
    fn ab_comparison_refuses_byte_identical_bundles() {
        let stable = stable_bundle();
        let clone_stable = stable_bundle();

        let result = validate_ab_comparison(&stable, &clone_stable);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("A/B comparison refused"));
        assert!(err.contains("byte-identical"));

        let desc_result = PromptCandidateDescriptor::new(
            "identical-eval",
            "Identical Candidate",
            "Should fail",
            &stable,
            &clone_stable,
        );
        assert!(desc_result.is_err());
        let desc_err = desc_result.unwrap_err();
        assert!(desc_err.contains("A/B comparison refused"));
        assert!(desc_err.contains("byte-identical"));
    }
}
