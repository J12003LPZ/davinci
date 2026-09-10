//! Versioned prompt engineering subsystem.

pub mod coding;
pub mod collaboration;
pub mod composer;
pub mod core;
pub mod manifest;
pub mod runtime_state;
pub mod tool_strategy;
pub mod verification;
pub mod version;

pub use coding::{
    coding_change_quality_module, coding_exploration_module, coding_scope_discipline_module,
};
pub use collaboration::collaboration_user_intent_module;
pub use composer::{
    compose_default_prompt, compose_legacy_default, compose_modules, ComposedPrompt,
    PromptCacheClass, PromptContext, PromptModule,
};
pub use core::{core_autonomy_module, core_identity_module};
pub use manifest::{PromptManifest, PromptModuleIdentity};
pub use runtime_state::{runtime_state_module, runtime_state_text, RuntimePromptState};
pub use tool_strategy::tool_strategy_module;
pub use verification::verification_completion_module;
