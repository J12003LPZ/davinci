use super::{CodeModeError, CodeModeToolValue};
use serde_json::Value;

/// Count serialization without allocating an unbounded intermediate buffer.
pub fn serialized_size(
    value: &impl serde::Serialize,
    maximum: usize,
) -> Result<usize, CodeModeError> {
    struct Counter {
        bytes: usize,
        maximum: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.maximum.saturating_sub(self.bytes) {
                return Err(std::io::Error::other("serialized byte limit exceeded"));
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { bytes: 0, maximum };
    serde_json::to_writer(&mut counter, value)
        .map_err(|_| CodeModeError::new("INCOMPLETE_DATA", "serialized byte limit exceeded"))?;
    Ok(counter.bytes)
}

pub fn validate_json(value: &Value, max_depth: usize) -> Result<(), CodeModeError> {
    fn visit(value: &Value, depth: usize, maximum: usize) -> Result<(), CodeModeError> {
        if depth > maximum {
            return Err(CodeModeError::new(
                "LIMIT_EXCEEDED",
                "JSON nesting exceeds limit",
            ));
        }
        match value {
            Value::Number(number)
                if number.as_f64().is_none_or(|n| {
                    !n.is_finite() || (n.fract() == 0.0 && n.abs() > 9007199254740991.0)
                }) =>
            {
                Err(CodeModeError::new(
                    "INCOMPLETE_DATA",
                    "numeric value cannot be represented safely in JavaScript",
                ))
            }
            Value::Array(values) => {
                for value in values {
                    visit(value, depth + 1, maximum)?;
                }
                Ok(())
            }
            Value::Object(values) => {
                for value in values.values() {
                    visit(value, depth + 1, maximum)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    visit(value, 0, max_depth)
}

pub fn project_script_result(
    result: crate::ToolResult,
    structured: Option<Value>,
    operation_ref: String,
    maximum: usize,
) -> Result<CodeModeToolValue, CodeModeError> {
    project_script_result_with_artifact(result, structured, operation_ref, maximum, None, "")
}

/// Project a child result for the script. Oversized data is never cut into
/// invalid JSON: the full result goes to the evidence store and the script
/// receives `complete: false`, `structuredContent: null`, a bounded text
/// prefix and the host-issued artifact reference. Without a store, the
/// failure stays explicit.
pub fn project_script_result_with_artifact(
    result: crate::ToolResult,
    structured: Option<Value>,
    operation_ref: String,
    maximum: usize,
    evidence: Option<&crate::EvidenceStore>,
    tag: &str,
) -> Result<CodeModeToolValue, CodeModeError> {
    if result.is_error {
        return Err(CodeModeError::new("TOOL_FAILED", &result.content));
    }
    if let Some(value) = &structured {
        validate_json(value, 64)?;
    }
    let value = CodeModeToolValue {
        text: result.content,
        structured_content: structured,
        complete: true,
        artifact: None,
        operation_ref,
    };
    if serialized_size(&value, maximum).is_ok() {
        return Ok(value);
    }
    let unavailable = || {
        CodeModeError::new(
            "INCOMPLETE_DATA",
            "child output exceeds the script budget; no authorized artifact is available",
        )
    };
    let store = evidence.ok_or_else(unavailable)?;
    let (full, extension_tag) = match &value.structured_content {
        Some(structured) => (
            serde_json::to_string(
                &serde_json::json!({"text":value.text,"structuredContent":structured}),
            )
            .map_err(|_| unavailable())?,
            "json",
        ),
        None => (value.text.clone(), "text"),
    };
    let path = store
        .store(&format!("codemode-{extension_tag}-{tag}"), &full)
        .map_err(|_| unavailable())?;
    let mut incomplete = CodeModeToolValue {
        text: String::new(),
        structured_content: None,
        complete: false,
        artifact: Some(super::CodeModeArtifact {
            id: path.display().to_string(),
            kind: super::CodeModeArtifactKind::ToolOutput,
            bytes: full.len() as u64,
        }),
        operation_ref: value.operation_ref,
    };
    if serialized_size(&incomplete, maximum).is_err() {
        return Err(unavailable());
    }
    // Keep the largest text prefix that still fits; JSON escaping can expand it.
    let mut cut = value.text.len().min(maximum);
    while cut > 0 {
        let prefix = crate::evidence::cut_at_char_boundary(&value.text, cut);
        incomplete.text = prefix.to_owned();
        if serialized_size(&incomplete, maximum).is_ok() {
            return Ok(incomplete);
        }
        cut = prefix.len() / 2;
    }
    incomplete.text.clear();
    Ok(incomplete)
}
