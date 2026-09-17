use super::{graph, scanner, RepoIndex, RepoIntelligence, Symbol};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub(super) fn envelope(mut results: Vec<Value>, limit: usize) -> Value {
    let total = results.len();
    results.truncate(limit);
    json!({"results":results, "total":total, "truncated":total > limit})
}

pub(super) fn symbol_evidence(symbol: &Symbol, reason: &str) -> Value {
    json!({"source":"ast", "confidence":"structural", "symbol":symbol, "path":symbol.file,
        "range":symbol.range, "reason":reason})
}

pub(super) fn name_score(query: &str, name: &str) -> usize {
    let query = query.to_lowercase();
    let name = name.to_lowercase();
    if query == name {
        return 100;
    }
    if name.starts_with(&query) {
        return 80;
    }
    if name.contains(&query) {
        return 60;
    }
    let mut chars = name.chars();
    if query.len() >= 3
        && query
            .chars()
            .all(|needle| chars.by_ref().any(|c| c == needle))
    {
        30
    } else {
        0
    }
}

fn in_scope(path: &str, scope: &str) -> bool {
    scope == "." || path == scope || path.starts_with(&format!("{}/", scope.trim_end_matches('/')))
}

pub(super) fn search(
    index: &RepoIndex,
    query: &str,
    scope: &str,
    kind: Option<&str>,
    limit: usize,
) -> Value {
    let mut ranked: Vec<_> = index
        .files
        .values()
        .filter(|f| in_scope(&f.path, scope))
        .flat_map(|f| &f.symbols)
        .filter(|s| kind.is_none_or(|k| k == s.kind))
        .filter_map(|s| {
            let score = name_score(query, &s.name);
            (score > 0).then_some((score, s))
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.file.cmp(&b.1.file))
            .then(a.1.id.cmp(&b.1.id))
    });
    envelope(
        ranked
            .into_iter()
            .map(|(_, s)| symbol_evidence(s, "symbol name match"))
            .collect(),
        limit,
    )
}

pub(super) fn dependencies(index: &RepoIndex, path: &str, limit: usize) -> Value {
    let results = graph::dependencies(index)
        .into_iter()
        .filter(|e| e.from == path || e.to.as_deref() == Some(path))
        .map(|e| {
            json!({"source":"ast", "confidence":"structural",
            "direction": if e.from == path {"outgoing"} else {"incoming"},
            "kind": if e.reexport {"reexports"} else {"imports"},
            "path":e.from, "target":e.to, "specifier":e.specifier,
            "external":e.external})
        })
        .collect();
    envelope(results, limit)
}

fn test_stem(path: &str) -> String {
    let stem = Path::new(path)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    stem.trim_end_matches(".test")
        .trim_end_matches(".spec")
        .to_string()
}
fn is_test(path: &str) -> bool {
    path.contains(".test.")
        || path.contains(".spec.")
        || path.starts_with("tests/")
        || path.contains("/tests/")
        || path.starts_with("__tests__/")
        || path.contains("/__tests__/")
}

pub(super) fn related(
    index: &RepoIndex,
    path: Option<&str>,
    hint: &str,
    scope: &str,
    limit: usize,
) -> Value {
    let edges = graph::dependencies(index);
    let distances = path
        .map(|p| graph::distances(&edges, p, 2))
        .unwrap_or_default();
    let words: Vec<_> = hint
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 3)
        .collect();
    let mut ranked = Vec::new();
    for candidate in index.files.keys().chain(index.metadata.iter()) {
        if Some(candidate.as_str()) == path || !in_scope(candidate, scope) {
            continue;
        }
        let mut reasons = Vec::new();
        let mut score = 0;
        if let Some(origin) = path {
            for edge in &edges {
                if edge.from == origin && edge.to.as_ref() == Some(candidate) {
                    score += 100;
                    reasons.push("direct dependency");
                } else if edge.from == *candidate && edge.to.as_deref() == Some(origin) {
                    score += 100;
                    reasons.push("direct importer");
                }
            }
            if distances.get(candidate) == Some(&2) {
                score += 35;
                reasons.push("dependency distance 2");
            }
            if Path::new(origin).parent() == Path::new(candidate).parent() {
                score += 10;
                reasons.push("same module directory");
            }
            if test_stem(origin) == test_stem(candidate) && is_test(origin) != is_test(candidate) {
                score += 80;
                reasons.push("likely test pair");
            }
            if scanner::config_file(candidate) {
                let parent = Path::new(candidate).parent().unwrap_or(Path::new(""));
                if Path::new(origin).starts_with(parent) {
                    score += 15;
                    reasons.push("project configuration");
                }
            }
        }
        for word in &words {
            if candidate.to_lowercase().contains(&word.to_lowercase())
                || index
                    .files
                    .get(candidate)
                    .is_some_and(|f| f.symbols.iter().any(|s| name_score(word, &s.name) >= 60))
            {
                score += 20;
                reasons.push("task name match");
            }
        }
        if score > 0 {
            reasons.sort();
            reasons.dedup();
            ranked.push((score, candidate.clone(), reasons));
        }
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.dedup_by(|a, b| a.1 == b.1);
    envelope(
        ranked
            .into_iter()
            .map(|(score, path, reasons)| {
                json!({
                    "source":if scanner::config_file(&path) {"config"} else {"ast"},
                    "confidence":"heuristic", "path":path, "score":score, "reasons":reasons
                })
            })
            .collect(),
        limit,
    )
}

pub(super) fn repo_map(index: &RepoIndex, scope: &str, depth: usize, limit: usize) -> Value {
    let edges = graph::dependencies(index);
    let mut languages = BTreeMap::<String, usize>::new();
    let mut modules = BTreeMap::<String, usize>::new();
    let mut ranked = Vec::new();
    for file in index.files.values().filter(|f| in_scope(&f.path, scope)) {
        *languages.entry(format!("{:?}", file.language)).or_default() += 1;
        let directory = Path::new(&file.path).parent().unwrap_or(Path::new(""));
        let module = directory
            .components()
            .take(depth)
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        *modules.entry(module).or_default() += 1;
        let connections = edges
            .iter()
            .filter(|e| e.from == file.path || e.to.as_ref() == Some(&file.path))
            .count();
        ranked.push((connections, file));
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.path.cmp(&b.1.path)));
    let mut result = envelope(ranked.iter().map(|(connections, file)| json!({
        "source":"ast", "confidence":"structural", "path":file.path, "connections":connections,
        "symbols":file.symbols.iter().filter(|s| s.exported).take(5).map(|s| &s.name).collect::<Vec<_>>(),
        "parse_status":file.parse_status
    })).collect(), limit);
    let mut modules: Vec<_> = modules.into_iter().collect();
    modules.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    modules.truncate(limit);
    result["modules"] = json!(modules);
    result["languages"] = json!(languages);
    result["config_files"] = json!(index
        .metadata
        .iter()
        .filter(|p| in_scope(p, scope))
        .take(limit)
        .collect::<Vec<_>>());
    result["entry_points"] = json!(ranked
        .iter()
        .filter(|(_, f)| matches!(
            Path::new(&f.path).file_stem().and_then(|s| s.to_str()),
            Some("index" | "main" | "server" | "app")
        ))
        .take(limit)
        .map(|(_, f)| &f.path)
        .collect::<Vec<_>>());
    result
}

pub(super) fn relationships(index: &RepoIndex, symbol: &Symbol, limit: usize) -> Value {
    let mut results = Vec::new();
    for file in index.files.values() {
        for edge in &file.edges {
            let target = graph::edge_target(index, &file.path, &edge.source, &edge.target);
            if edge.source == symbol.id || target.is_some_and(|s| s.id == symbol.id) {
                results.push(
                    json!({"source":"ast","confidence":"structural","path":file.path,
                    "kind":edge.kind,"from":edge.source, "target":edge.target,
                    "target_symbol":target.map(|s| &s.id), "resolved":target.is_some(),
                    "direction":if edge.source == symbol.id {"outgoing"} else {"incoming"},
                    "line":edge.line}),
                );
            }
        }
    }
    envelope(results, limit)
}

impl RepoIntelligence {
    pub fn query(&self, name: &str, args: &Value) -> Result<Value, String> {
        super::tools::validate(name, args)?;
        let object = args.as_object().ok_or("query_invalid: object required")?;
        let limit = match object.get("limit") {
            None => self.config.max_results,
            Some(value) => value
                .as_u64()
                .filter(|v| *v > 0 && *v <= 100)
                .ok_or("result_limit_exceeded: limit must be 1..100")?
                as usize,
        }
        .min(self.config.max_results);
        let scope = match object.get("path") {
            None => ".".to_string(),
            Some(value) => {
                let raw = value.as_str().ok_or("invalid_path")?;
                let validated = self.validate_path(raw)?;
                let normalized = validated
                    .components()
                    .filter_map(|component| match component {
                        std::path::Component::Normal(part) => Some(part.to_string_lossy()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("/");
                if normalized.is_empty() {
                    ".".into()
                } else {
                    normalized
                }
            }
        };
        let query = object
            .get("query")
            .map(|v| v.as_str().ok_or("query_invalid"))
            .transpose()?
            .unwrap_or("");
        if query.len() > 2048 {
            return Err("query_invalid: query length".into());
        }
        let index = self.refresh()?;
        let mut result = match name {
            "repo_map" => repo_map(
                &index,
                &scope,
                object
                    .get("depth")
                    .map(|v| {
                        v.as_u64()
                            .filter(|d| *d > 0 && *d <= 12)
                            .ok_or("query_invalid: depth must be 1..12")
                    })
                    .transpose()?
                    .unwrap_or(3) as usize,
                limit,
            ),
            "symbol_search" => {
                if query.trim().is_empty() {
                    return Err("query_invalid: query required".into());
                }
                search(
                    &index,
                    query,
                    &scope,
                    object.get("kind").and_then(Value::as_str),
                    limit,
                )
            }
            "file_symbols" => {
                let file = index
                    .files
                    .get(&scope)
                    .ok_or("unsupported_language: file not indexed")?;
                envelope(
                    file.symbols
                        .iter()
                        .map(|s| symbol_evidence(s, "file outline"))
                        .collect(),
                    limit,
                )
            }
            "file_dependencies" => {
                if !index.files.contains_key(&scope) {
                    return Err("invalid_path: file not indexed".into());
                }
                dependencies(&index, &scope, limit)
            }
            "symbol_relationships" => {
                let id = object.get("symbolId").and_then(Value::as_str);
                let line = object.get("line").and_then(Value::as_u64);
                let symbol = index
                    .files
                    .values()
                    .flat_map(|f| &f.symbols)
                    .filter(|s| {
                        id.is_some_and(|id| s.id == id)
                            || (s.file == scope
                                && line.is_some_and(|line| {
                                    s.range.start_line as u64 <= line
                                        && s.range.end_line as u64 >= line
                                }))
                    })
                    .min_by_key(|s| s.range.end_line - s.range.start_line)
                    .ok_or("query_invalid: symbol not found")?;
                relationships(&index, symbol, limit)
            }
            "related_files" => {
                let by_symbol = object
                    .get("symbolId")
                    .and_then(Value::as_str)
                    .and_then(|id| {
                        index
                            .files
                            .values()
                            .flat_map(|f| &f.symbols)
                            .find(|s| s.id == id)
                    });
                if object.contains_key("symbolId") && by_symbol.is_none() {
                    return Err("query_invalid: symbol not found".into());
                }
                let origin = by_symbol
                    .map(|s| s.file.as_str())
                    .or_else(|| index.files.contains_key(&scope).then_some(scope.as_str()));
                related(
                    &index,
                    origin,
                    query,
                    if origin.is_some() { "." } else { &scope },
                    limit,
                )
            }
            "code_query" => self.unified_query(&index, query, &scope, limit, args)?,
            _ => return Err("query_invalid: unknown tool".into()),
        };
        result["warnings"] = json!(index.warnings);
        result["parse_failures"] = json!(index
            .files
            .values()
            .filter(|file| file.parse_status != "ok")
            .take(limit)
            .map(|file| json!({"path":file.path,"status":file.parse_status}))
            .collect::<Vec<_>>());
        if let Some(file) = index.files.get(&scope) {
            result["parse_status"] = json!(file.parse_status);
            result["unresolved"] = json!(file.unresolved);
        }
        result["index_updated_ms"] = json!(index.updated_ms);
        Ok(result)
    }
}
