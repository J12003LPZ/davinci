//! Native, read-only language intelligence. No model-facing generic LSP method.

mod client_requests;
mod config;
mod documents;
mod identity;
mod metadata;
#[cfg(test)]
mod integration_tests;
mod manager;
mod normalize;
mod protocol;
mod servers;
mod session;
mod tools;
mod transport;

pub use config::{LanguageIntelligenceConfig, PythonBackend, PythonConfig, RustBackend, RustConfig, ServerOverride, TypeScriptConfig};
pub use identity::{LanguageFamily, RequestBudget, ResolvedProject, ServerInvocation, SessionKey};
pub use manager::LanguageIntelligence;
pub use tools::{tool_spec, TOOL_NAMES};
