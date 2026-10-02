//! Cancellable provider HTTP reads. Poll beneath TLS and HTTP framing so a
//! timeout never discards a partial TLS record, chunk header, or SSE line.
use crate::provider_retry::{ProviderError, RequestPhase};
use std::io::{self, Read};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use ureq_stream::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, NextTimeout, RustlsConnector, TcpConnector, Transport,
};

const CANCEL_POLL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Default)]
pub(crate) struct Cancellation {
    closed: Arc<AtomicBool>,
    abort: Option<Arc<AtomicBool>>,
}

impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    fn check(&self) -> Result<(), ureq_stream::Error> {
        if self.closed.load(Ordering::Relaxed)
            || self
                .abort
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            // Interrupted would be retried automatically by Read helpers.
            Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Request aborted").into())
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
struct CancelConnector {
    cancellation: Cancellation,
    idle: Duration,
}

impl<T: Transport> Connector<T> for CancelConnector {
    type Out = CancelTransport<T>;
    fn connect(
        &self,
        _: &ConnectionDetails,
        chained: Option<T>,
    ) -> Result<Option<Self::Out>, ureq_stream::Error> {
        self.cancellation.check()?;
        Ok(chained.map(|inner| CancelTransport {
            inner,
            cancellation: self.cancellation.clone(),
            idle: self.idle,
        }))
    }
}

#[derive(Debug)]
struct CancelTransport<T> {
    inner: T,
    cancellation: Cancellation,
    idle: Duration,
}

impl<T: Transport> Transport for CancelTransport<T> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }
    fn transmit_output(
        &mut self,
        amount: usize,
        timeout: NextTimeout,
    ) -> Result<(), ureq_stream::Error> {
        self.cancellation.check()?;
        self.inner.transmit_output(amount, timeout)
    }
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq_stream::Error> {
        let started = Instant::now();
        let limit = self.idle.min(*timeout.after);
        loop {
            self.cancellation.check()?;
            let remaining = limit.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(ureq_stream::Error::Timeout(timeout.reason));
            }
            let slice = NextTimeout {
                after: remaining.min(CANCEL_POLL).into(),
                reason: timeout.reason,
            };
            match self.inner.await_input(slice) {
                Err(ureq_stream::Error::Timeout(_)) => continue,
                result => return result,
            }
        }
    }
    fn is_open(&mut self) -> bool {
        self.cancellation.check().is_ok() && self.inner.is_open()
    }
    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

pub(crate) struct Response {
    response: ureq_stream::http::Response<ureq_stream::Body>,
    cancellation: Cancellation,
}

impl Response {
    pub(crate) fn status(&self) -> u16 {
        self.response.status().as_u16()
    }
    pub(crate) fn headers_names(&self) -> Vec<String> {
        self.response
            .headers()
            .keys()
            .map(|name| name.to_string())
            .collect()
    }
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    }
    pub(crate) fn into_reader(self) -> (impl Read + Send + 'static, Cancellation) {
        (self.response.into_body().into_reader(), self.cancellation)
    }
    pub(crate) fn into_string(self) -> io::Result<String> {
        let (reader, _) = self.into_reader();
        let mut text = String::new();
        reader
            .take(crate::stream_reader::MAX_FRAME_BYTES as u64 + 1)
            .read_to_string(&mut text)?;
        if text.len() > crate::stream_reader::MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "provider response exceeds 16 MiB",
            ));
        }
        Ok(text)
    }
}

pub(crate) fn send(
    url: &str,
    headers: &[(String, String)],
    body: &serde_json::Value,
    timeout_ms: Option<u64>,
    compress_zstd: bool,
    abort: Option<Arc<AtomicBool>>,
) -> Result<Response, ProviderError> {
    let idle = timeout_ms
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(crate::http::PROVIDER_IDLE_TIMEOUT);
    let cancellation = Cancellation {
        abort,
        ..Cancellation::default()
    };
    let config = ureq_stream::Agent::config_builder()
        .http_status_as_error(false)
        // Match the existing provider agent: no environment proxy feature.
        .proxy(None)
        .max_redirects(5)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_resolve(Some(Duration::from_secs(30)))
        .timeout_send_request(Some(Duration::from_secs(60)))
        .timeout_send_body(Some(Duration::from_secs(60)))
        // A cancellation flag belongs to one request, never a pooled socket.
        .max_idle_connections(0)
        .build();
    let connector = TcpConnector::default()
        .chain(CancelConnector {
            cancellation: cancellation.clone(),
            idle,
        })
        .chain(RustlsConnector::default());
    let agent = ureq_stream::Agent::with_parts(
        config,
        connector,
        ureq_stream::unversioned::resolver::DefaultResolver::default(),
    );
    let mut request = agent.post(url);
    for (key, value) in headers {
        request = request.header(key.as_str(), value.as_str());
    }
    let bytes = if compress_zstd {
        let (bytes, compressed) = crate::codex::encode_codex_sse_body(body);
        if compressed {
            request = request.header("content-encoding", "zstd");
        }
        bytes
    } else {
        body.to_string().into_bytes()
    };
    let response = request.send(bytes.as_slice()).map_err(|error| {
        let phase = match &error {
            ureq_stream::Error::HostNotFound | ureq_stream::Error::ConnectionFailed => {
                RequestPhase::Connect
            }
            ureq_stream::Error::Io(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                RequestPhase::Connect
            }
            _ => RequestPhase::Response,
        };
        ProviderError::new(None, format!("Provider request failed: {error}")).with_phase(phase)
    })?;
    let response = Response {
        response,
        cancellation,
    };
    if response.status() >= 400 {
        let mut error = ProviderError::new(
            Some(response.status()),
            format!("Provider error: {}", response.status()),
        );
        for name in ["retry-after", "retry-after-ms", "x-should-retry"] {
            if let Some(value) = response.header(name) {
                error = error.with_header(name, value);
            }
        }
        if let Ok(text) = response.into_string() {
            if !text.is_empty() {
                error.message = format!("Provider request failed: {text}");
            }
        }
        return Err(error);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn consume_request(socket: &mut std::net::TcpStream) {
        let mut reader = BufReader::new(socket);
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(!line.is_empty());
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        reader.read_exact(&mut vec![0; length]).unwrap();
    }

    #[test]
    fn audit_slow_chunk_headers_and_payload_survive_cancellation_polls() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let expected = format!("data: {}\n\n", "x".repeat(40));
        let body = expected.clone();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            consume_request(&mut socket);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            let chunk = format!("{:x}\r\n{body}\r\n0\r\n\r\n", body.len());
            for part in [&chunk[..1], &chunk[1..12], &chunk[12..]] {
                socket.write_all(part.as_bytes()).unwrap();
                std::thread::sleep(Duration::from_millis(180));
            }
        });
        let response = send(&url, &[], &serde_json::json!({}), Some(1500), false, None).unwrap();
        assert_eq!(response.into_string().unwrap(), expected);
        server.join().unwrap();
    }

    #[test]
    fn audit_tls_handshake_cancellation_closes_the_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "https://localhost:{}",
            listener.local_addr().unwrap().port()
        );
        let abort = Arc::new(AtomicBool::new(false));
        let flag = abort.clone();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            // Read the client's TLS record, then leave the handshake silent.
            let mut header = [0; 5];
            socket.read_exact(&mut header).unwrap();
            assert_eq!(header[0], 22); // TLS handshake
            let length = u16::from_be_bytes([header[3], header[4]]) as usize;
            socket.read_exact(&mut vec![0; length]).unwrap();
            std::thread::sleep(Duration::from_millis(180));
            flag.store(true, Ordering::Relaxed);
            let mut bytes = [0; 1024];
            loop {
                match socket.read(&mut bytes) {
                    Ok(0) => return true,
                    Ok(_) => continue,
                    Err(error) => {
                        return matches!(
                            error.kind(),
                            io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
                        )
                    }
                }
            }
        });
        let result = send(&url, &[], &serde_json::json!({}), None, false, Some(abort));
        assert!(result.err().unwrap().message.contains("aborted"));
        assert!(server.join().unwrap(), "TLS socket survived cancellation");
    }

    #[test]
    fn audit_silent_body_still_obeys_the_configured_idle_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            consume_request(&mut socket);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).unwrap_or(0), 0);
        });
        let response = send(&url, &[], &serde_json::json!({}), Some(300), false, None).unwrap();
        let started = Instant::now();
        assert!(response.into_string().is_err());
        assert!(started.elapsed() >= Duration::from_millis(250));
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }
}
