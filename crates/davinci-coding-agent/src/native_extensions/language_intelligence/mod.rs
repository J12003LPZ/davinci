//! Native, read-only language intelligence. No model-facing generic LSP method.

mod documents;
#[cfg(test)]
mod integration_tests;
mod manager;
mod normalize;
mod protocol;
mod servers;
mod session;
mod tools;
mod transport;

pub use manager::{LanguageIntelligence, LanguageIntelligenceConfig};
pub use tools::{tool_spec, TOOL_NAMES};
