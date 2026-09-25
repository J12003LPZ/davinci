//! Streamable HTTP: one JSON-RPC message per `POST`, answered as JSON or as
//! an SSE stream. The `Mcp-Session-Id` the server hands back on
//! `initialize` and the negotiated `MCP-Protocol-Version` ride on every
//! later request; the session is `DELETE`d best-effort on drop.

use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

use crate::jsonrpc::{Notification, Request, Response};
use crate::{Error, Result, Rpc, CALL_TIMEOUT_SECS, PROTOCOL_VERSION};

/// The `initialize` result names the version both sides will speak; until
/// then requests carry none.
const SESSION_HEADER: &str = "Mcp-Session-Id";
const VERSION_HEADER: &str = "MCP-Protocol-Version";

/// `PI_MCP_FIXTURE` (or a `fixture:<path>` URL) is a JSON object
/// `{ method: result }` so tests never touch the network. A fixture value
/// of `{ "$rpcError": { "code", "message" } }` answers with that JSON-RPC
/// error instead of a result.
pub struct HttpTransport {
    url: String,
    headers: BTreeMap<String, String>,
    agent: ureq::Agent,
    next_id: u64,
    fixture: Option<Value>,
    session_id: Option<String>,
    protocol_version: Option<String>,
}

/// What one `POST` came back with.
struct Reply {
    session_id: Option<String>,
    value: Value,
}

impl HttpTransport {
    pub fn new(url: &str, headers: BTreeMap<String, String>) -> Result<Self> {
        let fixture = load_fixture(url);
        Ok(Self {
            url: url.to_string(),
            headers,
            agent: build_agent(Duration::from_secs(CALL_TIMEOUT_SECS)),
            next_id: 1,
            fixture,
            session_id: None,
            protocol_version: None,
        })
    }

    pub fn set_call_timeout(&mut self, timeout: Duration) {
        self.agent = build_agent(timeout);
    }

    /// Headers every request carries: content negotiation, the user's own,
    /// then the session id and protocol version once `initialize` set them.
    fn request_headers(&self) -> Vec<(String, String)> {
        let mut out = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            (
                "Accept".to_string(),
                "application/json, text/event-stream".to_string(),
            ),
        ];
        for (key, value) in &self.headers {
            out.push((key.clone(), value.clone()));
        }
        if let Some(session) = &self.session_id {
            out.push((SESSION_HEADER.to_string(), session.clone()));
        }
        if let Some(version) = &self.protocol_version {
            out.push((VERSION_HEADER.to_string(), version.clone()));
        }
        out
    }

    /// Remember what `initialize` negotiated: the server's session id (if it
    /// issued one) and the protocol version it answered with.
    fn absorb_initialize(&mut self, session_id: Option<String>, result: &Value) {
        if session_id.is_some() {
            self.session_id = session_id;
        }
        let version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(PROTOCOL_VERSION);
        self.protocol_version = Some(version.to_string());
    }

    fn post(&mut self, body: &Value, want: Option<&Value>) -> Result<Reply> {
        let response = self.send_post(body)?;
        let content_type = response
            .header("content-type")
            .unwrap_or("application/json")
            .to_string();
        let session_id = response
            .header(SESSION_HEADER)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let value = if content_type.contains("text/event-stream") {
            self.read_sse_response(response.into_reader(), want)?
        } else {
            let text = read_response_body(response.into_reader(), MAX_BODY_BYTES)?;
            parse_http_body(&content_type, &text, want)?
        };
        Ok(Reply { session_id, value })
    }

    fn send_post(&self, body: &Value) -> Result<ureq::Response> {
        let mut request = self.agent.post(&self.url);
        for (key, value) in self.request_headers() {
            request = request.set(&key, &value);
        }
        match request.send_json(body) {
            Ok(response) => Ok(response),
            Err(ureq::Error::Status(code, response)) => {
                let mut text = String::new();
                let _ = response.into_reader().take(4096).read_to_string(&mut text);
                let text = text.trim();
                Err(Error::Transport(if text.is_empty() {
                    format!("mcp http {code}")
                } else {
                    format!("mcp http {code}: {text}")
                }))
            }
            Err(err) => Err(Error::Transport(format!("mcp http: {err}"))),
        }
    }

    fn answer_server_request(&self, id: Value, method: &str) -> Result<()> {
        let reply = if method == "ping" {
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": {} })
        } else {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not supported" }
            })
        };
        let _ = self.send_post(&reply)?;
        Ok(())
    }

    fn read_sse_response(
        &self,
        reader: impl Read,
        want: Option<&Value>,
    ) -> Result<Value> {
        // Bound the underlying stream before read_line so one unterminated
        // SSE line cannot allocate past the response cap before we inspect it.
        let mut reader = BufReader::new(reader.take((MAX_BODY_BYTES + 1) as u64));
        let mut line = String::new();
        let mut data = String::new();
        let mut total = 0usize;
        let mut last = None;
        loop {
            line.clear();
            let read = reader
                .read_line(&mut line)
                .map_err(|err| Error::Transport(format!("mcp http SSE: {err}")))?;
            if read == 0 {
                break;
            }
            total = total.saturating_add(read);
            if total > MAX_BODY_BYTES {
                return Err(Error::Transport(format!(
                    "mcp http body exceeds {MAX_BODY_BYTES} bytes"
                )));
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                if data.is_empty() {
                    continue;
                }
                let event = std::mem::take(&mut data);
                let Ok(value) = serde_json::from_str::<Value>(&event) else {
                    continue;
                };
                if let (Some(id), Some(method)) = (
                    value.get("id").cloned(),
                    value.get("method").and_then(Value::as_str),
                ) {
                    self.answer_server_request(id, method)?;
                    continue;
                }
                match want {
                    Some(id) if value.get("id") == Some(id) => {
                        validate_response(&value, id)?;
                        return Ok(value);
                    }
                    Some(_) => {}
                    None => last = Some(value),
                }
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            }
        }
        if !data.is_empty() {
            let value: Value = serde_json::from_str(&data)
                .map_err(|err| Error::Transport(format!("mcp http SSE: {err}")))?;
            if let Some(id) = want {
                if value.get("id") == Some(id) {
                    validate_response(&value, id)?;
                    return Ok(value);
                }
            } else {
                last = Some(value);
            }
        }
        match want {
            Some(id) => Err(Error::Transport(format!(
                "SSE stream ended without a response for id {id}"
            ))),
            None => last.ok_or_else(|| Error::Transport("SSE body had no data frame".into())),
        }
    }

    fn fixture_answer(fixture: &Value, method: &str) -> Result<Value> {
        let value = fixture
            .get(method)
            .cloned()
            .ok_or_else(|| Error::Protocol(format!("fixture has no `{method}`")))?;
        if let Some(error) = value.get("$rpcError") {
            return Err(Error::Rpc {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("fixture error")
                    .to_string(),
            });
        }
        Ok(value)
    }
}

impl Rpc for HttpTransport {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        if let Some(fixture) = &self.fixture {
            let result = Self::fixture_answer(fixture, method)?;
            if method == "initialize" {
                self.absorb_initialize(None, &result);
            }
            return Ok(result);
        }
        let request = Request::new(id, method, params);
        let body = serde_json::to_value(&request)
            .map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        let wanted = Value::from(id);
        let reply = self.post(&body, Some(&wanted))?;
        let parsed = reply.value;
        if parsed.is_null() {
            return Err(Error::Transport(format!(
                "mcp http: empty reply to {method}"
            )));
        }
        let response: Response = serde_json::from_value(parsed)
            .map_err(|err| Error::Protocol(format!("decode {method} reply: {err}")))?;
        if let Some(error) = response.error {
            return Err(Error::Rpc {
                code: error.code,
                message: error.message,
            });
        }
        let result = response.result.unwrap_or(Value::Null);
        if method == "initialize" {
            self.absorb_initialize(reply.session_id, &result);
        }
        Ok(result)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        if self.fixture.is_some() {
            return Ok(());
        }
        let note = Notification::new(method, params);
        let body =
            serde_json::to_value(&note).map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        // A compliant server answers `202 Accepted` with no body; anything
        // it does send back is not addressed to a request of ours.
        let _ = self.post(&body, None)?;
        Ok(())
    }
}

impl Drop for HttpTransport {
    fn drop(&mut self) {
        let (Some(session), None) = (&self.session_id, &self.fixture) else {
            return;
        };
        let mut request = self
            .agent
            .delete(&self.url)
            .timeout(Duration::from_secs(2))
            .set(SESSION_HEADER, session);
        for (key, value) in &self.headers {
            request = request.set(key, value);
        }
        let _ = request.call();
    }
}

const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

fn read_response_body(reader: impl Read, limit: usize) -> Result<String> {
    let mut text = String::new();
    reader
        .take(limit.saturating_add(1) as u64)
        .read_to_string(&mut text)
        .map_err(|err| Error::Transport(format!("mcp http body: {err}")))?;
    if text.len() > limit {
        return Err(Error::Transport(format!(
            "mcp http body exceeds {limit} bytes"
        )));
    }
    Ok(text)
}

fn validate_response(value: &Value, wanted: &Value) -> Result<()> {
    let valid = !wanted.is_null()
        && value.get("id") == Some(wanted)
        && value.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && value.get("method").is_none()
        && (value.get("result").is_some() != value.get("error").is_some());
    if !valid {
        return Err(Error::Protocol(
            "invalid or uncorrelated MCP response envelope".into(),
        ));
    }
    Ok(())
}

fn build_agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(timeout).build()
}

/// `fixture:<path>` loads a method→result map from disk so tests never hit
/// the network and never share `PI_MCP_FIXTURE` across threads.
fn load_fixture(url: &str) -> Option<Value> {
    if !crate::fixtures::enabled() {
        return None;
    }
    if let Some(path) = url.strip_prefix("fixture:") {
        return std::fs::read_to_string(path)
            .ok()
            .and_then(|body| serde_json::from_str(&body).ok());
    }
    std::env::var("PI_MCP_FIXTURE")
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|body| serde_json::from_str(&body).ok())
}

/// Decode one HTTP reply. An empty body is `Null` (a `202 Accepted` to a
/// notification). An SSE body is split into events, each event's `data:`
/// lines joined, and the response whose `id` is `want` returned — server
/// notifications and requests interleaved in the stream are skipped. With
/// no `want`, the last event wins.
pub fn parse_http_body(content_type: &str, text: &str, want: Option<&Value>) -> Result<Value> {
    if text.len() > MAX_BODY_BYTES {
        return Err(Error::Transport(
            "mcp http body exceeds the input limit".into(),
        ));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    if !content_type.contains("text/event-stream") {
        let value =
            serde_json::from_str(text).map_err(|err| Error::Transport(format!("json: {err}")))?;
        if let Some(id) = want {
            validate_response(&value, id)?;
        }
        return Ok(value);
    }
    let mut last = None;
    for data in sse_events(text) {
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        match want {
            Some(id) => {
                if value.get("method").is_none() && value.get("id") == Some(id) {
                    validate_response(&value, id)?;
                    return Ok(value);
                }
            }
            None => last = Some(value),
        }
    }
    match want {
        Some(id) => Err(Error::Transport(format!(
            "SSE body had no response for id {id}"
        ))),
        None => last.ok_or_else(|| Error::Transport("SSE body had no data frame".into())),
    }
}

/// The `data` payload of every event in an SSE body, multi-line `data:`
/// fields joined with `\n` as the spec says.
fn sse_events(text: &str) -> impl Iterator<Item = String> + '_ {
    // Materialize only one event at a time, and stop when the caller finds its
    // reply. The aggregate input was already bounded before reaching this point.
    let mut lines = text.lines().chain(std::iter::once(""));
    std::iter::from_fn(move || {
        let mut data = String::new();
        let mut seen = false;
        for line in lines.by_ref() {
            if line.is_empty() {
                if seen {
                    return Some(data);
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                if seen {
                    data.push('\n');
                }
                data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                seen = true;
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read_test_request(stream: &mut std::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 1024];
        let mut header_end = None;
        while header_end.is_none() {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            header_end = bytes.windows(4).position(|window| window == b"\r\n\r\n").map(|i| i + 4);
        }
        if let Some(end) = header_end {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            while bytes.len() < end + length {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn sse_reply_returns_before_the_stream_closes() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_test_request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            write!(
                stream,
                "data: {{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"ok\":true}}}}\n\n"
            )
            .unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_secs(2));
        });
        let mut transport =
            HttpTransport::new(&format!("http://{addr}/mcp"), BTreeMap::new()).unwrap();
        transport.set_call_timeout(Duration::from_secs(5));
        let started = std::time::Instant::now();
        let result = transport.call("tools/call", serde_json::json!({})).unwrap();
        assert_eq!(result["ok"], true);
        assert!(started.elapsed() < Duration::from_secs(1));
        server.join().unwrap();
    }

    #[test]
    fn sse_ping_is_answered_before_our_reply() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let _ = read_test_request(&mut first);
            write!(
                first,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            write!(
                first,
                "data: {{\"jsonrpc\":\"2.0\",\"id\":\"s1\",\"method\":\"ping\"}}\n\n"
            )
            .unwrap();
            first.flush().unwrap();

            let (mut second, _) = listener.accept().unwrap();
            let request = read_test_request(&mut second);
            assert!(request.contains("\"id\":\"s1\""), "{request}");
            assert!(request.contains("\"result\":{}"), "{request}");
            write!(second, "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n").unwrap();
            second.flush().unwrap();

            write!(
                first,
                "data: {{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"ok\":true}}}}\n\n"
            )
            .unwrap();
            first.flush().unwrap();
        });
        let mut transport =
            HttpTransport::new(&format!("http://{addr}/mcp"), BTreeMap::new()).unwrap();
        let result = transport.call("tools/call", serde_json::json!({})).unwrap();
        assert_eq!(result["ok"], true);
        server.join().unwrap();
    }

    #[test]
    fn security_http_body_is_bounded_before_buffering() {
        let mut reader = std::io::Cursor::new(vec![b'x'; 1024]);
        assert!(read_response_body(&mut reader, 64).is_err());
        assert!(reader.position() <= 65, "read beyond the configured bound");
        assert_eq!(
            read_response_body(std::io::Cursor::new("éé"), 4).unwrap(),
            "éé"
        );
        assert!(read_response_body(std::io::Cursor::new("ééé"), 4).is_err());
        assert_eq!(read_response_body(std::io::Cursor::new(""), 0).unwrap(), "");
    }

    #[test]
    fn security_replies_require_correlated_valid_envelopes() {
        let wanted = json!(7);
        for invalid in [
            json!({"jsonrpc":"2.0","id":8,"result":{}}),
            json!({"jsonrpc":"2.0","result":{}}),
            json!({"jsonrpc":"2.0","id":null,"result":{}}),
            json!({"jsonrpc":"2.0","id":"7","result":{}}),
            json!({"jsonrpc":"1.0","id":7,"result":{}}),
            json!({"id":7,"result":{}}),
            json!({"jsonrpc":"2.0","id":7,"method":"tools/call","result":{}}),
            json!({"jsonrpc":"2.0","id":7,"result":{},"error":{}}),
            json!({"jsonrpc":"2.0","id":7}),
        ] {
            let text = invalid.to_string();
            assert!(
                parse_http_body("application/json", &text, Some(&wanted)).is_err(),
                "accepted invalid JSON reply: {text}"
            );
            let sse = format!("data: {text}\n\n");
            assert!(
                parse_http_body("text/event-stream", &sse, Some(&wanted)).is_err(),
                "accepted invalid SSE reply: {text}"
            );
        }
        let valid = json!({"jsonrpc":"2.0","id":7,"result":null});
        assert_eq!(
            parse_http_body("application/json", &valid.to_string(), Some(&wanted)).unwrap(),
            valid
        );
    }

    #[test]
    fn an_empty_body_is_null_and_json_parses() {
        assert_eq!(
            parse_http_body("application/json", "", None).unwrap(),
            Value::Null
        );
        assert_eq!(
            parse_http_body("text/event-stream", "\n", None).unwrap(),
            Value::Null
        );
        assert_eq!(
            parse_http_body("application/json", r#"{"a":1}"#, None).unwrap(),
            json!({"a": 1})
        );
    }

    #[test]
    fn sse_picks_the_event_answering_our_id() {
        let body = concat!(
            "event: message\n",
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n",
            "\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"sampling/createMessage\"}\n",
            "\n",
            "data: {\"jsonrpc\":\"2.0\",\n",
            "data: \"id\":7,\"result\":{\"ok\":true}}\n",
            "\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":8,\"result\":{\"ok\":false}}\n",
        );
        let picked = parse_http_body("text/event-stream", body, Some(&json!(7))).unwrap();
        assert_eq!(picked["result"]["ok"], json!(true));
        let last = parse_http_body("text/event-stream", body, None).unwrap();
        assert_eq!(last["id"], json!(8));
        let missing = parse_http_body("text/event-stream", body, Some(&json!(9)));
        assert!(matches!(missing, Err(Error::Transport(_))));
    }

    #[test]
    fn the_session_id_and_negotiated_version_ride_on_later_requests() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_string(), "Bearer t".to_string());
        let mut transport = HttpTransport::new("fixture:/nonexistent", headers).unwrap();
        let before = transport.request_headers();
        assert!(before.iter().all(|(key, _)| key != SESSION_HEADER));
        assert!(before.iter().all(|(key, _)| key != VERSION_HEADER));
        assert!(before
            .iter()
            .any(|(key, value)| key == "Authorization" && value == "Bearer t"));

        transport.absorb_initialize(
            Some("sess-1".into()),
            &json!({ "protocolVersion": "2025-06-18" }),
        );
        let after = transport.request_headers();
        assert!(after
            .iter()
            .any(|(key, value)| key == SESSION_HEADER && value == "sess-1"));
        assert!(after
            .iter()
            .any(|(key, value)| key == VERSION_HEADER && value == "2025-06-18"));

        // A server without sessions leaves the id alone; the version falls
        // back to ours when the reply omits it.
        transport.absorb_initialize(None, &json!({}));
        let again = transport.request_headers();
        assert!(again
            .iter()
            .any(|(key, value)| key == SESSION_HEADER && value == "sess-1"));
        assert!(again
            .iter()
            .any(|(key, value)| key == VERSION_HEADER && value == PROTOCOL_VERSION));
    }

    #[test]
    fn a_fixture_can_answer_with_an_rpc_error() {
        let fixture = json!({
            "tools/call": { "$rpcError": { "code": -32602, "message": "bad params" } },
            "ping": { "ok": true }
        });
        match HttpTransport::fixture_answer(&fixture, "tools/call") {
            Err(Error::Rpc { code, message }) => {
                assert_eq!(code, -32602);
                assert_eq!(message, "bad params");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            HttpTransport::fixture_answer(&fixture, "ping").unwrap(),
            json!({ "ok": true })
        );
    }
}
