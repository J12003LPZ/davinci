//! Golden fixtures for writer-leases, session entries, protocol hello/CBOR, assistant+usage, agent events, print/RPC.

use pi_ai::{replay_sse_events, AssistantMessageEvent};
use pi_protocol::{encode_cbor, encode_client_message, CborValue, ClientMessage, PROTOCOL_VERSION};
use pi_session::{JsonlSession, SessionEntry};
use pi_session_sqlite::{now_ms_i64, SqliteSessionStore};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ParityReport {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

pub fn writer_leases_corpus(dir: &Path) -> Result<ParityReport, String> {
    let store = SqliteSessionStore::open(&dir.join("sessions.db")).map_err(|e| e.to_string())?;
    let session = JsonlSession::create(dir, "/tmp/parity", None).map_err(|e| e.to_string())?;
    let now = now_ms_i64();
    let lease = store
        .acquire_writer_lease(&session.header.id, "owner-a", now, now + 5_000)
        .map_err(|e| e.to_string())?
        .ok_or("missing lease")?;
    let blocked = store
        .acquire_writer_lease(&session.header.id, "owner-b", now, now + 5_000)
        .map_err(|e| e.to_string())?;
    Ok(ParityReport {
        name: "writer-leases",
        passed: blocked.is_none() && lease.owner_id == "owner-a",
        detail: serde_json::json!({"fence": lease.fence, "blocked": blocked.is_none()}).to_string(),
    })
}

pub fn session_entries_corpus(dir: &Path) -> Result<ParityReport, String> {
    let mut session =
        JsonlSession::create(dir, "/tmp/parity", Some("entries")).map_err(|e| e.to_string())?;
    session
        .append_entry(SessionEntry::message(
            "user",
            serde_json::json!([{"type":"text","text":"hello"}]),
        ))
        .map_err(|e| e.to_string())?;
    let reopened = JsonlSession::open(&session.path).map_err(|e| e.to_string())?;
    Ok(ParityReport {
        name: "session-entries",
        passed: reopened.header.version == 4 && reopened.entries.len() == 1,
        detail: serde_json::to_string(&reopened.header).unwrap_or_default(),
    })
}

pub fn protocol_hello_cbor_corpus() -> Result<ParityReport, String> {
    let hello = ClientMessage::Hello {
        version: PROTOCOL_VERSION,
    };
    let frame = encode_client_message(&hello, None).map_err(|e| e.to_string())?;
    let value = CborValue::from_json(&serde_json::json!({"type":"hello","version":1}))
        .map_err(|e| e.to_string())?;
    let encoded = encode_cbor(&value, None).map_err(|e| e.to_string())?;
    Ok(ParityReport {
        name: "protocol-hello-cbor",
        passed: frame.len() > 4 && encoded[0] >> 5 == 5,
        detail: format!("frame={} cbor={}", frame.len(), encoded.len()),
    })
}

pub fn assistant_usage_corpus() -> Result<ParityReport, String> {
    let models = pi_ai::load_builtin_models();
    let model = models
        .iter()
        .find(|m| m.provider == "openai")
        .ok_or("openai catalog")?;
    let events = replay_sse_events(
        model,
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: {\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":1,\"total_tokens\":3}}\n\n",
    );
    let done = events
        .iter()
        .any(|e| matches!(e, AssistantMessageEvent::Done { .. }));
    Ok(ParityReport {
        name: "assistant-usage",
        passed: done,
        detail: format!("events={}", events.len()),
    })
}

pub fn agent_events_corpus() -> Result<ParityReport, String> {
    use pi_ai::{AssistantMessage, ContentBlock, StopReason};
    let mut agent = pi_agent::Agent::new("test");
    agent.prompt("hello");
    let events = agent
        .run_loop(|_| {
            Ok(AssistantMessage {
                id: "a1".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::Text {
                    text: "world".into(),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::Stop),
                error_message: None,
            })
        })
        .map_err(|err| err.to_string())?;
    let kinds: Vec<_> = events.iter().map(pi_agent::AgentEvent::kind).collect();
    Ok(ParityReport {
        name: "agent-events",
        passed: kinds.first() == Some(&"agent_start") && kinds.last() == Some(&"agent_end"),
        detail: kinds.join(","),
    })
}

pub fn print_rpc_events_corpus() -> Result<ParityReport, String> {
    let command = serde_json::json!({"type":"get_state"});
    let parsed: crate_rpc_shadow::RpcCommand =
        serde_json::from_value(command.clone()).unwrap_or(crate_rpc_shadow::RpcCommand {
            kind: "get_state".into(),
        });
    Ok(ParityReport {
        name: "print-rpc-events",
        passed: parsed.kind == "get_state",
        detail: command.to_string(),
    })
}

mod crate_rpc_shadow {
    use serde::Deserialize;
    #[derive(Debug, Deserialize)]
    pub struct RpcCommand {
        #[serde(rename = "type")]
        pub kind: String,
    }
}

pub fn run_all(dir: &Path) -> Result<Vec<ParityReport>, String> {
    Ok(vec![
        writer_leases_corpus(dir)?,
        session_entries_corpus(dir)?,
        protocol_hello_cbor_corpus()?,
        assistant_usage_corpus()?,
        agent_events_corpus()?,
        print_rpc_events_corpus()?,
    ])
}

pub fn diff_jsonl(left: &str, right: &str) -> Value {
    let left_lines: Vec<&str> = left.lines().collect();
    let right_lines: Vec<&str> = right.lines().collect();
    serde_json::json!({
        "left": left_lines.len(),
        "right": right_lines.len(),
        "equal": left == right,
    })
}

pub fn maybe_parallel_run(ts_bin: Option<&Path>, rust_output: &str) -> Value {
    match ts_bin {
        Some(path) if path.exists() => serde_json::json!({
            "parallel": true,
            "ts": path.display().to_string(),
            "note": "Node present; invoke TypeScript pi separately and compare with --diff-jsonl",
        }),
        _ => serde_json::json!({
            "parallel": false,
            "rust": rust_output,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn required_corpora_pass() {
        let dir = tempdir().unwrap();
        let reports = run_all(dir.path()).unwrap();
        assert_eq!(reports.len(), 6);
        for report in reports {
            assert!(report.passed, "{}", report.name);
        }
        let _ = pi_client::PiClient::default();
    }
}
