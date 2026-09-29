//! `--output-schema <file>`: the final answer of a print or json run has to be
//! JSON that matches a schema. No TS counterpart; the flag mirrors Codex
//! `exec --output-schema`.
//!
//! OpenAI routes also constrain the answer on the wire (`davinci-ai`
//! `StreamOptions::output_schema`). Every provider's answer is checked here,
//! because most providers have no such mode and a wire constraint is not proof.
//!
//! The validator covers the subset of JSON Schema that structured-output
//! schemas use: `type` (a name or a list of names), `properties`, `required`,
//! `additionalProperties` (a boolean or a schema), `items`, `minItems`,
//! `maxItems`, `enum`, `const` and `anyOf`, plus boolean schemas. Other
//! keywords (`$ref`, `pattern`, `format`, ...) are not checked. A schema that
//! relies on them is passed to the provider as written and simply validated
//! less strictly here.

use serde_json::Value;
use std::path::Path;

/// The repair prompt lists at most this many errors, so one badly broken
/// answer cannot turn the follow-up request into a wall of text.
pub const MAX_REPORTED_ERRORS: usize = 20;

/// Reads and parses the schema file. The schema must be a JSON object, as
/// the OpenAI structured-output formats require.
pub fn load_schema(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("--output-schema: could not read {}: {err}", path.display()))?;
    let schema: Value = serde_json::from_str(&text).map_err(|err| {
        format!(
            "--output-schema: {} is not valid JSON: {err}",
            path.display()
        )
    })?;
    if !schema.is_object() {
        return Err(format!(
            "--output-schema: {} must hold a JSON object (a JSON schema)",
            path.display()
        ));
    }
    Ok(schema)
}

/// The JSON text inside a reply: the whole reply, or the body of one fenced
/// code block (```` ```json ```` or a bare fence) that is the whole reply.
/// Surrounding whitespace is ignored. Prose around the JSON is not accepted.
pub fn extract_json(reply: &str) -> Option<&str> {
    let trimmed = reply.trim();
    let Some(fenced) = trimmed.strip_prefix("```") else {
        return Some(trimmed);
    };
    let inner = fenced.strip_suffix("```")?;
    let (info, body) = inner.split_once('\n')?;
    let info = info.trim();
    if !(info.is_empty() || info.eq_ignore_ascii_case("json")) || body.contains("```") {
        return None;
    }
    Some(body.trim())
}

/// Checks a final reply against the schema. `Ok` carries the JSON text
/// (fence removed) that stdout and `--output-last-message` then hold; `Err`
/// carries the problems, at most [`MAX_REPORTED_ERRORS`] of them.
pub fn check_reply(schema: &Value, reply: &str) -> Result<String, Vec<String>> {
    let Some(text) = extract_json(reply) else {
        return Err(vec![
            "the reply is not a single JSON value: remove any prose and code fences".into(),
        ]);
    };
    let value: Value = serde_json::from_str(text)
        .map_err(|err| vec![format!("the reply is not valid JSON: {err}")])?;
    let mut errors = validate(schema, &value);
    if errors.is_empty() {
        return Ok(text.to_string());
    }
    errors.truncate(MAX_REPORTED_ERRORS);
    Err(errors)
}

/// Every place `value` breaks `schema`, as `<path>: <problem>` lines with a
/// JSONPath-like location (`$`, `$.name`, `$.items[2]`).
pub fn validate(schema: &Value, value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    validate_at(schema, value, "$", &mut errors);
    errors
}

/// The follow-up user message of the single repair turn.
pub fn repair_prompt(errors: &[String]) -> String {
    let listed: Vec<String> = errors.iter().map(|error| format!("- {error}")).collect();
    format!(
        "Your final answer does not match the required JSON schema:\n{}\n\n\
         Reply with only the corrected JSON value that matches the schema. \
         No prose, no code fences, no tool calls.",
        listed.join("\n")
    )
}

fn validate_at(schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    let schema = match schema {
        Value::Bool(true) => return,
        Value::Bool(false) => {
            errors.push(format!("{path}: no value is allowed here"));
            return;
        }
        Value::Object(schema) => schema,
        // Not a schema at all; nothing a value could be checked against.
        _ => return,
    };

    if let Some(expected) = schema.get("type") {
        let names: Vec<&str> = match expected {
            Value::String(name) => vec![name.as_str()],
            Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !names.is_empty() && !names.iter().any(|name| has_type(value, name)) {
            errors.push(format!(
                "{path}: expected {}, got {}",
                names.join(" or "),
                type_name(value)
            ));
            // The remaining keywords describe a value of the right type.
            return;
        }
    }

    if let Some(expected) = schema.get("const") {
        if expected != value {
            errors.push(format!("{path}: expected the constant {expected}"));
        }
    }

    if let Some(Value::Array(allowed)) = schema.get("enum") {
        if !allowed.contains(value) {
            let listed: Vec<String> = allowed.iter().map(Value::to_string).collect();
            errors.push(format!(
                "{path}: {value} is not one of {}",
                listed.join(", ")
            ));
        }
    }

    if let Some(Value::Array(branches)) = schema.get("anyOf") {
        let matches_one = branches.iter().any(|branch| {
            let mut scratch = Vec::new();
            validate_at(branch, value, path, &mut scratch);
            scratch.is_empty()
        });
        if !branches.is_empty() && !matches_one {
            errors.push(format!("{path}: matches none of the anyOf alternatives"));
        }
    }

    if let Value::Object(object) = value {
        let properties = schema.get("properties").and_then(Value::as_object);
        if let Some(Value::Array(required)) = schema.get("required") {
            for name in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(name) {
                    errors.push(format!("{path}: missing required property \"{name}\""));
                }
            }
        }
        for (key, item) in object {
            let child = format!("{path}.{key}");
            match properties.and_then(|properties| properties.get(key)) {
                Some(property) => validate_at(property, item, &child, errors),
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => {
                        errors.push(format!("{path}: property \"{key}\" is not allowed"))
                    }
                    Some(extra @ Value::Object(_)) => validate_at(extra, item, &child, errors),
                    _ => {}
                },
            }
        }
    }

    if let Value::Array(items) = value {
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
            if (items.len() as u64) < min {
                errors.push(format!(
                    "{path}: expected at least {min} items, got {}",
                    items.len()
                ));
            }
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
            if items.len() as u64 > max {
                errors.push(format!(
                    "{path}: expected at most {max} items, got {}",
                    items.len()
                ));
            }
        }
        if let Some(item_schema) = schema.get("items") {
            for (index, item) in items.iter().enumerate() {
                validate_at(item_schema, item, &format!("{path}[{index}]"), errors);
            }
        }
    }
}

fn has_type(value: &Value, name: &str) -> bool {
    match name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => match value {
            Value::Number(number) => {
                number.is_i64()
                    || number.is_u64()
                    || number.as_f64().is_some_and(|float| float.fract() == 0.0)
            }
            _ => false,
        },
        // An unknown type name cannot be met.
        _ => false,
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn errors(schema: Value, value: Value) -> Vec<String> {
        validate(&schema, &value)
    }

    #[test]
    fn type_accepts_matching_values_and_names_the_mismatch() {
        let cases = [
            ("string", json!("x"), true),
            ("string", json!(1), false),
            ("integer", json!(3), true),
            ("integer", json!(3.0), true),
            ("integer", json!(3.5), false),
            ("number", json!(3.5), true),
            ("boolean", json!(false), true),
            ("null", json!(null), true),
            ("object", json!({}), true),
            ("array", json!([]), true),
            ("array", json!({}), false),
            ("mystery", json!(1), false),
        ];
        for (name, value, ok) in cases {
            let found = errors(json!({"type": name}), value.clone());
            assert_eq!(found.is_empty(), ok, "{name} {value}: {found:?}");
        }
        assert_eq!(
            errors(json!({"type": "string"}), json!(1)),
            vec!["$: expected string, got integer"]
        );
    }

    #[test]
    fn type_lists_accept_any_member() {
        let schema = json!({"type": ["string", "null"]});
        assert!(errors(schema.clone(), json!("x")).is_empty());
        assert!(errors(schema.clone(), json!(null)).is_empty());
        assert_eq!(
            errors(schema, json!(7)),
            vec!["$: expected string or null, got integer"]
        );
    }

    #[test]
    fn required_and_properties_report_nested_paths() {
        let schema = json!({
            "type": "object",
            "required": ["user"],
            "properties": {
                "user": {
                    "type": "object",
                    "required": ["name", "age"],
                    "properties": {
                        "name": {"type": "string"},
                        "age": {"type": "integer"}
                    }
                }
            }
        });
        assert!(errors(schema.clone(), json!({"user": {"name": "a", "age": 1}})).is_empty());
        assert_eq!(
            errors(schema.clone(), json!({})),
            vec!["$: missing required property \"user\""]
        );
        assert_eq!(
            errors(schema.clone(), json!({"user": {"name": 5}})),
            vec![
                "$.user: missing required property \"age\"",
                "$.user.name: expected string, got integer",
            ]
        );
    }

    #[test]
    fn enum_and_const_reject_other_values() {
        let schema = json!({"enum": ["red", "green"]});
        assert!(errors(schema.clone(), json!("red")).is_empty());
        assert_eq!(
            errors(schema, json!("blue")),
            vec!["$: \"blue\" is not one of \"red\", \"green\""]
        );
        let schema = json!({"properties": {"v": {"const": 2}}});
        assert!(errors(schema.clone(), json!({"v": 2})).is_empty());
        assert_eq!(
            errors(schema, json!({"v": 3})),
            vec!["$.v: expected the constant 2"]
        );
    }

    #[test]
    fn additional_properties_false_or_schema() {
        let closed = json!({"properties": {"a": {}}, "additionalProperties": false});
        assert!(errors(closed.clone(), json!({"a": 1})).is_empty());
        assert_eq!(
            errors(closed, json!({"a": 1, "b": 2})),
            vec!["$: property \"b\" is not allowed"]
        );
        let typed = json!({"additionalProperties": {"type": "number"}});
        assert!(errors(typed.clone(), json!({"x": 1.5})).is_empty());
        assert_eq!(
            errors(typed, json!({"x": "no"})),
            vec!["$.x: expected number, got string"]
        );
        // Absent means allowed.
        assert!(errors(json!({"properties": {}}), json!({"z": 1})).is_empty());
    }

    #[test]
    fn items_and_item_counts() {
        let schema = json!({
            "type": "array",
            "minItems": 1,
            "maxItems": 2,
            "items": {"type": "object", "required": ["id"]}
        });
        assert!(errors(schema.clone(), json!([{"id": 1}])).is_empty());
        assert_eq!(
            errors(schema.clone(), json!([])),
            vec!["$: expected at least 1 items, got 0"]
        );
        assert_eq!(
            errors(schema.clone(), json!([{"id": 1}, {}, {"id": 3}])),
            vec![
                "$: expected at most 2 items, got 3",
                "$[1]: missing required property \"id\"",
            ]
        );
    }

    #[test]
    fn any_of_and_boolean_schemas() {
        let schema = json!({"anyOf": [{"type": "string"}, {"type": "integer"}]});
        assert!(errors(schema.clone(), json!(1)).is_empty());
        assert_eq!(
            errors(schema, json!(true)),
            vec!["$: matches none of the anyOf alternatives"]
        );
        assert!(errors(json!(true), json!({"anything": 1})).is_empty());
        assert_eq!(
            errors(json!({"properties": {"gone": false}}), json!({"gone": 1})),
            vec!["$.gone: no value is allowed here"]
        );
    }

    #[test]
    fn extract_json_accepts_whitespace_and_one_fence_only() {
        assert_eq!(extract_json("  {\"a\":1}\n"), Some("{\"a\":1}"));
        assert_eq!(extract_json("```json\n{\"a\":1}\n```"), Some("{\"a\":1}"));
        assert_eq!(extract_json("\n```\n[1]\n```\n"), Some("[1]"));
        assert_eq!(extract_json("```python\n{}\n```"), None);
        assert_eq!(extract_json("```json\n{}\n```\n```json\n{}\n```"), None);
        assert_eq!(extract_json("```json {} ```"), None);
    }

    #[test]
    fn check_reply_returns_the_json_text_or_the_errors() {
        let schema = json!({"type": "object", "required": ["ok"]});
        assert_eq!(
            check_reply(&schema, "```json\n{\"ok\": true}\n```"),
            Ok("{\"ok\": true}".to_string())
        );
        assert_eq!(
            check_reply(&schema, "{}"),
            Err(vec!["$: missing required property \"ok\"".to_string()])
        );
        let prose = check_reply(&schema, "Here it is: {\"ok\": true}").unwrap_err();
        assert!(
            prose[0].starts_with("the reply is not valid JSON"),
            "{prose:?}"
        );
        let many = json!({"type": "array", "items": {"type": "string"}});
        let numbers: Vec<u32> = (0..50).collect();
        let found = check_reply(&many, &serde_json::to_string(&numbers).unwrap()).unwrap_err();
        assert_eq!(found.len(), MAX_REPORTED_ERRORS);
    }

    #[test]
    fn load_schema_reports_missing_invalid_and_non_object_files() {
        let dir = tempfile::tempdir().unwrap();
        let missing = load_schema(&dir.path().join("none.json")).unwrap_err();
        assert!(missing.contains("could not read"), "{missing}");
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{not json").unwrap();
        assert!(load_schema(&bad).unwrap_err().contains("is not valid JSON"));
        let list = dir.path().join("list.json");
        std::fs::write(&list, "[1]").unwrap();
        assert!(load_schema(&list)
            .unwrap_err()
            .contains("must hold a JSON object"));
        let good = dir.path().join("good.json");
        std::fs::write(&good, r#"{"type":"object"}"#).unwrap();
        assert_eq!(load_schema(&good).unwrap(), json!({"type": "object"}));
    }

    #[test]
    fn repair_prompt_lists_every_error() {
        let prompt = repair_prompt(&["$: a".into(), "$.b: c".into()]);
        assert!(prompt.contains("- $: a\n- $.b: c"), "{prompt}");
        assert!(prompt.contains("only the corrected JSON"));
    }
}
