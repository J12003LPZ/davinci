use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::types::SessionEntry;
use crate::{JsonlSession, SessionError};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionUsageStats {
    pub user_messages: u64,
    pub assistant_messages: u64,
    pub tool_calls: u64,
    pub tool_results: u64,
    pub total_messages: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

impl SessionUsageStats {
    pub fn token_total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

pub fn resolved_labels(entries: &[SessionEntry]) -> HashMap<String, (Option<String>, Option<u64>)> {
    let mut labels = HashMap::new();
    for entry in entries {
        if entry.entry_type != "label" {
            continue;
        }
        let Some(target) = entry
            .extra
            .get("targetId")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let label = entry
            .extra
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_string);
        labels.insert(target, (label, Some(entry.timestamp)));
    }
    labels
}

/// TS `buildSessionPath`: walk parent chain from `leaf_id`. Missing/unset leaf
/// falls back to the last entry (not `getBranch`, which returns empty).
pub fn build_session_path<'a>(
    entries: &'a [SessionEntry],
    leaf_id: Option<&str>,
) -> Vec<&'a SessionEntry> {
    let leaf = match leaf_id {
        Some(id) => entries
            .iter()
            .find(|entry| entry.id == id)
            .or_else(|| entries.last()),
        None => entries.last(),
    };
    let Some(leaf) = leaf else {
        return Vec::new();
    };
    let mut by_id = HashMap::new();
    for entry in entries {
        by_id.insert(entry.id.as_str(), entry);
    }
    let mut path = Vec::new();
    let mut current = Some(leaf);
    while let Some(entry) = current {
        path.push(entry);
        current = entry
            .parent_id
            .as_deref()
            .and_then(|id| by_id.get(id).copied());
    }
    path.reverse();
    path
}

/// TS `buildContextEntries`: leaf path, latest compaction + firstKept + after.
pub fn build_context_entries<'a>(
    entries: &'a [SessionEntry],
    leaf_id: Option<&str>,
) -> Vec<&'a SessionEntry> {
    let path = build_session_path(entries, leaf_id);
    let Some(compaction) = path
        .iter()
        .copied()
        .filter(|entry| entry.entry_type == "compaction")
        .next_back()
    else {
        return path;
    };
    let Some(compaction_idx) = path.iter().position(|entry| entry.id == compaction.id) else {
        return path;
    };
    let first_kept_id = compaction
        .extra
        .get("firstKeptEntryId")
        .and_then(Value::as_str);
    let mut context = vec![compaction];
    let mut found_first_kept = false;
    for entry in path.iter().take(compaction_idx) {
        if first_kept_id == Some(entry.id.as_str()) {
            found_first_kept = true;
        }
        if found_first_kept {
            context.push(*entry);
        }
    }
    context.extend(path.iter().skip(compaction_idx + 1).copied());
    context
}

pub fn branch_entries<'a>(
    entries: &'a [SessionEntry],
    leaf_id: Option<&str>,
) -> Vec<&'a SessionEntry> {
    let mut by_id = HashMap::new();
    for entry in entries {
        by_id.insert(entry.id.as_str(), entry);
    }
    let mut path = Vec::new();
    let mut current = leaf_id.and_then(|id| by_id.get(id).copied());
    while let Some(entry) = current {
        path.push(entry);
        current = entry
            .parent_id
            .as_deref()
            .and_then(|id| by_id.get(id).copied());
    }
    path.reverse();
    path
}

pub fn build_session_tree(entries: &[SessionEntry]) -> Value {
    let labels = resolved_labels(entries);
    let mut nodes: HashMap<String, Value> = HashMap::new();
    for entry in entries {
        let (label, label_timestamp) = labels.get(&entry.id).cloned().unwrap_or((None, None));
        let mut node = json!({
            "entry": entry,
            "children": [],
        });
        if let Some(label) = label {
            node["label"] = Value::String(label);
        }
        if let Some(ts) = label_timestamp {
            node["labelTimestamp"] = json!(ts);
        }
        nodes.insert(entry.id.clone(), node);
    }
    let mut child_ids: HashMap<String, Vec<String>> = HashMap::new();
    let mut roots = Vec::new();
    for entry in entries {
        match &entry.parent_id {
            None => roots.push(entry.id.clone()),
            Some(parent) if parent == &entry.id => roots.push(entry.id.clone()),
            Some(parent) if nodes.contains_key(parent) => {
                child_ids
                    .entry(parent.clone())
                    .or_default()
                    .push(entry.id.clone());
            }
            Some(_) => roots.push(entry.id.clone()),
        }
    }
    for children in child_ids.values_mut() {
        children.sort_by_key(|id| {
            nodes
                .get(id)
                .and_then(|node| node.get("entry"))
                .and_then(|entry| entry.get("timestamp"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        });
    }
    fn assemble(
        id: &str,
        nodes: &HashMap<String, Value>,
        child_ids: &HashMap<String, Vec<String>>,
    ) -> Value {
        let mut node = nodes.get(id).cloned().unwrap_or(Value::Null);
        let children = child_ids
            .get(id)
            .into_iter()
            .flatten()
            .map(|child| assemble(child, nodes, child_ids))
            .collect::<Vec<_>>();
        if let Some(object) = node.as_object_mut() {
            object.insert("children".into(), Value::Array(children));
        }
        node
    }
    Value::Array(
        roots
            .iter()
            .map(|id| assemble(id, &nodes, &child_ids))
            .collect(),
    )
}

pub fn fork_user_messages(entries: &[SessionEntry]) -> Vec<Value> {
    let mut messages = Vec::new();
    for entry in entries {
        if entry.entry_type != "message" {
            continue;
        }
        let Some(message) = &entry.message else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let text = content_text(message.get("content"));
        if !text.is_empty() {
            messages.push(json!({ "entryId": entry.id, "text": text }));
        }
    }
    messages
}

pub fn entries_since(
    entries: &[SessionEntry],
    since: Option<&str>,
) -> Result<Vec<SessionEntry>, String> {
    match since {
        None => Ok(entries.to_vec()),
        Some(id) => {
            let index = entries
                .iter()
                .position(|entry| entry.id == id)
                .ok_or_else(|| format!("Entry not found: {id}"))?;
            Ok(entries[index + 1..].to_vec())
        }
    }
}

pub fn session_usage_stats(entries: &[SessionEntry]) -> SessionUsageStats {
    let mut stats = SessionUsageStats::default();
    for entry in entries {
        if matches!(entry.entry_type.as_str(), "branch_summary" | "compaction") {
            add_usage(
                &mut stats,
                entry
                    .extra
                    .get("usage")
                    .or_else(|| entry.message.as_ref().and_then(|m| m.get("usage"))),
            );
        }
        if entry.entry_type != "message" {
            continue;
        }
        let Some(message) = &entry.message else {
            continue;
        };
        stats.total_messages += 1;
        match message.get("role").and_then(Value::as_str) {
            Some("user") => stats.user_messages += 1,
            Some("toolResult") => {
                stats.tool_results += 1;
                add_usage(&mut stats, message.get("usage"));
            }
            Some("assistant") => {
                stats.assistant_messages += 1;
                if let Some(content) = message.get("content").and_then(Value::as_array) {
                    stats.tool_calls += content
                        .iter()
                        .filter(|block| {
                            block.get("type").and_then(Value::as_str) == Some("toolCall")
                        })
                        .count() as u64;
                }
                add_usage(&mut stats, message.get("usage"));
            }
            _ => {}
        }
    }
    stats
}

fn add_usage(stats: &mut SessionUsageStats, usage: Option<&Value>) {
    let Some(usage) = usage else {
        return;
    };
    stats.input += usage.get("input").and_then(Value::as_u64).unwrap_or(0);
    stats.output += usage.get("output").and_then(Value::as_u64).unwrap_or(0);
    stats.cache_read += usage
        .get("cacheRead")
        .or_else(|| usage.get("cache_read"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    stats.cache_write += usage
        .get("cacheWrite")
        .or_else(|| usage.get("cache_write"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    stats.cost += usage
        .get("cost")
        .and_then(|cost| {
            cost.as_f64()
                .or_else(|| cost.get("total").and_then(Value::as_f64))
        })
        .unwrap_or(0.0);
}

fn content_text(content: Option<&Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    let Some(items) = content.as_array() else {
        return String::new();
    };
    items
        .iter()
        .filter_map(|item| {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                item.get("text").and_then(Value::as_str)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

pub fn export_session_jsonl(session: &JsonlSession, output: &Path) -> Result<String, SessionError> {
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                SessionError::storage(format!("Unable to create export directory: {err}"))
            })?;
        }
    }
    let mut lines = vec![serde_json::to_string(&json!({
        "type": "session",
        "version": session.header.version,
        "id": session.header.id,
        "timestamp": session.header.created_at,
        "cwd": session.header.cwd,
    }))
    .map_err(|err| SessionError::storage(err.to_string()))?];
    let mut parent_id: Option<String> = None;
    for entry in branch_entries(&session.entries, session.leaf_id.as_deref()) {
        let mut value =
            serde_json::to_value(entry).map_err(|err| SessionError::storage(err.to_string()))?;
        value["parentId"] = match &parent_id {
            Some(id) => Value::String(id.clone()),
            None => Value::Null,
        };
        lines.push(
            serde_json::to_string(&value).map_err(|err| SessionError::storage(err.to_string()))?,
        );
        parent_id = Some(entry.id.clone());
    }
    fs::write(output, format!("{}\n", lines.join("\n")))
        .map_err(|err| SessionError::storage(format!("Unable to write JSONL export: {err}")))?;
    Ok(output.display().to_string())
}
