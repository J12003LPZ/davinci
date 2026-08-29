//! Differential fixtures for the TypeScript-authoritative rewrite.

pub fn fixture(name: &str) -> serde_json::Value {
    let raw = match name {
        "assistant-message" => include_str!("../fixtures/assistant-message.json"),
        "client-hello" => include_str!("../fixtures/client-hello.json"),
        "writer-lease-errors" => include_str!("../fixtures/writer-lease-errors.json"),
        "writer-leases" => include_str!("../fixtures/writer-leases.json"),
        "session-entry" => include_str!("../fixtures/session-entry.json"),
        "agent-events" => include_str!("../fixtures/agent-events.json"),
        "print-events" => include_str!("../fixtures/print-events.json"),
        other => panic!("unknown fixture {other}"),
    };
    serde_json::from_str(raw).expect("fixture json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_ai::{AssistantContent, Message, StopReason, Usage, UsageCost};
    use pi_protocol::{
        encode_cbor, encode_client_message, CborValue, ClientMessage, PROTOCOL_VERSION,
    };
    use pi_session::Entry;
    use pi_session_sqlite::{
        acquire_writer_lease, release_writer_lease, renew_writer_lease, WriterLease, INITIAL_SCHEMA,
    };
    use rusqlite::Connection;

    #[test]
    fn assistant_message_fixture_round_trips() {
        let expected = fixture("assistant-message");
        let message = Message::Assistant {
            content: vec![
                AssistantContent::Text {
                    text: "echo:hi".into(),
                },
                AssistantContent::ToolCall {
                    id: "c1".into(),
                    name: "echo".into(),
                    arguments: serde_json::json!({"text":"hi"}),
                },
            ],
            api: "openai-completions".into(),
            provider: "mock".into(),
            model: "mock-1".into(),
            usage: Usage {
                input: 8,
                output: 7,
                cache_read: 0,
                cache_write: 0,
                total_tokens: 15,
                cost: UsageCost {
                    input: 0.008,
                    output: 0.014,
                    cache_read: 0.0,
                    cache_write: 0.0,
                    total: 0.022,
                },
            },
            stop_reason: StopReason::ToolUse,
            timestamp: 0,
            error_message: None,
        };
        let mut actual = serde_json::to_value(message).unwrap();
        actual.as_object_mut().unwrap().remove("timestamp");
        assert_eq!(actual, expected);
    }

    #[test]
    fn client_hello_matches_typescript_shape_and_cbor_frame() {
        let expected = fixture("client-hello");
        let message = ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        };
        assert_eq!(serde_json::to_value(&message).unwrap(), expected);
        let framed = encode_client_message(&message).unwrap();
        assert!(framed.len() > 4);
        let encoded = encode_cbor(&CborValue::from_json(&expected).unwrap()).unwrap();
        assert_eq!(&framed[4..], encoded.as_slice());
    }

    #[test]
    fn writer_lease_error_strings_are_locked() {
        let errors = fixture("writer-lease-errors");
        assert_eq!(
            errors["activeWriter"],
            "SQLite session {id} already has an active writer"
        );
        assert_eq!(
            errors["lostWriter"],
            "SQLite session {id} writer lease was lost"
        );
    }

    #[test]
    fn writer_lease_sql_trace_matches_typescript() {
        let doc = fixture("writer-leases");
        for case in doc["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(INITIAL_SCHEMA).unwrap();
            for step in case["steps"].as_array().unwrap() {
                match step["op"].as_str().unwrap() {
                    "acquire" => {
                        let got = acquire_writer_lease(
                            &db,
                            "sess",
                            step["ownerId"].as_str().unwrap(),
                            step["now"].as_i64().unwrap(),
                            step["expiresAtMs"].as_i64().unwrap(),
                        )
                        .unwrap();
                        if step["expectNone"].as_bool().unwrap_or(false) {
                            assert!(got.is_none(), "{name} expected none");
                        } else {
                            assert_eq!(
                                got.expect(name).fence,
                                step["expectFence"].as_i64().unwrap(),
                                "{name}"
                            );
                        }
                    }
                    "renew" => {
                        let mut lease = WriterLease {
                            owner_id: step["ownerId"].as_str().unwrap().into(),
                            fence: step["fence"].as_i64().unwrap(),
                            expires_at_ms: 0,
                        };
                        let ok = renew_writer_lease(
                            &db,
                            "sess",
                            &mut lease,
                            step["now"].as_i64().unwrap(),
                            step["expiresAtMs"].as_i64().unwrap(),
                        )
                        .unwrap();
                        assert_eq!(ok, step["expect"].as_bool().unwrap(), "{name}");
                    }
                    "release" => {
                        let lease = WriterLease {
                            owner_id: step["ownerId"].as_str().unwrap().into(),
                            fence: step["fence"].as_i64().unwrap(),
                            expires_at_ms: 0,
                        };
                        release_writer_lease(&db, "sess", &lease).unwrap();
                    }
                    other => panic!("unknown op {other}"),
                }
            }
        }
    }

    #[test]
    fn session_entry_fixture_round_trips() {
        let expected = fixture("session-entry");
        let entry: Entry = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(entry.id(), "entry-golden-1");
        assert_eq!(serde_json::to_value(&entry).unwrap(), expected);
    }

    #[test]
    fn agent_event_lifecycle_is_locked() {
        let expected = fixture("agent-events");
        let names: Vec<&str> = expected["lifecycle"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "AgentStart",
                "TurnStart",
                "MessageStart",
                "MessageUpdate",
                "MessageEnd",
                "TurnEnd",
                "AgentEnd"
            ]
        );
    }

    #[test]
    fn print_json_events_match_fixture() {
        let expected = fixture("print-events");
        let raw = pi_coding_agent::run_print("hi", true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let types: Vec<&str> = value
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["type"].as_str().unwrap())
            .collect();
        assert_eq!(types.first().copied(), expected["first"].as_str());
        assert_eq!(types.last().copied(), expected["last"].as_str());
        for required in expected["required"].as_array().unwrap() {
            assert!(
                types.contains(&required.as_str().unwrap()),
                "missing {}",
                required
            );
        }
    }

    #[test]
    fn typescript_sha_is_recorded() {
        assert_eq!(
            pi_core::TYPESCRIPT_UPSTREAM_SHA,
            "853a80d26c90a14c1886f0ebb8ffaae133ca2185"
        );
    }
}
