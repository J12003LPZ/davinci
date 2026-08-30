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
    // Move the map out instead of cloning it; large message payloads make the
    // clone the dominant cost of a session load.
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(JsonlDecodeError::schema("is not a JSON object")),
    }
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
        all_messages_text: String::new(),
    }
}

pub fn parse_mutation(line: &str) -> Result<SessionMutation, JsonlDecodeError> {
    let value = parse_object(line)?;
    let kind = value.get("kind").and_then(Value::as_str);
    let seq = value
        .get("seq")
        .map(|v| require_sequence(Some(v)))
        .transpose()?
        .or_else(|| value.get("seq").and_then(Value::as_u64))
        .unwrap_or(1);
    match kind {
        Some("record") => parse_record(value, seq),
        Some("lane") => parse_lane(value, seq),
        Some("fact") => parse_fact(value, seq),
        Some("entry") | None => {
            if value.get("type").and_then(Value::as_str) == Some("header") {
                return Err(JsonlDecodeError::schema("is not a header"));
            }
            parse_entry(value, seq)
        }
        Some(_) => Err(JsonlDecodeError::schema("has unknown mutation kind")),
    }
}

fn parse_lane(
    value: serde_json::Map<String, Value>,
    seq: u64,
) -> Result<SessionMutation, JsonlDecodeError> {
    let seq = require_sequence(value.get("seq")).unwrap_or(seq);
    Ok(SessionMutation::Lane {
        seq,
        lane: require_string(value.get("lane"), "lane")?,
        leaf_id: match value.get("leafId") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) => Some(id.clone()),
            _ => return Err(JsonlDecodeError::schema("has invalid leafId")),
        },
    })
}

fn parse_fact(
    value: serde_json::Map<String, Value>,
    seq: u64,
) -> Result<SessionMutation, JsonlDecodeError> {
    let seq = require_sequence(value.get("seq")).unwrap_or(seq);
    match value.get("fact").and_then(Value::as_str) {
        Some("name") => {
            if let Some(name) = value.get("name") {
                if !name.is_string() {
                    return Err(JsonlDecodeError::schema("has invalid name"));
                }
            }
            Ok(SessionMutation::FactName {
                seq,
                name: value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        }
        Some("label") => {
            if let Some(label) = value.get("label") {
                if !label.is_string() {
                    return Err(JsonlDecodeError::schema("has invalid label"));
                }
            }
            Ok(SessionMutation::FactLabel {
                seq,
                target_id: require_string(value.get("targetId"), "targetId")?,
                label: value
                    .get("label")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        }
        _ => Err(JsonlDecodeError::schema("has unknown fact type")),
    }
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
    let id = require_string(value.get("id"), "id")?;
    let parent_id = match value.get("parentId") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.clone()),
        _ => return Err(JsonlDecodeError::schema("has invalid parentId")),
    };
    let timestamp = require_timestamp(value.get("timestamp"))?;
    let custom_type = value
        .get("customType")
        .and_then(Value::as_str)
        .map(str::to_string);
    let lane = value
        .get("lane")
        .and_then(Value::as_str)
        .map(str::to_string);
    // Move the message out and reuse the remaining map as `extra` — cloning
    // the whole object per line dominated large session loads.
    let mut value = value;
    let message = value.remove("message");
    for key in [
        "id",
        "type",
        "parentId",
        "seq",
        "timestamp",
        "customType",
        "kind",
        "lane",
    ] {
        value.remove(key);
    }
    Ok(SessionMutation::Entry {
        lane,
        entry: SessionEntry {
            id,
            entry_type,
            parent_id,
            seq,
            timestamp,
            message,
            custom_type,
            extra: value,
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
    let id = require_string(value.get("id"), "id")?;
    let seq = value
        .get("seq")
        .map(|v| require_sequence(Some(v)))
        .transpose()?
        .unwrap_or(seq);
    let timestamp = require_timestamp(value.get("timestamp"))?;
    let lane = value
        .get("lane")
        .and_then(Value::as_str)
        .map(str::to_string);
    // Reuse the map as `extra` instead of deep-cloning it per record.
    let mut value = value;
    for key in ["id", "type", "seq", "timestamp", "lane", "kind"] {
        value.remove(key);
    }
    Ok(SessionMutation::Record {
        lane: lane.clone(),
        record: LaneRecord {
            id,
            record_type,
            seq,
            timestamp,
            lane,
            extra: value,
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
        SessionMutation::Lane { seq, lane, leaf_id } => format!(
            "{}\n",
            serde_json::json!({
                "kind": "lane",
                "seq": seq,
                "lane": lane,
                "leafId": leaf_id,
            })
        ),
        SessionMutation::FactName { seq, name } => format!(
            "{}\n",
            serde_json::json!({
                "kind": "fact",
                "seq": seq,
                "fact": "name",
                "name": name,
            })
        ),
        SessionMutation::FactLabel {
            seq,
            target_id,
            label,
        } => format!(
            "{}\n",
            serde_json::json!({
                "kind": "fact",
                "seq": seq,
                "fact": "label",
                "targetId": target_id,
                "label": label,
            })
        ),
    }
}

pub fn migrate_v3_to_v4<R: BufRead>(
    path: &Path,
    first_line: &str,
    rest: std::io::Lines<R>,
) -> Result<JsonlSession, SessionError> {
    // TS v3 files begin with a `{"type":"session",...}` header carrying the
    // authoritative id, ISO timestamp, cwd, and optional parent session path.
    let v3_header = serde_json::from_str::<Value>(first_line.trim())
        .ok()
        .and_then(|value| value.as_object().cloned())
        .filter(|object| object.get("type").and_then(Value::as_str) == Some("session"));
    let mut entries = Vec::new();
    if v3_header.is_none() {
        entries.push(parse_v3_line(first_line, 1)?);
    }
    for (index, line) in rest.enumerate() {
        let line = line
            .map_err(|err| SessionError::storage(format!("Unable to read session file: {err}")))?;
        if line.trim().is_empty() {
            continue;
        }
        entries.push(parse_v3_line(&line, index + 2)?);
    }
    let header_text = |key: &str| {
        v3_header
            .as_ref()
            .and_then(|header| header.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let cwd = header_text("cwd").unwrap_or_else(|| {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .map(decode_cwd_component)
            .unwrap_or_else(|| ".".into())
    });
    // TS names files `<fileTimestamp>_<id>.jsonl`; the id after the last `_`
    // is the fallback when the header is missing.
    let id = header_text("id").unwrap_or_else(|| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| stem.rsplit('_').next().unwrap_or(stem).to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string())
    });
    let created_at = header_text("timestamp")
        .as_deref()
        .and_then(parse_iso_ms)
        .or_else(|| entries.first().map(|entry| entry.timestamp))
        .unwrap_or(0);
    let header = JsonlV4Header {
        kind: "header".into(),
        version: 4,
        id,
        created_at,
        cwd,
        parent_session_id: None,
        // The v3 header's parent path when present; otherwise the file's own
        // path, which `source_format_hint` relies on as the migrated-from-v3
        // marker (nothing ever traverses this field as a link).
        legacy_parent_session_path: header_text("parentSession")
            .or_else(|| Some(path.display().to_string())),
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
        timestamp: object.get("timestamp").map(v3_timestamp_ms).unwrap_or(0),
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

/// TS v3 wrote entry timestamps as `Date.toISOString()` strings; numeric
/// milliseconds are accepted for robustness.
fn v3_timestamp_ms(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(parse_iso_ms))
        .unwrap_or(0)
}

/// Parse the strict `Date.toISOString()` shape (`YYYY-MM-DDTHH:MM:SS[.fff]Z`)
/// into Unix milliseconds without a date-time dependency.
fn parse_iso_ms(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u64> {
        let slice = text.get(range)?;
        if !slice.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        slice.parse().ok()
    };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;
    let (millis, timezone_start) = if bytes[19] == b'.' {
        let fraction_end = 20 + text[20..].bytes().take_while(u8::is_ascii_digit).count();
        let mut padded = text[20..fraction_end].to_string();
        while padded.len() < 3 {
            padded.push('0');
        }
        (padded[..3].parse::<u64>().ok()?, fraction_end)
    } else {
        (0, 19)
    };
    if text.get(timezone_start..) != Some("Z") {
        return None;
    }
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    // Howard Hinnant's days-from-civil.
    let year = year as i64 - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = (year - era * 400) as u64;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era as i64 - 719_468;
    let seconds = days * 86_400 + (hour * 3_600 + minute * 60 + second) as i64;
    u64::try_from(seconds * 1_000 + millis as i64).ok()
}

fn decode_cwd_component(encoded: &str) -> String {
    // Only a last resort when a v3 header lacks a cwd. TS-format names
    // (`--..--`) are lossy — `-` may be a separator or a literal dash — so the
    // decoded path is best-effort.
    if let Some(inner) = encoded
        .strip_prefix("--")
        .and_then(|rest| rest.strip_suffix("--"))
    {
        return format!("/{}", inner.replace("--", "/"));
    }
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
