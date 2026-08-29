//! Differential fixtures shared by TypeScript (authority) and the Rust port.

pub fn load_json(name: &str) -> serde_json::Value {
    let path = format!("{}/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(path).expect("fixture");
    serde_json::from_str(&raw).expect("json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pi_agent::Agent;
    use pi_ai::MockLanguageModel;
    use pi_client::{ClientError, ClientTransport, PiClient};
    use pi_core::{
        decode_cbor, encode_cbor, encode_json, parse_client_message, CborValue, ClientMessage,
        Command, ProtocolError, Role, ServerMessage, WriterLease, WriterLeaseOptions,
        PROTOCOL_VERSION,
    };
    use pi_server::PiServer;
    use pi_session_sqlite::{
        acquire_writer_lease, release_writer_lease, renew_writer_lease, SqliteSessionRepository,
    };
    use rusqlite::Connection;
    use std::sync::Arc;

    struct Loopback {
        server: PiServer,
        hello_done: tokio::sync::Mutex<bool>,
    }

    #[async_trait]
    impl ClientTransport for Loopback {
        async fn send(&self, message: ClientMessage) -> Result<ServerMessage, ClientError> {
            let mut hello_done = self.hello_done.lock().await;
            if !*hello_done && !matches!(message, ClientMessage::Hello { .. }) {
                return Ok(ServerMessage::HelloError {
                    error: ProtocolError::invalid_request("request before hello"),
                });
            }
            let reply = self
                .server
                .handle_message("loopback", message.clone())
                .await;
            if matches!(message, ClientMessage::Hello { .. })
                && matches!(reply, ServerMessage::Hello { .. })
            {
                *hello_done = true;
            }
            Ok(reply)
        }
    }

    #[test]
    fn cbor_fixture_vectors() {
        let doc = load_json("cbor_vectors.json");
        for vector in doc["vectors"].as_array().unwrap() {
            let hex = vector["hex"].as_str().unwrap();
            let json = &vector["json"];
            let bytes = hex::decode(hex).unwrap();
            let decoded = decode_cbor(&bytes).unwrap();
            let expected = CborValue::from_json(json).unwrap();
            assert_eq!(decoded, expected, "decode {hex}");
            assert_eq!(
                hex::encode(encode_cbor(&expected).unwrap()),
                hex,
                "encode {hex}"
            );
            assert_eq!(hex::encode(encode_json(json).unwrap()), hex);
        }
    }

    #[test]
    fn writer_lease_fixture_trace() {
        let doc = load_json("writer_leases.json");
        for case in doc["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(pi_session_sqlite::schema::INITIAL_SCHEMA)
                .unwrap();
            let mut current: Option<WriterLease> = None;
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
                            let lease = got.expect(name);
                            assert_eq!(
                                lease.fence,
                                step["expectFence"].as_i64().unwrap(),
                                "{name}"
                            );
                            current = Some(lease);
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
                        if ok {
                            current = Some(lease);
                        }
                    }
                    "release" => {
                        let lease = WriterLease {
                            owner_id: step["ownerId"].as_str().unwrap().into(),
                            fence: step["fence"].as_i64().unwrap(),
                            expires_at_ms: 0,
                        };
                        release_writer_lease(&db, "sess", &lease).unwrap();
                        current = None;
                    }
                    other => panic!("unknown op {other}"),
                }
            }
            let _ = current;
        }
    }

    #[test]
    fn protocol_envelope_fixture() {
        let doc = load_json("protocol_envelopes.json");
        let hello = parse_client_message(&doc["clientHello"]).unwrap();
        match hello {
            ClientMessage::Hello { version } => assert_eq!(version, PROTOCOL_VERSION),
            other => panic!("{other:?}"),
        }
        let create = parse_client_message(&doc["createCommand"]).unwrap();
        match create {
            ClientMessage::Request {
                request: Command::Create { cwd, name, .. },
                ..
            } => {
                assert_eq!(cwd.as_deref(), Some("/tmp"));
                assert_eq!(name.as_deref(), Some("demo"));
            }
            other => panic!("{other:?}"),
        }
        let err: ProtocolError =
            serde_json::from_value(doc["helloErrorVersion"]["error"].clone()).unwrap();
        assert_eq!(err.message, "Unsupported protocol version");
        let not_impl = ProtocolError::not_implemented();
        assert_eq!(
            serde_json::to_value(&not_impl).unwrap(),
            doc["notImplemented"]
        );
        let internal = ProtocolError::internal();
        assert_eq!(
            serde_json::to_value(&internal).unwrap(),
            doc["internalError"]
        );
    }

    #[tokio::test]
    async fn client_server_loopback_matches_print_path() {
        let store = Arc::new(
            SqliteSessionRepository::open_in_memory(WriterLeaseOptions::default()).unwrap(),
        );
        let agent = Arc::new(Agent::new(
            store.clone(),
            Arc::new(MockLanguageModel::new("Pi Rust: ")),
        ));
        let server = PiServer::new(store, agent);
        let client = PiClient::new(Arc::new(Loopback {
            server,
            hello_done: tokio::sync::Mutex::new(false),
        }));
        client.connect().await.unwrap();
        let mut lease = client
            .create_session(Some("/tmp"), Some("conformance"))
            .await
            .unwrap();
        let snap = lease.prompt("Hello from fixture").await.unwrap();
        assert_eq!(snap.transcript.len(), 2);
        assert_eq!(snap.transcript[1]["content"], "Pi Rust: Hello from fixture");
        lease.detach().await.unwrap();
    }

    #[test]
    fn session_repository_trace() {
        let repo = SqliteSessionRepository::open_in_memory(WriterLeaseOptions::default()).unwrap();
        repo.create(
            Some("src"),
            "/tmp",
            None,
            Some(&serde_json::json!({"k": "v"})),
        )
        .unwrap();
        repo.append_message("src", "main", "m1", Role::User, "one")
            .unwrap();
        repo.append_message("src", "main", "m2", Role::Assistant, "two")
            .unwrap();
        repo.fork("src", "dst", "/tmp", true).unwrap();
        assert_eq!(repo.entries("dst").unwrap().len(), 2);
        repo.delete("dst").unwrap();
        repo.delete("dst").unwrap();
        assert_eq!(repo.list(None).unwrap().len(), 1);
    }
}
