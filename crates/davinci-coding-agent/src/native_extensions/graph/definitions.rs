//! Validated native engineering graph definition parser, store, and bounds.
#![allow(dead_code, unused_imports)]

use super::topology::{EdgeCondition, EdgeDefinition, GraphMode, NodeDefinition};
use super::types::{ArtifactKind, Role};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

pub fn safe_graph_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && ![
            "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
            "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
        ]
        .contains(&lower.as_str())
}

pub fn safe_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedGraphTopology {
    pub graph_id: String,
    pub version: u32,
    pub mode: GraphMode,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub nodes: Vec<NodeDefinition>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub edges: Vec<EdgeDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedStageBinding {
    pub node_id: String,
    pub stage: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub input_artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedBudgets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedVerificationPolicy {
    pub command_profile: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedParameter {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub param_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub allowed_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedGraphDefinitionV1 {
    pub schema_version: u32,
    pub name: String,
    pub description: String,
    pub graph: SavedGraphTopology,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub bindings: Vec<SavedStageBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budgets: Option<SavedBudgets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_policy: Option<SavedVerificationPolicy>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub artifact_contract_versions: BTreeMap<String, u32>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub parameters: Vec<SavedParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for DefinitionParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for DefinitionParseError {}

pub fn compute_definition_digest(def: &SavedGraphDefinitionV1) -> String {
    let mut hasher = Sha256::new();
    let json_bytes = serde_json::to_vec(def).unwrap_or_default();
    hasher.update(&json_bytes);
    format!("{:x}", hasher.finalize())
}

pub fn has_case_collision(dir: &Path, name: &str) -> bool {
    let lower = name.to_lowercase();
    let target_filename = format!("{name}.yaml");
    let target_lower = format!("{lower}.yaml");
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_str = file_name.to_string_lossy();
            let file_lower = file_str.to_lowercase();
            if file_lower == target_lower && file_str != target_filename {
                return true;
            }
        }
    }
    false
}

pub fn validate_graphs_dir(path: &Path, workspace_root: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("graphs directory must not be a symlink".into());
    }
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let canonical_root = workspace_root.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(&canonical_root) {
        return Err("graphs directory path escapes workspace root".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Strict Streaming YAML 1.2 Event Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum YamlVal {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<YamlVal>),
    Map(Vec<(String, YamlVal)>),
}

impl YamlVal {
    fn to_json(&self) -> serde_json::Value {
        match self {
            YamlVal::Null => serde_json::Value::Null,
            YamlVal::Bool(b) => serde_json::Value::Bool(*b),
            YamlVal::Int(i) => serde_json::Value::Number((*i).into()),
            YamlVal::Float(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            YamlVal::String(s) => serde_json::Value::String(s.clone()),
            YamlVal::List(items) => {
                serde_json::Value::Array(items.iter().map(|item| item.to_json()).collect())
            }
            YamlVal::Map(entries) => {
                let mut map = serde_json::Map::new();
                for (k, v) in entries {
                    map.insert(k.clone(), v.to_json());
                }
                serde_json::Value::Object(map)
            }
        }
    }
}

#[derive(Debug, Clone)]
struct LineEntry {
    line_num: usize,
    indent: usize,
    raw: String,
}

pub fn parse_saved_definition_bytes(
    bytes: &[u8],
) -> Result<SavedGraphDefinitionV1, DefinitionParseError> {
    if bytes.len() > 256 * 1024 {
        return Err(DefinitionParseError {
            line: 1,
            column: 1,
            message: format!(
                "definition exceeds maximum size of 256 KiB (size: {} bytes)",
                bytes.len()
            ),
        });
    }
    let s = match std::str::from_utf8(bytes) {
        Ok(valid) => valid,
        Err(e) => {
            return Err(DefinitionParseError {
                line: 1,
                column: e.valid_up_to() + 1,
                message: "invalid UTF-8 encoding in definition".into(),
            });
        }
    };
    parse_saved_definition(s)
}

pub fn parse_saved_definition(
    yaml_str: &str,
) -> Result<SavedGraphDefinitionV1, DefinitionParseError> {
    let mut lines = Vec::new();
    let mut doc_start_count = 0;
    let mut seen_first_doc_content = false;

    for (idx, line_raw) in yaml_str.lines().enumerate() {
        let line_num = idx + 1;
        let trimmed = line_raw.trim();

        if trimmed == "---" {
            doc_start_count += 1;
            if doc_start_count > 1 || seen_first_doc_content {
                return Err(DefinitionParseError {
                    line: line_num,
                    column: 1,
                    message: "multiple YAML documents are prohibited".into(),
                });
            }
            continue;
        }
        if trimmed == "..." {
            continue;
        }

        let without_comment = if let Some(pos) = line_raw.find('#') {
            let before = &line_raw[..pos];
            let in_quotes = before.chars().filter(|&c| c == '"').count() % 2 != 0
                || before.chars().filter(|&c| c == '\'').count() % 2 != 0;
            if in_quotes {
                line_raw
            } else {
                before
            }
        } else {
            line_raw
        };

        let content_trimmed = without_comment.trim_end();
        if content_trimmed.trim().is_empty() {
            continue;
        }

        seen_first_doc_content = true;
        let indent = without_comment.chars().take_while(|c| *c == ' ').count();
        lines.push(LineEntry {
            line_num,
            indent,
            raw: content_trimmed.to_string(),
        });
    }

    if lines.is_empty() {
        return Err(DefinitionParseError {
            line: 1,
            column: 1,
            message: "empty definition document".into(),
        });
    }

    let first_trimmed = lines[0].raw.trim();
    let parsed_val = if first_trimmed.starts_with('{') || first_trimmed.starts_with('[') {
        let full_text = lines
            .iter()
            .map(|l| l.raw.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let mut idx = 0;
        let mut depth = 0;
        parse_flow_val(&full_text, &mut idx, lines[0].line_num, &mut depth)?
    } else {
        let mut idx = 0;
        let mut depth = 0;
        parse_block_val(&lines, &mut idx, &mut depth)?
    };

    let json_val = parsed_val.to_json();
    let def: SavedGraphDefinitionV1 =
        serde_json::from_value(json_val).map_err(|e| DefinitionParseError {
            line: 1,
            column: 1,
            message: format!("schema validation error: {e}"),
        })?;

    if def.schema_version != 1 {
        return Err(DefinitionParseError {
            line: 1,
            column: 1,
            message: format!(
                "unsupported schema_version: {}, expected 1",
                def.schema_version
            ),
        });
    }
    if !safe_graph_name(&def.name) {
        return Err(DefinitionParseError {
            line: 1,
            column: 1,
            message: format!("unsafe graph name: {}", def.name),
        });
    }
    if def.graph.nodes.len() > 64 {
        return Err(DefinitionParseError {
            line: 1,
            column: 1,
            message: format!(
                "maximum of 64 nodes exceeded (found {})",
                def.graph.nodes.len()
            ),
        });
    }
    if def.graph.edges.len() > 256 {
        return Err(DefinitionParseError {
            line: 1,
            column: 1,
            message: format!(
                "maximum of 256 edges exceeded (found {})",
                def.graph.edges.len()
            ),
        });
    }
    if def.parameters.len() > 32 {
        return Err(DefinitionParseError {
            line: 1,
            column: 1,
            message: format!(
                "maximum of 32 parameters exceeded (found {})",
                def.parameters.len()
            ),
        });
    }

    let mut seen_nodes = std::collections::HashSet::new();
    for node in &def.graph.nodes {
        if !safe_identifier(&node.id) {
            return Err(DefinitionParseError {
                line: 1,
                column: 1,
                message: format!("unsafe node identifier: {}", node.id),
            });
        }
        if !seen_nodes.insert(&node.id) {
            return Err(DefinitionParseError {
                line: 1,
                column: 1,
                message: format!("duplicate node id in graph: {}", node.id),
            });
        }
    }

    let graph_def = super::topology::GraphDefinition::from(&def.graph);
    super::topology::validate_definition(&graph_def).map_err(|e| DefinitionParseError {
        line: 1,
        column: 1,
        message: format!("topology validation error: {e}"),
    })?;

    for (artifact_name, &version) in &def.artifact_contract_versions {
        if !super::validate::is_known_artifact_contract(artifact_name, version) {
            return Err(DefinitionParseError {
                line: 1,
                column: 1,
                message: format!(
                    "unknown artifact contract kind or version: {artifact_name} v{version}"
                ),
            });
        }
    }

    const ALLOWED_PARAM_TYPES: &[&str] = &[
        "string",
        "goal",
        "enum",
        "path_list",
        "relative_paths",
        "paths",
        "integer",
        "budget_ceiling",
        "number",
        "boolean",
    ];

    let mut seen_params = std::collections::HashSet::new();
    for param in &def.parameters {
        if !safe_identifier(&param.name) {
            return Err(DefinitionParseError {
                line: 1,
                column: 1,
                message: format!("unsafe parameter identifier: {}", param.name),
            });
        }
        if !seen_params.insert(&param.name) {
            return Err(DefinitionParseError {
                line: 1,
                column: 1,
                message: format!("duplicate parameter name: {}", param.name),
            });
        }
        if !ALLOWED_PARAM_TYPES.contains(&param.param_type.as_str()) {
            return Err(DefinitionParseError {
                line: 1,
                column: 1,
                message: format!(
                    "unsupported parameter type '{}' for parameter '{}'",
                    param.param_type, param.name
                ),
            });
        }
        if param.param_type == "enum" {
            if param.allowed_values.is_empty() {
                return Err(DefinitionParseError {
                    line: 1,
                    column: 1,
                    message: format!("enum parameter '{}' must define allowed_values", param.name),
                });
            }
            if let Some(ref def_val) = param.default {
                if !param.allowed_values.contains(def_val) {
                    return Err(DefinitionParseError {
                        line: 1,
                        column: 1,
                        message: format!(
                            "default value '{def_val}' not in allowed_values for enum parameter '{}'",
                            param.name
                        ),
                    });
                }
            }
        }
        if matches!(
            param.param_type.as_str(),
            "paths" | "path_list" | "relative_paths"
        ) {
            for val in param.default.iter().chain(param.allowed_values.iter()) {
                if val.contains("..")
                    || val.starts_with('/')
                    || val.starts_with('\\')
                    || (val.len() > 1 && val.as_bytes()[1] == b':')
                {
                    return Err(DefinitionParseError {
                        line: 1,
                        column: 1,
                        message: format!(
                            "path parameter '{}' contains disallowed traversal or absolute path: '{val}'",
                            param.name
                        ),
                    });
                }
            }
        }
    }

    let node_ids: std::collections::HashSet<&str> =
        def.graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut seen_binding_nodes = std::collections::HashSet::new();
    for binding in &def.bindings {
        if !node_ids.contains(binding.node_id.as_str()) {
            return Err(DefinitionParseError {
                line: 1,
                column: 1,
                message: format!("stage binding references unknown node: {}", binding.node_id),
            });
        }
        if !seen_binding_nodes.insert(&binding.node_id) {
            return Err(DefinitionParseError {
                line: 1,
                column: 1,
                message: format!("duplicate stage binding for node: {}", binding.node_id),
            });
        }
    }

    Ok(def)
}

fn parse_block_val(
    lines: &[LineEntry],
    idx: &mut usize,
    depth: &mut usize,
) -> Result<YamlVal, DefinitionParseError> {
    *depth += 1;
    if *depth > 16 {
        return Err(DefinitionParseError {
            line: if *idx < lines.len() {
                lines[*idx].line_num
            } else {
                1
            },
            column: 1,
            message: "maximum nesting depth of 16 exceeded".into(),
        });
    }

    if *idx >= lines.len() {
        *depth -= 1;
        return Ok(YamlVal::Null);
    }

    let first_line = &lines[*idx];
    let trimmed = first_line.raw.trim();

    if trimmed.starts_with("- ") || trimmed == "-" {
        let mut list = Vec::new();
        let list_indent = first_line.indent;
        while *idx < lines.len() {
            let line = &lines[*idx];
            if line.indent < list_indent {
                break;
            }
            if line.indent == list_indent {
                let trimmed = line.raw.trim();
                if let Some(after_dash) = trimmed.strip_prefix('-') {
                    let item_str = after_dash.trim();
                    *idx += 1;
                    if item_str.is_empty() {
                        if *idx < lines.len() && lines[*idx].indent > list_indent {
                            let item_val = parse_block_val(lines, idx, depth)?;
                            list.push(item_val);
                        } else {
                            list.push(YamlVal::Null);
                        }
                    } else if item_str.starts_with('{') || item_str.starts_with('[') {
                        let mut flow_idx = 0;
                        let mut flow_depth = *depth;
                        let flow_val = parse_flow_val(
                            item_str,
                            &mut flow_idx,
                            line.line_num,
                            &mut flow_depth,
                        )?;
                        list.push(flow_val);
                    } else if let Some((k, v_inline)) = split_map_entry(item_str, line.line_num)? {
                        let mut map_entries = Vec::new();
                        let parsed_v = if v_inline.is_empty() {
                            if *idx < lines.len() && lines[*idx].indent > list_indent {
                                parse_block_val(lines, idx, depth)?
                            } else {
                                YamlVal::Null
                            }
                        } else {
                            parse_scalar_or_flow(v_inline, line.line_num, depth)?
                        };
                        map_entries.push((k, parsed_v));

                        while *idx < lines.len() {
                            let next_l = &lines[*idx];
                            if next_l.indent <= list_indent {
                                break;
                            }
                            let next_trimmed = next_l.raw.trim();
                            if next_trimmed.starts_with('-') {
                                break;
                            }
                            if let Some((nk, nv_inline)) =
                                split_map_entry(next_trimmed, next_l.line_num)?
                            {
                                if map_entries.iter().any(|(existing_k, _)| existing_k == &nk) {
                                    return Err(DefinitionParseError {
                                        line: next_l.line_num,
                                        column: next_l.indent + 1,
                                        message: format!("duplicate mapping key: {nk}"),
                                    });
                                }
                                *idx += 1;
                                let nv = if nv_inline.is_empty() {
                                    if *idx < lines.len() && lines[*idx].indent > next_l.indent {
                                        parse_block_val(lines, idx, depth)?
                                    } else {
                                        YamlVal::Null
                                    }
                                } else {
                                    parse_scalar_or_flow(nv_inline, next_l.line_num, depth)?
                                };
                                map_entries.push((nk, nv));
                            } else {
                                break;
                            }
                        }
                        list.push(YamlVal::Map(map_entries));
                    } else {
                        let val = parse_scalar_or_flow(item_str, line.line_num, depth)?;
                        list.push(val);
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        *depth -= 1;
        return Ok(YamlVal::List(list));
    }

    // Otherwise it's a mapping
    let mut map_entries = Vec::new();
    let map_indent = first_line.indent;
    while *idx < lines.len() {
        let line = &lines[*idx];
        if line.indent < map_indent {
            break;
        }
        if line.indent == map_indent {
            let trimmed = line.raw.trim();
            if let Some((k, v_inline)) = split_map_entry(trimmed, line.line_num)? {
                if map_entries.iter().any(|(existing_k, _)| existing_k == &k) {
                    return Err(DefinitionParseError {
                        line: line.line_num,
                        column: line.indent + 1,
                        message: format!("duplicate mapping key: {k}"),
                    });
                }
                *idx += 1;
                let val = if v_inline.is_empty() {
                    if *idx < lines.len() && lines[*idx].indent > map_indent {
                        parse_block_val(lines, idx, depth)?
                    } else {
                        YamlVal::Null
                    }
                } else {
                    parse_scalar_or_flow(v_inline, line.line_num, depth)?
                };
                map_entries.push((k, val));
            } else {
                break;
            }
        } else {
            break;
        }
    }

    *depth -= 1;
    Ok(YamlVal::Map(map_entries))
}

fn split_map_entry(
    s: &str,
    line_num: usize,
) -> Result<Option<(String, &str)>, DefinitionParseError> {
    let mut in_dquote = false;
    let mut in_squote = false;
    let chars: Vec<(usize, char)> = s.char_indices().collect();

    for (i, &(byte_idx, c)) in chars.iter().enumerate() {
        if c == '"' && !in_squote {
            in_dquote = !in_dquote;
        } else if c == '\'' && !in_dquote {
            in_squote = !in_squote;
        } else if c == ':' && !in_dquote && !in_squote {
            let is_end = i + 1 == chars.len();
            let has_space_after = !is_end && chars[i + 1].1 == ' ';
            if is_end || has_space_after {
                let key_raw = &s[..byte_idx].trim();
                let key = parse_scalar_string(key_raw, line_num)?;
                if key == "<<" {
                    return Err(DefinitionParseError {
                        line: line_num,
                        column: byte_idx + 1,
                        message: "YAML merge keys are prohibited".into(),
                    });
                }
                let val_raw = if is_end { "" } else { s[byte_idx + 1..].trim() };
                return Ok(Some((key, val_raw)));
            }
        }
    }
    Ok(None)
}

fn parse_scalar_or_flow(
    s: &str,
    line: usize,
    depth: &mut usize,
) -> Result<YamlVal, DefinitionParseError> {
    let trimmed = s.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        let mut idx = 0;
        parse_flow_val(trimmed, &mut idx, line, depth)
    } else {
        parse_scalar_val(trimmed, line)
    }
}

fn parse_scalar_string(s: &str, line: usize) -> Result<String, DefinitionParseError> {
    let trimmed = s.trim();
    if trimmed.starts_with('&') {
        return Err(DefinitionParseError {
            line,
            column: 1,
            message: "YAML anchors are prohibited".into(),
        });
    }
    if trimmed.starts_with('*') {
        return Err(DefinitionParseError {
            line,
            column: 1,
            message: "YAML aliases / alias bombs are prohibited".into(),
        });
    }
    if trimmed.starts_with('!') {
        return Err(DefinitionParseError {
            line,
            column: 1,
            message: "custom YAML tags are prohibited".into(),
        });
    }
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('u') => {
                        let digits = chars.by_ref().take(4).collect::<String>();
                        if digits.len() != 4 {
                            return Err(DefinitionParseError {
                                line,
                                column: 1,
                                message: "incomplete unicode escape".into(),
                            });
                        }
                        let value =
                            u32::from_str_radix(&digits, 16).map_err(|_| DefinitionParseError {
                                line,
                                column: 1,
                                message: "invalid unicode escape".into(),
                            })?;
                        let decoded =
                            char::from_u32(value).ok_or_else(|| DefinitionParseError {
                                line,
                                column: 1,
                                message: "invalid unicode scalar".into(),
                            })?;
                        out.push(decoded);
                    }
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        return Ok(out);
    }
    if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
        let inner = &trimmed[1..trimmed.len() - 1];
        return Ok(inner.replace("''", "'"));
    }
    Ok(trimmed.to_string())
}

fn parse_scalar_val(s: &str, line: usize) -> Result<YamlVal, DefinitionParseError> {
    let raw = s.trim();
    if raw.starts_with('&') {
        return Err(DefinitionParseError {
            line,
            column: 1,
            message: "YAML anchors are prohibited".into(),
        });
    }
    if raw.starts_with('*') {
        return Err(DefinitionParseError {
            line,
            column: 1,
            message: "YAML aliases / alias bombs are prohibited".into(),
        });
    }
    if raw.starts_with('!') {
        return Err(DefinitionParseError {
            line,
            column: 1,
            message: "custom YAML tags are prohibited".into(),
        });
    }

    let lower = raw.to_lowercase();
    if lower == ".nan"
        || lower == "nan"
        || lower == ".inf"
        || lower == "-.inf"
        || lower == "inf"
        || lower == "-inf"
        || lower == "+inf"
    {
        return Err(DefinitionParseError {
            line,
            column: 1,
            message: "NaN and Infinity are prohibited in saved definitions".into(),
        });
    }

    if lower == "true" {
        return Ok(YamlVal::Bool(true));
    }
    if lower == "false" {
        return Ok(YamlVal::Bool(false));
    }
    if lower == "null" || lower == "~" || raw.is_empty() {
        return Ok(YamlVal::Null);
    }

    if let Ok(i) = raw.parse::<i64>() {
        return Ok(YamlVal::Int(i));
    }
    if (raw.contains('.') || raw.contains('e') || raw.contains('E')) && raw.parse::<f64>().is_ok() {
        let f = raw.parse::<f64>().unwrap();
        if f.is_nan() || f.is_infinite() {
            return Err(DefinitionParseError {
                line,
                column: 1,
                message: "NaN and Infinity are prohibited in saved definitions".into(),
            });
        }
        return Ok(YamlVal::Float(f));
    }

    let parsed_str = parse_scalar_string(raw, line)?;
    Ok(YamlVal::String(parsed_str))
}

// ---------------------------------------------------------------------------
// Flow Parser for { ... } and [ ... ]
// ---------------------------------------------------------------------------

fn parse_flow_val(
    s: &str,
    idx: &mut usize,
    line: usize,
    depth: &mut usize,
) -> Result<YamlVal, DefinitionParseError> {
    *depth += 1;
    if *depth > 16 {
        return Err(DefinitionParseError {
            line,
            column: *idx + 1,
            message: "maximum nesting depth of 16 exceeded".into(),
        });
    }

    skip_flow_whitespace(s, idx);
    if *idx >= s.len() {
        *depth -= 1;
        return Ok(YamlVal::Null);
    }

    let c = s[*idx..].chars().next().unwrap();
    let res = match c {
        '{' => {
            *idx += 1;
            let mut entries = Vec::new();
            loop {
                skip_flow_whitespace(s, idx);
                if *idx >= s.len() {
                    return Err(DefinitionParseError {
                        line,
                        column: *idx + 1,
                        message: "unclosed flow mapping".into(),
                    });
                }
                if s[*idx..].starts_with('}') {
                    *idx += 1;
                    break;
                }
                let key_str = read_flow_scalar_or_quoted(s, idx, line)?;
                let key = parse_scalar_string(&key_str, line)?;
                if key == "<<" {
                    return Err(DefinitionParseError {
                        line,
                        column: *idx + 1,
                        message: "YAML merge keys are prohibited".into(),
                    });
                }
                if entries.iter().any(|(existing_k, _)| existing_k == &key) {
                    return Err(DefinitionParseError {
                        line,
                        column: *idx + 1,
                        message: format!("duplicate mapping key: {key}"),
                    });
                }

                skip_flow_whitespace(s, idx);
                if !s[*idx..].starts_with(':') {
                    return Err(DefinitionParseError {
                        line,
                        column: *idx + 1,
                        message: format!("expected ':' in flow mapping after key {key}"),
                    });
                }
                *idx += 1;
                skip_flow_whitespace(s, idx);

                let val = parse_flow_val(s, idx, line, depth)?;
                entries.push((key, val));

                skip_flow_whitespace(s, idx);
                if s[*idx..].starts_with(',') {
                    *idx += 1;
                } else if s[*idx..].starts_with('}') {
                    *idx += 1;
                    break;
                } else {
                    return Err(DefinitionParseError {
                        line,
                        column: *idx + 1,
                        message: "expected ',' or '}' in flow mapping".into(),
                    });
                }
            }
            Ok(YamlVal::Map(entries))
        }
        '[' => {
            *idx += 1;
            let mut items = Vec::new();
            loop {
                skip_flow_whitespace(s, idx);
                if *idx >= s.len() {
                    return Err(DefinitionParseError {
                        line,
                        column: *idx + 1,
                        message: "unclosed flow sequence".into(),
                    });
                }
                if s[*idx..].starts_with(']') {
                    *idx += 1;
                    break;
                }
                let val = parse_flow_val(s, idx, line, depth)?;
                items.push(val);

                skip_flow_whitespace(s, idx);
                if s[*idx..].starts_with(',') {
                    *idx += 1;
                } else if s[*idx..].starts_with(']') {
                    *idx += 1;
                    break;
                } else {
                    return Err(DefinitionParseError {
                        line,
                        column: *idx + 1,
                        message: "expected ',' or ']' in flow sequence".into(),
                    });
                }
            }
            Ok(YamlVal::List(items))
        }
        _ => {
            let scalar_str = read_flow_scalar_or_quoted(s, idx, line)?;
            parse_scalar_val(&scalar_str, line)
        }
    };

    *depth -= 1;
    res
}

fn skip_flow_whitespace(s: &str, idx: &mut usize) {
    while *idx < s.len() {
        let c = s[*idx..].chars().next().unwrap();
        if c.is_whitespace() {
            *idx += c.len_utf8();
        } else {
            break;
        }
    }
}

fn read_flow_scalar_or_quoted(
    s: &str,
    idx: &mut usize,
    line: usize,
) -> Result<String, DefinitionParseError> {
    skip_flow_whitespace(s, idx);
    if *idx >= s.len() {
        return Ok(String::new());
    }
    let c = s[*idx..].chars().next().unwrap();
    if c == '"' {
        let start = *idx;
        *idx += 1;
        while *idx < s.len() {
            let ch = s[*idx..].chars().next().unwrap();
            *idx += ch.len_utf8();
            if ch == '\\' && *idx < s.len() {
                let esc = s[*idx..].chars().next().unwrap();
                *idx += esc.len_utf8();
            } else if ch == '"' {
                return Ok(s[start..*idx].to_string());
            }
        }
        return Err(DefinitionParseError {
            line,
            column: start + 1,
            message: "unclosed double quote in flow scalar".into(),
        });
    }
    if c == '\'' {
        let start = *idx;
        *idx += 1;
        while *idx < s.len() {
            let ch = s[*idx..].chars().next().unwrap();
            *idx += ch.len_utf8();
            if ch == '\'' {
                if *idx < s.len() && s[*idx..].starts_with('\'') {
                    *idx += 1;
                } else {
                    return Ok(s[start..*idx].to_string());
                }
            }
        }
        return Err(DefinitionParseError {
            line,
            column: start + 1,
            message: "unclosed single quote in flow scalar".into(),
        });
    }

    let start = *idx;
    while *idx < s.len() {
        let ch = s[*idx..].chars().next().unwrap();
        if ch == ':' || ch == ',' || ch == '}' || ch == ']' || ch.is_whitespace() {
            break;
        }
        *idx += ch.len_utf8();
    }
    Ok(s[start..*idx].to_string())
}

fn yaml_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

pub fn to_yaml_string(def: &SavedGraphDefinitionV1) -> String {
    let mut out = String::new();
    out.push_str(&format!("schema_version: {}\n", def.schema_version));
    out.push_str(&format!("name: {}\n", yaml_quote(&def.name)));
    out.push_str(&format!("description: {}\n", yaml_quote(&def.description)));
    out.push_str("graph:\n");
    out.push_str(&format!(
        "  graph_id: {}\n",
        yaml_quote(&def.graph.graph_id)
    ));
    out.push_str(&format!("  version: {}\n", def.graph.version));
    out.push_str(&format!(
        "  mode: \"{}\"\n",
        match def.graph.mode {
            GraphMode::Simple => "simple",
            GraphMode::Standard => "standard",
            GraphMode::Complex => "complex",
        }
    ));
    if def.graph.nodes.is_empty() {
        out.push_str("  nodes: []\n");
    } else {
        out.push_str("  nodes:\n");
        for node in &def.graph.nodes {
            out.push_str(&format!("    - id: {}\n", yaml_quote(&node.id)));
            out.push_str(&format!("      role: {}\n", yaml_quote(node.role.as_str())));
            out.push_str(&format!(
                "      expect: {}\n",
                yaml_quote(node.expect.as_str())
            ));
            out.push_str(&format!("      required: {}\n", node.required));
            out.push_str(&format!("      allowsMutation: {}\n", node.allows_mutation));
        }
    }
    if def.graph.edges.is_empty() {
        out.push_str("  edges: []\n");
    } else {
        out.push_str("  edges:\n");
        for edge in &def.graph.edges {
            out.push_str(&format!("    - from: {}\n", yaml_quote(&edge.from)));
            out.push_str(&format!("      to: {}\n", yaml_quote(&edge.to)));
            out.push_str(&format!(
                "      condition: \"{}\"\n",
                match edge.condition {
                    EdgeCondition::Always => "always",
                    EdgeCondition::OnSuccess => "onSuccess",
                    EdgeCondition::OnFailure => "onFailure",
                }
            ));
        }
    }
    if def.bindings.is_empty() {
        out.push_str("bindings: []\n");
    } else {
        out.push_str("bindings:\n");
        for b in &def.bindings {
            out.push_str(&format!("  - node_id: {}\n", yaml_quote(&b.node_id)));
            out.push_str(&format!("    stage: {}\n", yaml_quote(&b.stage)));
            if b.input_artifacts.is_empty() {
                out.push_str("    input_artifacts: []\n");
            } else {
                out.push_str("    input_artifacts:\n");
                for art in &b.input_artifacts {
                    out.push_str(&format!("      - {}\n", yaml_quote(art)));
                }
            }
        }
    }
    if let Some(budgets) = &def.budgets {
        out.push_str("budgets:\n");
        if let Some(d) = budgets.max_duration_ms {
            out.push_str(&format!("  max_duration_ms: {}\n", d));
        }
        if let Some(c) = budgets.max_cost_usd {
            out.push_str(&format!("  max_cost_usd: {}\n", c));
        }
        if let Some(t) = budgets.max_tokens {
            out.push_str(&format!("  max_tokens: {}\n", t));
        }
    }
    if let Some(vp) = &def.verification_policy {
        out.push_str("verification_policy:\n");
        out.push_str(&format!(
            "  command_profile: {}\n",
            yaml_quote(&vp.command_profile)
        ));
        out.push_str(&format!("  required: {}\n", vp.required));
    }
    if def.artifact_contract_versions.is_empty() {
        out.push_str("artifact_contract_versions: {}\n");
    } else {
        out.push_str("artifact_contract_versions:\n");
        for (k, v) in &def.artifact_contract_versions {
            out.push_str(&format!("  {}: {}\n", yaml_quote(k), v));
        }
    }
    if def.parameters.is_empty() {
        out.push_str("parameters: []\n");
    } else {
        out.push_str("parameters:\n");
        for p in &def.parameters {
            out.push_str(&format!("  - name: {}\n", yaml_quote(&p.name)));
            out.push_str(&format!("    param_type: {}\n", yaml_quote(&p.param_type)));
            if let Some(d) = &p.description {
                out.push_str(&format!("    description: {}\n", yaml_quote(d)));
            }
            if let Some(def_val) = &p.default {
                out.push_str(&format!("    default: {}\n", yaml_quote(def_val)));
            }
            if p.allowed_values.is_empty() {
                out.push_str("    allowed_values: []\n");
            } else {
                out.push_str("    allowed_values:\n");
                for v in &p.allowed_values {
                    out.push_str(&format!("      - {}\n", yaml_quote(v)));
                }
            }
        }
    }
    out
}

pub fn role_contract_valid(role: &str, artifact: &str, mutates: bool) -> bool {
    match role {
        "writer" => artifact == "patch-report" && mutates,
        "reviewer" => artifact == "review" && !mutates,
        "planner" => artifact == "plan" && !mutates,
        "classifier" => artifact == "classification" && !mutates,
        "researcher" | "test-analyzer" | "historian" => artifact == "evidence" && !mutates,
        _ => false,
    }
}

pub fn role_contract_valid_typed(role: Role, artifact: ArtifactKind, mutates: bool) -> bool {
    role_contract_valid(role.as_str(), artifact.as_str(), mutates)
}

impl From<&SavedGraphTopology> for super::topology::GraphDefinition {
    fn from(t: &SavedGraphTopology) -> Self {
        super::topology::GraphDefinition {
            graph_id: t.graph_id.clone(),
            version: t.version,
            mode: t.mode,
            nodes: t.nodes.clone(),
            edges: t.edges.clone(),
        }
    }
}

pub fn save_destination_allowed(exists: bool, explicit_overwrite: bool, contained: bool) -> bool {
    contained && (!exists || explicit_overwrite)
}

pub fn project_graphs_dir(workspace_root: &Path) -> std::path::PathBuf {
    let davinci = workspace_root.join(super::store::CONFIG_DIR).join("graphs");
    if davinci.exists() {
        davinci
    } else {
        let pi = workspace_root
            .join(super::store::LEGACY_CONFIG_DIR)
            .join("graphs");
        if pi.exists() {
            pi
        } else {
            davinci
        }
    }
}

pub fn global_graphs_dir() -> Option<std::path::PathBuf> {
    let home = davinci_session::home_dir()?;
    let davinci = home
        .join(super::store::CONFIG_DIR)
        .join("agent")
        .join("graphs");
    if davinci.exists() {
        Some(davinci)
    } else {
        let pi = home
            .join(super::store::LEGACY_CONFIG_DIR)
            .join("agent")
            .join("graphs");
        if pi.exists() {
            Some(pi)
        } else {
            Some(davinci)
        }
    }
}

pub fn save_graph_definition_to_dir(
    dir: &Path,
    name: &str,
    def: &SavedGraphDefinitionV1,
    overwrite: bool,
    workspace_root: Option<&Path>,
) -> Result<std::path::PathBuf, String> {
    if !safe_graph_name(name) {
        return Err(format!("invalid graph definition name '{name}'"));
    }
    if let Some(ws_root) = workspace_root {
        validate_graphs_dir(dir, ws_root)?;
    }
    if has_case_collision(dir, name) {
        return Err(format!(
            "case-collision with existing graph definition in {}",
            dir.display()
        ));
    }

    let target_file = dir.join(format!("{name}.yaml"));
    let exists = target_file.exists();
    let contained = !name.contains('/') && !name.contains('\\') && !name.contains("..");

    if !save_destination_allowed(exists, overwrite, contained) {
        if !contained {
            return Err("graph destination path escapes directory".into());
        }
        return Err(format!(
            "graph definition '{}' already exists at {}; use --overwrite to replace it",
            name,
            target_file.display()
        ));
    }

    let yaml_str = to_yaml_string(def);
    super::store::atomic_write(&target_file, yaml_str.as_bytes())
        .map_err(|e| format!("failed to write graph definition: {e}"))?;

    Ok(target_file)
}

pub fn load_saved_definition(path: &Path) -> Result<SavedGraphDefinitionV1, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "failed to read graph definition from {}: {e}",
            path.display()
        )
    })?;
    parse_saved_definition(&raw).map_err(|e| {
        format!(
            "failed to parse graph definition at {}: {e}",
            path.display()
        )
    })
}

pub fn resolve_and_load_graph_definition(
    name: &str,
    cwd: &Path,
) -> Result<SavedGraphDefinitionV1, String> {
    if !safe_graph_name(name) {
        return Err(format!("invalid graph definition name '{name}'"));
    }

    // 1. Check project directory: <cwd>/.davinci/graphs/<name>.yaml
    let project_dir = project_graphs_dir(cwd);
    let project_file = project_dir.join(format!("{name}.yaml"));
    if project_file.exists() {
        return load_saved_definition(&project_file);
    }

    // 2. Check global directory: ~/.davinci/agent/graphs/<name>.yaml
    if let Some(global_dir) = global_graphs_dir() {
        let global_file = global_dir.join(format!("{name}.yaml"));
        if global_file.exists() {
            return load_saved_definition(&global_file);
        }
    }

    Err(format!(
        "graph definition '{name}' not found in project ({}) or global directory",
        project_dir.display()
    ))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedGraphSummary {
    pub name: String,
    pub description: String,
    pub digest: String,
    pub path: std::path::PathBuf,
    pub is_project: bool,
}

pub fn list_saved_definitions(cwd: &Path) -> Vec<SavedGraphSummary> {
    let mut summaries = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    // 1. Scan project graphs dir
    let project_dir = project_graphs_dir(cwd);
    if let Ok(entries) = std::fs::read_dir(&project_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension()
                .map(|e| e == "yaml" || e == "yml")
                .unwrap_or(false)
            {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    if safe_graph_name(stem) {
                        if let Ok(def) = load_saved_definition(&p) {
                            let digest = compute_definition_digest(&def);
                            summaries.push(SavedGraphSummary {
                                name: stem.to_string(),
                                description: def.description.clone(),
                                digest,
                                path: p.clone(),
                                is_project: true,
                            });
                            seen_names.insert(stem.to_string());
                        }
                    }
                }
            }
        }
    }

    // 2. Scan global graphs dir
    if let Some(global_dir) = global_graphs_dir() {
        if let Ok(entries) = std::fs::read_dir(&global_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension()
                    .map(|e| e == "yaml" || e == "yml")
                    .unwrap_or(false)
                {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        if safe_graph_name(stem) && !seen_names.contains(stem) {
                            if let Ok(def) = load_saved_definition(&p) {
                                let digest = compute_definition_digest(&def);
                                summaries.push(SavedGraphSummary {
                                    name: stem.to_string(),
                                    description: def.description.clone(),
                                    digest,
                                    path: p.clone(),
                                    is_project: false,
                                });
                                seen_names.insert(stem.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    summaries
}

pub fn is_safe_param_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !name.starts_with('-')
}

pub fn is_safe_param_value(val: &str) -> bool {
    !val.chars().any(|c| {
        matches!(
            c,
            '$' | ';' | '&' | '|' | '`' | '<' | '>' | '\n' | '\r' | '\0'
        )
    })
}

pub fn validate_param_value(
    name: &str,
    val: &str,
    param_type: &str,
    allowed_values: &[String],
) -> Result<(), String> {
    if !is_safe_param_value(val) {
        return Err(format!(
            "unsafe parameter value for '{name}': contains forbidden characters"
        ));
    }
    if !allowed_values.is_empty() && !allowed_values.iter().any(|allowed| allowed == val) {
        return Err(format!(
            "parameter '{name}' value '{val}' is not in allowed values: {:?}",
            allowed_values
        ));
    }
    match param_type {
        "path" | "scope" => {
            if val.contains("..")
                || val.starts_with('/')
                || val.starts_with('\\')
                || val.contains(':')
            {
                return Err(format!(
                    "parameter '{name}' must be a canonical relative scope/path without escaping '..' or drive letters: '{val}'"
                ));
            }
        }
        "integer" | "int" => {
            if val.parse::<i64>().is_err() {
                return Err(format!(
                    "parameter '{name}' expects integer value, got '{val}'"
                ));
            }
        }
        "boolean" | "bool" => {
            if val != "true" && val != "false" {
                return Err(format!(
                    "parameter '{name}' expects boolean value (true|false), got '{val}'"
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn validate_saved_definition(def: &SavedGraphDefinitionV1) -> Result<(), String> {
    let yaml = to_yaml_string(def);
    parse_saved_definition(&yaml)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn bind_parameters(
    def: &SavedGraphDefinitionV1,
    supplied: &std::collections::HashMap<String, String>,
) -> Result<std::collections::HashMap<String, String>, String> {
    for k in supplied.keys() {
        if !def.parameters.iter().any(|p| p.name == *k) {
            return Err(format!("unknown parameter '{k}' for graph '{}'", def.name));
        }
    }
    let mut bound = std::collections::HashMap::new();
    for param in &def.parameters {
        if let Some(val) = supplied.get(&param.name) {
            validate_param_value(&param.name, val, &param.param_type, &param.allowed_values)?;
            bound.insert(param.name.clone(), val.clone());
        } else if let Some(default) = &param.default {
            validate_param_value(
                &param.name,
                default,
                &param.param_type,
                &param.allowed_values,
            )?;
            bound.insert(param.name.clone(), default.clone());
        } else {
            return Err(format!(
                "missing required parameter '{}' for graph '{}'",
                param.name, def.name
            ));
        }
    }
    Ok(bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f12_safe_graph_name() {
        assert!(safe_graph_name("security-audit"));
        for bad in ["../x", "CON", "aux", "a/b", "name.", "", "x:y"] {
            assert!(!safe_graph_name(bad));
        }
    }

    #[test]
    fn test_ordinary_block_and_flow_yaml_roundtrip() {
        let sample_def = SavedGraphDefinitionV1 {
            schema_version: 1,
            name: "security-audit".into(),
            description: "Security audit pipeline".into(),
            graph: SavedGraphTopology {
                graph_id: "sec-1".into(),
                version: 1,
                mode: GraphMode::Standard,
                nodes: vec![
                    NodeDefinition {
                        id: "classify".into(),
                        role: Role::Classifier,
                        expect: ArtifactKind::Classification,
                        required: true,
                        allows_mutation: false,
                    },
                    NodeDefinition {
                        id: "verify".into(),
                        role: Role::Researcher,
                        expect: ArtifactKind::Evidence,
                        required: true,
                        allows_mutation: false,
                    },
                ],
                edges: vec![EdgeDefinition {
                    from: "classify".into(),
                    to: "verify".into(),
                    condition: EdgeCondition::Always,
                }],
            },
            bindings: vec![
                SavedStageBinding {
                    node_id: "classify".into(),
                    stage: "classify".into(),
                    input_artifacts: vec![],
                },
                SavedStageBinding {
                    node_id: "verify".into(),
                    stage: "verify".into(),
                    input_artifacts: vec!["classification".into()],
                },
            ],
            budgets: Some(SavedBudgets {
                max_duration_ms: Some(60000),
                max_cost_usd: Some(1.5),
                max_tokens: Some(100000),
            }),
            verification_policy: Some(SavedVerificationPolicy {
                command_profile: "cargo test".into(),
                required: true,
            }),
            artifact_contract_versions: {
                let mut m = BTreeMap::new();
                m.insert("classification".into(), 1);
                m
            },
            parameters: vec![SavedParameter {
                name: "profile".into(),
                description: Some("Test profile".into()),
                param_type: "enum".into(),
                default: Some("fast".into()),
                allowed_values: vec!["fast".into(), "full".into()],
            }],
        };

        // Block roundtrip
        let yaml_str = to_yaml_string(&sample_def);
        let parsed = parse_saved_definition(&yaml_str).expect("should parse generated block YAML");
        assert_eq!(parsed.name, sample_def.name);
        assert_eq!(parsed.graph.nodes.len(), 2);
        assert_eq!(parsed.bindings.len(), 2);
        assert_eq!(parsed.budgets.as_ref().unwrap().max_tokens, Some(100000));

        // Flow YAML parsing
        let flow_yaml = r#"{
            "schema_version": 1,
            "name": "flow-graph",
            "description": "Flow test",
            "graph": {
                "graph_id": "g-flow",
                "version": 1,
                "mode": "standard",
                "nodes": [
                    {"id": "node1", "role": "classifier", "expect": "classification", "required": true, "allowsMutation": false}
                ],
                "edges": []
            },
            "bindings": [
                {"node_id": "node1", "stage": "classify", "input_artifacts": []}
            ]
        }"#;
        let flow_parsed = parse_saved_definition(flow_yaml).expect("should parse flow YAML");
        assert_eq!(flow_parsed.name, "flow-graph");
        assert_eq!(flow_parsed.graph.nodes.len(), 1);
    }

    #[test]
    fn test_duplicate_key_rejected() {
        let yaml = r#"
schema_version: 1
name: "test"
name: "duplicate"
description: "desc"
"#;
        let err = parse_saved_definition(yaml).unwrap_err();
        assert!(
            err.message.contains("duplicate mapping key"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_nested_alias_bomb_rejected() {
        let yaml_anchor = r#"
schema_version: 1
name: &anch "test"
description: "desc"
"#;
        assert!(parse_saved_definition(yaml_anchor)
            .unwrap_err()
            .message
            .contains("anchor"));

        let yaml_alias = r#"
schema_version: 1
name: *alias_bomb
description: "desc"
"#;
        assert!(parse_saved_definition(yaml_alias)
            .unwrap_err()
            .message
            .contains("alias"));
    }

    #[test]
    fn test_custom_tag_rejected() {
        let yaml = r#"
schema_version: 1
name: !custom "tag"
description: "desc"
"#;
        let err = parse_saved_definition(yaml).unwrap_err();
        assert!(
            err.message.contains("custom YAML tags are prohibited"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_multidocument_rejected() {
        let yaml = r#"
---
schema_version: 1
name: "first"
description: "desc"
---
schema_version: 1
name: "second"
description: "desc"
"#;
        let err = parse_saved_definition(yaml).unwrap_err();
        assert!(
            err.message
                .contains("multiple YAML documents are prohibited"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_nan_infinity_rejected() {
        let yaml_nan = r#"
schema_version: 1
name: "test"
description: "desc"
budgets:
  max_cost_usd: .nan
"#;
        let err = parse_saved_definition(yaml_nan).unwrap_err();
        assert!(
            err.message.contains("NaN and Infinity are prohibited"),
            "got: {}",
            err.message
        );

        let yaml_inf = r#"
schema_version: 1
name: "test"
description: "desc"
budgets:
  max_cost_usd: .inf
"#;
        let err2 = parse_saved_definition(yaml_inf).unwrap_err();
        assert!(
            err2.message.contains("NaN and Infinity are prohibited"),
            "got: {}",
            err2.message
        );
    }

    #[test]
    fn test_utf8_error_rejected() {
        let invalid_utf8 = b"schema_version: 1\nname: \xff\xfe invalid";
        let err = parse_saved_definition_bytes(invalid_utf8).unwrap_err();
        assert!(
            err.message.contains("invalid UTF-8"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_depth_limit_rejected() {
        let mut yaml = String::from("schema_version: 1\nname: test\ndescription: desc\n");
        for i in 0..18 {
            yaml.push_str(&format!("{}level{}:\n", "  ".repeat(i), i));
        }
        yaml.push_str(&format!("{}val: 1\n", "  ".repeat(18)));

        let err = parse_saved_definition(&yaml).unwrap_err();
        assert!(err.message.contains("depth"), "got: {}", err.message);
    }

    #[test]
    fn test_65th_node_rejected() {
        let mut def = SavedGraphDefinitionV1 {
            schema_version: 1,
            name: "big-graph".into(),
            description: "65 nodes".into(),
            graph: SavedGraphTopology {
                graph_id: "big".into(),
                version: 1,
                mode: GraphMode::Standard,
                nodes: Vec::new(),
                edges: Vec::new(),
            },
            bindings: Vec::new(),
            budgets: None,
            verification_policy: None,
            artifact_contract_versions: BTreeMap::new(),
            parameters: Vec::new(),
        };

        for i in 0..65 {
            def.graph.nodes.push(NodeDefinition {
                id: format!("node-{i}"),
                role: Role::Classifier,
                expect: ArtifactKind::Classification,
                required: false,
                allows_mutation: false,
            });
            def.bindings.push(SavedStageBinding {
                node_id: format!("node-{i}"),
                stage: "classify".into(),
                input_artifacts: Vec::new(),
            });
        }

        let yaml = to_yaml_string(&def);
        let err = parse_saved_definition(&yaml).unwrap_err();
        assert!(
            err.message.contains("maximum of 64 nodes exceeded"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_case_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("security-audit.yaml");
        std::fs::write(&path, "content").unwrap();

        assert!(has_case_collision(tmp.path(), "Security-Audit"));
        assert!(has_case_collision(tmp.path(), "SECURITY-AUDIT"));
        assert!(!has_case_collision(tmp.path(), "security-audit"));
        assert!(!has_case_collision(tmp.path(), "other-graph"));
    }

    #[test]
    fn test_symlinked_graphs_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path().join("workspace");
        std::fs::create_dir_all(&ws_root).unwrap();

        let graphs_dir = ws_root.join(".davinci").join("graphs");
        std::fs::create_dir_all(&graphs_dir).unwrap();

        assert!(validate_graphs_dir(&graphs_dir, &ws_root).is_ok());
    }

    #[test]
    fn f12_role_artifact_coherence() {
        assert!(role_contract_valid("writer", "patch-report", true));
        assert!(role_contract_valid("reviewer", "review", false));
        assert!(!role_contract_valid("reviewer", "patch-report", true));
        assert!(!role_contract_valid("researcher", "evidence", true));
    }

    #[test]
    fn f12_unknown_artifact_contract_version() {
        let yaml = r#"
schema_version: 1
name: "test-contracts"
description: "testing contract version validation"
graph:
  graph_id: "test-g"
  version: 1
  mode: "simple"
  nodes:
    - id: "classify"
      role: "classifier"
      expect: "classification"
      required: true
      allowsMutation: false
  edges: []
artifact_contract_versions:
  unknown-artifact: 1
"#;
        let err = parse_saved_definition(yaml).unwrap_err();
        assert!(
            err.message
                .contains("unknown artifact contract kind or version"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn f12_definition_with_duplicate_node_id_rejected() {
        let yaml = r#"
schema_version: 1
name: "test-dup-nodes"
description: "testing duplicate node ids"
graph:
  graph_id: "test-g"
  version: 1
  mode: "simple"
  nodes:
    - id: "classify"
      role: "classifier"
      expect: "classification"
      required: true
      allowsMutation: false
    - id: "classify"
      role: "classifier"
      expect: "classification"
      required: false
      allowsMutation: false
  edges: []
"#;
        let err = parse_saved_definition(yaml).unwrap_err();
        assert!(
            err.message.contains("duplicate node id"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn f12_definition_with_reviewer_writer_privilege_rejected() {
        let yaml = r#"
schema_version: 1
name: "test-reviewer-mutation"
description: "testing reviewer writer privilege rejection"
graph:
  graph_id: "test-g"
  version: 1
  mode: "standard"
  nodes:
    - id: "classify"
      role: "classifier"
      expect: "classification"
      required: true
      allowsMutation: false
    - id: "review-1"
      role: "reviewer"
      expect: "review"
      required: true
      allowsMutation: true
  edges:
    - from: "classify"
      to: "review-1"
      condition: "always"
"#;
        let err = parse_saved_definition(yaml).unwrap_err();
        assert!(
            err.message
                .contains("reviewer node cannot have mutation privileges")
                || err
                    .message
                    .contains("incoherent role and artifact contract"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn f12_definition_with_failure_branch_bypass_rejected() {
        let yaml = r#"
schema_version: 1
name: "test-failure-bypass"
description: "testing failure branch bypassing review rejection"
graph:
  graph_id: "test-g"
  version: 1
  mode: "standard"
  nodes:
    - id: "classify"
      role: "classifier"
      expect: "classification"
      required: true
      allowsMutation: false
    - id: "implement-1"
      role: "writer"
      expect: "patch-report"
      required: true
      allowsMutation: true
    - id: "review-1"
      role: "reviewer"
      expect: "review"
      required: true
      allowsMutation: false
    - id: "fallback-exit"
      role: "researcher"
      expect: "evidence"
      required: false
      allowsMutation: false
  edges:
    - from: "classify"
      to: "implement-1"
      condition: "onSuccess"
    - from: "implement-1"
      to: "review-1"
      condition: "onSuccess"
    - from: "implement-1"
      to: "fallback-exit"
      condition: "onFailure"
"#;
        let err = parse_saved_definition(yaml).unwrap_err();
        assert!(
            err.message
                .contains("topology allows mutation without review"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn f12_definition_with_impossible_condition_rejected() {
        let yaml = r#"
schema_version: 1
name: "test-impossible-conjunction"
description: "testing impossible condition conjunction"
graph:
  graph_id: "test-g"
  version: 1
  mode: "simple"
  nodes:
    - id: "classify"
      role: "classifier"
      expect: "classification"
      required: true
      allowsMutation: false
    - id: "implement-1"
      role: "writer"
      expect: "patch-report"
      required: true
      allowsMutation: true
  edges:
    - from: "classify"
      to: "implement-1"
      condition: "onSuccess"
    - from: "classify"
      to: "implement-1"
      condition: "onFailure"
"#;
        let err = parse_saved_definition(yaml).unwrap_err();
        assert!(
            err.message.contains("impossible condition conjunction"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn f12_definition_parameter_allowlist() {
        let yaml_bad_type = r#"
schema_version: 1
name: "test-params"
description: "param testing"
graph:
  graph_id: "test-g"
  version: 1
  mode: "simple"
  nodes:
    - id: "classify"
      role: "classifier"
      expect: "classification"
      required: true
      allowsMutation: false
  edges: []
parameters:
  - name: "arbitrary_script"
    param_type: "bash_script"
"#;
        let err = parse_saved_definition(yaml_bad_type).unwrap_err();
        assert!(
            err.message.contains("unsupported parameter type"),
            "got: {}",
            err.message
        );

        let yaml_traversal_path = r#"
schema_version: 1
name: "test-params-path"
description: "path traversal param"
graph:
  graph_id: "test-g"
  version: 1
  mode: "simple"
  nodes:
    - id: "classify"
      role: "classifier"
      expect: "classification"
      required: true
      allowsMutation: false
  edges: []
parameters:
  - name: "scope"
    param_type: "paths"
    default: "../../escaped"
"#;
        let err2 = parse_saved_definition(yaml_traversal_path).unwrap_err();
        assert!(
            err2.message.contains("disallowed traversal"),
            "got: {}",
            err2.message
        );
    }

    #[test]
    fn f12_save_destination_allowed() {
        // Not contained -> always rejected
        assert!(!save_destination_allowed(false, false, false));
        assert!(!save_destination_allowed(true, true, false));

        // Contained, does not exist -> allowed
        assert!(save_destination_allowed(false, false, true));
        assert!(save_destination_allowed(false, true, true));

        // Contained, exists: only allowed if explicit_overwrite is true
        assert!(!save_destination_allowed(true, false, true));
        assert!(save_destination_allowed(true, true, true));
    }

    fn sample_valid_saved_def(name: &str) -> SavedGraphDefinitionV1 {
        SavedGraphDefinitionV1 {
            schema_version: 1,
            name: name.into(),
            description: "Test description".into(),
            graph: SavedGraphTopology {
                graph_id: "test-g".into(),
                version: 1,
                mode: GraphMode::Simple,
                nodes: vec![
                    NodeDefinition {
                        id: "classify".into(),
                        role: Role::Classifier,
                        expect: ArtifactKind::Classification,
                        required: true,
                        allows_mutation: false,
                    },
                    NodeDefinition {
                        id: "implement".into(),
                        role: Role::Writer,
                        expect: ArtifactKind::PatchReport,
                        required: true,
                        allows_mutation: true,
                    },
                ],
                edges: vec![EdgeDefinition {
                    from: "classify".into(),
                    to: "implement".into(),
                    condition: EdgeCondition::OnSuccess,
                }],
            },
            bindings: vec![
                SavedStageBinding {
                    node_id: "classify".into(),
                    stage: "classify".into(),
                    input_artifacts: vec![],
                },
                SavedStageBinding {
                    node_id: "implement".into(),
                    stage: "implement".into(),
                    input_artifacts: vec!["classify".into()],
                },
            ],
            budgets: None,
            verification_policy: None,
            artifact_contract_versions: BTreeMap::new(),
            parameters: vec![],
        }
    }

    #[test]
    fn saved_definition_round_trips_quotes_backslashes_and_newlines() {
        let mut def = sample_valid_saved_def("quoted-roundtrip");
        def.description = "say \"hi\" at C:\\temp\nsecond line".into();
        def.parameters = vec![SavedParameter {
            name: "target".into(),
            description: Some("line one\nline \"two\"".into()),
            param_type: "string".into(),
            default: Some("C:\\work\\file".into()),
            allowed_values: vec!["a\\b".into(), "say \"yes\"".into()],
        }];

        let yaml = to_yaml_string(&def);
        let parsed = parse_saved_definition(&yaml).unwrap();
        assert_eq!(parsed.description, def.description);
        assert_eq!(parsed.parameters, def.parameters);
    }

    #[test]
    fn test_atomic_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let def = sample_valid_saved_def("save-roundtrip");
        let path =
            save_graph_definition_to_dir(dir.path(), "save-roundtrip", &def, false, None).unwrap();
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), "save-roundtrip.yaml");

        let loaded = load_saved_definition(&path).unwrap();
        assert_eq!(loaded.name, "save-roundtrip");
        assert_eq!(
            compute_definition_digest(&loaded),
            compute_definition_digest(&def)
        );
    }

    #[test]
    fn test_collision_refused_without_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let def = sample_valid_saved_def("collision-test");
        save_graph_definition_to_dir(dir.path(), "collision-test", &def, false, None).unwrap();

        // Second save without overwrite fails
        let err = save_graph_definition_to_dir(dir.path(), "collision-test", &def, false, None)
            .unwrap_err();
        assert!(err.contains("already exists"));

        // Save with overwrite succeeds
        let mut def2 = def.clone();
        def2.description = "Updated description".into();
        save_graph_definition_to_dir(dir.path(), "collision-test", &def2, true, None).unwrap();
        let loaded = load_saved_definition(&dir.path().join("collision-test.yaml")).unwrap();
        assert_eq!(loaded.description, "Updated description");
    }

    #[test]
    fn test_case_collision_refused() {
        let dir = tempfile::tempdir().unwrap();
        let def = sample_valid_saved_def("my-pipe");
        save_graph_definition_to_dir(dir.path(), "my-pipe", &def, false, None).unwrap();

        let mut def2 = def.clone();
        def2.name = "My-Pipe".into();
        let err =
            save_graph_definition_to_dir(dir.path(), "My-Pipe", &def2, false, None).unwrap_err();
        assert!(err.contains("case-collision"));
    }

    #[test]
    fn test_invalid_and_escaping_name_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let def = sample_valid_saved_def("valid");
        for bad_name in [
            "../escape",
            "a/b",
            "a\\b",
            "CON",
            "",
            "name with space",
            "has:colon",
        ] {
            let err = save_graph_definition_to_dir(dir.path(), bad_name, &def, false, None);
            assert!(err.is_err(), "expected error for bad name '{bad_name}'");
        }
    }

    #[test]
    fn test_list_and_resolve_project_shadows_global() {
        let workspace = tempfile::tempdir().unwrap();
        let project_dir = workspace.path().join(".davinci").join("graphs");
        std::fs::create_dir_all(&project_dir).unwrap();

        let def_project = sample_valid_saved_def("shadow-pipe");
        save_graph_definition_to_dir(&project_dir, "shadow-pipe", &def_project, false, None)
            .unwrap();

        let resolved = resolve_and_load_graph_definition("shadow-pipe", workspace.path()).unwrap();
        assert_eq!(resolved.name, "shadow-pipe");

        let list = list_saved_definitions(workspace.path());
        assert!(list.iter().any(|s| s.name == "shadow-pipe" && s.is_project));
    }

    #[test]
    fn test_parameter_binding_success_and_defaults() {
        let mut def = sample_valid_saved_def("param-test");
        def.parameters = vec![
            SavedParameter {
                name: "scope".into(),
                description: Some("target scope".into()),
                param_type: "scope".into(),
                default: Some("crates/agent".into()),
                allowed_values: vec![],
            },
            SavedParameter {
                name: "max_workers".into(),
                description: None,
                param_type: "integer".into(),
                default: Some("4".into()),
                allowed_values: vec![],
            },
        ];

        let mut supplied = std::collections::HashMap::new();
        supplied.insert("scope".into(), "crates/davinci-coding-agent".into());

        let bound = bind_parameters(&def, &supplied).unwrap();
        assert_eq!(bound.get("scope").unwrap(), "crates/davinci-coding-agent");
        assert_eq!(bound.get("max_workers").unwrap(), "4"); // default used
    }

    #[test]
    fn test_parameter_binding_extra_unknown_rejected() {
        let def = sample_valid_saved_def("param-test");
        let mut supplied = std::collections::HashMap::new();
        supplied.insert("unknown_param".into(), "value".into());

        let err = bind_parameters(&def, &supplied).unwrap_err();
        assert!(err.contains("unknown parameter 'unknown_param'"));
    }

    #[test]
    fn test_parameter_binding_unsafe_characters_rejected() {
        let mut def = sample_valid_saved_def("param-test");
        def.parameters = vec![SavedParameter {
            name: "target".into(),
            description: None,
            param_type: "string".into(),
            default: None,
            allowed_values: vec![],
        }];

        for bad in [
            "$(whoami)",
            "target; rm -rf /",
            "`id`",
            "foo && bar",
            "val\nevil",
        ] {
            let mut supplied = std::collections::HashMap::new();
            supplied.insert("target".into(), bad.into());
            let err = bind_parameters(&def, &supplied).unwrap_err();
            assert!(
                err.contains("unsafe parameter value"),
                "bad value '{bad}' should be rejected"
            );
        }
    }

    #[test]
    fn test_parameter_binding_path_escape_rejected() {
        let mut def = sample_valid_saved_def("param-test");
        def.parameters = vec![SavedParameter {
            name: "rel_path".into(),
            description: None,
            param_type: "path".into(),
            default: None,
            allowed_values: vec![],
        }];

        for bad_path in [
            "../secret",
            "/etc/passwd",
            "\\windows\\system32",
            "C:\\escaped",
        ] {
            let mut supplied = std::collections::HashMap::new();
            supplied.insert("rel_path".into(), bad_path.into());
            let err = bind_parameters(&def, &supplied).unwrap_err();
            assert!(
                err.contains("canonical relative scope/path"),
                "bad path '{bad_path}' should be rejected"
            );
        }
    }

    #[test]
    fn test_parameter_binding_path_with_spaces_accepted() {
        let mut def = sample_valid_saved_def("param-test");
        def.parameters = vec![SavedParameter {
            name: "rel_path".into(),
            description: None,
            param_type: "path".into(),
            default: None,
            allowed_values: vec![],
        }];

        let mut supplied = std::collections::HashMap::new();
        supplied.insert("rel_path".into(), "crates/my folder/file.rs".into());
        let bound = bind_parameters(&def, &supplied).unwrap();
        assert_eq!(bound.get("rel_path").unwrap(), "crates/my folder/file.rs");
    }

    #[test]
    fn test_parameter_binding_allowed_values_and_missing_required() {
        let mut def = sample_valid_saved_def("param-test");
        def.parameters = vec![SavedParameter {
            name: "mode".into(),
            description: None,
            param_type: "string".into(),
            default: None,
            allowed_values: vec!["fast".into(), "thorough".into()],
        }];

        // Missing required
        let empty = std::collections::HashMap::new();
        let err = bind_parameters(&def, &empty).unwrap_err();
        assert!(err.contains("missing required parameter 'mode'"));

        // Disallowed value
        let mut supplied = std::collections::HashMap::new();
        supplied.insert("mode".into(), "invalid".into());
        let err = bind_parameters(&def, &supplied).unwrap_err();
        assert!(err.contains("not in allowed values"));

        // Allowed value
        supplied.insert("mode".into(), "fast".into());
        let bound = bind_parameters(&def, &supplied).unwrap();
        assert_eq!(bound.get("mode").unwrap(), "fast");
    }
}
