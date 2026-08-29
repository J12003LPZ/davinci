use pi_client::{create_memory_transport_pair, PiClient};
use pi_protocol::ProtocolMessage;
use pi_server::PiServer;

#[tokio::test]
async fn test_client_server_handshake() {
    let (t_client, t_server) = create_memory_transport_pair();
    let client = PiClient::new(t_client, "client-001");
    let server = PiServer::new(t_server.sender, t_server.receiver);

    client.send_hello().await.expect("send hello");
    let msg = server.handle_next().await.expect("handle message");
    match msg {
        ProtocolMessage::Hello { client_id, .. } => {
            assert_eq!(client_id, "client-001");
        }
        _ => panic!("Expected Hello message"),
    }
}
