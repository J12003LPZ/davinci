//! ureq 3 agents that behave like the ureq 2 agents this crate used before:
//! no proxy environment, a 4xx or 5xx answer is an `Err`, no size cap on a
//! JSON body. A call site changes only what it needs to.

use std::time::Duration;

use ureq::config::ConfigBuilder;
use ureq::typestate::AgentScope;

/// A config builder with ureq 2's defaults. ureq 3 reads `HTTP_PROXY` and
/// friends unless told not to; ureq 2 never did.
pub fn config() -> ConfigBuilder<AgentScope> {
    ureq::Agent::config_builder().proxy(None).max_redirects(5)
}

/// An agent with no timeouts.
pub fn agent() -> ureq::Agent {
    config().build().into()
}

/// An agent whose whole call, body included, stops after `timeout` (ureq 2
/// `AgentBuilder::timeout`).
pub fn agent_with_timeout(timeout: Duration) -> ureq::Agent {
    config().timeout_global(Some(timeout)).build().into()
}

/// Send `body` as compact JSON with `Content-Type: application/json`, as ureq
/// 2 did. ureq 3's own `send_json` pretty-prints and appends a charset.
pub fn send_json(
    request: ureq::RequestBuilder<ureq::typestate::WithBody>,
    body: &impl serde::Serialize,
) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let bytes = serde_json::to_vec(body)?;
    request
        .header("Content-Type", "application/json")
        .send(&bytes)
}

/// The body as JSON. ureq 2's `into_json` had no size cap; `read_json` caps at
/// 10 MB.
pub fn read_json<T: serde::de::DeserializeOwned>(
    response: &mut ureq::http::Response<ureq::Body>,
) -> Result<T, ureq::Error> {
    response
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_json()
}

/// A response header as text, when it is valid text.
pub fn header<'a>(response: &'a ureq::http::Response<ureq::Body>, name: &str) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Serve `reply` to the next connection, return the URL.
    fn serve(reply: &'static str) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let _ = socket.read(&mut buffer);
            socket.write_all(reply.as_bytes()).unwrap();
        });
        (url, handle)
    }

    #[test]
    fn send_json_sends_compact_json_with_the_plain_content_type() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut buffer = [0u8; 1024];
            while !String::from_utf8_lossy(&seen).contains("}") {
                let n = socket.read(&mut buffer).unwrap();
                seen.extend_from_slice(&buffer[..n]);
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            String::from_utf8_lossy(&seen).to_lowercase()
        });
        send_json(agent().post(&url), &serde_json::json!({"a": [1, 2]})).unwrap();
        let request = server.join().unwrap();
        assert!(request.ends_with("{\"a\":[1,2]}"), "{request}");
        assert!(
            request.contains("content-type: application/json\r\n"),
            "{request}"
        );
    }

    #[test]
    fn agents_keep_the_five_redirect_limit() {
        assert_eq!(agent().config().max_redirects(), 5);
        assert_eq!(
            agent_with_timeout(Duration::from_secs(10))
                .config()
                .max_redirects(),
            5
        );
    }

    #[test]
    fn a_client_error_is_an_err_with_its_status() {
        let (url, server) =
            serve("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let error = agent().get(&url).call().unwrap_err();
        server.join().unwrap();
        assert!(matches!(error, ureq::Error::StatusCode(404)), "{error:?}");
    }

    #[test]
    fn a_json_body_over_ten_megabytes_is_read_whole() {
        let body = format!("[\"{}\"]", "x".repeat(11 * 1024 * 1024));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let _ = socket.read(&mut buffer);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).unwrap();
            socket.write_all(body.as_bytes()).unwrap();
        });
        let mut response = agent().get(&url).call().unwrap();
        let parsed: Vec<String> = read_json(&mut response).unwrap();
        server.join().unwrap();
        assert_eq!(parsed[0].len(), 11 * 1024 * 1024);
    }

    #[test]
    fn the_whole_call_stops_at_the_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (_socket, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(2));
        });
        let started = std::time::Instant::now();
        let result = agent_with_timeout(Duration::from_millis(300))
            .get(&url)
            .call();
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        server.join().unwrap();
    }
}
