//! Differential fixtures for the TypeScript-authoritative rewrite.


pub fn fixture(name: &str) -> serde_json::Value {
    let raw = match name {
        "assistant-message" => include_str!("../fixtures/assistant-message.json"),
        "client-hello" => include_str!("../fixtures/client-hello.json"),
        "writer-lease-errors" => include_str!("../fixtures/writer-lease-errors.json"),
        other => panic!("unknown fixture {other}"),
    };
    serde_json::from_str(raw).expect("fixture json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_ai::{AssistantContent, Message, StopReason, Usage, UsageCost};
    use pi_protocol::{
        encode_cbor, encode_client_message, ClientMessage, CborValue, PROTOCOL_VERSION,
    };

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
    fn typescript_sha_is_recorded() {
        assert_eq!(
            pi_core::TYPESCRIPT_UPSTREAM_SHA,
            "853a80d26c90a14c1886f0ebb8ffaae133ca2185"
        );
    }
}
