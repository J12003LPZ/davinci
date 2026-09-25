//! Compatibility facade from host-neutral core semantic DTOs to the single
//! native language-intelligence process owner.

use crate::native_extensions::language_intelligence::LanguageIntelligence;
use davinci_agent::semantic::{
    Diagnostic, DiagnosticSeverity, Location, Position, Range, RenamePreview, SemanticCapabilities,
    SemanticQuery, SemanticRequestContext, SemanticResult, SemanticService, SymbolItem,
};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SemanticServiceFacade {
    language: LanguageIntelligence,
}

impl SemanticServiceFacade {
    pub fn new(language: LanguageIntelligence) -> Self {
        Self { language }
    }

    fn source_supported(path: &Path) -> bool {
        matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" | "rs" | "py" | "pyi")
        )
    }

    fn run(&self, tool: &str, args: Value, capability: &str) -> Result<SemanticResult, String> {
        let output = self
            .language
            .execute(tool, &args)
            .map_err(|error| error.to_string())?;
        if output.is_error {
            return Err(output.content);
        }
        let details = output.details.unwrap_or(Value::Null);
        semantic_result(capability, details)
    }
}

impl SemanticService for SemanticServiceFacade {
    fn is_server_available(&self, _language: &str, path: &Path) -> bool {
        // This is intentionally a no-launch observation. Installed executable
        // discovery happens only inside an authorized direct query.
        Self::source_supported(path)
    }

    fn capabilities(&self, _language: &str, path: &Path) -> SemanticCapabilities {
        if !Self::source_supported(path) {
            return SemanticCapabilities::default();
        }
        SemanticCapabilities {
            definition: true,
            references: true,
            outline: true,
            diagnostics: true,
            call_hierarchy: false,
            rename_preview: false,
        }
    }

    fn definition(
        &self,
        _cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<SemanticResult, String> {
        self.run(
            "lsp_definition",
            json!({
                "path":file_path,
                "line": line.checked_add(1).ok_or("line overflow")?,
                "column": character.checked_add(1).ok_or("column overflow")?
            }),
            "definition",
        )
    }

    fn references(
        &self,
        _cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<SemanticResult, String> {
        self.run(
            "lsp_references",
            json!({
                "path":file_path,
                "line": line.checked_add(1).ok_or("line overflow")?,
                "column": character.checked_add(1).ok_or("column overflow")?,
                "includeDeclaration":include_declaration
            }),
            "references",
        )
    }

    fn outline(&self, _cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
        self.run(
            "lsp_document_symbols",
            json!({"path":file_path}),
            "outline",
        )
    }

    fn diagnostics(&self, _cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
        self.run(
            "lsp_diagnostics",
            json!({"path":file_path}),
            "diagnostics",
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
        Err("Semantic call hierarchy is not supported by the shared LSP facade".into())
    }

    fn rename_preview(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _new_name: &str,
    ) -> Result<RenamePreview, String> {
        Err("Semantic rename preview is not supported by the shared LSP facade".into())
    }

    fn query_with_context(
        &self,
        query: SemanticQuery<'_>,
        context: &SemanticRequestContext,
    ) -> Result<SemanticResult, String> {
        if context
            .abort
            .as_ref()
            .is_some_and(|abort| abort.load(std::sync::atomic::Ordering::Acquire))
        {
            return Err("Operation aborted".into());
        }
        if context
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return Err("Semantic request deadline expired".into());
        }
        match query {
            SemanticQuery::Definition { cwd, path, position } => {
                self.definition(cwd, path, position.line, position.character)
            }
            SemanticQuery::References {
                cwd,
                path,
                position,
                include_declaration,
            } => self.references(
                cwd,
                path,
                position.line,
                position.character,
                include_declaration,
            ),
            SemanticQuery::Outline { cwd, path } => self.outline(cwd, path),
            SemanticQuery::Diagnostics { cwd, path } => self.diagnostics(cwd, path),
        }
    }
}

fn semantic_result(capability: &str, details: Value) -> Result<SemanticResult, String> {
    let mut result = SemanticResult {
        request_id: format!("native-lsp-{capability}"),
        server_identity: details
            .get("backend")
            .map(Value::to_string),
        capability: capability.into(),
        document_version: details
            .get("documentVersion")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok()),
        source_manifest: details
            .get("sourceHash")
            .and_then(Value::as_str)
            .map(str::to_string),
        locations: Vec::new(),
        diagnostics: Vec::new(),
        symbols: Vec::new(),
        calls: Vec::new(),
        partial: details
            .get("remaining")
            .and_then(Value::as_u64)
            .is_some_and(|remaining| remaining > 0)
            || details.get("available") == Some(&Value::Bool(false)),
        fallback_reason: None,
    };

    for item in details
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let range = core_range(
            item.get("range")
                .ok_or_else(|| "Semantic result is missing a range".to_string())?,
        )?;
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "Semantic result is missing a path".to_string())?
            .to_string();
        if capability == "diagnostics" {
            let severity = match item.get("severity").and_then(Value::as_str) {
                Some("error") => DiagnosticSeverity::Error,
                Some("warning") => DiagnosticSeverity::Warning,
                Some("information") => DiagnosticSeverity::Information,
                _ => DiagnosticSeverity::Hint,
            };
            result.diagnostics.push(Diagnostic {
                range,
                severity,
                code: item.get("code").map(|code| {
                    code.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| code.to_string())
                }),
                source: item
                    .get("source")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                message: item
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        } else if capability == "outline" {
            result.symbols.push(SymbolItem {
                name: item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                kind: item
                    .get("kind")
                    .map(Value::to_string)
                    .unwrap_or_default(),
                range,
                selection_range: range,
                detail: None,
            });
        } else {
            result.locations.push(Location {
                path,
                range,
                snippet: None,
                is_textual_fallback: false,
            });
        }
    }
    Ok(result)
}

fn core_range(value: &Value) -> Result<Range, String> {
    let point = |name: &str| -> Result<Position, String> {
        let value = value
            .get(name)
            .ok_or_else(|| "Semantic range point is missing".to_string())?;
        let line = value
            .get("line")
            .and_then(Value::as_u64)
            .and_then(|line| line.checked_sub(1))
            .ok_or_else(|| "Semantic line is invalid".to_string())?;
        let character = value
            .get("column")
            .and_then(Value::as_u64)
            .and_then(|column| column.checked_sub(1))
            .ok_or_else(|| "Semantic column is invalid".to_string())?;
        Ok(Position {
            line: u32::try_from(line).map_err(|_| "Semantic line overflow".to_string())?,
            character: u32::try_from(character)
                .map_err(|_| "Semantic column overflow".to_string())?,
        })
    };
    Ok(Range {
        start: point("start")?,
        end: point("end")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_ranges_convert_once_to_core_zero_based_coordinates() {
        let range = core_range(&json!({
            "start":{"line":2,"column":5},
            "end":{"line":2,"column":8}
        }))
        .unwrap();
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 4);
        assert!(core_range(&json!({
            "start":{"line":0,"column":1},
            "end":{"line":1,"column":1}
        }))
        .is_err());
    }
}
