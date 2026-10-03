//! Deterministic, bounded semantic records; never pass raw server JSON through.

use super::protocol::{IntelligenceError, Result};
use serde_json::{json, Value};
use std::path::Path;

fn invalid() -> IntelligenceError {
    IntelligenceError::new(
        "protocol_error",
        "Malformed language-server semantic response",
    )
}

pub(super) fn compact(text: &str, limit: usize) -> String {
    let mut out = String::new();
    for character in text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
    {
        if out.len() + character.len_utf8() > limit {
            break;
        }
        out.push(character);
    }
    out
}

fn normalized_range(range: &Value) -> Result<Value> {
    let point = |name| -> Result<Value> {
        let point = &range[name];
        let line = point["line"]
            .as_u64()
            .filter(|n| *n < u32::MAX as u64)
            .ok_or_else(invalid)?;
        let column = point["character"]
            .as_u64()
            .filter(|n| *n < u32::MAX as u64)
            .ok_or_else(invalid)?;
        Ok(json!({"line":line+1,"column":column+1}))
    };
    let start = point("start")?;
    let end = point("end")?;
    if (end["line"].as_u64(), end["column"].as_u64())
        < (start["line"].as_u64(), start["column"].as_u64())
    {
        return Err(invalid());
    }
    Ok(json!({"start":start,"end":end}))
}

fn location(workspace: &Path, uri: &str, range: &Value) -> Result<Option<Value>> {
    let uri = url::Url::parse(uri).map_err(|_| invalid())?;
    let Ok(path) = uri.to_file_path() else {
        return Ok(None);
    };
    // Never follow returned URIs to external files or expose absolute paths.
    let Ok(path) = path.canonicalize() else {
        return Ok(None);
    };
    let Ok(relative) = path.strip_prefix(workspace) else {
        return Ok(None);
    };
    Ok(Some(
        json!({"path":compact(&relative.to_string_lossy().replace('\\', "/"), 2048), "range":normalized_range(range)?}),
    ))
}

fn hover(raw: &Value) -> Result<Value> {
    if raw.is_null() {
        return Ok(json!({"text":"", "truncated":false}));
    }
    let contents = raw.get("contents").ok_or_else(invalid)?;
    let parts: Vec<&Value> = match contents.as_array() {
        Some(items) => items.iter().collect(),
        None => vec![contents],
    };
    let mut text = String::new();
    let mut total = 0usize;
    for part in parts {
        let part = part
            .as_str()
            .or_else(|| part.get("value").and_then(Value::as_str))
            .ok_or_else(invalid)?;
        total = total.saturating_add(part.len());
        if text.len() < 4096 {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&compact(part, 4096usize.saturating_sub(text.len())));
        }
    }
    Ok(json!({"text":text, "truncated":total > text.len()}))
}

#[cfg(test)]
pub(super) fn normalize(
    tool: &str,
    raw: Value,
    workspace: &Path,
    source_uri: Option<&str>,
    limit: usize,
) -> Result<Value> {
    normalize_retained(tool, raw, workspace, source_uri, limit, |_| None)
}

pub(super) fn normalize_retained(
    tool: &str,
    raw: Value,
    workspace: &Path,
    source_uri: Option<&str>,
    limit: usize,
    retain: impl FnOnce(&str) -> Option<String>,
) -> Result<Value> {
    if tool == "lsp_hover" {
        return hover(&raw);
    }
    let mut pending: Vec<&Value> = if raw.is_null() {
        Vec::new()
    } else if let Some(array) = raw.as_array() {
        array.iter().collect()
    } else if raw.is_object() {
        vec![&raw]
    } else {
        return Err(invalid());
    };
    let mut records = Vec::new();
    let mut omitted = 0usize;
    let mut visited = 0usize;
    while let Some(item) = pending.pop() {
        visited += 1;
        if visited > 100_000 {
            return Err(IntelligenceError::new(
                "response_too_large",
                "Too many semantic records",
            ));
        }
        let (uri, range) = match tool {
            "lsp_document_symbols" | "lsp_workspace_symbols" => {
                if let Some(children) = item.get("children") {
                    pending.extend(children.as_array().ok_or_else(invalid)?);
                }
                if let Some(loc) = item.get("location") {
                    (loc["uri"].as_str(), &loc["range"])
                } else {
                    (
                        source_uri,
                        item.get("selectionRange").unwrap_or(&item["range"]),
                    )
                }
            }
            "lsp_diagnostics" => (source_uri, &item["range"]),
            "lsp_definition" | "lsp_references" | "lsp_implementations" | "lsp_type_definition" => {
                if item.get("targetUri").is_some() {
                    (item["targetUri"].as_str(), &item["targetSelectionRange"])
                } else {
                    (item["uri"].as_str(), &item["range"])
                }
            }
            _ => return Err(invalid()),
        };
        let Some(mut record) = location(workspace, uri.ok_or_else(invalid)?, range)? else {
            omitted += 1;
            continue;
        };
        if matches!(tool, "lsp_document_symbols" | "lsp_workspace_symbols") {
            record["name"] = json!(compact(item["name"].as_str().ok_or_else(invalid)?, 256));
            record["kind"] = json!(item["kind"]
                .as_u64()
                .filter(|n| (1..=26).contains(n))
                .ok_or_else(invalid)?);
        }
        if tool == "lsp_diagnostics" {
            record["severity"] = json!(match item["severity"].as_u64() {
                Some(1) => "error",
                Some(2) => "warning",
                Some(3) => "information",
                Some(4) => "hint",
                None => "unknown",
                _ => return Err(invalid()),
            });
            record["message"] = json!(compact(item["message"].as_str().ok_or_else(invalid)?, 1024));
            if let Some(code) = item.get("code") {
                record["code"] = if let Some(code) = code.as_str() {
                    json!(compact(code, 128))
                } else if code.is_i64() {
                    code.clone()
                } else {
                    return Err(invalid());
                };
            }
            if let Some(source) = item.get("source") {
                record["source"] = json!(compact(source.as_str().ok_or_else(invalid)?, 128));
            }
        }
        records.push(record);
    }
    let key = |v: &Value| {
        (
            v["path"].as_str().unwrap_or("").to_owned(),
            v["range"]["start"]["line"].as_u64(),
            v["range"]["start"]["column"].as_u64(),
            v.to_string(),
        )
    };
    records.sort_by_cached_key(key);
    records.dedup();
    let total = records.len();
    let output_id = if total > limit.clamp(1, 200) {
        retain(
            &serde_json::to_string_pretty(
                &json!({"total":total,"items":records,"omittedExternal":omitted}),
            )
            .map_err(|_| invalid())?,
        )
    } else {
        None
    };
    records.truncate(limit.clamp(1, 200));
    let mut result = json!({"total":total,"remaining":total-records.len(),"items":records,"omittedExternal":omitted});
    if let Some(id) = output_id {
        result["fullResult"] = json!({"tool":"retrieve_output","id":id});
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.ts"), "x").unwrap();
        std::fs::write(root.join("b.ts"), "x").unwrap();
        let a = super::super::documents::file_uri(&root.join("a.ts")).unwrap();
        let b = super::super::documents::file_uri(&root.join("b.ts")).unwrap();
        (dir, root, a, b)
    }

    fn range(line: u64) -> Value {
        json!({"start":{"line":line,"character":2},"end":{"line":line,"character":5}})
    }

    #[test]
    fn locations_links_order_dedupe_caps_and_external_filter() {
        let (_dir, root, a, b) = fixture();
        let raw = json!([
            {"uri":b,"range":range(4)},
            {"targetUri":a,"targetRange":range(0),"targetSelectionRange":range(1)},
            {"uri":b,"range":range(4)},
            {"uri":"https://outside.example/secret.ts","range":range(1)}
        ]);
        for tool in [
            "lsp_definition",
            "lsp_references",
            "lsp_implementations",
            "lsp_type_definition",
        ] {
            let result = normalize(tool, raw.clone(), &root, None, 1).unwrap();
            assert_eq!(result["total"], 2);
            assert_eq!(result["remaining"], 1);
            assert_eq!(result["items"][0]["path"], "a.ts");
            assert_eq!(
                result["items"][0]["range"]["start"],
                json!({"line":2,"column":3})
            );
            assert_eq!(result["omittedExternal"], 1);
        }
    }

    #[test]
    fn hover_shapes_and_limits() {
        let (_dir, root, _, _) = fixture();
        for contents in [
            json!("greet(name: string): string"),
            json!({"kind":"markdown","value":"greet(name: string): string"}),
            json!([{ "language":"typescript","value":"greet(name: string): string"}]),
        ] {
            assert!(
                normalize("lsp_hover", json!({"contents":contents}), &root, None, 50).unwrap()
                    ["text"]
                    .as_str()
                    .unwrap()
                    .contains("greet")
            );
        }
        let result = normalize(
            "lsp_hover",
            json!({"contents":"🦀".repeat(20000)}),
            &root,
            None,
            50,
        )
        .unwrap();
        assert!(result["text"].as_str().unwrap().len() <= 4096);
        assert_eq!(result["truncated"], true);
        assert_eq!(
            normalize("lsp_hover", Value::Null, &root, None, 50).unwrap()["text"],
            ""
        );
    }

    #[test]
    fn nested_document_and_flat_workspace_symbols() {
        let (_dir, root, a, _) = fixture();
        let raw = json!([{ "name":"Container","kind":5,"range":range(0),"selectionRange":range(0),"children":[{"name":"greet","kind":12,"range":range(1),"selectionRange":range(1)}]}]);
        let result = normalize("lsp_document_symbols", raw, &root, Some(&a), 50).unwrap();
        assert_eq!(result["total"], 2);
        assert_eq!(result["items"][1]["name"], "greet");
        let raw = json!([{ "name":"greet","kind":12,"location":{"uri":a,"range":range(1)}}]);
        assert_eq!(
            normalize("lsp_workspace_symbols", raw, &root, None, 50).unwrap()["items"][0]["name"],
            "greet"
        );
    }

    #[test]
    fn diagnostics_are_compact_and_malformed_positions_fail() {
        let (_dir, root, a, _) = fixture();
        let raw = json!([{ "severity":1,"code":2322,"source":"ts","message":"wrong type","range":range(3),"data":{"secret":"not forwarded"}}]);
        let result = normalize("lsp_diagnostics", raw, &root, Some(&a), 100).unwrap();
        assert_eq!(result["items"][0]["severity"], "error");
        assert_eq!(result["items"][0]["code"], 2322);
        assert_eq!(result["items"][0]["source"], "ts");
        assert!(!result.to_string().contains("secret"));
        assert_eq!(
            normalize(
                "lsp_definition",
                json!({"uri":a,"range":{}}),
                &root,
                None,
                50
            )
            .unwrap_err()
            .code,
            "protocol_error"
        );
    }
}
