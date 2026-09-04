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
            format!(
                "status={} error={}",
                event
                    .pointer("/response/status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-"),
                event
                    .pointer("/response/error")
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "-".into())
            )
        }
        "error" => event.to_string(),
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
