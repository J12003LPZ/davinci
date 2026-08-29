use pi_protocol::ProtocolMessage;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct MemoryTransport {
    pub sender: mpsc::UnboundedSender<ProtocolMessage>,
    pub receiver: mpsc::UnboundedReceiver<ProtocolMessage>,
}

pub fn create_memory_transport_pair() -> (MemoryTransport, MemoryTransport) {
    let (tx1, rx1) = mpsc::unbounded_channel();
    let (tx2, rx2) = mpsc::unbounded_channel();
    (
        MemoryTransport {
            sender: tx1,
            receiver: rx2,
        },
        MemoryTransport {
            sender: tx2,
            receiver: rx1,
        },
    )
}

pub struct PiClient {
    transport: Arc<tokio::sync::Mutex<MemoryTransport>>,
    client_id: String,
}

impl PiClient {
    pub fn new(transport: MemoryTransport, client_id: impl Into<String>) -> Self {
        Self {
            transport: Arc::new(tokio::sync::Mutex::new(transport)),
            client_id: client_id.into(),
        }
    }

    pub async fn send_hello(&self) -> std::io::Result<()> {
        let msg = ProtocolMessage::Hello {
            version: pi_protocol::PROTOCOL_VERSION,
            client_id: self.client_id.clone(),
        };
        let guard = self.transport.lock().await;
        guard.sender.send(msg).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Failed to send hello")
        })?;
        Ok(())
    }
}
