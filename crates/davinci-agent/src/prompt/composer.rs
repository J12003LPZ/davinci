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

pub fn compose_default_prompt(_ctx: &PromptContext<'_>) -> ComposedPrompt {
    compose_legacy_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_module(id: &str, version: u32, cache_class: PromptCacheClass, body: &str) -> PromptModule {
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
