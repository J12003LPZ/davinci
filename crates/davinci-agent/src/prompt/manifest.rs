//! Versioned prompt manifest and budgeting.

use crate::prompt::composer::{PromptCacheClass, PromptModule};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptModuleIdentity {
    pub id: String,
    pub version: u32,
    pub cache_class: PromptCacheClass,
    pub sha256: String,
    pub estimated_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptManifest {
    pub profile: String,
    pub profile_version: u32,
    pub stable_sha256: String,
    pub full_sha256: String,
    pub stable_estimated_tokens: usize,
    pub dynamic_estimated_tokens: usize,
    pub modules: Vec<PromptModuleIdentity>,
}

pub fn estimate_tokens_from_str(s: &str) -> usize {
    s.len().div_ceil(4)
}

pub fn hash_text(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

impl PromptManifest {
    pub fn from_parts(
        profile: &str,
        profile_version: u32,
        modules: &[PromptModule],
        stable_text: &str,
        full_text: &str,
    ) -> Self {
        let mut module_identities = Vec::with_capacity(modules.len());

        for m in modules {
            let tokens = estimate_tokens_from_str(&m.body);
            let sha = hash_text(&m.body);
            module_identities.push(PromptModuleIdentity {
                id: m.id.clone(),
                version: m.version,
                cache_class: m.cache_class,
                sha256: sha,
                estimated_tokens: tokens,
            });
        }

        Self {
            profile: profile.to_string(),
            profile_version,
            stable_sha256: hash_text(stable_text),
            full_sha256: hash_text(full_text),
            stable_estimated_tokens: estimate_tokens_from_str(stable_text),
            dynamic_estimated_tokens: estimate_tokens_from_str(
                if full_text.len() > stable_text.len() {
                    &full_text[stable_text.len()..]
                } else {
                    ""
                },
            ),
            modules: module_identities,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::PermissionMode;
    use crate::prompt::composer::{compose_default_prompt, compose_modules, PromptContext};

    fn fixture_module(
        id: &str,
        version: u32,
        cache_class: PromptCacheClass,
        body: &str,
    ) -> PromptModule {
        PromptModule {
            id: id.to_string(),
            version,
            cache_class,
            body: body.to_string(),
        }
    }

    fn fixture_context() -> PromptContext<'static> {
        PromptContext {
            provider: "anthropic",
            model_id: "claude-3-5-sonnet",
            permission_mode: PermissionMode::Ask,
            plan_active: false,
        }
    }

    #[test]
    fn dynamic_suffix_does_not_change_stable_hash() {
        let stable = fixture_module("core", 1, PromptCacheClass::Stable, "same");
        let d1 = fixture_module("runtime", 1, PromptCacheClass::Dynamic, "mode=ask");
        let d2 = fixture_module("runtime", 1, PromptCacheClass::Dynamic, "mode=read-only");

        let a = compose_modules(&[stable.clone(), d1]);
        let b = compose_modules(&[stable, d2]);

        assert_eq!(a.manifest.stable_sha256, b.manifest.stable_sha256);
        assert_ne!(a.manifest.full_sha256, b.manifest.full_sha256);
    }

    #[test]
    fn stable_prompt_stays_within_budget() {
        let prompt = compose_default_prompt(&fixture_context());
        assert!(prompt.manifest.stable_estimated_tokens <= 2_800);
    }
}
