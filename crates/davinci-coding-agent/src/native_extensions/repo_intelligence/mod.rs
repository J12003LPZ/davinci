//! Cached structural source intelligence. AST facts are never semantic LSP facts.
mod graph;
mod index;
mod languages;
mod manager;
mod modules;
mod parser;
mod queries;
mod scanner;
mod storage;
mod symbols;
mod tools;
mod unified;
pub use tools::{is_repo_tool, tool_spec};
// This module is compiled separately by the CLI and library; these are library adapter contracts.
#[allow(unused_imports)]
pub use unified::{SemanticEvidence, SemanticLanguageProvider, SemanticOperation};

pub use index::RepoIndex;
pub use manager::{RepoIntelligence, RepoIntelligenceConfig};

pub use languages::LanguageAdapter;
pub use parser::parse_source;
pub use symbols::{Edge, Import, RepoFile, SourceRange, Symbol};
