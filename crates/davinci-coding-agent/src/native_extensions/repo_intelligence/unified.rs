use super::{queries, scanner, RepoIndex, RepoIntelligence, SourceRange};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

/// Stable boundary for Language Intelligence. No transport or server lifecycle here.
#[derive(Debug, Clone, Copy)]
pub enum SemanticOperation {
    Definition,
    References,
    Implementations,
    TypeDefinition,
    Diagnostics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticEvidence {
    pub path: String,
    pub range: SourceRange,
    pub description: String,
}

pub trait SemanticLanguageProvider: Send + Sync + std::fmt::Debug {
    fn query(
        &self,
        operation: SemanticOperation,
        path: &str,
        range: &SourceRange,
        limit: usize,
    ) -> Result<Vec<SemanticEvidence>, String>;
}

impl RepoIntelligence {
    pub(super) fn unified_query(
        &self,
        index: &RepoIndex,
        query: &str,
        scope: &str,
        limit: usize,
        args: &Value,
    ) -> Result<Value, String> {
        if query.trim().is_empty() {
            return Err("query_invalid: query required".into());
        }
        if let Some(anchor) = args.get("semantic").and_then(Value::as_object) {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or("query_invalid: semantic anchor requires path")?;
            let operation_name = anchor
                .get("operation")
                .and_then(Value::as_str)
                .ok_or("query_invalid: semantic.operation required")?;
            let operation = match operation_name {
                "definition" => SemanticOperation::Definition,
                "references" => SemanticOperation::References,
                "implementations" => SemanticOperation::Implementations,
                "typeDefinition" => SemanticOperation::TypeDefinition,
                "diagnostics" => SemanticOperation::Diagnostics,
                _ => return Err("query_invalid: invalid semantic.operation".into()),
            };
            let range = if matches!(operation, SemanticOperation::Diagnostics) {
                SourceRange {
                    start_line: 1,
                    start_column: 1,
                    end_line: 1,
                    end_column: 1,
                }
            } else {
                let line = anchor.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
                let column = anchor.get("column").and_then(Value::as_u64).unwrap_or(0) as usize;
                if line == 0 || column == 0 {
                    return Err("query_invalid: semantic position must be one-based".into());
                }
                SourceRange {
                    start_line: line,
                    start_column: column,
                    end_line: line,
                    end_column: column,
                }
            };
            if let Some(provider) = &self.semantic {
                match provider.query(operation, path, &range, limit) {
                    Ok(evidence) => {
                        let results = evidence
                            .into_iter()
                            .take(limit)
                            .filter(|item| self.validate_path(&item.path).is_ok())
                            .map(|item| {
                                json!({
                                    "source":"lsp",
                                    "confidence":"semantic",
                                    "path":item.path,
                                    "range":item.range,
                                    "reason":item.description.chars().take(500).collect::<String>()
                                })
                            })
                            .collect::<Vec<_>>();
                        let mut value = queries::envelope(results, limit);
                        value["route"] = json!("semantic");
                        value["semantic_status"] = json!("provider");
                        return Ok(value);
                    }
                    Err(error) => {
                        let mut fallback_args = args.clone();
                        if let Some(object) = fallback_args.as_object_mut() {
                            object.remove("semantic");
                        }
                        let mut value =
                            self.unified_query(index, query, scope, limit, &fallback_args)?;
                        value["semantic_status"] = json!("unavailable");
                        value["semantic_warning"] = json!(format!(
                            "semantic provider unavailable: {}",
                            error.chars().take(300).collect::<String>()
                        ));
                        return Ok(value);
                    }
                }
            }
            let mut fallback_args = args.clone();
            if let Some(object) = fallback_args.as_object_mut() {
                object.remove("semantic");
            }
            let mut value = self.unified_query(index, query, scope, limit, &fallback_args)?;
            value["semantic_status"] = json!("unavailable");
            value["semantic_warning"] =
                json!("semantic provider unavailable; structural/text evidence only");
            return Ok(value);
        }
        let lower = query.to_lowercase();
        let words: Vec<_> = query
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .collect();
        let exact = index.files.values().flat_map(|f| &f.symbols).find(|s| {
            (scope == "." || s.file == scope || s.file.starts_with(&format!("{scope}/")))
                && words.iter().any(|w| *w == s.name)
        });
        let dependency = ["imports", "dependency", "dependencies", "depends on"]
            .iter()
            .any(|w| lower.contains(w));
        let mentioned_path = index
            .files
            .keys()
            .filter(|path| scope == "." || *path == scope || path.starts_with(&format!("{scope}/")))
            .find(|path| {
                query
                    .split_whitespace()
                    .any(|word| word.trim_matches(['\'', '"', '`', ',', '?']) == path.as_str())
            });
        let literal = query.trim().starts_with(['"', '\'', '`']);
        let mut result = if !literal
            && dependency
            && (mentioned_path.is_some() || exact.is_some() || index.files.contains_key(scope))
        {
            let path = mentioned_path
                .map(String::as_str)
                .or_else(|| exact.map(|s| s.file.as_str()))
                .unwrap_or(scope);
            let mut value = queries::dependencies(index, path, limit);
            value["route"] = json!("dependency");
            value
        } else if !literal && exact.is_some() {
            let symbol = exact.unwrap();
            let mut results = vec![queries::symbol_evidence(symbol, "exact symbol in query")];
            let mut semantic_error = None;
            if let Some(provider) = &self.semantic {
                let operation = if lower.contains("implement") {
                    SemanticOperation::Implementations
                } else if lower.contains("reference")
                    || lower.contains("uses")
                    || lower.contains("consum")
                {
                    SemanticOperation::References
                } else if lower.contains("type definition") {
                    SemanticOperation::TypeDefinition
                } else if lower.contains("diagnostic") {
                    SemanticOperation::Diagnostics
                } else {
                    SemanticOperation::Definition
                };
                match provider.query(operation, &symbol.file, &symbol.range, limit) {
                    Ok(evidence) => {
                        for item in evidence.into_iter().take(limit) {
                            if self.validate_path(&item.path).is_ok()
                                && (scope == "."
                                    || item.path == scope
                                    || item.path.starts_with(&format!("{scope}/")))
                            {
                                results.push(json!({"source":"lsp","confidence":"semantic","path":item.path,
                                "range":item.range,"reason":item.description.chars().take(500).collect::<String>()}));
                            }
                        }
                    }
                    Err(_) => {
                        semantic_error = Some("semantic provider unavailable; structural fallback")
                    }
                }
            }
            let relations = queries::relationships(index, symbol, limit);
            if let Some(items) = relations["results"].as_array() {
                results.extend(items.clone());
            }
            let deps = queries::dependencies(index, &symbol.file, limit);
            if let Some(items) = deps["results"].as_array() {
                results.extend(items.clone());
            }
            if lower.contains("impact") || lower.contains("change") || lower.contains("test") {
                let related = queries::related(index, Some(&symbol.file), "", scope, limit);
                if let Some(items) = related["results"].as_array() {
                    results.extend(items.clone());
                }
            }
            let mut value = queries::envelope(results, limit);
            value["route"] = json!("symbol");
            value["semantic_status"] = json!(if self.semantic.is_none() {
                "unavailable"
            } else {
                "provider"
            });
            if let Some(error) = semantic_error {
                value["semantic_warning"] = json!(error);
            }
            value
        } else {
            let needle = query.trim().trim_matches(['"', '\'', '`']);
            let mut related = if literal {
                queries::envelope(Vec::new(), limit)
            } else {
                queries::related(index, None, query, scope, limit)
            };
            let mut results = related["results"].as_array().cloned().unwrap_or_default();
            let mut bytes = 0usize;
            let mut partial = false;
            // Bounded literal search also works for malformed/unsupported source and docs.
            for path in &index.text_files {
                if scope != "." && path != scope && !path.starts_with(&format!("{scope}/")) {
                    continue;
                }
                if bytes >= 4 * 1024 * 1024 || results.len() > limit {
                    partial = true;
                    break;
                }
                let remaining = 4 * 1024 * 1024 - bytes;
                let Ok(body) = scanner::read_bounded(
                    Path::new(&index.root),
                    Path::new(path),
                    self.config.max_file_bytes.min(remaining),
                ) else {
                    partial = true;
                    continue;
                };
                bytes += body.len();
                for (line, text) in body.lines().enumerate() {
                    let matches = if literal {
                        text.contains(needle)
                    } else {
                        words
                            .iter()
                            .filter(|w| w.len() >= 4)
                            .any(|w| text.to_lowercase().contains(&w.to_lowercase()))
                    };
                    if matches {
                        results.push(json!({"source":"text","confidence":"textual","path":path,
                            "line":line + 1,"excerpt":text.chars().take(240).collect::<String>(),
                            "reason":"bounded text match"}));
                        if results.len() > limit {
                            partial = true;
                            break;
                        }
                    }
                }
            }
            related = queries::envelope(results, limit);
            related["route"] = json!(if literal { "text" } else { "architecture" });
            related["text_bytes_read"] = json!(bytes);
            related["text_search_partial"] = json!(partial);
            related
        };
        if args.get("includeEvidence").and_then(Value::as_bool) == Some(false) {
            if let Some(results) = result["results"].as_array_mut() {
                for item in results {
                    if let Some(object) = item.as_object_mut() {
                        object.remove("excerpt");
                    }
                }
            }
        }
        Ok(result)
    }
}
