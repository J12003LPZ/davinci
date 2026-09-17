//! Exact read-only tool contracts; server methods never come from tool input.
use super::protocol::{IntelligenceError, Result};
use serde::Deserialize;
use serde_json::{json, Value};

pub const TOOL_NAMES: &[&str] = &[
    "lsp_definition",
    "lsp_references",
    "lsp_hover",
    "lsp_document_symbols",
    "lsp_workspace_symbols",
    "lsp_implementations",
    "lsp_type_definition",
    "lsp_diagnostics",
];

pub(super) fn operation(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "lsp_definition" => ("textDocument/definition", "definitionProvider"),
        "lsp_references" => ("textDocument/references", "referencesProvider"),
        "lsp_hover" => ("textDocument/hover", "hoverProvider"),
        "lsp_document_symbols" => ("textDocument/documentSymbol", "documentSymbolProvider"),
        "lsp_workspace_symbols" => ("workspace/symbol", "workspaceSymbolProvider"),
        "lsp_implementations" => ("textDocument/implementation", "implementationProvider"),
        "lsp_type_definition" => ("textDocument/typeDefinition", "typeDefinitionProvider"),
        "lsp_diagnostics" => ("textDocument/diagnostic", "diagnosticProvider"),
        _ => return None,
    })
}

pub fn tool_spec(name: &str) -> Option<davinci_ai::ToolSpec> {
    operation(name)?;
    let mut properties = json!({"path":{"type":"string","minLength":1,"maxLength":4096,"description":"Workspace-relative source path. Optional source anchor for monorepo workspace symbols."}});
    let mut required = vec!["path"];
    let description = match name {
        "lsp_workspace_symbols" => {
            required = vec!["query"];
            properties["query"] = json!({"type":"string","maxLength":256});
            "Find TypeScript/JavaScript workspace symbols. Use path to select a monorepo project. Read-only and advisory."
        }
        "lsp_document_symbols" => {
            "List TypeScript/JavaScript document symbols. Read-only and advisory."
        }
        "lsp_diagnostics" => {
            properties["severity"] =
                json!({"type":"string","enum":["all","error","warning","information","hint"]});
            "Inspect TypeScript/JavaScript diagnostics for current disk contents. Advisory; never substitutes for compiler, lint or tests."
        }
        _ => {
            required.extend(["line", "column"]);
            properties["line"] = json!({"type":"integer","minimum":1,"maximum":u32::MAX,"description":"1-based line"});
            properties["column"] = json!({"type":"integer","minimum":1,"maximum":u32::MAX,"description":"1-based UTF-16 column"});
            match name {
                "lsp_definition" => "Find TypeScript/JavaScript symbol definitions. Read-only and advisory.",
                "lsp_references" => "Find TypeScript/JavaScript symbol references. Read-only and advisory.",
                "lsp_hover" => "Inspect a TypeScript/JavaScript type signature and short documentation. Read-only and advisory.",
                "lsp_implementations" => "Find TypeScript/JavaScript implementations. Read-only and advisory.",
                _ => "Find TypeScript/JavaScript type definitions. Read-only and advisory.",
            }
        }
    };
    if name != "lsp_hover" {
        properties["limit"] = json!({"type":"integer","minimum":1,"maximum":200});
    }
    if name == "lsp_references" {
        properties["includeDeclaration"] = json!({"type":"boolean","default":true});
    }
    Some(davinci_ai::ToolSpec {
        name: name.into(),
        description: description.into(),
        parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
        constrained_sampling: None,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Arguments {
    pub path: Option<String>,
    pub line: Option<u64>,
    pub column: Option<u64>,
    pub query: Option<String>,
    pub include_declaration: Option<bool>,
    pub limit: Option<usize>,
    pub severity: Option<String>,
}

impl Arguments {
    pub fn parse(name: &str, args: &Value) -> Result<Self> {
        let invalid = || {
            IntelligenceError::new(
                "invalid_arguments",
                "Arguments must match the semantic tool schema (1-based UTF-16 positions)",
            )
        };
        let schema = tool_spec(name).ok_or_else(invalid)?.parameters;
        let object = args.as_object().ok_or_else(invalid)?;
        if object
            .keys()
            .any(|key| schema["properties"].get(key).is_none())
            || schema["required"]
                .as_array()
                .expect("internal schema")
                .iter()
                .any(|key| !object.contains_key(key.as_str().expect("internal key")))
        {
            return Err(invalid());
        }
        let parsed: Self = serde_json::from_value(args.clone()).map_err(|_| invalid())?;
        if parsed
            .path
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > 4096 || v.contains('\0'))
            || parsed
                .query
                .as_ref()
                .is_some_and(|v| v.len() > 256 || v.contains('\0'))
            || parsed.limit.is_some_and(|v| !(1..=200).contains(&v))
            || [parsed.line, parsed.column]
                .into_iter()
                .flatten()
                .any(|v| v == 0 || v > u32::MAX as u64)
            || parsed
                .severity
                .as_deref()
                .is_some_and(|v| !["all", "error", "warning", "information", "hint"].contains(&v))
            || object.values().any(Value::is_null)
        {
            return Err(invalid());
        }
        Ok(parsed)
    }

    pub fn params(&self, name: &str) -> Value {
        let mut result = json!({});
        if let (Some(line), Some(column)) = (self.line, self.column) {
            result["line"] = json!(line);
            result["column"] = json!(column);
        }
        if name == "lsp_references" {
            result["context"] =
                json!({"includeDeclaration":self.include_declaration.unwrap_or(true)});
        }
        if let Some(query) = &self.query {
            result["query"] = json!(query);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_schemas_reject_generic_methods_and_invalid_positions() {
        for tool in TOOL_NAMES {
            assert!(tool_spec(tool).is_some());
        }
        assert!(tool_spec("lsp_rename").is_none());
        for args in [
            json!({"path":"a.ts","line":0,"column":1}),
            json!({"path":"a.ts","line":1,"column":1,"method":"workspace/applyEdit"}),
            json!({"path":null,"line":1,"column":1}),
            json!({"path":"a.ts","line":1,"column":1,"query":"x"}),
        ] {
            assert!(Arguments::parse("lsp_definition", &args).is_err());
        }
        assert!(Arguments::parse("lsp_workspace_symbols", &json!({"query":"a"})).is_ok());
    }
}
