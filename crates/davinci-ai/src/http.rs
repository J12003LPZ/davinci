//! Shared HTTP agents for provider and control requests.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub(crate) const PROVIDER_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const CONTROL_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Returns a pooled agent whose timeout bounds each socket read. 4xx and 5xx
/// answers are `Err(ureq::Error::StatusCode)`; use `.config()` on a request to
/// read the error body instead.
pub(crate) fn agent(idle: Duration) -> ureq::Agent {
    static AGENTS: OnceLock<Mutex<HashMap<(Duration, bool), ureq::Agent>>> = OnceLock::new();
    let budgeted = crate::provider_observation::active_budget().is_some();
    let agents = AGENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut agents = agents.lock().unwrap_or_else(|err| err.into_inner());
    agents
        .entry((idle, budgeted))
        .or_insert_with(|| crate::stream_http::pooled_agent(idle))
        .clone()
}

/// Match ureq 2's `set`: replace ordinary headers, append `X-*` headers.
pub(crate) fn set_header<S>(
    mut request: ureq::RequestBuilder<S>,
    name: &str,
    value: &str,
) -> ureq::RequestBuilder<S> {
    if !name.to_ascii_lowercase().starts_with("x-") {
        if let Some(headers) = request.headers_mut() {
            headers.remove(name);
        }
    }
    request.header(name, value)
}

/// ureq 2's `into_json` had no size cap (`read_json` caps at 10 MB), and an
/// image answer is base64 in JSON.
/// The largest decoded JSON response body accepted from a provider.
pub(crate) const MAX_JSON_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) fn read_json<T: serde::de::DeserializeOwned>(
    response: &mut ureq::http::Response<ureq::Body>,
) -> Result<T, ureq::Error> {
    response
        .body_mut()
        .with_config()
        .limit(MAX_JSON_RESPONSE_BYTES)
        .read_json()
}

#[cfg(test)]
mod tests {
    use super::agent;
    use std::io::Read;
    use std::time::{Duration, Instant};

    #[test]
    fn header_overrides_replace_authorization_and_content_type_but_append_x_headers() {
        let request = agent(Duration::from_secs(1))
            .post("http://localhost/")
            .header("Authorization", "old")
            .header("Content-Type", "text/plain")
            .header("X-Custom", "first");
        let request = super::set_header(request, "authorization", "new");
        let request = super::set_header(request, "content-type", "application/json");
        let request = super::set_header(request, "x-custom", "second");
        let headers = request.headers_ref().unwrap();
        assert_eq!(headers.get_all("authorization").iter().count(), 1);
        assert_eq!(headers["authorization"], "new");
        assert_eq!(headers.get_all("content-type").iter().count(), 1);
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(headers.get_all("x-custom").iter().count(), 2);
    }

    #[test]
    fn budgeted_requests_never_follow_redirects() {
        use crate::provider_observation::{
            AttemptBudget, ObservationScope, ProviderAttemptObservation,
        };
        use std::io::{BufRead, BufReader, Write};
        struct Budget;
        impl AttemptBudget for Budget {
            fn reserve(
                &self,
                _: &ProviderAttemptObservation,
                _: Option<u64>,
            ) -> Result<(), String> {
                Ok(())
            }
            fn reconcile(&self, _: &ProviderAttemptObservation) -> Result<(), String> {
                Ok(())
            }
        }
        for (streaming, status) in [(false, 303), (true, 303), (false, 307), (true, 307)] {
            let source = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let destination = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            destination.set_nonblocking(true).unwrap();
            let url = format!("http://{}", source.local_addr().unwrap());
            let target = format!("http://{}", destination.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut socket, _) = source.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    assert!(!line.is_empty());
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap();
                        }
                    }
                }
                reader.read_exact(&mut vec![0; length]).unwrap();
                write!(socket, "HTTP/1.1 {status} Redirect\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            });
            let _scope = ObservationScope::capture().with_budget(std::sync::Arc::new(Budget));
            if streaming {
                let _ = crate::stream_http::send(
                    &url,
                    &[],
                    &serde_json::json!({}),
                    Some(200),
                    false,
                    None,
                );
            } else {
                let _ = agent(Duration::from_millis(200)).post(&url).send("{}");
            }
            server.join().unwrap();
            assert_eq!(
                destination.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock,
                "a budgeted request followed a redirect (streaming={streaming})"
            );
        }
    }

    #[test]
    fn a_silent_server_hits_the_idle_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            std::thread::sleep(Duration::from_secs(5));
        });

        let started = Instant::now();
        let result = agent(Duration::from_millis(300))
            .post(&format!("http://{addr}/v1/x"))
            .send("{}");

        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        server.join().unwrap();
    }
}
