use serde::{Deserialize, Serialize};

/// Add adapters here without changing normalized records or tool semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageAdapter {
    TypeScript,
    JavaScript,
}

impl LanguageAdapter {
    pub fn for_path(path: &str) -> Option<Self> {
        match path.rsplit('.').next()? {
            "ts" | "tsx" | "mts" | "cts" => Some(Self::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            _ => None,
        }
    }

    pub(super) fn grammar(self, path: &str) -> tree_sitter::Language {
        match self {
            Self::TypeScript if path.ends_with(".tsx") => {
                tree_sitter_typescript::LANGUAGE_TSX.into()
            }
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        }
    }
}
