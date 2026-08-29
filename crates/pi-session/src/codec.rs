use std::io::BufRead;
use std::path::Path;

use serde_json::Value;
use uuid::Uuid;

use crate::errors::{JsonlDecodeError, SessionError};
use crate::types::{
    JsonlV4Header, LaneRecord, SessionEntry, SessionMutation, ENTRY_TYPES, RECORD_TYPES,
};
use crate::JsonlSession;

fn parse_object(line: &str) -> Result<serde_json::Map<String, Value>, JsonlDecodeError> {
    let value: Value =
        serde_json::from_str(line).map_err(|_| JsonlDecodeError::syntax("is not valid JSON"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| JsonlDecodeError::schema("is not a JSON object"))
}

fn require_string(value: Option<&Value>, field: &str) -> Result<String, JsonlDecodeError> {
    value
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| JsonlDecodeError::schema(format!("has invalid {field}")))
}

fn require_timestamp(value: Option<&Value>) -> Result<u64, JsonlDecodeError> {
    value
        .and_then(Value::as_u64)
        .ok_or_else(|| JsonlDecodeError::schema("has invalid timestamp"))
}

fn require_sequence(value: Option<&Value>) -> Result<u64, JsonlDecodeError> {
    match value.and_then(Value::as_u64) {
        Some(seq) if seq > 0 => Ok(seq),
        _ => Err(JsonlDecodeError::schema("has invalid seq")),
    }
}

pub fn parse_header(line: &str) -> Result<JsonlV4Header, JsonlDecodeError> {
    let value = parse_object(line)?;
    if value.get("kind").and_then(Value::as_str) != Some("header") {
        return Err(JsonlDecodeError::schema("is not a header"));
    }
    if value.get("version").and_then(Value::as_u64) != Some(4) {
        return Err(JsonlDecodeError::schema("has unsupported session version"));
    }
    let parent = value.get("parentSessionId");
    if parent.is_some() && !parent.unwrap().is_string() {
        return Err(JsonlDecodeError::schema("has invalid parentSessionId"));
    }
    let legacy = value.get("legacyParentSessionPath");
    if legacy.is_some() && !legacy.unwrap().is_string() {
        return Err(JsonlDecodeError::schema(
            "has invalid legacyParentSessionPath",
        ));
    }
    if parent.is_some() && legacy.is_some() {
        return Err(JsonlDecodeError::schema(
            "has both parentSessionId and legacyParentSessionPath",
        ));
    }
    if let Some(metadata) = value.get("metadata") {
        if !metadata.is_object() {
            return Err(JsonlDecodeError::schema("has invalid metadata"));
        }
    }
    Ok(JsonlV4Header {
        kind: "header".into(),
        version: 4,
        id: require_string(value.get("id"), "id")?,
        created_at: require_timestamp(value.get("createdAt"))?,
        cwd: require_string(value.get("cwd"), "cwd")?,
        parent_session_id: parent.and_then(Value::as_str).map(str::to_string),
        legacy_parent_session_path: legacy.and_then(Value::as_str).map(str::to_string),
        metadata: value.get("metadata").cloned(),
    })
}

pub fn encode_header(header: &JsonlV4Header) -> String {
    format!("{}\n", serde_json::to_string(header).expect("header json"))
}

pub fn metadata_from_header(
    header: &JsonlV4Header,
    path: &Path,
    modified_at: u64,
) -> crate::discovery::SessionSummary {
    crate::discovery::SessionSummary {
        id: header.id.clone(),
        path: path.to_path_buf(),
        cwd: header.cwd.clone(),
        created_at: header.created_at,
        modified_at,
        name: header
            .metadata
            .as_ref()
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        parent_session_id: header.parent_session_id.clone(),
        source_format: if header.legacy_parent_session_path.is_some() {
            3
        } else {
            4
        },
    }
}

pub fn parse_mutation(line: &str) -> Result<SessionMutation, JsonlDecodeError> {
    let value = parse_object(line)?;
    let kind = value.get("kind").and_then(Value::as_str).unwrap_or("entry");
    let seq = value
        .get("seq")
        .map(Some)
        .unwrap_or(None)
        .and_then(|v| require_sequence(Some(v)).ok())
        .or_else(|| value.get("seq").and_then(Value::as_u64))
        .unwrap_or(1);
    if kind == "record"
        || RECORD_TYPES.contains(&value.get("type").and_then(Value::as_str).unwrap_or(""))
            && value.get("message").is_none()
            && value.get("kind").and_then(Value::as_str) == Some("record")
    {
        return parse_record(value, seq);
    }
    if value.get("type").and_then(Value::as_str) == Some("header") {
        return Err(JsonlDecodeError::schema("is not a header"));
    }
    parse_entry(value, seq)
}

fn parse_entry(
    value: serde_json::Map<String, Value>,
    seq: u64,
) -> Result<SessionMutation, JsonlDecodeError> {
    let entry_type = require_string(value.get("type"), "entry type")?;
    if !ENTRY_TYPES.contains(&entry_type.as_str()) {
        return Err(JsonlDecodeError::schema(format!(
            "has unknown entry type {entry_type}"
        )));
    }
    if entry_type == "custom" {
        require_string(value.get("customType"), "customType")?;
    }
    let seq = value
        .get("seq")
        .map(|v| require_sequence(Some(v)))
        .transpose()?
        .unwrap_or(seq);
    let mut extra = value.clone();
    extra.remove("id");
    extra.remove("type");
    extra.remove("parentId");
    extra.remove("seq");
    extra.remove("timestamp");
    extra.remove("message");
    extra.remove("customType");
    extra.remove("kind");
    extra.remove("lane");
    Ok(SessionMutation::Entry {
        lane: value
            .get("lane")
            .and_then(Value::as_str)
            .map(str::to_string),
        entry: SessionEntry {
            id: require_string(value.get("id"), "id")?,
            entry_type,
            parent_id: match value.get("parentId") {
                None | Some(Value::Null) => None,
                Some(Value::String(id)) => Some(id.clone()),
                _ => return Err(JsonlDecodeError::schema("has invalid parentId")),
            },
            seq,
            timestamp: require_timestamp(value.get("timestamp"))?,
            message: value.get("message").cloned(),
            custom_type: value
                .get("customType")
                .and_then(Value::as_str)
                .map(str::to_string),
            extra,
        },
    })
}

fn parse_record(
    value: serde_json::Map<String, Value>,
    seq: u64,
) -> Result<SessionMutation, JsonlDecodeError> {
    let record_type = require_string(value.get("type"), "record type")?;
    if !RECORD_TYPES.contains(&record_type.as_str()) {
        return Err(JsonlDecodeError::schema(format!(
            "has unknown record type {record_type}"
        )));
    }
    let mut extra = value.clone();
    extra.remove("id");
    extra.remove("type");
    extra.remove("seq");
    extra.remove("timestamp");
    extra.remove("lane");
    extra.remove("kind");
    Ok(SessionMutation::Record {
        lane: value
            .get("lane")
            .and_then(Value::as_str)
            .map(str::to_string),
        record: LaneRecord {
            id: require_string(value.get("id"), "id")?,
            record_type,
            seq: value
                .get("seq")
                .map(|v| require_sequence(Some(v)))
                .transpose()?
                .unwrap_or(seq),
            timestamp: require_timestamp(value.get("timestamp"))?,
            lane: value
                .get("lane")
                .and_then(Value::as_str)
                .map(str::to_string),
            extra,
        },
    })
}

pub fn encode_mutation(mutation: &SessionMutation) -> String {
    match mutation {
        SessionMutation::Entry { lane, entry } => {
            let mut value = serde_json::to_value(entry).expect("entry json");
            if let Some(object) = value.as_object_mut() {
                object.insert("kind".into(), Value::String("entry".into()));
                if let Some(lane) = lane {
                    object.insert("lane".into(), Value::String(lane.clone()));
                }
            }
            format!("{}\n", serde_json::to_string(&value).expect("entry line"))
        }
        SessionMutation::Record { lane, record } => {
            let mut value = serde_json::to_value(record).expect("record json");
            if let Some(object) = value.as_object_mut() {
                object.insert("kind".into(), Value::String("record".into()));
                if let Some(lane) = lane {
                    object.insert("lane".into(), Value::String(lane.clone()));
                }
            }
            format!("{}\n", serde_json::to_string(&value).expect("record line"))
        }
    }
}

pub fn migrate_v3_to_v4<R: BufRead>(
    path: &Path,
    first_line: &str,
    rest: std::io::Lines<R>,
) -> Result<JsonlSession, SessionError> {
    let first = parse_v3_line(first_line, 1)?;
    let mut entries = vec![first];
    for (index, line) in rest.enumerate() {
        let line = line
            .map_err(|err| SessionError::storage(format!("Unable to read session file: {err}")))?;
        if line.trim().is_empty() {
            continue;
        }
        entries.push(parse_v3_line(&line, index + 2)?);
    }
    let cwd = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(decode_cwd_component)
        .unwrap_or_else(|| ".".into());
    let header = JsonlV4Header {
        kind: "header".into(),
        version: 4,
        id: path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&Uuid::new_v4().to_string())
            .to_string(),
        created_at: entries.first().map(|e| e.timestamp).unwrap_or(0),
        cwd,
        parent_session_id: None,
        legacy_parent_session_path: Some(path.display().to_string()),
        metadata: None,
    };
    let leaf_id = entries.last().map(|e| e.id.clone());
    Ok(JsonlSession {
        path: path.to_path_buf(),
        header,
        entries,
        records: Vec::new(),
        leaf_id,
    })
}

fn parse_v3_line(line: &str, line_no: usize) -> Result<SessionEntry, SessionError> {
    let value: Value = serde_json::from_str(line).map_err(|_| {
        SessionError::invalid_entry(format!(
            "Invalid JSONL v3 session line {line_no} is not valid JSON"
        ))
    })?;
    let object = value.as_object().ok_or_else(|| {
        SessionError::invalid_entry(format!(
            "Invalid JSONL v3 session line {line_no} is not a JSON object"
        ))
    })?;
    Ok(SessionEntry {
        id: object
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(&Uuid::new_v4().to_string())
            .to_string(),
        entry_type: object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message")
            .to_string(),
        parent_id: object
            .get("parentId")
            .and_then(Value::as_str)
            .map(str::to_string),
        seq: line_no as u64,
        timestamp: object.get("timestamp").and_then(Value::as_u64).unwrap_or(0),
        message: object.get("message").cloned(),
        custom_type: object
            .get("customType")
            .and_then(Value::as_str)
            .map(str::to_string),
        extra: object
            .iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "id" | "type" | "parentId" | "timestamp" | "message" | "customType"
                )
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    })
}

fn decode_cwd_component(encoded: &str) -> String {
    if let Some(rest) = encoded.strip_prefix("--") {
        format!("/{}", rest.replace("--", "/"))
    } else {
        encoded.replace("--", "/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_error_strings_match_ts() {
        assert_eq!(
            parse_header("[]").unwrap_err().to_string(),
            "is not a JSON object"
        );
        assert_eq!(
            parse_header("{").unwrap_err().to_string(),
            "is not valid JSON"
        );
        assert_eq!(
            parse_header(r#"{"kind":"nope"}"#).unwrap_err().to_string(),
            "is not a header"
        );
        assert_eq!(
            parse_header(r#"{"kind":"header","version":3,"id":"x","createdAt":1,"cwd":"/"}"#)
                .unwrap_err()
                .to_string(),
            "has unsupported session version"
        );
    }
}
