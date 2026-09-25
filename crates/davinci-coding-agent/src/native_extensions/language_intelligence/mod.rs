//! Native, read-only language intelligence. No model-facing generic LSP method.

mod config;
mod documents;
mod identity;
#[cfg(test)]
mod integration_tests;
mod manager;
mod normalize;
mod protocol;
mod servers;
mod session;
mod tools;
mod transport;

pub use config::{LanguageIntelligenceConfig, PythonBackend, PythonConfig, RustBackend, RustConfig, TypeScriptBackend, TypeScriptConfig};
pub use identity::LanguageFamily;
pub use manager::LanguageIntelligence;
pub use tools::{tool_spec, TOOL_NAMES};
