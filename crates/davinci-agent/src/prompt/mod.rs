//! Versioned prompt engineering subsystem.

pub mod composer;
pub mod core;
pub mod manifest;
pub mod runtime_state;
pub mod tool_strategy;
pub mod version;

pub use composer::{
    compose_default_prompt, compose_legacy_default, compose_modules, ComposedPrompt,
    PromptCacheClass, PromptContext, PromptModule,
};
pub use manifest::{PromptManifest, PromptModuleIdentity};
pub use runtime_state::{runtime_state_module, runtime_state_text, RuntimePromptState};
