//! Tool definitions for P9 Change Impact Engine.

use serde::Deserialize;
use serde_json::json;

pub const TOOL_NAMES: &[&str] = &["impact_analyze"];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpactAnalyzeArgs {
    #[serde(default)]
    pub files: Option<Vec<String>>,
    #[serde(default)]
    pub symbols: Option<Vec<String>>,
    #[serde(default, alias = "transaction_id")]
    pub transaction_id: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub fn tool_spec(name: &str) -> Option<davinci_ai::ToolSpec> {
    match name {
        "impact_analyze" => Some(davinci_ai::ToolSpec {
            name: name.to_string(),
            description: "Analyze the likely blast radius of proposed or applied code edits across AST references, LSP semantics, tests, packages, build targets, public APIs, and browser flows.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of workspace-relative file paths to analyze"
                    },
                    "symbols": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of symbol names or identifiers to analyze"
                    },
                    "transactionId": {
                        "type": "string",
                        "description": "Optional ID of a recorded P4 transaction to analyze"
                    },
                    "scope": {
                        "type": "string",
                        "description": "Optional workspace subdirectory scope"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Maximum number of items per section"
                    }
                }
            }),
            constrained_sampling: None,
        }),
        _ => None,
    }
}
