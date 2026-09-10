//! Versioned prompt engineering subsystem.

pub mod composer;
pub mod core;
pub mod tool_strategy;
pub mod version;

pub use composer::{
    compose_default_prompt, compose_legacy_default, compose_modules, ComposedPrompt,
    PromptCacheClass, PromptContext, PromptModule,
};
