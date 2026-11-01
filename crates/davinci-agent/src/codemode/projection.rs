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

/// Visible script output assembled from the guest's explicit emit channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptOutput {
    pub text: String,
    pub complete: bool,
    pub omitted_bytes: usize,
}

/// Fixed bytes of the omission marker, excluding the digits of `N`.
const OMISSION_MARKER_FIXED_BYTES: usize = "\n[...  bytes omitted ...]\n".len();

fn omission_marker(omitted: usize) -> String {
    format!("\n[... {omitted} bytes omitted ...]\n")
}

/// Keep the head and the tail of an over-budget text, separated by an explicit
/// marker naming the bytes dropped in between.
///
/// Guarantees, in order:
/// - the result is at most `limit` bytes (the marker counts against it);
/// - both cuts land on a `char` boundary, so the result is valid UTF-8;
/// - the head is a prefix and the tail a suffix of the input;
/// - short input is returned unchanged with `0` omitted.
pub fn truncate_head_and_tail(text: &str, limit: usize) -> (String, usize) {
    if text.len() <= limit {
        return (text.to_owned(), 0);
    }
    // The marker's length depends on the digits of N, and N depends on how much
    // room the marker leaves, so iterate to a fixed point.
    let mut digits = 1usize;
    loop {
        let marker_len = OMISSION_MARKER_FIXED_BYTES + digits;
        if marker_len >= limit {
            // The marker cannot fit; keep a bare head cut rather than panic.
            let mut end = limit.min(text.len());
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            return (text[..end].to_owned(), text.len() - end);
        }
        let remaining = limit - marker_len;
        // Head keeps two thirds of the remaining budget, the tail one third.
        let head_budget = remaining * 2 / 3;
        let tail_budget = remaining - head_budget;
        let mut head_end = head_budget.min(text.len());
        while !text.is_char_boundary(head_end) {
            head_end -= 1;
        }
        let mut tail_start = text.len().saturating_sub(tail_budget);
        while tail_start < text.len() && !text.is_char_boundary(tail_start) {
            tail_start += 1;
        }
        let omitted = tail_start - head_end;
        if omitted.to_string().len() == digits {
            return (
                format!(
                    "{}{}{}",
                    &text[..head_end],
                    omission_marker(omitted),
                    &text[tail_start..]
                ),
                omitted,
            );
        }
        digits = omitted.to_string().len();
    }
}

/// Assemble `text()`/`console` items into the model-visible output. Text is
/// truncated only with an explicit omitted marker and `complete: false`.
/// Image items fail closed until a trusted image projection exists.
pub fn assemble_script_output(
    items: &[Value],
    limit: usize,
    collected_limit: usize,
) -> Result<ScriptOutput, CodeModeError> {
    let mut collected = String::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = item.get("text").and_then(Value::as_str).ok_or_else(|| {
                    CodeModeError::new("PROTOCOL_ERROR", "malformed text output item")
                })?;
                if collected.len() + text.len() > collected_limit {
                    return Err(CodeModeError::new(
                        "LIMIT_EXCEEDED",
                        "collected output limit",
                    ));
                }
                collected.push_str(text);
            }
            Some("image") => {
                // No trusted bounded image projection exists yet, so claiming
                // image output would be a false parity claim.
                return Err(CodeModeError::new(
                    "UNSUPPORTED_OUTPUT",
                    "image output is not yet supported; OPEN parity gap",
                ));
            }
            _ => {
                return Err(CodeModeError::new(
                    "PROTOCOL_ERROR",
                    "unsupported output item",
                ))
            }
        }
    }
    if collected.len() <= limit {
        return Ok(ScriptOutput {
            text: collected,
            complete: true,
            omitted_bytes: 0,
        });
    }
    // Keep the head and the tail, separated by an explicit marker naming the
    // bytes dropped in between (Stage 0 behavior).
    let (text, omitted) = truncate_head_and_tail(&collected, limit);
    Ok(ScriptOutput {
        text,
        complete: false,
        omitted_bytes: omitted,
    })
}

/// Script-visible shape of a child result. Command tools keep their structured
/// exit metadata instead of flattening it into text.
pub fn adapt_script_tool_result(
    tool: &str,
    result: crate::ToolResult,
    structured: Option<Value>,
    operation_ref: String,
    maximum: usize,
) -> Result<CodeModeToolValue, CodeModeError> {
    let command = matches!(tool, "exec_command" | "bash" | "powershell");
    let structured = structured.or_else(|| {
        // Command tools already report their exit status; hoist it instead of
        // making the script parse prose.
        if !command {
            return None;
        }
        let details = result.details.as_ref()?;
        let exit_code = details.get("exitCode")?;
        let mut value = serde_json::Map::new();
        value.insert("exitCode".into(), exit_code.clone());
        if let Some(duration) = details.get("durationMs") {
            value.insert("durationMs".into(), duration.clone());
        }
        if let Some(cancelled) = details.get("cancelled") {
            value.insert("cancelled".into(), cancelled.clone());
        }
        Some(Value::Object(value))
    });
    // A command that ran and exited non-zero is a value with its status, not a
    // delivered tool error: a failing test run must stay inspectable.
    let failed_command = command
        && result.details.as_ref().is_some_and(|details| {
            details
                .get("exitCode")
                .is_some_and(|code| code.as_i64() != Some(0))
        });
    let mut result = result;
    if failed_command {
        result.is_error = false;
    }
    project_script_result(result, structured, operation_ref, maximum)
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
