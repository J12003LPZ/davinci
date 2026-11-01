//! Bounded, session-bound, append-only Script logs.
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const SCRIPT_LOG_BYTES: usize = 1024 * 1024;
pub const SCRIPT_LOG_EVENTS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptLogEvent {
    pub kind: String,
    pub tool: Option<String>,
    pub text: String,
}

impl ScriptLogEvent {
    pub fn status(text: impl Into<String>) -> Self {
        Self {
            kind: "status".into(),
            tool: None,
            text: text.into(),
        }
    }

    pub fn output(text: impl Into<String>) -> Self {
        Self {
            kind: "output".into(),
            tool: None,
            text: text.into(),
        }
    }

    pub fn child(
        tool: impl Into<String>,
        summary: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self {
            kind: "child".into(),
            tool: Some(tool.into()),
            text: format!("{}\n{}", summary.into(), output.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptLogRef {
    pub id: String,
    pub bytes: u64,
    pub events: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLogSlice {
    pub text: String,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLogError {
    pub code: String,
    pub message: String,
}

pub struct ScriptLogStore {
    dir: PathBuf,
    session_id: String,
}

const REDACTED: &str = "[redacted]";

/// Scrub likely credentials before anything is retained or shown.
pub fn scrub_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(' ').map(|index| index + 1).unwrap_or(rest.len());
        let (word, tail) = rest.split_at(end);
        let trimmed = word.trim_end();
        let lower = trimmed.to_ascii_lowercase();
        let secret = trimmed.starts_with("sk-")
            || trimmed.starts_with("Bearer ")
            || lower.contains("password=")
            || lower.contains("token=")
            || lower.starts_with("authorization:");
        if secret {
            out.push_str(REDACTED);
            if word.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push_str(word);
        }
        rest = tail;
    }
    out
}

impl ScriptLogStore {
    pub fn new(dir: impl Into<PathBuf>, session_id: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            session_id: session_id.into(),
        }
    }

    /// Append one run's bounded, scrubbed evidence and return its opaque id.
    pub fn persist_script_log(
        &self,
        run_id: &str,
        events: &[ScriptLogEvent],
    ) -> Result<ScriptLogRef, ScriptLogError> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|_| ScriptLogError::new("UNAVAILABLE", "script log store unavailable"))?;
        let id = format!(
            "script-log-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        );
        let mut body = format!(
            "{{\"v\":1,\"runId\":{},\"sessionId\":{}}}\n",
            serde_json::to_string(run_id).unwrap_or_else(|_| "\"\"".into()),
            serde_json::to_string(&self.session_id).unwrap_or_else(|_| "\"\"".into())
        );
        let mut retained = 0usize;
        let mut kept = 0usize;
        let mut truncated = events.len() > SCRIPT_LOG_EVENTS;
        for event in events.iter().take(SCRIPT_LOG_EVENTS) {
            let record = serde_json::json!({
                "kind": event.kind,
                "tool": event.tool,
                "text": scrub_secrets(&event.text),
            })
            .to_string();
            if retained + record.len() + 1 > SCRIPT_LOG_BYTES {
                truncated = true;
                break;
            }
            retained += record.len() + 1;
            kept += 1;
            body.push_str(&record);
            body.push('\n');
        }
        std::fs::write(self.path(&id), &body)
            .map_err(|_| ScriptLogError::new("UNAVAILABLE", "script log write failed"))?;
        Ok(ScriptLogRef {
            id,
            bytes: body.len() as u64,
            events: kept,
            truncated,
        })
    }

    /// Read a bounded slice of a retained log. A foreign session cannot see it.
    pub fn read_script_log(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<ScriptLogSlice, ScriptLogError> {
        if !id.starts_with("script-log-")
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ScriptLogError::new(
                "INVALID_INPUT",
                "invalid script log id",
            ));
        }
        let body = std::fs::read_to_string(self.path(id))
            .map_err(|_| ScriptLogError::new("UNAVAILABLE", "script log unavailable"))?;
        let (meta, records) = body.split_once('\n').unwrap_or((&body, ""));
        let meta: serde_json::Value = serde_json::from_str(meta)
            .map_err(|_| ScriptLogError::new("PROTOCOL_ERROR", "malformed script log"))?;
        if meta.get("sessionId").and_then(|value| value.as_str()) != Some(self.session_id.as_str())
        {
            return Err(ScriptLogError::new(
                "DENIED",
                "script log belongs to another session",
            ));
        }
        let mut text = String::new();
        for record in records.lines() {
            let Ok(record) = serde_json::from_str::<serde_json::Value>(record) else {
                continue;
            };
            let kind = record
                .get("kind")
                .and_then(|value| value.as_str())
                .unwrap_or("output");
            let content = record
                .get("text")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            match record.get("tool").and_then(|value| value.as_str()) {
                Some(tool) => text.push_str(&format!("[{kind} {tool}] {content}\n")),
                None => text.push_str(&format!("[{kind}] {content}\n")),
            }
        }
        let complete = offset.saturating_add(limit) >= text.len();
        let mut start = offset.min(text.len());
        while start < text.len() && !text.is_char_boundary(start) {
            start += 1;
        }
        let mut end = start.saturating_add(limit).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        Ok(ScriptLogSlice {
            text: text[start..end].to_string(),
            complete,
        })
    }

    fn path(&self, id: &str) -> PathBuf {
        Path::new(&self.dir).join(format!("{id}.jsonl"))
    }
}

impl ScriptLogError {
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
