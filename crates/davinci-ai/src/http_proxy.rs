//! HTTP(S) proxy resolution matching TS `utils/node-http-proxy.ts`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use url::Url;

pub const UNSUPPORTED_PROXY_PROTOCOL_MESSAGE: &str =
    "Unsupported proxy protocol. SOCKS and PAC proxy URLs are not supported; use an HTTP or HTTPS proxy URL.";

const DEFAULT_PROXY_PORTS: &[(&str, u16)] = &[
    ("ftp", 21),
    ("gopher", 70),
    ("http", 80),
    ("https", 443),
    ("ws", 80),
    ("wss", 443),
];

/// An explicit `env` map replaces the process environment: a key missing from
/// it resolves to empty instead of falling back to the inherited value.
fn get_proxy_env(key: &str, env: Option<&HashMap<String, String>>) -> String {
    let lower = key.to_ascii_lowercase();
    let upper = key.to_ascii_uppercase();
    if let Some(env) = env {
        return env
            .get(&lower)
            .filter(|value| !value.is_empty())
            .or_else(|| env.get(&upper).filter(|value| !value.is_empty()))
            .cloned()
            .unwrap_or_default();
    }
    std::env::var(&lower)
        .or_else(|_| std::env::var(&upper))
        .unwrap_or_default()
}

fn parse_proxy_target_url(target_url: &str) -> Option<Url> {
    Url::parse(target_url).ok()
}

fn should_proxy_hostname(hostname: &str, port: u16, env: Option<&HashMap<String, String>>) -> bool {
    let no_proxy = get_proxy_env("no_proxy", env).to_ascii_lowercase();
    if no_proxy.is_empty() {
        return true;
    }
    if no_proxy == "*" {
        return false;
    }
    no_proxy
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .all(|proxy| {
            if proxy.is_empty() {
                return true;
            }
            let (mut proxy_hostname, proxy_port) =
                if let Some((host, port_text)) = proxy.rsplit_once(':') {
                    if let Ok(parsed_port) = port_text.parse::<u16>() {
                        (host.to_string(), parsed_port)
                    } else {
                        (proxy.to_string(), 0)
                    }
                } else {
                    (proxy.to_string(), 0)
                };
            if proxy_port != 0 && proxy_port != port {
                return true;
            }
            if !proxy_hostname.starts_with(['.', '*']) {
                // A bare domain also covers its subdomains, on a label boundary.
                return hostname != proxy_hostname
                    && !hostname.ends_with(&format!(".{proxy_hostname}"));
            }
            if let Some(stripped) = proxy_hostname.strip_prefix('*') {
                proxy_hostname = stripped.to_string();
            }
            !hostname.ends_with(&proxy_hostname)
        })
}

fn get_proxy_for_url(target_url: &str, env: Option<&HashMap<String, String>>) -> String {
    let Some(parsed) = parse_proxy_target_url(target_url) else {
        return String::new();
    };
    if parsed.scheme().is_empty() || parsed.host_str().is_none() {
        return String::new();
    }
    let protocol = parsed.scheme();
    let hostname = parsed.host_str().unwrap_or_default();
    let port = parsed.port().unwrap_or_else(|| {
        DEFAULT_PROXY_PORTS
            .iter()
            .find(|(name, _)| *name == protocol)
            .map(|(_, port)| *port)
            .unwrap_or(0)
    });
    if !should_proxy_hostname(hostname, port, env) {
        return String::new();
    }
    let mut proxy = get_proxy_env(&format!("{protocol}_proxy"), env);
    if proxy.is_empty() {
        proxy = get_proxy_env("all_proxy", env);
    }
    if !proxy.is_empty() && !proxy.contains("://") {
        proxy = format!("{protocol}://{proxy}");
    }
    proxy
}

pub fn resolve_http_proxy_url_for_target(
    target_url: &str,
    env: Option<&HashMap<String, String>>,
) -> Result<Option<Url>, String> {
    let proxy = get_proxy_for_url(target_url, env);
    if proxy.is_empty() {
        return Ok(None);
    }
    let proxy_url =
        Url::parse(&proxy).map_err(|error| format!("Invalid proxy URL {proxy:?}: {error}"))?;
    if proxy_url.scheme() != "http" && proxy_url.scheme() != "https" {
        return Err(format!(
            "{UNSUPPORTED_PROXY_PROTOCOL_MESSAGE} Got {}:",
            proxy_url.scheme()
        ));
    }
    Ok(Some(proxy_url))
}

pub fn http_connect_request(target_host: &str, target_port: u16, proxy: &Url) -> String {
    let authority = format!("{target_host}:{target_port}");
    let mut request = format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\n"
    );
    if !proxy.username().is_empty() {
        let password = proxy.password().unwrap_or("");
        let token = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{password}", proxy.username()),
        );
        request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    request.push_str("\r\n");
    request
}

pub fn connect_response_ok(response: &str) -> bool {
    let status_line = response.lines().next().unwrap_or_default();
    let mut fields = status_line.splitn(3, ' ');
    matches!(fields.next(), Some("HTTP/1.1" | "HTTP/1.0")) && fields.next() == Some("200")
}

/// Upper bound on the proxy's CONNECT response headers. Real proxies answer
/// with well under 1 KiB; the cap stops a hostile one from growing memory.
const MAX_CONNECT_RESPONSE_BYTES: usize = 16 * 1024;

/// Tries each address in turn, giving every attempt only what is left of the
/// shared `deadline`, so N unreachable addresses cost one timeout, not N.
fn connect_within_deadline<I, F>(
    addrs: I,
    deadline: Instant,
    mut connect: F,
) -> Result<TcpStream, String>
where
    I: IntoIterator<Item = SocketAddr>,
    F: FnMut(&SocketAddr, Duration) -> std::io::Result<TcpStream>,
{
    let mut last_error = "WebSocket connect failed".to_string();
    for addr in addrs {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            last_error = "WebSocket connect failed: timed out".to_string();
            break;
        }
        match connect(&addr, remaining) {
            Ok(stream) => return Ok(stream),
            Err(err) => last_error = format!("WebSocket connect failed: {err}"),
        }
    }
    Err(last_error)
}

pub fn tcp_connect_via_http_proxy(
    proxy: &Url,
    target_host: &str,
    target_port: u16,
    timeout: Duration,
) -> Result<TcpStream, String> {
    let proxy_host = proxy
        .host_str()
        .ok_or_else(|| "Invalid proxy URL: missing host".to_string())?;
    let proxy_port = proxy
        .port_or_known_default()
        .ok_or_else(|| "Invalid proxy URL: missing port".to_string())?;
    let addrs = (proxy_host, proxy_port)
        .to_socket_addrs()
        .map_err(|err| format!("WebSocket connect failed: {err}"))?;
    // One deadline covers address attempts and the header read.
    let deadline = Instant::now() + timeout;
    let mut tcp = connect_within_deadline(addrs, deadline, |addr, remaining| {
        TcpStream::connect_timeout(addr, remaining)
    })?;
    tcp.set_nodelay(true)
        .map_err(|err| format!("WebSocket connect failed: {err}"))?;
    tcp.set_read_timeout(Some(timeout))
        .map_err(|err| format!("WebSocket connect failed: {err}"))?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(|err| format!("WebSocket connect failed: {err}"))?;
    let request = http_connect_request(target_host, target_port, proxy);
    tcp.write_all(request.as_bytes())
        .and_then(|_| tcp.flush())
        .map_err(|err| format!("WebSocket connect failed: {err}"))?;
    // Read one byte at a time: the stream is returned to the caller as the
    // tunnel, so nothing past the header terminator may be consumed here.
    let mut byte = [0u8; 1];
    let mut collected = Vec::new();
    // The per-read timeout alone never fires on a proxy that keeps
    // trickling bytes, so every read is bounded by the shared deadline.
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("WebSocket connect failed: proxy CONNECT response timed out".to_string());
        }
        tcp.set_read_timeout(Some(remaining))
            .map_err(|err| format!("WebSocket connect failed: {err}"))?;
        let n = tcp
            .read(&mut byte)
            .map_err(|err| format!("WebSocket connect failed: {err}"))?;
        if n == 0 {
            break;
        }
        collected.push(byte[0]);
        if collected.ends_with(b"\r\n\r\n") {
            break;
        }
        if collected.len() > MAX_CONNECT_RESPONSE_BYTES {
            return Err(
                "WebSocket connect failed: proxy CONNECT response headers too large".to_string(),
            );
        }
    }
    let response = String::from_utf8_lossy(&collected);
    if !connect_response_ok(&response) {
        let status = response.lines().next().unwrap_or("proxy CONNECT failed");
        return Err(format!("WebSocket connect failed: {status}"));
    }
    Ok(tcp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn respects_no_proxy_exclusions() {
        let scoped = env(&[
            ("HTTPS_PROXY", "http://proxy.example:8080"),
            ("NO_PROXY", "bedrock-runtime.us-east-1.amazonaws.com"),
        ]);
        assert_eq!(
            resolve_http_proxy_url_for_target(
                "https://bedrock-runtime.us-east-1.amazonaws.com",
                Some(&scoped)
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn resolves_http_and_https_proxy_urls() {
        let scoped = env(&[("HTTPS_PROXY", "http://proxy.example:8080")]);
        assert_eq!(
            resolve_http_proxy_url_for_target(
                "https://bedrock-runtime.us-east-1.amazonaws.com",
                Some(&scoped)
            )
            .unwrap()
            .map(|url| url.to_string()),
            Some("http://proxy.example:8080/".into())
        );
    }

    #[test]
    fn prefers_scoped_proxy_env_before_process() {
        let scoped = env(&[("HTTPS_PROXY", "http://scoped-proxy.example:8080")]);
        assert_eq!(
            resolve_http_proxy_url_for_target(
                "https://bedrock-runtime.us-east-1.amazonaws.com",
                Some(&scoped)
            )
            .unwrap()
            .map(|url| url.to_string()),
            Some("http://scoped-proxy.example:8080/".into())
        );
    }

    #[test]
    fn rejects_socks_and_pac_proxy_urls() {
        let scoped = env(&[("HTTPS_PROXY", "socks5://proxy.example:1080")]);
        let error = resolve_http_proxy_url_for_target(
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            Some(&scoped),
        )
        .unwrap_err();
        assert!(error.starts_with(UNSUPPORTED_PROXY_PROTOCOL_MESSAGE));
        assert!(error.contains("socks5"));
    }

    #[test]
    fn connect_request_and_loopback_handshake_match_ts() {
        let proxy = Url::parse("http://user:secret@proxy.example:8080").unwrap();
        let request = http_connect_request("chatgpt.com", 443, &proxy);
        assert!(request.starts_with("CONNECT chatgpt.com:443 HTTP/1.1\r\n"));
        assert!(request.contains("Host: chatgpt.com:443\r\n"));
        assert!(request.contains("Proxy-Authorization: Basic "));
        assert!(connect_response_ok(
            "HTTP/1.1 200 Connection Established\r\n\r\n"
        ));
        assert!(!connect_response_ok(
            "HTTP/1.1 407 Proxy Authentication Required\r\n\r\n"
        ));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap();
            let received = String::from_utf8_lossy(&buf[..n]).to_string();
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .unwrap();
            received
        });
        let proxy = Url::parse(&format!("http://{addr}")).unwrap();
        let tcp =
            tcp_connect_via_http_proxy(&proxy, "chatgpt.com", 443, Duration::from_secs(2)).unwrap();
        drop(tcp);
        let received = thread.join().unwrap();
        assert!(received.starts_with("CONNECT chatgpt.com:443 HTTP/1.1"));
    }

    #[test]
    fn wor85_explicit_env_map_does_not_fall_back_to_process_env() {
        // Unique key so parallel tests that read the real proxy variables are unaffected.
        std::env::set_var("WOR85_PROBE_PROXY", "http://inherited.example:1");
        let empty = HashMap::new();
        assert_eq!(get_proxy_env("wor85_probe_proxy", Some(&empty)), "");
        let scoped = env(&[("WOR85_PROBE_PROXY", "http://scoped.example:2")]);
        assert_eq!(
            get_proxy_env("wor85_probe_proxy", Some(&scoped)),
            "http://scoped.example:2"
        );
        assert_eq!(
            get_proxy_env("wor85_probe_proxy", None),
            "http://inherited.example:1"
        );
        std::env::remove_var("WOR85_PROBE_PROXY");
    }

    fn proxied(no_proxy: &str, target: &str) -> bool {
        let scoped = env(&[
            ("HTTPS_PROXY", "http://proxy.example:8080"),
            ("NO_PROXY", no_proxy),
        ]);
        resolve_http_proxy_url_for_target(target, Some(&scoped))
            .unwrap()
            .is_some()
    }

    #[test]
    fn wor77_bare_domain_no_proxy_covers_subdomains() {
        assert!(!proxied("example.com", "https://example.com"));
        assert!(!proxied("example.com", "https://api.example.com"));
        assert!(!proxied("example.com", "https://a.b.example.com"));
        assert!(proxied("example.com", "https://evil-example.com"));
        assert!(proxied("example.com", "https://example.com.evil.org"));
        // Port scoped entries keep their port scope.
        assert!(!proxied("example.com:8443", "https://api.example.com:8443"));
        assert!(proxied("example.com:8443", "https://api.example.com"));
    }

    #[test]
    fn wor76_wildcard_entry_inside_list_bypasses_proxy() {
        assert!(!proxied("localhost,*", "https://example.org"));
        assert!(!proxied("*,localhost", "https://example.org"));
        assert!(!proxied("localhost *", "https://example.org"));
        assert!(proxied("localhost,internal.test", "https://example.org"));
    }

    #[test]
    fn wor75_connect_response_requires_exact_200_status_field() {
        assert!(connect_response_ok(
            "HTTP/1.1 200 Connection established\r\n\r\n"
        ));
        assert!(connect_response_ok("HTTP/1.0 200 OK\r\n\r\n"));
        assert!(connect_response_ok("HTTP/1.1 200\r\n\r\n"));
        assert!(!connect_response_ok("HTTP/1.1 2000 Bad\r\n\r\n"));
        assert!(!connect_response_ok("HTTP/1.1 200X\r\n\r\n"));
        assert!(!connect_response_ok("HTTP/1.1  200 OK\r\n\r\n"));
        assert!(!connect_response_ok("HTTP/2 200\r\n\r\n"));
        assert!(!connect_response_ok(""));
    }

    /// Runs `serve` against the first connection and returns the client result
    /// plus how long the client call took.
    fn connect_to_fake_proxy<F>(
        timeout: Duration,
        serve: F,
    ) -> (Result<TcpStream, String>, Duration)
    where
        F: FnOnce(TcpStream) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            serve(stream);
        });
        let proxy = Url::parse(&format!("http://{addr}")).unwrap();
        let started = Instant::now();
        let result = tcp_connect_via_http_proxy(&proxy, "chatgpt.com", 443, timeout);
        let elapsed = started.elapsed();
        if result.is_err() {
            server.join().unwrap();
        }
        (result, elapsed)
    }

    #[test]
    fn wor69_connect_response_headers_are_size_capped() {
        let (result, _) = connect_to_fake_proxy(Duration::from_secs(10), |mut stream| {
            // Endless header bytes with no CRLFCRLF terminator, then close.
            let chunk = [b'a'; 4096];
            for _ in 0..64 {
                if stream.write_all(&chunk).is_err() {
                    return;
                }
            }
        });
        let error = result.unwrap_err();
        assert!(error.contains("too large"), "{error}");
    }

    #[test]
    fn wor69_connect_response_has_total_deadline_against_slow_drip() {
        let (result, elapsed) = connect_to_fake_proxy(Duration::from_millis(400), |mut stream| {
            // One byte every 100ms keeps every individual read under the read timeout.
            for _ in 0..60 {
                if stream.write_all(b"a").is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        assert!(result.is_err());
        assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");
    }

    #[test]
    fn wor70_connect_keeps_tunneled_bytes_that_arrive_with_headers() {
        let (result, _) = connect_to_fake_proxy(Duration::from_secs(5), |mut stream| {
            // Headers and the first tunneled bytes in a single write.
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\ntunnel-payload")
                .unwrap();
            thread::sleep(Duration::from_millis(200));
        });
        let mut tcp = result.unwrap();
        let mut payload = [0u8; 14];
        tcp.read_exact(&mut payload).unwrap();
        assert_eq!(&payload, b"tunnel-payload");
    }

    #[test]
    fn wor80_connect_deadline_is_shared_across_resolved_addresses() {
        let addrs: Vec<SocketAddr> = (1..=4)
            .map(|port| SocketAddr::from(([192, 0, 2, 1], port)))
            .collect();
        let timeout = Duration::from_millis(300);
        let started = Instant::now();
        let mut attempts = 0;
        let result = connect_within_deadline(addrs, started + timeout, |_, remaining| {
            attempts += 1;
            // An unreachable address burns whatever time it is granted.
            thread::sleep(remaining);
            Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
        });
        let elapsed = started.elapsed();
        assert!(result.is_err());
        assert_eq!(attempts, 1);
        assert!(elapsed < Duration::from_millis(600), "took {elapsed:?}");
    }
}
