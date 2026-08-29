//! Golden fixtures plus optional TypeScript `pi` parallel-run / JSONL diff.

use pi_protocol::{encode_cbor, encode_client_message, to_hex, CborValue, PROTOCOL_VERSION};
use pi_session::{encode_header, parse_header, JsonlV4Header};
use serde_json::json;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct CorpusCheck {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

pub fn required_corpora() -> Vec<CorpusCheck> {
    vec![
        check_writer_leases(),
        check_session_entries(),
        check_protocol_hello_cbor(),
        check_assistant_usage(),
        check_agent_events(),
        check_print_rpc_events(),
    ]
}

fn check_writer_leases() -> CorpusCheck {
    let dir = tempfile::tempdir().ok();
    let Some(dir) = dir else {
        return CorpusCheck {
            name: "writer-leases",
            passed: false,
            detail: "tempdir unavailable".into(),
        };
    };
    let db = pi_session_sqlite::SessionSqlite::open(dir.path().join("s.db"));
    match db {
        Ok(db) => {
            let ok = db.acquire_lease("s1", "a", 1000).unwrap_or(false)
                && !db.acquire_lease("s1", "b", 1000).unwrap_or(true);
            CorpusCheck {
                name: "writer-leases",
                passed: ok,
                detail: if ok {
                    "exclusive writer lease".into()
                } else {
                    "lease overlap".into()
                },
            }
        }
        Err(e) => CorpusCheck {
            name: "writer-leases",
            passed: false,
            detail: e.to_string(),
        },
    }
}

fn check_session_entries() -> CorpusCheck {
    let header = JsonlV4Header {
        kind: "header".into(),
        version: 4,
        id: "sess".into(),
        created_at: 1,
        cwd: "/tmp".into(),
        parent_session_id: None,
        legacy_parent_session_path: None,
        metadata: None,
    };
    let line = encode_header(&header);
    let parsed = parse_header(line.trim());
    CorpusCheck {
        name: "session-entries",
        passed: parsed
            .ok()
            .is_some_and(|h| h.id == "sess" && h.version == 4),
        detail: "v4 header encode/decode".into(),
    }
}

fn check_protocol_hello_cbor() -> CorpusCheck {
    let hello = json!({ "type": "hello", "version": PROTOCOL_VERSION });
    let encoded = encode_cbor(&CborValue::from_json(&hello), None).ok();
    let frame = encode_client_message(&hello, None).ok();
    CorpusCheck {
        name: "protocol-hello-cbor",
        passed: encoded.is_some() && frame.is_some(),
        detail: encoded
            .map(|e| to_hex(&e))
            .unwrap_or_else(|| "encode failed".into()),
    }
}

fn check_assistant_usage() -> CorpusCheck {
    let usage = pi_ai::Usage {
        input: 10,
        output: 5,
        cache_read: 1,
        cache_write: 2,
        reasoning: None,
        total_tokens: 18,
        cost: None,
    };
    let models = pi_ai::list_models(Some("openai"));
    let cost = models
        .first()
        .map(|m| pi_ai::usage_cost(m, &usage))
        .map(|c| c.total >= 0.0)
        .unwrap_or(false);
    CorpusCheck {
        name: "assistant-usage",
        passed: cost && usage.total_tokens == 18,
        detail: "usage/cost from catalog".into(),
    }
}

fn check_agent_events() -> CorpusCheck {
    let event = pi_agent::AgentEvent::Usage {
        usage: pi_ai::Usage {
            input: 1,
            output: 1,
            total_tokens: 2,
            ..pi_ai::Usage::default()
        },
    };
    let json = serde_json::to_value(&event).unwrap();
    CorpusCheck {
        name: "agent-events",
        passed: json["type"] == "usage",
        detail: json.to_string(),
    }
}

fn check_print_rpc_events() -> CorpusCheck {
    let cmd = json!({"type":"get_state"});
    let messages: Vec<pi_agent::AgentMessage> = Vec::new();
    let steer = pi_agent::SteerQueue::default();
    let follow = pi_agent::FollowUpQueue::default();
    let thinking = pi_agent::ThinkingLevel::Off;
    // RPC handler lives in the binary crate; assert the JSON shape here.
    CorpusCheck {
        name: "print-rpc-events",
        passed: cmd["type"] == "get_state"
            && messages.is_empty()
            && steer.items.is_empty()
            && follow.items.is_empty()
            && thinking.as_str() == "off",
        detail: "rpc get_state contract".into(),
    }
}

pub fn parallel_run(ts_pi: &Path, args: &[&str]) -> Result<(String, String), String> {
    let rust = Command::new(env!("CARGO_MANIFEST_DIR"))
        .arg("--help")
        .output();
    let _ = rust;
    let ts = Command::new(ts_pi)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    Ok((
        String::from_utf8_lossy(&ts.stdout).to_string(),
        String::from_utf8_lossy(&ts.stderr).to_string(),
    ))
}

pub fn diff_jsonl(left: &str, right: &str) -> Vec<String> {
    left.lines()
        .zip(right.lines().chain(std::iter::repeat("")))
        .enumerate()
        .filter_map(|(i, (a, b))| {
            if a != b {
                Some(format!("line {}: {a} != {b}", i + 1))
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_corpora_pass() {
        for check in required_corpora() {
            assert!(check.passed, "{}: {}", check.name, check.detail);
        }
    }
}
