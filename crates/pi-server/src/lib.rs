use pi_protocol::ProtocolMessage;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct PiServer {
    sender: mpsc::UnboundedSender<ProtocolMessage>,
    receiver: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ProtocolMessage>>>,
}

impl PiServer {
    pub fn new(
        sender: mpsc::UnboundedSender<ProtocolMessage>,
        receiver: mpsc::UnboundedReceiver<ProtocolMessage>,
    ) -> Self {
        Self {
            sender,
            receiver: Arc::new(tokio::sync::Mutex::new(receiver)),
        }
    }

    pub async fn handle_next(&self) -> Option<ProtocolMessage> {
        let mut guard = self.receiver.lock().await;
        if let Some(msg) = guard.recv().await {
            match &msg {
                ProtocolMessage::Hello { client_id, .. } => {
                    let resp = ProtocolMessage::Response {
                        id: "hello-resp".to_string(),
                        result: Some(serde_json::json!({
                            "status": "connected",
                            "client_id": client_id
                        })),
                        error: None,
                    };
                    let _ = self.sender.send(resp);
                }
                ProtocolMessage::Request { id, method, .. } => {
                    let resp = ProtocolMessage::Response {
                        id: id.clone(),
                        result: Some(serde_json::json!({
                            "method": method,
                            "acknowledged": true
                        })),
                        error: None,
                    };
                    let _ = self.sender.send(resp);
                }
                _ => {}
            }
            Some(msg)
        } else {
            None
        }
    }
}
