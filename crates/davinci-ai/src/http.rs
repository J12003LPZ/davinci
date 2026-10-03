//! Shared HTTP agents for provider and control requests.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub(crate) const PROVIDER_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const CONTROL_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// Returns a pooled agent whose timeout bounds each socket read.
pub(crate) fn agent(idle: Duration) -> ureq::Agent {
    static AGENTS: OnceLock<Mutex<HashMap<(Duration, bool), ureq::Agent>>> = OnceLock::new();
    let budgeted = crate::provider_observation::active_budget().is_some();
    let agents = AGENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut agents = agents.lock().unwrap_or_else(|err| err.into_inner());
    agents
        .entry((idle, budgeted))
        .or_insert_with(|| {
            ureq::AgentBuilder::new()
                // A redirect is another unreserved request to an unvalidated URL.
                .redirects(if budgeted { 0 } else { 5 })
                .timeout_connect(CONNECT_TIMEOUT)
                .timeout_read(idle)
                .timeout_write(WRITE_TIMEOUT)
                .build()
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::agent;
    use std::io::Read;
    use std::time::{Duration, Instant};

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
                let _ = agent(Duration::from_millis(200))
                    .post(&url)
                    .send_string("{}");
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
            .send_string("{}");

        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        server.join().unwrap();
    }
}
