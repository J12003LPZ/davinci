use async_trait::async_trait;
use pi_agent::Agent;
use pi_ai::MockLanguageModel;
use pi_client::{ClientError, ClientTransport, PiClient};
use pi_core::{AgentEvent, Message, Role, RpcRequest, RpcResponse, SessionMetadata, WriterLease};
use pi_server::PiServer;
use pi_session_sqlite::{SessionStore, SqliteSessionStore};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

struct DirectTransport {
    server: PiServer,
}

#[async_trait]
impl ClientTransport for DirectTransport {
    async fn send(&self, request: RpcRequest) -> Result<RpcResponse, ClientError> {
        let res = self.server.handle_rpc(request, None).await;
        Ok(res)
    }

    async fn subscribe_events(&self) -> Result<mpsc::Receiver<AgentEvent>, ClientError> {
        let (_tx, rx) = mpsc::channel(10);
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_golden_protocol_fixtures() {
        let fixture_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/golden_protocol.json");
        let raw = fs::read_to_string(fixture_path).expect("failed to read golden fixtures");
        let json_val: serde_json::Value = serde_json::from_str(&raw).unwrap();

        let cases = json_val["test_cases"].as_array().unwrap();
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let payload = &case["payload"];

            match name {
                "session_metadata_roundtrip" => {
                    let meta: SessionMetadata = serde_json::from_value(payload.clone()).unwrap();
                    assert_eq!(meta.id, "sess-golden-1");
                    assert_eq!(meta.title, "Conformance Session");
                    assert_eq!(meta.created_at, 1700000000000);
                    assert_eq!(meta.tags, vec!["rust", "typescript", "conformance"]);

                    let back_to_json = serde_json::to_value(&meta).unwrap();
                    assert_eq!(payload, &back_to_json);
                }
                "message_roundtrip" => {
                    let msg: Message = serde_json::from_value(payload.clone()).unwrap();
                    assert_eq!(msg.id, "msg-golden-1");
                    assert_eq!(msg.role, Role::User);
                    assert_eq!(msg.content, "Perform migration check");

                    let back_to_json = serde_json::to_value(&msg).unwrap();
                    assert_eq!(payload, &back_to_json);
                }
                "writer_lease_roundtrip" => {
                    let lease: WriterLease = serde_json::from_value(payload.clone()).unwrap();
                    assert_eq!(lease.session_id, "sess-golden-1");
                    assert_eq!(lease.holder_id, "worker-alpha");

                    let back_to_json = serde_json::to_value(&lease).unwrap();
                    assert_eq!(payload, &back_to_json);
                }
                _ => panic!("Unknown test case: {}", name),
            }
        }
    }

    #[tokio::test]
    async fn test_end_to_end_agent_client_server_conformance() {
        let store = Arc::new(SqliteSessionStore::new_in_memory().unwrap());
        let model = Arc::new(MockLanguageModel::new("Pi Rust: "));
        let agent = Arc::new(Agent::new(
            store.clone(),
            model,
            Some("conformance-agent".to_string()),
        ));
        let server = PiServer::new(store.clone(), agent);

        let transport = Arc::new(DirectTransport { server });
        let client = PiClient::new(transport);

        // 1. Create session via client
        let session = client
            .create_session("Conformance Run", &["conformance".to_string()])
            .await
            .unwrap();
        assert_eq!(session.title, "Conformance Run");

        // 2. Run prompt via agent RPC
        let reply = client
            .run_prompt(&session.id, "Hello from Rust test harness")
            .await
            .unwrap();
        assert_eq!(reply, "Pi Rust: Hello from Rust test harness");

        // 3. Fetch messages and verify history integrity
        let messages = client.get_messages(&session.id).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content, "Hello from Rust test harness");
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].content, "Pi Rust: Hello from Rust test harness");
    }
}
