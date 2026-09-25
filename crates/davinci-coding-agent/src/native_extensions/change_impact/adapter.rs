//! Adapter implementing RepoIntelligence SemanticLanguageProvider over LanguageIntelligence.

use crate::{
    native_extensions::{
        language_intelligence::LanguageIntelligence,
        repo_intelligence::{
            SemanticEvidence, SemanticLanguageProvider, SemanticOperation, SourceRange,
        },
    },
    semantic::SemanticClient,
};
use davinci_agent::semantic::SemanticRequestContext;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct LanguageIntelligenceAdapter {
    client: SemanticClient,
}

impl LanguageIntelligenceAdapter {
    pub fn new(language: LanguageIntelligence) -> Self {
        Self {
            client: SemanticClient::local(language),
        }
    }

    pub fn with_client(client: SemanticClient) -> Self {
        Self { client }
    }

    fn effective_client(&self) -> Result<SemanticClient, String> {
        if std::env::var_os("PI_GRAPH_ROLE").is_some() {
            return davinci_agent::runtime::task_transport::TaskCoordinatorClient::from_env()
                .map(SemanticClient::parent)
                .ok_or_else(|| {
                    "parent semantic transport unavailable; worker-local language server is forbidden"
                        .to_string()
                });
        }
        Ok(self.client.clone())
    }
}

impl SemanticLanguageProvider for LanguageIntelligenceAdapter {
    fn query(
        &self,
        operation: SemanticOperation,
        path: &str,
        range: &SourceRange,
        limit: usize,
    ) -> Result<Vec<SemanticEvidence>, String> {
        let tool_name = match operation {
            SemanticOperation::References => "lsp_references",
            SemanticOperation::Definition => "lsp_definition",
            SemanticOperation::Implementations => "lsp_implementations",
            SemanticOperation::TypeDefinition => "lsp_type_definition",
            SemanticOperation::Diagnostics => "lsp_diagnostics",
        };

        let args = if matches!(operation, SemanticOperation::Diagnostics) {
            json!({"path": path, "limit": limit})
        } else {
            json!({
                "path": path,
                "line": range.start_line,
                "column": range.start_column,
                "limit": limit
            })
        };

        let res = self.effective_client()?.execute_tool(
            tool_name,
            &args,
            &SemanticRequestContext::default(),
        )?;

        if res.is_error {
            return Err(res
                .details
                .as_ref()
                .and_then(|details| details.pointer("/error/message"))
                .and_then(|message| message.as_str())
                .unwrap_or(&res.content)
                .to_string());
        }

        let details = res
            .details
            .ok_or_else(|| "semantic provider returned no structured details".to_string())?;

        let mut evidence = Vec::new();
        if let Some(items) = details.get("items").and_then(|i| i.as_array()) {
            for item in items {
                let item_path = item["path"].as_str().unwrap_or(path).to_string();
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
