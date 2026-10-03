//! Native, read-only language intelligence. No model-facing generic LSP method.

mod client_requests;
mod config;
mod diagnostics;
mod documents;
mod identity;
#[cfg(test)]
mod integration_tests;
mod manager;
mod metadata;
mod normalize;
mod protocol;
#[cfg(test)]
mod python_integration_tests;
#[cfg(test)]
mod rust_integration_tests;
mod servers;
mod session;
#[cfg(test)]
mod test_support;
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
