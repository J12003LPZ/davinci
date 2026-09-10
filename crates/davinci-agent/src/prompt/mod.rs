//! Versioned prompt engineering subsystem.

pub mod capabilities;
pub mod coding;
pub mod collaboration;
pub mod composer;
pub mod core;
pub mod manifest;
pub mod provider;
pub mod runtime_state;
pub mod tool_strategy;
pub mod verification;
pub mod version;

pub use capabilities::{
    capability_module, detect_native_capabilities, CapabilityDecision, NativeBehaviorCapability,
    CAPABILITY_POLICY_MAX_TOKENS,
};
pub use coding::{
    coding_change_quality_module, coding_exploration_module, coding_scope_discipline_module,
};
pub use collaboration::collaboration_user_intent_module;
pub use composer::{
    compose_default_prompt, compose_legacy_default, compose_modules, compose_with_mutations,
    ComposedPrompt, PromptCacheClass, PromptContext, PromptModule, PromptMutation,
};
pub use core::{core_autonomy_module, core_identity_module};
pub use manifest::{PromptManifest, PromptModuleIdentity};
pub use provider::{
    prompt_model_family, provider_adapter, PromptModelFamily, PROVIDER_ADAPTER_MAX_TOKENS,
};
pub use runtime_state::{runtime_state_module, runtime_state_text, RuntimePromptState};
pub use tool_strategy::tool_strategy_module;
pub use verification::verification_completion_module;
