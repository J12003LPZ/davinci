use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;

use crate::jsonrpc::{Notification, Request, Response};
use crate::{Error, Result, Rpc};

/// `PI_MCP_FIXTURE` is a JSON object `{ method: result }` so tests never
/// touch the network.
pub struct HttpTransport {
    url: String,
    headers: BTreeMap<String, String>,
    next_id: u64,
    fixture: Option<Value>,
}

impl HttpTransport {
    pub fn new(url: &str, headers: BTreeMap<String, String>) -> Result<Self> {
        let fixture = load_fixture(url);
        Ok(Self {
            url: url.to_string(),
            headers,
            next_id: 1,
            fixture,
        })
    }

    fn post(&mut self, body: Value) -> Result<Value> {
        if let Some(fixture) = &self.fixture {
            let method = body.get("method").and_then(Value::as_str).unwrap_or("");
            if method.starts_with("notifications/") {
                return Ok(Value::Null);
            }
            return fixture
                .get(method)
                .cloned()
                .ok_or_else(|| Error::Protocol(format!("fixture has no `{method}`")));
        }
        let mut request = ureq::post(&self.url)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream");
        for (key, value) in &self.headers {
            request = request.set(key, value);
        }
        let response = request
            .send_json(body)
            .map_err(|err| Error::Http(format!("{err}")))?;
        let content_type = response
            .header("content-type")
            .unwrap_or("application/json")
            .to_string();
        let mut text = String::new();
        response
            .into_reader()
            .read_to_string(&mut text)
            .map_err(|err| Error::Http(format!("{err}")))?;
        parse_http_body(&content_type, &text)
    }
}

impl Rpc for HttpTransport {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        if let Some(fixture) = &self.fixture {
            return fixture
                .get(method)
                .cloned()
                .ok_or_else(|| Error::Protocol(format!("fixture has no `{method}`")));
        }
        let request = Request::new(id, method, params);
        let body = serde_json::to_value(&request)
            .map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        let parsed = self.post(body)?;
        if let Ok(response) = serde_json::from_value::<Response>(parsed.clone()) {
            if let Some(error) = response.error {
                return Err(Error::Protocol(format!(
                    "mcp {}: {}",
                    error.code, error.message
                )));
            }
            return Ok(response.result.unwrap_or(Value::Null));
        }
        Ok(parsed)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        if self.fixture.is_some() {
            return Ok(());
        }
        let note = Notification::new(method, params);
        let body =
            serde_json::to_value(&note).map_err(|err| Error::Protocol(format!("encode: {err}")))?;
        let _ = self.post(body)?;
        Ok(())
    }
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

fn parse_http_body(content_type: &str, text: &str) -> Result<Value> {
    if content_type.contains("text/event-stream") {
        let mut last = None;
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                last = Some(data.trim().to_string());
            }
        }
        let data = last.ok_or_else(|| Error::Http("SSE body had no data frame".into()))?;
        serde_json::from_str(&data).map_err(|err| Error::Http(format!("sse json: {err}")))
    } else {
        serde_json::from_str(text).map_err(|err| Error::Http(format!("json: {err}")))
    }
}
