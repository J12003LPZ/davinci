use super::LanguageAdapter;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    /// One-based lines; zero-based UTF-8 byte columns, end exclusive.
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub qualified_name: String,
    pub range: SourceRange,
    pub exported: bool,
    pub parent: Option<String>,
    pub signature: String,
    pub parameters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub specifier: String,
    pub bindings: BTreeMap<String, String>,
    pub reexport: bool,
    pub dynamic: bool,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub source: String,
    /// Unresolved spelling until the graph can prove a structural target.
    pub target: String,
    pub kind: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoFile {
    pub path: String,
    pub language: LanguageAdapter,
    pub content_hash: String,
    pub size: usize,
    pub module_identity: String,
    pub imports: Vec<Import>,
    pub exports: Vec<String>,
    pub export_bindings: BTreeMap<String, String>,
    pub symbols: Vec<Symbol>,
    pub edges: Vec<Edge>,
    pub parse_status: String,
    pub unresolved: Vec<String>,
}

pub(super) fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
