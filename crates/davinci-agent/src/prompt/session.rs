//! Session-level prompt state distinguishing built-in prompt profiles from custom replacement prompts.

use crate::prompt::composer::{compose_profile_prompt, ComposedPrompt, PromptContext};
use crate::prompt::manifest::{estimate_tokens_from_str, hash_text, PromptManifest};
use crate::prompt::version::PromptProfile;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptSource {
    Builtin { profile: PromptProfile },
    CustomReplacement { text: String },
}

/// Persisted session record storing prompt identity across session resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSessionRecord {
    pub profile: String,
    pub profile_version: u32,
    pub stable_sha256: String,
    pub candidate_id: Option<String>,
}

impl PromptSessionRecord {
    pub fn new(
        profile: impl Into<String>,
        profile_version: u32,
        stable_sha256: impl Into<String>,
        candidate_id: Option<String>,
    ) -> Self {
        Self {
            profile: profile.into(),
            profile_version,
            stable_sha256: stable_sha256.into(),
            candidate_id,
        }
    }

    pub fn from_session_state(state: &PromptSessionState) -> Option<Self> {
        let manifest = state.last_manifest.as_ref()?;
        Some(Self {
            profile: manifest.profile.clone(),
            profile_version: manifest.profile_version,
            stable_sha256: manifest.stable_sha256.clone(),
            candidate_id: state.candidate_id.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSessionState {
    pub source: PromptSource,
    pub append_text: Vec<String>,
    pub last_manifest: Option<PromptManifest>,
    pub stable_bundle_hash: Option<String>,
    pub candidate_id: Option<String>,
    pub transition_diagnostic: Option<String>,
}

impl PromptSessionState {
    pub fn builtin(profile: PromptProfile) -> Self {
        Self {
            source: PromptSource::Builtin { profile },
            append_text: Vec::new(),
            last_manifest: None,
            stable_bundle_hash: None,
            candidate_id: None,
            transition_diagnostic: None,
        }
    }

    pub fn custom(text: impl Into<String>) -> Self {
        Self {
            source: PromptSource::CustomReplacement { text: text.into() },
            append_text: Vec::new(),
            last_manifest: None,
            stable_bundle_hash: None,
            candidate_id: None,
            transition_diagnostic: None,
        }
    }

    pub fn with_candidate_id(mut self, candidate_id: impl Into<String>) -> Self {
        self.candidate_id = Some(candidate_id.into());
        self
    }

    pub fn with_append(mut self, text: impl Into<String>) -> Self {
        self.append_text.push(text.into());
        self
    }

    pub fn with_appends<I, S>(mut self, appends: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for append in appends {
            self.append_text.push(append.into());
        }
        self
    }

    pub fn append(&mut self, text: impl Into<String>) {
        self.append_text.push(text.into());
    }

    pub fn is_builtin(&self) -> bool {
        matches!(self.source, PromptSource::Builtin { .. })
    }

    pub fn is_custom(&self) -> bool {
        matches!(self.source, PromptSource::CustomReplacement { .. })
    }

    pub fn profile(&self) -> Option<PromptProfile> {
        match self.source {
            PromptSource::Builtin { profile } => Some(profile),
            PromptSource::CustomReplacement { .. } => None,
        }
    }

    /// Compose the full system prompt for the given context.
    ///
    /// Preserves strict ordering:
    /// - For built-in sessions: built-in stable prefix → dynamic runtime/capability suffix → user append.
    /// - For custom replacements: custom text → user append (with zero built-in modules in manifest).
    pub fn compose(&self, ctx: &PromptContext<'_>) -> ComposedPrompt {
        compose_session_prompt(self, ctx)
    }

    /// Compose the prompt and record the resulting manifest and stable bundle hash into session state.
    pub fn render_and_record(&mut self, ctx: &PromptContext<'_>) -> ComposedPrompt {
        let composed = self.compose(ctx);
        self.last_manifest = Some(composed.manifest.clone());
        self.stable_bundle_hash = Some(composed.manifest.stable_sha256.clone());
        composed
    }
}

/// Resolves the prompt session state upon resuming an existing session according to:
/// 1. Explicit invocation profile override takes precedence if given.
/// 2. Explicit legacy-v1 pin is preserved if persisted record had profile "legacy-v1".
/// 3. Otherwise, applies the current-Stable-on-resume policy (or preview if recorded) and
///    generates a transition diagnostic if the stable prompt hash changed across resumes.
pub fn resolve_resume_prompt_session(
    record: Option<&PromptSessionRecord>,
    profile_override: Option<PromptProfile>,
) -> (PromptSessionState, Option<String>) {
    if let Some(profile) = profile_override {
        let mut state = PromptSessionState::builtin(profile);
        let mut diag = None;
        if let Some(rec) = record {
            let current_hash = profile.bundle().stable_sha256();
            if rec.stable_sha256 != current_hash {
                let msg = format!(
                    "Prompt hash transition on resume: {} hash changed from {} to {}",
                    profile.id(),
                    rec.stable_sha256,
                    current_hash
                );
                state.transition_diagnostic = Some(msg.clone());
                diag = Some(msg);
            }
        }
        return (state, diag);
    }

    let Some(rec) = record else {
        return (PromptSessionState::builtin(PromptProfile::Stable), None);
    };

    if rec.profile == "legacy-v1" {
        let mut state = PromptSessionState::builtin(PromptProfile::LegacyV1);
        let current_hash = PromptProfile::LegacyV1.bundle().stable_sha256();
        let mut diag = None;
        if rec.stable_sha256 != current_hash {
            let msg = format!(
                "Prompt hash transition on resume: legacy-v1 hash changed from {} to {}",
                rec.stable_sha256, current_hash
            );
            state.transition_diagnostic = Some(msg.clone());
            diag = Some(msg);
        }
        return (state, diag);
    }

    if rec.profile == "preview" {
        let mut state = PromptSessionState::builtin(PromptProfile::Preview);
        state.candidate_id = rec.candidate_id.clone();
        let current_hash = PromptProfile::Preview.bundle().stable_sha256();
        let mut diag = None;
        if rec.stable_sha256 != current_hash {
            let msg = format!(
                "Prompt hash transition on resume: preview hash changed from {} to {}",
                rec.stable_sha256, current_hash
            );
            state.transition_diagnostic = Some(msg.clone());
            diag = Some(msg);
        }
        return (state, diag);
    }

    let mut state = PromptSessionState::builtin(PromptProfile::Stable);
    let current_hash = PromptProfile::Stable.bundle().stable_sha256();
    let mut diag = None;
    if rec.stable_sha256 != current_hash {
        let msg = format!(
            "Prompt hash transition on resume: stable hash changed from {} to {}",
            rec.stable_sha256, current_hash
        );
        state.transition_diagnostic = Some(msg.clone());
        diag = Some(msg);
    }
    (state, diag)
}

pub fn compose_session_prompt(
    session: &PromptSessionState,
    ctx: &PromptContext<'_>,
) -> ComposedPrompt {
    let appends = session
        .append_text
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    match &session.source {
        PromptSource::Builtin { profile } => {
            let base = compose_profile_prompt(*profile, ctx);
            if appends.is_empty() {
                base
            } else {
                let dynamic_text = if base.dynamic_text.is_empty() {
                    appends.clone()
                } else {
                    format!("{}\n\n{}", base.dynamic_text, appends)
                };

                let text = if base.stable_text.is_empty() {
                    dynamic_text.clone()
                } else {
                    format!("{}\n\n{}", base.stable_text, dynamic_text)
                };

                let mut manifest = base.manifest;
                manifest.full_sha256 = hash_text(&text);
                manifest.dynamic_estimated_tokens =
                    estimate_tokens_from_str(if text.len() > base.stable_text.len() {
                        &text[base.stable_text.len()..]
                    } else {
                        ""
                    });

                ComposedPrompt {
                    text,
                    stable_text: base.stable_text,
                    dynamic_text,
                    manifest,
                }
            }
        }
        PromptSource::CustomReplacement { text } => {
            let stable_text = text.trim().to_string();
            let dynamic_text = appends;
            let full_text = match (stable_text.is_empty(), dynamic_text.is_empty()) {
                (false, false) => format!("{stable_text}\n\n{dynamic_text}"),
                (false, true) => stable_text.clone(),
                (true, false) => dynamic_text.clone(),
                (true, true) => String::new(),
            };

            let manifest = PromptManifest {
                profile: "custom".to_string(),
                profile_version: 0,
                stable_sha256: hash_text(&stable_text),
                full_sha256: hash_text(&full_text),
                stable_estimated_tokens: estimate_tokens_from_str(&stable_text),
                dynamic_estimated_tokens: estimate_tokens_from_str(
                    if full_text.len() > stable_text.len() {
                        &full_text[stable_text.len()..]
                    } else {
                        ""
                    },
                ),
                modules: Vec::new(),
            };

            ComposedPrompt {
                text: full_text,
                stable_text,
                dynamic_text,
                manifest,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::PermissionMode;

    fn fixture_context() -> PromptContext<'static> {
        PromptContext {
            provider: "anthropic",
            model_id: "claude-3-5-sonnet",
            permission_mode: PermissionMode::Ask,
            plan_active: false,
        }
    }

    #[test]
    fn custom_replacement_receives_no_builtin_modules() {
        let state = PromptSessionState::custom("You are a specialized auditor.");
        let composed = state.compose(&fixture_context());

        assert_eq!(composed.text, "You are a specialized auditor.");
        assert_eq!(composed.stable_text, "You are a specialized auditor.");
        assert!(composed.dynamic_text.is_empty());
        assert_eq!(composed.manifest.profile, "custom");
        assert_eq!(composed.manifest.profile_version, 0);
        assert!(
            composed.manifest.modules.is_empty(),
            "custom replacement must receive zero built-in modules"
        );
    }

    #[test]
    fn freeze_append_order_builtin_prefix_dynamic_suffix_user_append() {
        let state = PromptSessionState::builtin(PromptProfile::Stable)
            .with_append("CUSTOM_APPEND_1")
            .with_append("CUSTOM_APPEND_2");

        let composed = state.compose(&fixture_context());

        // Builtin stable text must contain stable modules, but not dynamic runtime state or user appends
        assert!(
            composed.stable_text.contains("DaVinci"),
            "stable text must contain core identity"
        );
        assert!(!composed.stable_text.contains("CUSTOM_APPEND_1"));
        assert!(!composed.stable_text.contains("Permission mode: Ask."));

        // Dynamic text must contain dynamic runtime state followed by user appends
        assert!(composed.dynamic_text.contains("Permission mode: Ask."));
        assert!(composed.dynamic_text.contains("CUSTOM_APPEND_1"));
        assert!(composed.dynamic_text.contains("CUSTOM_APPEND_2"));

        // Ordering in full text: stable prefix < dynamic suffix < user appends
        let stable_pos = composed
            .text
            .find("DaVinci")
            .expect("missing stable prefix");
        let suffix_pos = composed
            .text
            .find("Permission mode: Ask.")
            .expect("missing dynamic suffix");
        let append1_pos = composed
            .text
            .find("CUSTOM_APPEND_1")
            .expect("missing append 1");
        let append2_pos = composed
            .text
            .find("CUSTOM_APPEND_2")
            .expect("missing append 2");

        assert!(
            stable_pos < suffix_pos,
            "built-in prefix must precede dynamic suffix"
        );
        assert!(
            suffix_pos < append1_pos,
            "dynamic suffix must precede user append 1"
        );
        assert!(
            append1_pos < append2_pos,
            "user append 1 must precede user append 2"
        );
    }

    #[test]
    fn stable_prefix_and_hash_remain_invariant_under_user_appends() {
        let clean = PromptSessionState::builtin(PromptProfile::Stable);
        let with_appends = PromptSessionState::builtin(PromptProfile::Stable)
            .with_append("EXTRA_USER_INSTRUCTION");

        let ctx = fixture_context();
        let comp_clean = clean.compose(&ctx);
        let comp_appended = with_appends.compose(&ctx);

        assert_eq!(comp_clean.stable_text, comp_appended.stable_text);
        assert_eq!(
            comp_clean.manifest.stable_sha256, comp_appended.manifest.stable_sha256,
            "stable prefix SHA256 must remain byte-identical regardless of user appends"
        );
        assert_ne!(
            comp_clean.manifest.full_sha256, comp_appended.manifest.full_sha256,
            "full prompt SHA256 must reflect user appends"
        );
    }

    #[test]
    fn custom_replacement_with_appends_preserves_custom_then_appends() {
        let state = PromptSessionState::custom("MY_CUSTOM_BASE")
            .with_append("APPEND_A")
            .with_append("APPEND_B");

        let composed = state.compose(&fixture_context());

        assert_eq!(composed.stable_text, "MY_CUSTOM_BASE");
        assert_eq!(composed.dynamic_text, "APPEND_A\n\nAPPEND_B");
        assert_eq!(composed.text, "MY_CUSTOM_BASE\n\nAPPEND_A\n\nAPPEND_B");
        assert!(composed.manifest.modules.is_empty());
    }

    #[test]
    fn render_and_record_stores_last_manifest_and_stable_hash() {
        let mut state = PromptSessionState::builtin(PromptProfile::Stable);
        assert!(state.last_manifest.is_none());
        assert!(state.stable_bundle_hash.is_none());

        let composed = state.render_and_record(&fixture_context());
        assert_eq!(state.last_manifest, Some(composed.manifest.clone()));
        assert_eq!(
            state.stable_bundle_hash.as_deref(),
            Some(composed.manifest.stable_sha256.as_str())
        );
    }

    #[test]
    fn resume_preserves_explicit_legacy_v1_pin() {
        let current_legacy_hash = PromptProfile::LegacyV1.bundle().stable_sha256();
        let record = PromptSessionRecord::new("legacy-v1", 1, &current_legacy_hash, None);

        let (resumed, diag) = resolve_resume_prompt_session(Some(&record), None);
        assert_eq!(resumed.profile(), Some(PromptProfile::LegacyV1));
        assert!(diag.is_none());
        assert!(resumed.transition_diagnostic.is_none());
    }

    #[test]
    fn resume_current_stable_policy_detects_hash_transition() {
        let old_fake_hash = "deadbeef00000000deadbeef00000000deadbeef00000000deadbeef00000000";
        let record = PromptSessionRecord::new("stable", 2, old_fake_hash, None);

        let (resumed, diag) = resolve_resume_prompt_session(Some(&record), None);
        assert_eq!(resumed.profile(), Some(PromptProfile::Stable));

        let current_hash = PromptProfile::Stable.bundle().stable_sha256();
        assert!(diag.is_some());
        let msg = diag.unwrap();
        assert!(msg.contains("Prompt hash transition on resume: stable hash changed from"));
        assert!(msg.contains(old_fake_hash));
        assert!(msg.contains(&current_hash));
        assert_eq!(resumed.transition_diagnostic.as_deref(), Some(msg.as_str()));
    }

    #[test]
    fn resume_preview_preserves_candidate_id() {
        let current_preview_hash = PromptProfile::Preview.bundle().stable_sha256();
        let record =
            PromptSessionRecord::new("preview", 3, &current_preview_hash, Some("cand-42".into()));

        let (resumed, diag) = resolve_resume_prompt_session(Some(&record), None);
        assert_eq!(resumed.profile(), Some(PromptProfile::Preview));
        assert_eq!(resumed.candidate_id.as_deref(), Some("cand-42"));
        assert!(diag.is_none());
    }

    #[test]
    fn resume_without_record_defaults_to_stable() {
        let (resumed, diag) = resolve_resume_prompt_session(None, None);
        assert_eq!(resumed.profile(), Some(PromptProfile::Stable));
        assert!(diag.is_none());
    }
}
