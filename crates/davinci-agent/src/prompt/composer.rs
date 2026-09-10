//! Deterministic prompt composer and module definitions.

use crate::permission::PermissionMode;
use crate::prompt::{core, tool_strategy, version};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptCacheClass {
    Stable,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptModule {
    pub id: String,
    pub version: u32,
    pub cache_class: PromptCacheClass,
    pub body: String,
}

pub struct PromptContext<'a> {
    pub provider: &'a str,
    pub model_id: &'a str,
    pub permission_mode: PermissionMode,
    pub plan_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedPrompt {
    pub text: String,
    pub stable_text: String,
    pub dynamic_text: String,
    pub manifest: crate::prompt::manifest::PromptManifest,
}

fn join_modules(modules: &[&PromptModule]) -> String {
    modules
        .iter()
        .map(|m| m.body.trim())
        .filter(|body| !body.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn compose_modules(modules: &[PromptModule]) -> ComposedPrompt {
    let stable = modules
        .iter()
        .filter(|m| m.cache_class == PromptCacheClass::Stable)
        .collect::<Vec<_>>();

    let dynamic = modules
        .iter()
        .filter(|m| m.cache_class == PromptCacheClass::Dynamic)
        .collect::<Vec<_>>();

    let stable_text = join_modules(&stable);
    let dynamic_text = join_modules(&dynamic);

    let text = match (stable_text.is_empty(), dynamic_text.is_empty()) {
        (false, false) => format!("{stable_text}\n\n{dynamic_text}"),
        (false, true) => stable_text.clone(),
        (true, false) => dynamic_text.clone(),
        (true, true) => String::new(),
    };

    let manifest = crate::prompt::manifest::PromptManifest::from_parts(
        "default",
        version::STABLE_PROMPT_VERSION,
        modules,
        &stable_text,
        &text,
    );

    ComposedPrompt {
        text,
        stable_text,
        dynamic_text,
        manifest,
    }
}

pub fn legacy_default_module() -> PromptModule {
    let body = [
        core::LEGACY_IDENTITY,
        core::LEGACY_TODO,
        core::LEGACY_BACKGROUND_JOBS,
        core::LEGACY_WEB_NOTEBOOK,
        tool_strategy::TOOL_USE_STRATEGY,
    ]
    .join("\n");

    PromptModule {
        id: "legacy.default_v1".to_string(),
        version: version::LEGACY_PROMPT_VERSION,
        cache_class: PromptCacheClass::Stable,
        body,
    }
}

pub fn compose_legacy_default() -> ComposedPrompt {
    compose_modules(&[legacy_default_module()])
}

pub fn stable_v2_modules() -> Vec<PromptModule> {
    vec![
        core::core_identity_module(),
        core::core_autonomy_module(),
        crate::prompt::coding::coding_exploration_module(),
        crate::prompt::coding::coding_scope_discipline_module(),
        crate::prompt::coding::coding_change_quality_module(),
        crate::prompt::collaboration::collaboration_user_intent_module(),
        crate::prompt::verification::verification_completion_module(),
    ]
}

pub fn compose_profile_prompt(
    profile: version::PromptProfile,
    ctx: &PromptContext<'_>,
) -> ComposedPrompt {
    match profile {
        version::PromptProfile::LegacyV1 => {
            let mut composed = compose_legacy_default();
            composed.manifest.profile = profile.id().to_string();
            composed.manifest.profile_version = profile.version();
            composed
        }
        version::PromptProfile::Stable | version::PromptProfile::Preview => {
            let mut modules = stable_v2_modules();
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
            composed.manifest.profile = profile.id().to_string();
            composed.manifest.profile_version = profile.version();
            composed
        }
    }
}

pub fn compose_default_prompt(ctx: &PromptContext<'_>) -> ComposedPrompt {
    compose_profile_prompt(version::PromptProfile::Stable, ctx)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptMutation {
    RemoveModule { id: String },
    ReplaceModuleBody { id: String, body: String },
    DowngradeModuleVersion { id: String, version: u32 },
}

pub fn compose_with_mutations(
    ctx: &PromptContext<'_>,
    mutations: &[PromptMutation],
) -> ComposedPrompt {
    let mut modules = stable_v2_modules();
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

    for mutation in mutations {
        match mutation {
            PromptMutation::RemoveModule { id } => {
                modules.retain(|m| &m.id != id);
            }
            PromptMutation::ReplaceModuleBody { id, body } => {
                for m in &mut modules {
                    if &m.id == id {
                        m.body = body.clone();
                    }
                }
            }
            PromptMutation::DowngradeModuleVersion { id, version } => {
                for m in &mut modules {
                    if &m.id == id {
                        m.version = *version;
                    }
                }
            }
        }
    }

    compose_modules(&modules)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn compose_for_test(modules: Vec<PromptModule>) -> ComposedPrompt {
        compose_modules(&modules)
    }

    #[test]
    fn stable_modules_are_emitted_before_dynamic_modules() {
        let composed = compose_for_test(vec![
            fixture_module("runtime.mode", 1, PromptCacheClass::Dynamic, "DYNAMIC"),
            fixture_module("core.identity", 1, PromptCacheClass::Stable, "STABLE"),
        ]);

        assert_eq!(composed.text, "STABLE\n\nDYNAMIC");
        assert_eq!(composed.stable_text, "STABLE");
        assert_eq!(composed.dynamic_text, "DYNAMIC");
    }

    #[test]
    fn module_order_is_deterministic() {
        let a = compose_default_prompt(&fixture_context());
        let b = compose_default_prompt(&fixture_context());
        assert_eq!(a.text, b.text);
        assert_eq!(a.stable_text, b.stable_text);
        assert_eq!(a.dynamic_text, b.dynamic_text);
    }
}
