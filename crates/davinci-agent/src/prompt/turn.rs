//! Per-turn prompt preparation and capability composition.

use crate::prompt::capabilities::{capability_module, CapabilityDecision};
use crate::prompt::composer::{compose_modules, ComposedPrompt, PromptContext};
use crate::prompt::manifest::{estimate_tokens_from_str, hash_text};
use crate::prompt::runtime_state::{runtime_state_module, RuntimePromptState};
use crate::prompt::session::PromptSessionState;
use serde::{Deserialize, Serialize};

/// The result of preparing the prompt for a real user turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedTurnPrompt {
    pub composed: ComposedPrompt,
    pub capabilities: CapabilityDecision,
    pub runtime_state: RuntimePromptState,
}

/// Compose a full turn prompt for a built-in session with active capabilities and runtime state.
pub fn compose_turn_prompt(
    session: &PromptSessionState,
    ctx: &PromptContext<'_>,
    capabilities: &CapabilityDecision,
    runtime_state: &RuntimePromptState,
) -> Result<ComposedPrompt, String> {
    let profile = session.profile().ok_or_else(|| {
        "Cannot compose turn prompt: session is not a builtin profile".to_string()
    })?;

    let bundle = profile.bundle();
    let mut modules = bundle.modules.clone();

    // 1. Dynamic provider adapter
    let family = crate::prompt::provider::prompt_model_family(ctx.provider, ctx.model_id);
    if let Some(adapter) = crate::prompt::provider::provider_adapter(family) {
        modules.push(adapter);
    }

    // 2. Dynamic runtime state
    modules.push(runtime_state_module(runtime_state));

    // 3. Dynamic active capabilities
    for cap in &capabilities.capabilities {
        modules.push(capability_module(*cap));
    }

    let mut composed = compose_modules(&modules);
    composed.manifest.profile = bundle.id.to_string();
    composed.manifest.profile_version = bundle.version;

    // 4. Dynamic user appends
    let appends = session
        .append_text
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    if !appends.is_empty() {
        let dynamic_text = if composed.dynamic_text.is_empty() {
            appends
        } else {
            format!("{}\n\n{}", composed.dynamic_text, appends)
        };

        let text = if composed.stable_text.is_empty() {
            dynamic_text.clone()
        } else {
            format!("{}\n\n{}", composed.stable_text, dynamic_text)
        };

        composed.manifest.full_sha256 = hash_text(&text);
        composed.manifest.dynamic_estimated_tokens =
            estimate_tokens_from_str(if text.len() > composed.stable_text.len() {
                &text[composed.stable_text.len()..]
            } else {
                ""
            });

        composed.text = text;
        composed.dynamic_text = dynamic_text;
    }

    Ok(composed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::capabilities::NativeBehaviorCapability;
    use crate::prompt::version::PromptProfile;
    use crate::PermissionMode;

    #[test]
    fn compose_turn_prompt_includes_active_capabilities_in_dynamic_suffix() {
        let session = PromptSessionState::builtin(PromptProfile::Stable);
        let ctx = PromptContext {
            provider: "google",
            model_id: "gemini-2.5-pro",
            permission_mode: PermissionMode::Edits,
            plan_active: false,
        };
        let capabilities = CapabilityDecision {
            capabilities: vec![NativeBehaviorCapability::FrontendDesign],
            reasons: vec!["frontend-design triggered".to_string()],
            evidence: Vec::new(),
        };
        let state = RuntimePromptState {
            permission_mode: PermissionMode::Edits,
            plan_revision: None,
            plan_approved: false,
            active_contract: false,
            visual_verification_available: false,
        };

        let composed = compose_turn_prompt(&session, &ctx, &capabilities, &state).unwrap();

        // 1. Stable prefix and hash MUST remain invariant
        assert_eq!(
            composed.stable_text,
            PromptProfile::Stable.bundle().stable_text()
        );
        assert_eq!(
            composed.manifest.stable_sha256,
            PromptProfile::Stable.bundle().stable_sha256()
        );

        // 2. Dynamic text contains the capability policy
        assert!(
            composed.dynamic_text.contains("frontend_design_policy"),
            "Dynamic text should include Frontend Design policy"
        );

        // 3. Manifest contains capability module
        assert!(composed
            .manifest
            .modules
            .iter()
            .any(|m| m.id == "capability.frontend-design"));
    }

    #[test]
    fn compose_turn_prompt_refuses_custom_session() {
        let session = PromptSessionState::custom("Custom replacement prompt");
        let ctx = PromptContext {
            provider: "google",
            model_id: "gemini-2.5-pro",
            permission_mode: PermissionMode::Edits,
            plan_active: false,
        };
        let capabilities = CapabilityDecision {
            capabilities: vec![NativeBehaviorCapability::FrontendDesign],
            reasons: Vec::new(),
            evidence: Vec::new(),
        };
        let state = RuntimePromptState {
            permission_mode: PermissionMode::Edits,
            plan_revision: None,
            plan_approved: false,
            active_contract: false,
            visual_verification_available: false,
        };

        let result = compose_turn_prompt(&session, &ctx, &capabilities, &state);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("session is not a builtin profile"));
    }

    #[test]
    fn compose_turn_prompt_preserves_user_appends_after_capabilities() {
        let session = PromptSessionState::builtin(PromptProfile::Stable)
            .with_append("USER_APPEND_SECTION_CUSTOM_INSTRUCTIONS");
        let ctx = PromptContext {
            provider: "google",
            model_id: "gemini-2.5-pro",
            permission_mode: PermissionMode::Edits,
            plan_active: false,
        };
        let capabilities = CapabilityDecision {
            capabilities: vec![NativeBehaviorCapability::Debugging],
            reasons: Vec::new(),
            evidence: Vec::new(),
        };
        let state = RuntimePromptState {
            permission_mode: PermissionMode::Edits,
            plan_revision: None,
            plan_approved: false,
            active_contract: false,
            visual_verification_available: false,
        };

        let composed = compose_turn_prompt(&session, &ctx, &capabilities, &state).unwrap();
        let debug_pos = composed.text.find("debugging_policy").unwrap();
        let append_pos = composed
            .text
            .find("USER_APPEND_SECTION_CUSTOM_INSTRUCTIONS")
            .unwrap();
        assert!(
            append_pos > debug_pos,
            "User appends must appear after capability modules"
        );
    }
}
