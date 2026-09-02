//! Streamable HTTP: one JSON-RPC message per `POST`, answered as JSON or as
//! an SSE stream. The `Mcp-Session-Id` the server hands back on
//! `initialize` and the negotiated `MCP-Protocol-Version` ride on every
//! later request; the session is `DELETE`d best-effort on drop.

use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;
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
    content_type: String,
    session_id: Option<String>,
    text: String,
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

    fn post(&mut self, body: &Value) -> Result<Reply> {
        let mut request = self.agent.post(&self.url);
        for (key, value) in self.request_headers() {
            request = request.set(&key, &value);
        }
        let response = match request.send_json(body) {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) => {
                let mut text = String::new();
                let _ = response.into_reader().take(4096).read_to_string(&mut text);
                let text = text.trim();
                return Err(Error::Transport(if text.is_empty() {
                    format!("mcp http {code}")
                } else {
                    format!("mcp http {code}: {text}")
                }));
            }
            Err(err) => return Err(Error::Transport(format!("mcp http: {err}"))),
        };
        let content_type = response
            .header("content-type")
            .unwrap_or("application/json")
            .to_string();
        let session_id = response
            .header(SESSION_HEADER)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut text = String::new();
        response
            .into_reader()
            .read_to_string(&mut text)
            .map_err(|err| Error::Transport(format!("mcp http body: {err}")))?;
        Ok(Reply {
            content_type,
            session_id,
            text,
        })
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
        let reply = self.post(&body)?;
        let parsed = parse_http_body(&reply.content_type, &reply.text, Some(&Value::from(id)))?;
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
        let _ = self.post(&body)?;
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

fn build_agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(timeout).build()
}

/// `fixture:<path>` loads a method→result map from disk so tests never hit
/// the network and never share `PI_MCP_FIXTURE` across threads.
fn load_fixture(url: &str) -> Option<Value> {
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
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    if !content_type.contains("text/event-stream") {
        return serde_json::from_str(text).map_err(|err| Error::Transport(format!("json: {err}")));
    }
    let mut last = None;
    for data in sse_events(text) {
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        match want {
            Some(id) => {
                if value.get("method").is_none() && value.get("id") == Some(id) {
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
fn sse_events(text: &str) -> Vec<String> {
    let mut events = Vec::new();
    let mut data: Vec<&str> = Vec::new();
    let mut flush = |data: &mut Vec<&str>| {
        if !data.is_empty() {
            events.push(data.join("\n"));
            data.clear();
        }
    };
    for line in text.lines() {
        if line.is_empty() {
            flush(&mut data);
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    flush(&mut data);
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
