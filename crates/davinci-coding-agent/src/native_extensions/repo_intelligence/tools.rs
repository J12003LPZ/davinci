use super::RepoIntelligence;
use davinci_agent::{ToolError, ToolResult};
use serde_json::{json, Value};

pub fn is_repo_tool(name: &str) -> bool {
    matches!(
        name,
        "repo_map"
            | "symbol_search"
            | "file_symbols"
            | "file_dependencies"
            | "symbol_relationships"
            | "related_files"
            | "code_query"
    )
}

pub fn tool_spec(name: &str) -> Option<davinci_ai::ToolSpec> {
    let description = match name {
        "repo_map" => "Orient within a repository using a compact cached TS/JS structural map, modules, entry points and dependency hotspots.",
        "symbol_search" => "Find TS/JS symbols by exact, prefix or fuzzy name without reading whole source files. Results are structural AST facts.",
        "file_symbols" => "Read a TS/JS file outline with symbol ranges for targeted source reads.",
        "file_dependencies" => "Inspect local imports, reexports, reverse importers and external packages. Structural evidence, not semantic usage.",
        "symbol_relationships" => "Inspect a symbol's structural contains, calls, inheritance and type edges by symbolId or path and line.",
        "related_files" => "Rank files related to a file, symbolId or task query, with dependency, test pairing and configuration reasons.",
        "code_query" => "Find code using deterministic structural, semantic-provider and bounded text evidence. Ask a code question; routing is automatic.",
        _ => return None,
    };
    let mut properties = json!({
        "path":{"type":"string","minLength":1,"description":"Workspace-relative file or directory."},
        "limit":{"type":"integer","minimum":1,"maximum":100}
    });
    if matches!(name, "symbol_search" | "related_files" | "code_query") {
        properties["query"] = json!({"type":"string","minLength":1,"maxLength":2048});
    }
    if name == "repo_map" {
        properties["depth"] = json!({"type":"integer","minimum":1,"maximum":12});
    }
    if name == "symbol_search" {
        properties["kind"] = json!({"type":"string","enum":["function","method","class","interface",
            "type_alias","enum","variable","namespace","property"]});
    }
    if matches!(name, "symbol_relationships" | "related_files") {
        properties["symbolId"] = json!({"type":"string","minLength":1,"maxLength":64});
    }
    if name == "symbol_relationships" {
        properties["line"] = json!({"type":"integer","minimum":1});
    }
    if name == "code_query" {
        properties["includeEvidence"] = json!({"type":"boolean","default":true});
        properties["semantic"] = json!({
            "type":"object",
            "additionalProperties":false,
            "properties":{
                "operation":{"type":"string","enum":["definition","references","implementations","typeDefinition","diagnostics"]},
                "line":{"type":"integer","minimum":1},
                "column":{"type":"integer","minimum":1}
            },
            "required":["operation"]
        });
    }
    let required: Vec<&str> = match name {
        "symbol_search" | "code_query" => vec!["query"],
        "file_symbols" | "file_dependencies" => vec!["path"],
        _ => Vec::new(),
    };
    Some(davinci_ai::ToolSpec {
        name: name.into(),
        description: description.into(),
        parameters: json!({"type":"object","additionalProperties":false,"properties":properties,"required":required}),
        constrained_sampling: None,
    })
}


fn validate_semantic_anchor(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("query_invalid: semantic must be an object")?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "operation" | "line" | "column"))
    {
        return Err("query_invalid: semantic has unknown fields".into());
    }
    let operation = object
        .get("operation")
        .and_then(Value::as_str)
        .ok_or("query_invalid: semantic.operation required")?;
    if !matches!(
        operation,
        "definition" | "references" | "implementations" | "typeDefinition" | "diagnostics"
    ) {
        return Err("query_invalid: unsupported semantic.operation".into());
    }
    let line = object.get("line");
    let column = object.get("column");
    if operation == "diagnostics" {
        if line.is_some() || column.is_some() {
            return Err("query_invalid: diagnostics semantic anchor has no position".into());
        }
    } else if !line
        .and_then(Value::as_u64)
        .is_some_and(|value| value >= 1)
        || !column
            .and_then(Value::as_u64)
            .is_some_and(|value| value >= 1)
    {
        return Err("query_invalid: semantic line and column must be >= 1".into());
    }
    Ok(())
}

impl RepoIntelligence {
    pub fn execute_tool(&self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        let value = self.query(name, args).map_err(ToolError::Failed)?;
        Ok(ToolResult {
            content: serde_json::to_string(&value).map_err(|e| ToolError::Failed(e.to_string()))?,
            is_error: false,
            details: Some(value),
        })
    }
}

pub(super) fn validate(name: &str, args: &Value) -> Result<(), String> {
    let spec = tool_spec(name).ok_or("query_invalid: unknown tool")?;
    let object = args.as_object().ok_or("query_invalid: object required")?;
    for required in spec.parameters["required"].as_array().into_iter().flatten() {
        if !object.contains_key(required.as_str().unwrap_or("")) {
            return Err(format!("query_invalid: missing {required}"));
        }
    }
    for (key, value) in object {
        let schema = &spec.parameters["properties"][key];
        if name == "code_query" && key == "semantic" {
            validate_semantic_anchor(value)?;
            continue;
        }
        let valid = match schema["type"].as_str() {
            Some("string") => value.as_str().is_some_and(|s| {
                !s.trim().is_empty()
                    && s.len() <= schema["maxLength"].as_u64().unwrap_or(4096) as usize
            }),
            Some("integer") => value.as_u64().is_some_and(|v| {
                v >= schema["minimum"].as_u64().unwrap_or(0)
                    && v <= schema["maximum"].as_u64().unwrap_or(u64::MAX)
            }),
            Some("boolean") => value.is_boolean(),
            _ => false,
        };
        if !valid
            || schema["enum"]
                .as_array()
                .is_some_and(|choices| !choices.contains(value))
        {
            return Err(format!("query_invalid: invalid {key}"));
        }
    }
    if name == "symbol_relationships"
        && !object.contains_key("symbolId")
        && !(object.contains_key("path") && object.contains_key("line"))
    {
        return Err("query_invalid: symbolId or path and line required".into());
    }
    if name == "related_files"
        && !["path", "symbolId", "query"]
            .iter()
            .any(|key| object.contains_key(*key))
    {
        return Err("query_invalid: file, symbol or task hint required".into());
    }
    Ok(())
}
