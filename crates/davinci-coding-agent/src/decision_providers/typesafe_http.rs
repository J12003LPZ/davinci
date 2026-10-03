use davinci_agent::decision::{
    provider::DecisionError, response::MAX_RESPONSE_BYTES, NORMAL_DECISION_BUDGET,
};
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Clone)]
pub(super) struct TypeSafeHttp {
    agent: ureq::Agent,
    endpoint: Arc<str>,
    busy: Arc<AtomicBool>,
}

impl TypeSafeHttp {
    pub fn new(endpoint: &str) -> Self {
        Self {
            agent: ureq::AgentBuilder::new().redirects(0).build(),
            endpoint: endpoint.into(),
            busy: Default::default(),
        }
    }

    pub fn send(
        &self,
        key: &str,
        body: &impl serde::Serialize,
        budget: Duration,
        attempts: usize,
    ) -> Result<Vec<u8>, DecisionError> {
        let started = Instant::now();
        if key.trim().is_empty() {
            return Err(DecisionError::CredentialInvalid);
        }
        if budget.is_zero() {
            return Err(deadline_error());
        }
        let body = serde_json::to_string(body)
            .map_err(|e| DecisionError::InvalidRequest(e.to_string()))?;
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(DecisionError::Busy);
        }
        let client = self.clone();
        let authorization = Zeroizing::new(format!("Bearer {key}"));
        let (tx, rx) = mpsc::sync_channel(1);
        // ureq's socket deadline does not bound OS DNS resolution. A bounded
        // worker + deadline receiver covers DNS as well. Keep busy until the
        // actual worker exits; repeated timeouts cannot accumulate threads.
        std::thread::Builder::new()
            .name("jev-http".into())
            .spawn(move || {
                struct Release(Arc<AtomicBool>);
                impl Drop for Release {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }
                let release = Release(client.busy.clone());
                let result = client.attempts(&authorization, &body, started, budget, attempts);
                drop(release);
                let _ = tx.send(result);
            })
            .map_err(|_| {
                self.busy.store(false, Ordering::Release);
                DecisionError::Unavailable("HTTP worker could not start".into())
            })?;
        rx.recv_timeout(budget.saturating_sub(started.elapsed()))
            .map_err(|_| deadline_error())?
    }

    fn attempts(
        &self,
        auth: &str,
        body: &str,
        started: Instant,
        budget: Duration,
        attempts: usize,
    ) -> Result<Vec<u8>, DecisionError> {
        for attempt in 0..attempts {
            let remaining = budget.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(deadline_error());
            }
            let timeout = if attempt == 0 && attempts > 1 {
                remaining.min(NORMAL_DECISION_BUDGET)
            } else {
                remaining
            };
            let result = self
                .agent
                .post(&self.endpoint)
                .timeout(timeout)
                .set("Authorization", auth)
                .set("Content-Type", "application/json")
                .send_string(body);
            let result = match result {
                Ok(response) if (200..300).contains(&response.status()) => read_response(response),
                Ok(response) => Err(DecisionError::HttpStatus(response.status())),
                Err(ureq::Error::Status(status, _)) => Err(DecisionError::HttpStatus(status)),
                Err(ureq::Error::Transport(_)) => Err(DecisionError::Unavailable(
                    "provider network request failed".into(),
                )),
            };
            match result {
                Err(ref e)
                    if e.retryable() && attempt + 1 < attempts && started.elapsed() < budget => {}
                result => return result,
            }
        }
        Err(deadline_error())
    }
}

fn deadline_error() -> DecisionError {
    DecisionError::Unavailable("hard decision deadline exceeded".into())
}

fn read_response(response: ureq::Response) -> Result<Vec<u8>, DecisionError> {
    let mut body = Vec::new();
    response
        .into_reader()
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| DecisionError::Unavailable("provider response unreadable".into()))?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(DecisionError::SchemaMismatch(
            "decision response exceeds the bounded response size".into(),
        ));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};

    fn accept(listener: &TcpListener) -> (TcpStream, std::net::SocketAddr) {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((stream, address)) => {
                    stream.set_nonblocking(false).unwrap();
                    return (stream, address);
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(error) => panic!("fixture accept failed: {error}"),
            }
        }
    }

    fn read_request(stream: &TcpStream) {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut size = 0;
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                size = value.trim().parse().unwrap();
            }
        }
        reader.read_exact(&mut vec![0; size]).unwrap();
    }

    fn reply(mut stream: &TcpStream, status: u16) {
        write!(
            stream,
            "HTTP/1.1 {status} Fixture\r\nContent-Length: 2\r\n\r\n{{}}"
        )
        .unwrap();
        stream.flush().unwrap();
    }

    #[test]
    #[ignore = "paired loopback latency measurement; run with --ignored --nocapture"]
    fn cold_and_warm_http_performance() {
        const SAMPLES: usize = 32;
        let mut measurements = Vec::new();
        for warm in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                if warm {
                    let (stream, _) = accept(&listener);
                    for _ in 0..=SAMPLES {
                        read_request(&stream);
                        reply(&stream, 200);
                    }
                    1
                } else {
                    for _ in 0..SAMPLES {
                        let (stream, _) = accept(&listener);
                        read_request(&stream);
                        reply(&stream, 200);
                    }
                    SAMPLES
                }
            });
            let pooled = TypeSafeHttp::new(&endpoint);
            if warm {
                pooled
                    .send("fixture", &(), Duration::from_secs(1), 1)
                    .unwrap();
            }
            let mut samples = Vec::new();
            for _ in 0..SAMPLES {
                let started = Instant::now();
                let client = if warm {
                    pooled.clone()
                } else {
                    TypeSafeHttp::new(&endpoint)
                };
                assert_eq!(
                    client
                        .send("fixture", &(), Duration::from_secs(1), 1)
                        .unwrap(),
                    b"{}"
                );
                samples.push(started.elapsed().as_micros() as u64);
            }
            samples.sort_unstable();
            let connections = server.join().unwrap();
            assert_eq!(connections, if warm { 1 } else { SAMPLES });
            measurements.push(serde_json::json!({
                "mode": if warm { "warm_pool" } else { "cold_client" },
                "samples": SAMPLES, "connections": connections,
                "p50_us": samples[SAMPLES / 2], "p95_us": samples[(SAMPLES * 95).div_ceil(100) - 1],
                "p99_us": samples[(SAMPLES * 99).div_ceil(100) - 1],
            }));
        }
        println!(
            "JEV_HTTP_PERF {}",
            serde_json::to_string(&measurements).unwrap()
        );
    }

    #[test]
    fn warm_requests_reuse_one_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TypeSafeHttp::new(&format!("http://{}", listener.local_addr().unwrap()));
        let server = std::thread::spawn(move || {
            let (stream, _) = accept(&listener);
            for _ in 0..3 {
                read_request(&stream);
                reply(&stream, 200);
            }
        });
        for _ in 0..3 {
            assert_eq!(
                client
                    .send("fixture", &(), Duration::from_secs(1), 1)
                    .unwrap(),
                b"{}"
            );
        }
        server.join().unwrap();
    }

    #[test]
    fn shadow_has_one_attempt_and_hard_deadline_bounds_body_wait() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TypeSafeHttp::new(&format!("http://{}", listener.local_addr().unwrap()));
        let server = std::thread::spawn(move || {
            let (stream, _) = accept(&listener);
            read_request(&stream);
            reply(&stream, 429);
        });
        assert_eq!(
            client.send("fixture", &(), Duration::from_secs(1), 1),
            Err(DecisionError::HttpStatus(429))
        );
        server.join().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TypeSafeHttp::new(&format!("http://{}", listener.local_addr().unwrap()));
        let server = std::thread::spawn(move || {
            let (stream, _) = accept(&listener);
            read_request(&stream);
            std::thread::sleep(Duration::from_millis(200));
        });
        let started = Instant::now();
        assert!(client
            .send("fixture", &(), Duration::from_millis(60), 1)
            .is_err());
        assert!(started.elapsed() < Duration::from_millis(180));
        server.join().unwrap();
    }

    #[test]
    fn soft_deadline_leaves_time_for_one_retry_before_hard_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TypeSafeHttp::new(&format!("http://{}", listener.local_addr().unwrap()));
        let server = std::thread::spawn(move || {
            let (first, _) = accept(&listener);
            read_request(&first);
            // Keep the first socket silent while accepting the retry.
            let (second, _) = accept(&listener);
            read_request(&second);
            reply(&second, 200);
            drop(first);
        });
        let started = Instant::now();
        client
            .send("fixture", &(), Duration::from_millis(1500), 2)
            .unwrap();
        assert!(started.elapsed() >= NORMAL_DECISION_BUDGET);
        assert!(started.elapsed() < Duration::from_millis(1500));
        server.join().unwrap();
    }
}
