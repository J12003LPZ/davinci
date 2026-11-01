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

/// Adapt a Droid-shaped call to the canonical tool's native input schema.
/// Only explicit, tested differences are rewritten; anything else passes
/// through untouched so no argument silently changes meaning.
pub fn adapt_script_arguments(tool: &str, args: Value) -> Result<Value, CodeModeError> {
    if tool != "apply_patch" || args.get("input").is_some() {
        return Ok(args);
    }
    let Some(object) = args.as_object() else {
        return Err(CodeModeError::new(
            "INVALID_INPUT",
            "apply_patch arguments must be an object",
        ));
    };
    let patch = object.get("patch").and_then(Value::as_str);
    if object.len() != 1 || patch.is_none() {
        return Err(CodeModeError::new(
            "INVALID_INPUT",
            "apply_patch requires the native `input` or exactly one `patch` string",
        ));
    }
    Ok(serde_json::json!({"input": patch.unwrap_or_default()}))
}

pub fn project_script_result(
    result: crate::ToolResult,
    structured: Option<Value>,
    operation_ref: String,
    maximum: usize,
) -> Result<CodeModeToolValue, CodeModeError> {
    if result.is_error {
        return Err(CodeModeError::new("TOOL_FAILED", &result.content));
    }
    if let Some(value) = &structured {
        validate_json(value, 64)?;
    }
    let complete = result.details.as_ref().is_none_or(|details| {
        // Script values carry text only: a truncated read or an image that
        // lives in `details.image` is not the whole result.
        let truncated = details
            .pointer("/truncation/truncated")
            .or_else(|| details.get("truncated"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        !truncated && details.get("image").is_none_or(Value::is_null)
    });
    let value = CodeModeToolValue {
        text: result.content,
        structured_content: structured,
        complete,
        artifact: None,
        operation_ref,
    };
    if serialized_size(&value, maximum).is_err() {
        return Err(CodeModeError::new(
            "INCOMPLETE_DATA",
            "child output exceeds the script budget; no authorized artifact is available",
        ));
    }
    Ok(value)
}
