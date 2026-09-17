use super::RepoFile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(super) const SCHEMA_VERSION: u32 = 3;
pub(super) const PARSER_VERSION: &str = "ts-0.23.2-js-0.23.1-extract-2";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIndex {
    pub schema_version: u32,
    pub parser_version: String,
    pub root: String,
    pub config_identity: String,
    pub files: BTreeMap<String, Arc<RepoFile>>,
    pub metadata: Vec<String>,
    pub(super) aliases: Vec<super::modules::ModuleAlias>,
    pub text_files: Vec<String>,
    pub updated_ms: u64,
    #[serde(skip)]
    pub reparsed: usize,
    #[serde(skip)]
    pub bytes_read: usize,
    #[serde(skip)]
    pub files_read: usize,
    #[serde(skip)]
    pub refresh_mode: String,
    pub warnings: Vec<String>,
}

impl RepoIndex {
    /// The same resolved import edges used by native repository queries.
    pub fn dependencies(&self) -> Vec<super::Dependency> {
        super::graph::dependencies(self)
    }

    /// Conservative path hints for an unresolved import (for deleted-source impact).
    pub fn missing_dependency_candidates(&self, from: &str, specifier: &str) -> Vec<String> {
        super::graph::missing_candidates(self, from, specifier)
    }

    pub(super) fn empty(root: String, config_identity: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            parser_version: PARSER_VERSION.into(),
            root,
            config_identity,
            files: BTreeMap::new(),
            metadata: Vec::new(),
            aliases: Vec::new(),
            text_files: Vec::new(),
            updated_ms: 0,
            reparsed: 0,
            bytes_read: 0,
            files_read: 0,
            refresh_mode: "full".into(),
            warnings: Vec::new(),
        }
    }
}
