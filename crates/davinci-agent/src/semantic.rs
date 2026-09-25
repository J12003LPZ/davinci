//! Host-neutral semantic service trait, DTOs, and fallback mechanisms matching F10.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Contract helper: route semantic queries based on availability, support, and freshness.
pub fn semantic_route(server_available: bool, supported: bool, stale: bool) -> &'static str {
    if !server_available {
        "text_fallback"
    } else if !supported {
        "unsupported"
    } else if stale {
        "stale_requery"
    } else {
        "semantic"
    }
}

/// A zero-indexed position in a text document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// A range in a text document expressed as start and end positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// Represents a location inside a resource, such as a line inside a text file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub path: String,
    pub range: Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default)]
    pub is_textual_fallback: bool,
}

/// The severity of a diagnostic message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// A diagnostic message, such as a compiler error or linter warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: DiagnosticSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub message: String,
}

/// Represents a programming construct like a variable, function, or class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolItem {
    pub name: String,
    pub kind: String,
    pub range: Range,
    pub selection_range: Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Represents a call hierarchy item (function, method, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallHierarchyItem {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub range: Range,
    pub selection_range: Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A call hierarchy incoming or outgoing call edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallHierarchyCall {
    pub item: CallHierarchyItem,
    pub from_ranges: Vec<Range>,
}

/// Capabilities advertised by a language server for a given language/file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SemanticCapabilities {
    pub definition: bool,
    pub references: bool,
    pub outline: bool,
    pub diagnostics: bool,
    pub call_hierarchy: bool,
    pub rename_preview: bool,
}

/// A text replacement edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

/// A safe, validated preview of a workspace-wide rename operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenamePreview {
    pub id: String,
    pub server_identity: String,
    pub document_versions: HashMap<String, i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_manifest: Option<String>,
    pub edits: HashMap<String, Vec<TextEdit>>,
    pub affected_paths: Vec<String>,
    pub digest: String,
}

/// Unified result from a semantic query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticResult {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_identity: Option<String>,
    pub capability: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_manifest: Option<String>,
    #[serde(default)]
    pub locations: Vec<Location>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub symbols: Vec<SymbolItem>,
    #[serde(default)]
    pub calls: Vec<CallHierarchyCall>,
    #[serde(default)]
    pub partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticRequestContext {
    pub deadline: Option<std::time::Instant>,
    pub abort: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

#[derive(Debug, Clone)]
pub enum SemanticQuery<'a> {
    Definition {
        cwd: &'a Path,
        path: &'a str,
        position: Position,
    },
    References {
        cwd: &'a Path,
        path: &'a str,
        position: Position,
        include_declaration: bool,
    },
    Outline {
        cwd: &'a Path,
        path: &'a str,
    },
    Diagnostics {
        cwd: &'a Path,
        path: &'a str,
    },
}

/// Core interface for semantic capabilities, injected into ToolContext.
pub trait SemanticService: std::fmt::Debug + Send + Sync {
    fn is_server_available(&self, language: &str, path: &Path) -> bool;
    fn capabilities(&self, language: &str, path: &Path) -> SemanticCapabilities;
    fn definition(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<SemanticResult, String>;
    fn references(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<SemanticResult, String>;
    fn outline(&self, cwd: &Path, file_path: &str) -> Result<SemanticResult, String>;
    fn diagnostics(&self, cwd: &Path, file_path: &str) -> Result<SemanticResult, String>;
    fn call_hierarchy(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        incoming: bool,
    ) -> Result<SemanticResult, String>;
    fn rename_preview(
        &self,
        cwd: &Path,
        file_path: &str,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Result<RenamePreview, String>;

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

/// Fallback definition finder using text search patterns.
pub fn text_fallback_definition(
    cwd: &Path,
    symbol: &str,
    target_path: Option<&Path>,
    aborted: Option<&std::sync::atomic::AtomicBool>,
) -> Result<SemanticResult, String> {
    if let Some(ab) = aborted {
        if ab.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("Operation aborted".into());
        }
    }
    if symbol.trim().is_empty() {
        return Err("Symbol cannot be empty for definition lookup".into());
    }

    let mut locations = Vec::new();
    let def_keywords = [
        "fn ",
        "function ",
        "def ",
        "class ",
        "struct ",
        "enum ",
        "trait ",
        "interface ",
        "type ",
        "const ",
        "let ",
        "var ",
    ];

    let search_files: Vec<PathBuf> = if let Some(tp) = target_path {
        let full = safe_workspace_path(cwd, tp)?;
        if full.is_file() {
            vec![full]
        } else {
            collect_code_files(&full, cwd, 200)
        }
    } else {
        collect_code_files(cwd, cwd, 200)
    };

    for file in search_files {
        if let Some(ab) = aborted {
            if ab.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("Operation aborted".into());
            }
        }
        if let Ok(content) = fs::read_to_string(&file) {
            for (line_idx, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                let stripped = trimmed
                    .strip_prefix("pub(crate) ")
                    .or_else(|| trimmed.strip_prefix("pub "))
                    .or_else(|| trimmed.strip_prefix("export default "))
                    .or_else(|| trimmed.strip_prefix("export "))
                    .or_else(|| trimmed.strip_prefix("async "))
                    .unwrap_or(trimmed);
                let stripped_async = stripped.strip_prefix("async ").unwrap_or(stripped);
                let declared_symbol = def_keywords.iter().find_map(|keyword| {
                    let rest = stripped_async.strip_prefix(keyword)?.trim_start();
                    let suffix = rest.strip_prefix(symbol)?;
                    (!suffix
                        .chars()
                        .next()
                        .is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '$')))
                    .then_some(line.len().saturating_sub(rest.len()))
                });

                if let Some(col) = declared_symbol {
                    let rel_path = file
                        .strip_prefix(cwd)
                        .unwrap_or(&file)
                        .to_string_lossy()
                        .replace('\\', "/");
                    locations.push(Location {
                        path: rel_path,
                        range: Range {
                            start: Position {
                                line: line_idx as u32,
                                character: col as u32,
                            },
                            end: Position {
                                line: line_idx as u32,
                                character: (col + symbol.len()) as u32,
                            },
                        },
                        snippet: Some(trimmed.to_string()),
                        is_textual_fallback: true,
                    });
                    if locations.len() >= 50 {
                        break;
                    }
                }
            }
        }
        if locations.len() >= 50 {
            break;
        }
    }

    Ok(SemanticResult {
        request_id: format!("fallback_def_{symbol}"),
        server_identity: None,
        capability: "text_fallback_definition".into(),
        document_version: None,
        source_manifest: None,
        locations,
        diagnostics: Vec::new(),
        symbols: Vec::new(),
        calls: Vec::new(),
        partial: false,
        fallback_reason: Some(
            "Language server unavailable or unsupported; results retrieved via text-pattern fallback"
                .into(),
        ),
    })
}

/// Fallback reference finder using word-based text search.
pub fn text_fallback_references(
    cwd: &Path,
    symbol: &str,
    target_path: Option<&Path>,
    aborted: Option<&std::sync::atomic::AtomicBool>,
) -> Result<SemanticResult, String> {
    if let Some(ab) = aborted {
        if ab.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("Operation aborted".into());
        }
    }
    if symbol.trim().is_empty() {
        return Err("Symbol cannot be empty for references lookup".into());
    }

    let mut locations = Vec::new();
    let search_files = if let Some(tp) = target_path {
        let full = safe_workspace_path(cwd, tp)?;
        if full.is_file() {
            vec![full]
        } else {
            collect_code_files(&full, cwd, 200)
        }
    } else {
        collect_code_files(cwd, cwd, 200)
    };

    for file in search_files {
        if let Some(ab) = aborted {
            if ab.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("Operation aborted".into());
            }
        }
        if let Ok(content) = fs::read_to_string(&file) {
            for (line_idx, line) in content.lines().enumerate() {
                if let Some(col) = line.find(symbol) {
                    let rel_path = file
                        .strip_prefix(cwd)
                        .unwrap_or(&file)
                        .to_string_lossy()
                        .replace('\\', "/");
                    locations.push(Location {
                        path: rel_path,
                        range: Range {
                            start: Position {
                                line: line_idx as u32,
                                character: col as u32,
                            },
                            end: Position {
                                line: line_idx as u32,
                                character: (col + symbol.len()) as u32,
                            },
                        },
                        snippet: Some(line.trim().to_string()),
                        is_textual_fallback: true,
                    });
                    if locations.len() >= 100 {
                        break;
                    }
                }
            }
        }
        if locations.len() >= 100 {
            break;
        }
    }

    Ok(SemanticResult {
        request_id: format!("fallback_ref_{symbol}"),
        server_identity: None,
        capability: "text_fallback_references".into(),
        document_version: None,
        source_manifest: None,
        locations,
        diagnostics: Vec::new(),
        symbols: Vec::new(),
        calls: Vec::new(),
        partial: false,
        fallback_reason: Some(
            "Language server unavailable or unsupported; references retrieved via text search"
                .into(),
        ),
    })
}

/// Fallback outline extracting top-level symbols via syntax patterns.
pub fn text_fallback_outline(cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
    let full = safe_workspace_path(cwd, Path::new(file_path))?;
    let content =
        fs::read_to_string(&full).map_err(|e| format!("Could not read file {file_path}: {e}"))?;

    let mut symbols = Vec::new();
    let def_keywords = [
        ("fn ", "function"),
        ("pub fn ", "function"),
        ("async fn ", "function"),
        ("pub async fn ", "function"),
        ("function ", "function"),
        ("def ", "function"),
        ("class ", "class"),
        ("struct ", "struct"),
        ("pub struct ", "struct"),
        ("enum ", "enum"),
        ("pub enum ", "enum"),
        ("trait ", "interface"),
        ("pub trait ", "interface"),
        ("interface ", "interface"),
        ("type ", "type"),
        ("const ", "constant"),
    ];

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        for (kw, kind) in &def_keywords {
            if let Some(rest) = trimmed.strip_prefix(kw) {
                let rest = rest.trim();
                let name = rest
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    symbols.push(SymbolItem {
                        name,
                        kind: (*kind).to_string(),
                        range: Range {
                            start: Position {
                                line: line_idx as u32,
                                character: 0,
                            },
                            end: Position {
                                line: line_idx as u32,
                                character: line.len() as u32,
                            },
                        },
                        selection_range: Range {
                            start: Position {
                                line: line_idx as u32,
                                character: 0,
                            },
                            end: Position {
                                line: line_idx as u32,
                                character: line.len() as u32,
                            },
                        },
                        detail: Some(line.trim().to_string()),
                    });
                }
                break;
            }
        }
    }

    Ok(SemanticResult {
        request_id: format!("fallback_outline_{file_path}"),
        server_identity: None,
        capability: "text_fallback_outline".into(),
        document_version: None,
        source_manifest: None,
        locations: Vec::new(),
        diagnostics: Vec::new(),
        symbols,
        calls: Vec::new(),
        partial: false,
        fallback_reason: Some(
            "Language server unavailable or unsupported; outline generated from syntax patterns"
                .into(),
        ),
    })
}

/// Fallback diagnostic probe that preserves the explicit limitation that no
/// compiler or language server was run. Reading the file proves the target is
/// available, but an empty diagnostic set must remain partial rather than being
/// presented as proof that the file is error-free.
pub fn text_fallback_diagnostics(cwd: &Path, file_path: &str) -> Result<SemanticResult, String> {
    let full = safe_workspace_path(cwd, Path::new(file_path))?;
    fs::read_to_string(&full).map_err(|e| format!("Could not read file {file_path}: {e}"))?;

    Ok(SemanticResult {
        request_id: format!("fallback_diagnostics_{file_path}"),
        server_identity: None,
        capability: "text_fallback_diagnostics".into(),
        document_version: None,
        source_manifest: None,
        locations: Vec::new(),
        diagnostics: Vec::new(),
        symbols: Vec::new(),
        calls: Vec::new(),
        partial: true,
        fallback_reason: Some(
            "Language server unavailable; no compiler diagnostics were executed".into(),
        ),
    })
}

/// Helper to scan directory for code files excluding standard ignore folders.
fn collect_code_files(dir: &Path, _root: &Path, limit: usize) -> Vec<PathBuf> {
    let mut results = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        if results.len() >= limit {
            break;
        }
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with('.') || file_name == "target" || file_name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                if let Some("rs" | "ts" | "js" | "py" | "go" | "c" | "cpp" | "h" | "hpp" | "java") =
                    path.extension().and_then(|e| e.to_str())
                {
                    results.push(path);
                    if results.len() >= limit {
                        break;
                    }
                }
            }
        }
    }
    results
}

fn safe_workspace_path(cwd: &Path, path: &Path) -> Result<PathBuf, String> {
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let (outside_lexical, symlink_escape) = crate::check_path_boundary(cwd, &full);
    if outside_lexical || symlink_escape {
        return Err(format!(
            "Semantic fallback target {} is outside workspace root {}",
            full.display(),
            cwd.display()
        ));
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn f10_capability_fallback() {
        assert_eq!(semantic_route(false, false, false), "text_fallback");
        assert_eq!(semantic_route(true, false, false), "unsupported");
        assert_eq!(semantic_route(true, true, true), "stale_requery");
        assert_eq!(semantic_route(true, true, false), "semantic");
    }

    #[test]
    fn test_no_server_installed_routes_to_text_fallback() {
        assert_eq!(semantic_route(false, true, false), "text_fallback");
        assert_eq!(semantic_route(false, false, true), "text_fallback");
    }

    #[test]
    fn test_unsupported_hierarchy() {
        // Server exists but does not support call hierarchy
        assert_eq!(semantic_route(true, false, false), "unsupported");
    }

    #[test]
    fn test_stale_server_triggers_stale_requery() {
        // Server is available and supports operation, but document state is stale
        assert_eq!(semantic_route(true, true, true), "stale_requery");
    }

    #[test]
    fn test_server_available_but_workspace_untrusted() {
        // Untrusted workspace forbids server launch: treat server_available as false
        let project_trusted = false;
        let server_binary_present = true;
        let effective_available = project_trusted && server_binary_present;
        assert_eq!(
            semantic_route(effective_available, true, false),
            "text_fallback"
        );
    }

    #[test]
    fn test_unknown_language_fallback() {
        // Unknown language has no configured server
        let supported_languages = ["rust", "typescript", "python"];
        let query_lang = "brainfuck";
        let has_server = supported_languages.contains(&query_lang);
        assert_eq!(semantic_route(has_server, true, false), "text_fallback");
    }

    #[test]
    fn text_fallback_rejects_missing_path_traversal() {
        let dir = tempdir().unwrap();
        let escaped = PathBuf::from("missing/../../escape.rs");
        let error = safe_workspace_path(dir.path(), &escaped).unwrap_err();
        assert!(error.contains("outside workspace root"));
    }

    #[test]
    fn test_text_fallback_cancellation() {
        let dir = tempdir().unwrap();
        let aborted = std::sync::atomic::AtomicBool::new(true);
        let res = text_fallback_definition(dir.path(), "my_symbol", None, Some(&aborted));
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), "Operation aborted");
    }

    #[test]
    fn test_text_fallback_definition_and_references() {
        let dir = tempdir().unwrap();
        let code = r#"
pub struct Alpha;

impl Alpha {
    pub fn execute_job(&self) {
        println!("job done");
    }
}

fn helper() {
    let a = Alpha;
    a.execute_job();
}
"#;
        let file_path = dir.path().join("code.rs");
        fs::write(&file_path, code).unwrap();

        // Find definition of execute_job
        let def_res = text_fallback_definition(
            dir.path(),
            "execute_job",
            Some(&PathBuf::from("code.rs")),
            None,
        )
        .unwrap();
        assert_eq!(def_res.locations.len(), 1);
        assert!(def_res.locations[0].is_textual_fallback);
        assert_eq!(def_res.locations[0].path, "code.rs");
        assert_eq!(def_res.locations[0].range.start.line, 4);

        // Find references of execute_job
        let ref_res = text_fallback_references(
            dir.path(),
            "execute_job",
            Some(&PathBuf::from("code.rs")),
            None,
        )
        .unwrap();
        assert_eq!(ref_res.locations.len(), 2);
        assert!(ref_res.locations[0].is_textual_fallback);

        // Outline
        let outline_res = text_fallback_outline(dir.path(), "code.rs").unwrap();
        assert_eq!(outline_res.symbols.len(), 3);
        assert_eq!(outline_res.symbols[0].name, "Alpha");
        assert_eq!(outline_res.symbols[1].name, "execute_job");
        assert_eq!(outline_res.symbols[2].name, "helper");
    }

    #[test]
    fn text_fallback_definition_ignores_calls_on_declaration_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("fixture.rs");
        fs::write(
            &file,
            "fn target() {}\nfn caller() { target(); }\nfn target_suffix() {}\n",
        )
        .unwrap();

        let result = text_fallback_definition(dir.path(), "target", Some(&file), None).unwrap();

        assert_eq!(result.locations.len(), 1);
        assert_eq!(result.locations[0].range.start.line, 0);
    }

    #[test]
    fn test_empty_legitimate_references() {
        // When text fallback finds nothing, it returns Ok with empty locations
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("empty.rs");
        fs::write(&file_path, "fn untouched() {}\n").unwrap();

        let ref_res = text_fallback_references(
            dir.path(),
            "nonexistent_symbol",
            Some(&PathBuf::from("empty.rs")),
            None,
        )
        .unwrap();
        assert_eq!(ref_res.locations.len(), 0);
        assert!(!ref_res.partial);
    }

    #[test]
    fn test_diagnostic_fallback_is_explicitly_partial() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("code.rs"), "fn main() {}\n").unwrap();

        let result = text_fallback_diagnostics(dir.path(), "code.rs").unwrap();

        assert!(result.partial);
        assert!(result.diagnostics.is_empty());
        assert!(result
            .fallback_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("no compiler diagnostics")));
    }

    #[test]
    fn semantic_fallback_rejects_targets_outside_the_workspace() {
        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("outside.rs");
        fs::write(&outside_file, "fn outside() {}\n").unwrap();

        assert!(text_fallback_outline(workspace.path(), outside_file.to_str().unwrap()).is_err());
        assert!(
            text_fallback_diagnostics(workspace.path(), outside_file.to_str().unwrap()).is_err()
        );
        assert!(
            text_fallback_definition(workspace.path(), "outside", Some(&outside_file), None)
                .is_err()
        );
    }
}
