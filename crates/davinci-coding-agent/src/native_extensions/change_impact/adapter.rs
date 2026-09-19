//! Adapter implementing RepoIntelligence SemanticLanguageProvider over LanguageIntelligence.

use crate::native_extensions::{
    language_intelligence::LanguageIntelligence,
    repo_intelligence::{
        SemanticEvidence, SemanticLanguageProvider, SemanticOperation, SourceRange,
    },
};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct LanguageIntelligenceAdapter {
    language: LanguageIntelligence,
}

impl LanguageIntelligenceAdapter {
    pub fn new(language: LanguageIntelligence) -> Self {
        Self { language }
    }
}

impl SemanticLanguageProvider for LanguageIntelligenceAdapter {
    fn query(
        &self,
        operation: SemanticOperation,
        path: &str,
        range: &SourceRange,
        _limit: usize,
    ) -> Result<Vec<SemanticEvidence>, String> {
        let tool_name = match operation {
            SemanticOperation::References => "lsp_references",
            SemanticOperation::Definition => "lsp_definition",
            SemanticOperation::Implementations => "lsp_implementations",
            SemanticOperation::TypeDefinition => "lsp_type_definition",
            SemanticOperation::Diagnostics => "lsp_diagnostics",
        };

        let args = json!({
            "path": path,
            "line": range.start_line,
            "column": range.start_column
        });

        let res = self
            .language
            .execute(tool_name, &args)
            .map_err(|e| e.to_string())?;

        if res.is_error {
            return Ok(Vec::new());
        }

        let Some(details) = res.details else {
            return Ok(Vec::new());
        };

        let mut evidence = Vec::new();
        if let Some(items) = details.get("items").and_then(|i| i.as_array()) {
            for item in items {
                let item_path = item["path"].as_str().unwrap_or("").to_string();
                let start_line = item["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
                let start_col = item["range"]["start"]["column"].as_u64().unwrap_or(0) as usize;
                let end_line = item["range"]["end"]["line"].as_u64().unwrap_or(0) as usize;
                let end_col = item["range"]["end"]["column"].as_u64().unwrap_or(0) as usize;

                evidence.push(SemanticEvidence {
                    path: item_path.clone(),
                    range: SourceRange {
                        start_line,
                        start_column: start_col,
                        end_line,
                        end_column: end_col,
                    },
                    description: format!("LSP {} at {}:{}", tool_name, item_path, start_line),
                });
            }
        }

        Ok(evidence)
    }
}
