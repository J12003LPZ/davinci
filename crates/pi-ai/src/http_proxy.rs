//! HTTP(S) proxy resolution matching TS `utils/node-http-proxy.ts`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

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

fn get_proxy_env(key: &str, env: Option<&HashMap<String, String>>) -> String {
    let lower = key.to_ascii_lowercase();
    let upper = key.to_ascii_uppercase();
    if let Some(env) = env {
        if let Some(value) = env.get(&lower).filter(|value| !value.is_empty()) {
            return value.clone();
        }
        if let Some(value) = env.get(&upper).filter(|value| !value.is_empty()) {
            return value.clone();
        }
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
                return hostname != proxy_hostname;
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
    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
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
    let mut last_error = "WebSocket connect failed".to_string();
    let mut tcp = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(err) => last_error = format!("WebSocket connect failed: {err}"),
        }
    }
    let mut tcp = tcp.ok_or(last_error)?;
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
    let mut buf = vec![0u8; 4096];
    let mut collected = Vec::new();
    loop {
        let n = tcp
            .read(&mut buf)
            .map_err(|err| format!("WebSocket connect failed: {err}"))?;
        if n == 0 {
            break;
        }
        collected.extend_from_slice(&buf[..n]);
        if collected.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
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
}
