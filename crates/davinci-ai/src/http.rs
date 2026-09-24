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
    static AGENTS: OnceLock<Mutex<HashMap<Duration, ureq::Agent>>> = OnceLock::new();
    let agents = AGENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut agents = agents.lock().unwrap_or_else(|err| err.into_inner());
    agents
        .entry(idle)
        .or_insert_with(|| {
            ureq::AgentBuilder::new()
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
