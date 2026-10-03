//! Provider wire tracing, off unless `PI_AI_TRACE` is set. `1` writes to
//! stderr; any other value is a file path that is appended to, which is what
//! an interactive session needs, since its stderr is the screen.

use std::io::Write;
use std::sync::OnceLock;

enum Sink {
    Off,
    Stderr,
    File(std::path::PathBuf),
}

fn sink() -> &'static Sink {
    static SINK: OnceLock<Sink> = OnceLock::new();
    SINK.get_or_init(|| match std::env::var("PI_AI_TRACE") {
        Ok(value) if value.is_empty() || value == "0" || value == "false" => Sink::Off,
        Ok(value) if value == "1" || value == "true" || value == "stderr" => Sink::Stderr,
        Ok(path) => Sink::File(std::path::PathBuf::from(path)),
        Err(_) => Sink::Off,
    })
}

pub fn enabled() -> bool {
    !matches!(sink(), Sink::Off)
}

/// Write one trace line. Cheap when tracing is off: callers guard the
/// formatting with `enabled()` where the message is costly to build.
pub fn log(line: &str) {
    match sink() {
        Sink::Off => {}
        Sink::Stderr => eprintln!("[pi-ai] {line}"),
        Sink::File(path) => {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "[pi-ai] {line}");
            }
        }
    }
}

/// Strip query parameters from traced URLs because providers and proxies may
/// put API keys, signatures, or other secrets there.
pub fn redact_url(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?<redacted>"),
        None => url.to_string(),
    }
}

/// A compact description of one provider event for the trace: its type and,
/// for a few shapes, the field that says what it did.
pub fn describe_event(event: &serde_json::Value) -> String {
    let kind = event
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?");
    let detail = match kind {
        "response.output_item.added" | "response.output_item.done" => event
            .pointer("/item/type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
        "response.completed" | "response.incomplete" | "response.done" | "response.failed" => {
            let error_type = event
                .pointer("/response/error/type")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    event
                        .pointer("/response/error/code")
                        .and_then(serde_json::Value::as_str)
                })
                .unwrap_or("-");
            format!(
                "status={} error_type={error_type}",
                event
                    .pointer("/response/status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-"),
            )
        }
        "error" => {
            let error_type = event
                .pointer("/error/type")
                .and_then(serde_json::Value::as_str)
                .or_else(|| event.get("code").and_then(serde_json::Value::as_str))
                .unwrap_or("-");
            format!("error_type={error_type}")
        }
        _ if event.get("choices").is_some() => event
            .pointer("/choices/0/finish_reason")
            .and_then(serde_json::Value::as_str)
            .map(|reason| format!("finish_reason={reason}"))
            .unwrap_or_default(),
        _ => String::new(),
    };
    if detail.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_url_drops_the_query() {
        assert_eq!(
            redact_url("https://h/models/x:generateContent?key=SECRET&alt=sse"),
            "https://h/models/x:generateContent?<redacted>"
        );
        assert_eq!(redact_url("https://h/v1/messages"), "https://h/v1/messages");
    }

    #[test]
    fn provider_trace_never_stringifies_prompt_or_opaque_error_content() {
        let event = serde_json::json!({
            "type": "error",
            "code": "invalid_request",
            "message": "RAW-PROMPT-SECRET",
            "encrypted_content": "OPAQUE-REASONING-SECRET"
        });
        let rendered = describe_event(&event);
        assert_eq!(rendered, "error error_type=invalid_request");
        assert!(!rendered.contains("RAW-PROMPT-SECRET"));
        assert!(!rendered.contains("OPAQUE-REASONING-SECRET"));
    }

    #[test]
    fn terminal_failure_trace_keeps_only_status_and_error_class() {
        let event = serde_json::json!({
            "type": "response.failed",
            "response": {
                "status": "failed",
                "error": {
                    "type": "invalid_request_error",
                    "message": "prompt fragment should stay private"
                },
                "output": [{
                    "type":"reasoning",
                    "encrypted_content":"OPAQUE"
                }]
            }
        });
        let rendered = describe_event(&event);
        assert_eq!(
            rendered,
            "response.failed status=failed error_type=invalid_request_error"
        );
        assert!(!rendered.contains("prompt fragment"));
        assert!(!rendered.contains("OPAQUE"));
    }
}
