//! Native, read-only language intelligence. No model-facing generic LSP method.

mod client_requests;
mod config;
mod diagnostics;
mod documents;
mod identity;
mod metadata;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod python_integration_tests;
#[cfg(test)]
mod rust_integration_tests;
#[cfg(test)]
mod test_support;
mod manager;
mod normalize;
mod protocol;
mod servers;
mod session;
mod tools;
mod transport;

pub use config::{
    LanguageIntelligenceConfig, PythonBackend, PythonConfig, RustBackend, RustConfig,
    TypeScriptBackend, TypeScriptConfig,
};
pub use identity::LanguageFamily;
pub use manager::LanguageIntelligence;
pub use protocol::RequestBudget;
pub use tools::{tool_spec, TOOL_NAMES};
