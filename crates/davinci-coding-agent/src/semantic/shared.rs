//! Compatibility facade from host-neutral core semantic tools to the canonical native LSP owner.
use crate::native_extensions::language_intelligence::{LanguageIntelligence, RequestBudget};
use davinci_agent::runtime::task_transport::TaskCoordinatorClient;
use davinci_agent::semantic::{
    Diagnostic, DiagnosticSeverity, Location, Position, Range, RenamePreview, SemanticCapabilities,
    SemanticQuery, SemanticRequestContext, SemanticResult, SemanticService, SymbolItem,
};
use davinci_agent::{ToolError, ToolResult};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum SemanticClient {
    Local(LanguageIntelligence),
    Parent(TaskCoordinatorClient),
}

impl SemanticClient {
    pub fn local(language: LanguageIntelligence) -> Self {
        Self::Local(language)
    }

    pub fn parent(client: TaskCoordinatorClient) -> Self {
        Self::Parent(client)
    }

    fn execute(
        &self,
        tool: &str,
        args: &Value,
        context: &SemanticRequestContext,
    ) -> Result<ToolResult, String> {
        let deadline = context
            .deadline
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(60));
        match self {
            Self::Local(language) => language
                .execute_with_budget(
                    tool,
                    args,
                    RequestBudget {
                        deadline,
                        cancelled: context.abort.clone(),
                    },
                )
                .map_err(|error| error.to_string()),
            Self::Parent(client) => {
                let timeout = deadline
                    .checked_duration_since(Instant::now())
                    .unwrap_or_default()
                    .max(Duration::from_millis(1));
                client
                    .call_with_timeout(tool, args, context.abort.as_deref(), timeout)
                    .map_err(|error| error.to_string())
            }
        }
    }

    fn live_language(&self, language: &str) -> bool {
        let Self::Local(manager) = self else {
            return false;
        };
        let expected = match language {
            "javascript" | "typescript" | "javascriptreact" | "typescriptreact" => "typescript",
            "rust" => "rust",
            "python" => "python",
            _ => return false,
        };
        manager.status()["sessions"]
            .as_array()
            .is_some_and(|sessions| {
                sessions.iter().any(|session| {
                    session["language"].as_str() == Some(expected)
                        && session["session"].as_str() == Some("running")
                })
            })
    }
}

#[derive(Debug, Clone)]
pub struct SemanticServiceFacade {
    client: SemanticClient,
}

impl SemanticServiceFacade {
    pub fn local(language: LanguageIntelligence) -> Self {
        Self {
            client: SemanticClient::local(language),
        }
    }

    pub fn parent(client: TaskCoordinatorClient) -> Self {
        Self {
            client: SemanticClient::parent(client),
        }
    }

    pub fn client(&self) -> &SemanticClient {
        &self.client
    }

    fn native_query(
        &self,
        query: SemanticQuery<'_>,
        context: &SemanticRequestContext,
    ) -> Result<SemanticResult, String> {
        let (tool, args, capability, cwd, path, fallback_position) = match query {
            SemanticQuery::Definition { cwd, path, position } => (
                "lsp_definition",
                positional_args(path, position)?,
                "definition",
                cwd,
                path,
                Some(position),
            ),
            SemanticQuery::References {
                cwd,
                path,
                position,
                include_declaration,
            } => {
                let mut args = positional_args(path, position)?;
                args["includeDeclaration"] = json!(include_declaration);
                (
                    "lsp_references",
                    args,
                    "references",
                    cwd,
                    path,
                    Some(position),
                )
            }
            SemanticQuery::Outline { cwd, path } => (
                "lsp_document_symbols",
                json!({"path":path}),
                "outline",
                cwd,
                path,
                None,
            ),
            SemanticQuery::Diagnostics { cwd, path } => (
                "lsp_diagnostics",
                json!({"path":path}),
                "diagnostics",
                cwd,
                path,
                None,
            ),
        };

        let output = self.client.execute(tool, &args, context)?;
        if output.is_error {
            let details = output.details.unwrap_or(Value::Null);
            let code = details.pointer("/error/code").and_then(Value::as_str).unwrap_or("semantic_unavailable");
            if matches!(
                code,
                "server_not_installed"
                    | "disabled"
                    | "unsupported_method"
                    | "project_not_found"
                    | "project_root_required"
            ) {
                return textual_fallback(cwd, path, capability, fallback_position, context);
            }
            return Err(format!(
                "{code}: {}",
                details
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("semantic query failed")
            ));
        }
        normalized_to_core(tool, capability, output.details.unwrap_or(Value::Null))
    }
}

impl SemanticService for SemanticServiceFacade {
    fn is_server_available(&self, language: &str, _path: &Path) -> bool {
        self.client.live_language(language)
    }

    fn capabilities(&self, language: &str, path: &Path) -> SemanticCapabilities {
        let live = self.is_server_available(language, path);
        SemanticCapabilities {
            definition: live,
            references: live,
            outline: live,
            diagnostics: live,
            call_hierarchy: false,
            rename_preview: false,
        }
    }

    fn definition(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<SemanticResult, String> {
        self.query_with_context(
            SemanticQuery::Definition {
                cwd,
                path: file_path,
                position: Position { line, character },
            },
            &SemanticRequestContext::default(),
        )
    }

    fn references(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<SemanticResult, String> {
        self.query_with_context(
            SemanticQuery::References {
                cwd,
                path: file_path,
                position: Position { line, character },
                include_declaration,
            },
            &SemanticRequestContext::default(),
        )
    }

    fn outline(&self, cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
        self.query_with_context(
            SemanticQuery::Outline {
                cwd,
                path: file_path,
            },
            &SemanticRequestContext::default(),
        )
    }

    fn diagnostics(&self, cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
        self.query_with_context(
            SemanticQuery::Diagnostics {
                cwd,
                path: file_path,
            },
            &SemanticRequestContext::default(),
        )
    }

    fn call_hierarchy(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _incoming: bool,
    ) -> Result<SemanticResult, String> {
        Err("unsupported_method: call hierarchy is outside the shared read-only LSP scope".into())
    }

    fn rename_preview(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _new_name: &str,
    ) -> Result<RenamePreview, String> {
        Err("unsupported_method: rename preview is outside the shared read-only LSP scope".into())
    }

    fn query_with_context(
        &self,
        query: SemanticQuery<'_>,
        context: &SemanticRequestContext,
    ) -> Result<SemanticResult, String> {
        self.native_query(query, context)
    }
}

fn positional_args(path: &str, position: Position) -> Result<Value, String> {
    let line = u64::from(position.line)
        .checked_add(1)
        .ok_or_else(|| "invalid_position: line overflow".to_string())?;
    let column = u64::from(position.character)
        .checked_add(1)
        .ok_or_else(|| "invalid_position: column overflow".to_string())?;
    Ok(json!({"path":path,"line":line,"column":column}))
}

fn normalized_to_core(
    tool: &str,
    capability: &str,
    details: Value,
) -> Result<SemanticResult, String> {
    let server_identity = Some(format!(
        "{}#{}",
        details["backend"].as_str().unwrap_or("language-server"),
        details["generation"].as_u64().unwrap_or(0)
    ));
    let document_version = details["documentVersion"]
        .as_i64()
        .and_then(|value| i32::try_from(value).ok());
    let limitations = details["limitations"].as_array().cloned().unwrap_or_default();
    let partial = details["remaining"].as_u64().unwrap_or(0) > 0
        || details["omittedExternal"].as_u64().unwrap_or(0) > 0
        || !limitations.is_empty()
        || details["workspaceCoverage"].as_str() == Some("partial")
        || matches!(
            details["freshness"].as_str(),
            Some("diagnostics_pending" | "unversioned-publication" | "stale_result")
        );
    let fallback_reason = if limitations.is_empty() {
        details["freshness"]
            .as_str()
            .filter(|_| partial)
            .map(str::to_string)
    } else {
        Some(
            limitations
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(","),
        )
    };
    let mut result = SemanticResult {
        request_id: format!(
            "native:{}:{}",
            tool,
            details["generation"].as_u64().unwrap_or(0)
        ),
        server_identity,
        capability: capability.into(),
        document_version,
        source_manifest: details["sourceHash"].as_str().map(str::to_string),
        locations: Vec::new(),
        diagnostics: Vec::new(),
        symbols: Vec::new(),
        calls: Vec::new(),
        partial,
        fallback_reason,
    };
    let items = details["items"]
        .as_array()
        .ok_or_else(|| "protocol_error: normalized semantic result contains no items".to_string())?;
    for item in items {
        let range = core_range(&item["range"])?;
        match tool {
            "lsp_definition" | "lsp_references" => {
                result.locations.push(Location {
                    path: item["path"].as_str().unwrap_or_default().into(),
                    range,
                    snippet: None,
                    is_textual_fallback: false,
                });
            }
            "lsp_document_symbols" => {
                result.symbols.push(SymbolItem {
                    name: item["name"].as_str().unwrap_or_default().into(),
                    kind: symbol_kind(item["kind"].as_u64().unwrap_or(0)).into(),
                    range,
                    selection_range: range,
                    detail: item["detail"].as_str().map(str::to_string),
                });
            }
            "lsp_diagnostics" => {
                result.diagnostics.push(Diagnostic {
                    range,
                    severity: match item["severity"].as_str() {
                        Some("error") => DiagnosticSeverity::Error,
                        Some("warning") => DiagnosticSeverity::Warning,
                        Some("information") => DiagnosticSeverity::Information,
                        _ => DiagnosticSeverity::Hint,
                    },
                    code: item["code"]
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| item["code"].as_i64().map(|value| value.to_string())),
                    source: item["source"].as_str().map(str::to_string),
                    message: item["message"].as_str().unwrap_or_default().into(),
                });
            }
            _ => {}
        }
    }
    Ok(result)
}

fn core_range(value: &Value) -> Result<Range, String> {
    let point = |name: &str| -> Result<Position, String> {
        let line = value[name]["line"]
            .as_u64()
            .and_then(|value| value.checked_sub(1))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| "protocol_error: invalid normalized semantic line".to_string())?;
        let character = value[name]["column"]
            .as_u64()
            .and_then(|value| value.checked_sub(1))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| "protocol_error: invalid normalized semantic column".to_string())?;
        Ok(Position { line, character })
    };
    Ok(Range {
        start: point("start")?,
        end: point("end")?,
    })
}

fn symbol_kind(kind: u64) -> &'static str {
    match kind {
        2 => "module",
        3 => "namespace",
        5 => "class",
        6 => "method",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        22 => "enumMember",
        23 => "struct",
        26 => "typeParameter",
        _ => "symbol",
    }
}

fn textual_fallback(
    cwd: &Path,
    path: &str,
    capability: &str,
    position: Option<Position>,
    context: &SemanticRequestContext,
) -> Result<SemanticResult, String> {
    match capability {
        "definition" | "references" => {
            let position = position.ok_or_else(|| "fallback position missing".to_string())?;
            let symbol = symbol_at_position(cwd, path, position)?;
            if capability == "definition" {
                davinci_agent::semantic::text_fallback_definition(
                    cwd,
                    &symbol,
                    Some(Path::new(path)),
                    context.abort.as_deref(),
                )
            } else {
                davinci_agent::semantic::text_fallback_references(
                    cwd,
                    &symbol,
                    Some(Path::new(path)),
                    context.abort.as_deref(),
                )
            }
        }
        "outline" => davinci_agent::semantic::text_fallback_outline(cwd, path),
        "diagnostics" => davinci_agent::semantic::text_fallback_diagnostics(cwd, path),
        _ => Err("unsupported_method: no fallback exists for this semantic operation".into()),
    }
}

fn symbol_at_position(cwd: &Path, path: &str, position: Position) -> Result<String, String> {
    let full = cwd.join(path);
    let text = std::fs::read_to_string(&full).map_err(|error| error.to_string())?;
    let line = text
        .lines()
        .nth(position.line as usize)
        .ok_or_else(|| "invalid_position: line is outside the source file".to_string())?;
    let target_units = position.character as usize;
    let mut byte = line.len();
    let mut units = 0usize;
    for (index, ch) in line.char_indices() {
        if units >= target_units {
            byte = index;
            break;
        }
        units += ch.len_utf16();
        if units > target_units {
            return Err("invalid_position: character splits a UTF-16 code point".into());
        }
    }
    if target_units > units {
        return Err("invalid_position: character is outside the source line".into());
    }
    let bytes = line.as_bytes();
    let is_ident = |value: u8| value.is_ascii_alphanumeric() || matches!(value, b'_' | b'$');
    let mut start = byte.min(bytes.len());
    while start > 0 && is_ident(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = byte.min(bytes.len());
    while end < bytes.len() && is_ident(bytes[end]) {
        end += 1;
    }
    if start == end {
        return Err("text_fallback_unavailable: no identifier at semantic position".into());
    }
    Ok(line[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_points_convert_to_zero_based_core_ranges() {
        let range = core_range(&json!({
            "start":{"line":2,"column":5},
            "end":{"line":2,"column":8}
        }))
        .unwrap();
        assert_eq!(range.start, Position { line: 1, character: 4 });
        assert_eq!(range.end, Position { line: 1, character: 7 });
        assert!(core_range(&json!({
            "start":{"line":0,"column":1},
            "end":{"line":1,"column":1}
        }))
        .is_err());
    }
}
