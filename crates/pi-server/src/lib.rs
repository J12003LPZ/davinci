//! Experimental server for remote pi sessions.

use pi_protocol::{encode_server_message, parse_client_message, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("{0}")]
    Message(String),
}

pub struct SessionLease {
    pub session_id: String,
    pub owner: String,
}

pub struct Server {
    pub server_id: String,
    pub sessions: Vec<Value>,
    pub models: Vec<Value>,
    pub leases: HashMap<String, String>,
    pub pending: HashMap<String, Value>,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            server_id: Uuid::new_v4().to_string(),
            sessions: Vec::new(),
            models: Vec::new(),
            leases: HashMap::new(),
            pending: HashMap::new(),
        }
    }
}

impl Server {
    pub fn snapshot(&self) -> Value {
        json!({
            "serverId": self.server_id,
            "protocolVersion": PROTOCOL_VERSION,
            "revision": 0,
            "sessions": self.sessions,
            "models": self.models
        })
    }

    pub fn hello(&self, connection_id: &str) -> Value {
        json!({
            "type": "hello",
            "version": PROTOCOL_VERSION,
            "connectionId": connection_id,
            "snapshot": self.snapshot()
        })
    }

    pub fn handle_client_bytes(&mut self, bytes: &[u8]) -> Result<Vec<u8>, ServerError> {
        let mut decoder =
            pi_protocol::FrameDecoder::new(None).map_err(|e| ServerError::Message(e.0))?;
        let frames = decoder.push(bytes).map_err(|e| ServerError::Message(e.0))?;
        decoder.end().ok();
        let mut out = Vec::new();
        for frame in frames {
            let value = pi_protocol::decode_cbor(&frame, None)
                .map_err(|e| ServerError::Message(e.0))?
                .to_json();
            let message = parse_client_message(&value).map_err(|e| ServerError::Message(e.0))?;
            let reply = self.dispatch(&message)?;
            out.extend(encode_server_message(&reply, None).map_err(|e| ServerError::Message(e.0))?);
        }
        Ok(out)
    }

    pub fn dispatch(&mut self, message: &Value) -> Result<Value, ServerError> {
        match message.get("type").and_then(|v| v.as_str()) {
            Some("hello") => Ok(self.hello("connection-1")),
            Some("request") => {
                let id = message
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("req")
                    .to_string();
                self.pending.insert(id.clone(), message.clone());
                let command = message.get("request").cloned().unwrap_or(json!({}));
                let result = match command.get("command").and_then(|v| v.as_str()) {
                    Some("list") => json!({"command":"list","sessions": self.sessions}),
                    Some("create") => {
                        let session = self.create_session(&command);
                        json!({"command":"create","session": session})
                    }
                    Some("attach") => {
                        let session_id = command
                            .get("sessionId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        match self.session_snapshot(session_id) {
                            Some(mut session) => {
                                session["attached"] = json!(true);
                                self.acquire_lease(session_id, "local");
                                json!({"command":"attach","session": session})
                            }
                            None => {
                                return Ok(json!({
                                    "type": "response",
                                    "id": id,
                                    "ok": false,
                                    "error": { "code": "not_found", "message": format!("session {session_id} not found") }
                                }));
                            }
                        }
                    }
                    Some("detach") => {
                        let session_id = command
                            .get("sessionId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        self.leases.remove(&session_id);
                        json!({"command":"detach","sessionId": session_id})
                    }
                    Some("prompt") | Some("steer") => {
                        let name = command
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or("prompt");
                        let session_id = command
                            .get("sessionId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        match self.apply_prompt(
                            session_id,
                            command.get("text").and_then(|v| v.as_str()).unwrap_or(""),
                            name == "steer",
                        ) {
                            Some(session) => json!({"command": name, "session": session}),
                            None => {
                                return Ok(json!({
                                    "type": "response",
                                    "id": id,
                                    "ok": false,
                                    "error": { "code": "not_found", "message": "session not found" }
                                }));
                            }
                        }
                    }
                    Some("abort") => {
                        let session_id = command
                            .get("sessionId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        match self.set_phase(session_id, "idle") {
                            Some(session) => json!({"command":"abort","session": session}),
                            None => {
                                return Ok(json!({
                                    "type": "response",
                                    "id": id,
                                    "ok": false,
                                    "error": { "code": "not_found", "message": "session not found" }
                                }));
                            }
                        }
                    }
                    Some("set_model") => {
                        let session_id = command
                            .get("sessionId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        match self.set_field(
                            session_id,
                            "model",
                            command.get("model").cloned().unwrap_or(json!({})),
                        ) {
                            Some(session) => json!({"command":"set_model","session": session}),
                            None => {
                                return Ok(json!({
                                    "type": "response",
                                    "id": id,
                                    "ok": false,
                                    "error": { "code": "not_found", "message": "session not found" }
                                }));
                            }
                        }
                    }
                    Some("set_thinking") => {
                        let session_id = command
                            .get("sessionId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        match self.set_field(
                            session_id,
                            "thinkingLevel",
                            command
                                .get("thinkingLevel")
                                .cloned()
                                .unwrap_or(json!("off")),
                        ) {
                            Some(session) => json!({"command":"set_thinking","session": session}),
                            None => {
                                return Ok(json!({
                                    "type": "response",
                                    "id": id,
                                    "ok": false,
                                    "error": { "code": "not_found", "message": "session not found" }
                                }));
                            }
                        }
                    }
                    Some(other) => {
                        return Ok(json!({
                            "type": "response",
                            "id": id,
                            "ok": false,
                            "error": { "code": "unknown_command", "message": format!("unknown command {other}") }
                        }));
                    }
                    None => {
                        return Ok(json!({
                            "type": "response",
                            "id": id,
                            "ok": false,
                            "error": { "code": "invalid_request", "message": "missing command" }
                        }));
                    }
                };
                Ok(json!({"type":"response","id": id, "ok": true, "result": result}))
            }
            _ => Ok(json!({
                "type": "hello_error",
                "error": { "code": "invalid_request", "message": "expected hello or request" }
            })),
        }
    }

    fn create_session(&mut self, command: &Value) -> Value {
        let session = json!({
            "id": Uuid::new_v4().to_string(),
            "cwd": command.get("cwd").and_then(|v| v.as_str()).unwrap_or("."),
            "createdAt": 1,
            "updatedAt": 1,
            "phase": "idle",
            "model": command.get("model").cloned().unwrap_or(json!({"provider":"anthropic","id":"claude-sonnet-4-5"})),
            "thinkingLevel": command.get("thinkingLevel").cloned().unwrap_or(json!("off")),
            "attached": true,
            "locked": false,
            "revision": 0,
            "transcript": [],
            "queuedSteer": [],
            "queuedSteerCount": 0
        });
        self.sessions.push(session.clone());
        session
    }

    fn session_index(&self, session_id: &str) -> Option<usize> {
        self.sessions
            .iter()
            .position(|s| s.get("id").and_then(|v| v.as_str()) == Some(session_id))
    }

    fn session_snapshot(&self, session_id: &str) -> Option<Value> {
        self.session_index(session_id)
            .and_then(|i| self.sessions.get(i).cloned())
    }

    fn apply_prompt(&mut self, session_id: &str, text: &str, steer: bool) -> Option<Value> {
        let idx = self.session_index(session_id)?;
        if let Some(obj) = self.sessions[idx].as_object_mut() {
            if steer {
                let mut queued = obj
                    .get("queuedSteer")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                queued.push(json!({"text": text}));
                let count = queued.len();
                obj.insert("queuedSteer".into(), json!(queued));
                obj.insert("queuedSteerCount".into(), json!(count));
            } else {
                let mut transcript = obj
                    .get("transcript")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                transcript.push(json!({"role":"user","content": text}));
                obj.insert("transcript".into(), json!(transcript));
                obj.insert("phase".into(), json!("running"));
            }
            obj.insert("updatedAt".into(), json!(1));
        }
        self.sessions.get(idx).cloned()
    }

    fn set_phase(&mut self, session_id: &str, phase: &str) -> Option<Value> {
        let idx = self.session_index(session_id)?;
        if let Some(obj) = self.sessions[idx].as_object_mut() {
            obj.insert("phase".into(), json!(phase));
        }
        self.sessions.get(idx).cloned()
    }

    fn set_field(&mut self, session_id: &str, field: &str, value: Value) -> Option<Value> {
        let idx = self.session_index(session_id)?;
        if let Some(obj) = self.sessions[idx].as_object_mut() {
            obj.insert(field.to_string(), value);
        }
        self.sessions.get(idx).cloned()
    }

    pub fn acquire_lease(&mut self, session_id: &str, owner: &str) -> bool {
        match self.leases.get(session_id) {
            Some(current) if current != owner => false,
            _ => {
                self.leases
                    .insert(session_id.to_string(), owner.to_string());
                true
            }
        }
    }
}

pub fn listen_unix(path: &Path) -> Result<UnixListener, ServerError> {
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path).map_err(|e| ServerError::Message(e.to_string()))
}

pub fn listen_tcp(addr: &str) -> Result<TcpListener, ServerError> {
    TcpListener::bind(addr).map_err(|e| ServerError::Message(e.to_string()))
}

pub fn serve_connection(
    server: &mut Server,
    stream: &mut (impl Read + Write),
) -> Result<(), ServerError> {
    let mut buf = [0u8; 8192];
    let n = stream
        .read(&mut buf)
        .map_err(|e| ServerError::Message(e.to_string()))?;
    let reply = server.handle_client_bytes(&buf[..n])?;
    stream
        .write_all(&reply)
        .map_err(|e| ServerError::Message(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_protocol::encode_client_message;

    #[test]
    fn hello_and_list_correlation() {
        let mut server = Server::default();
        let hello = encode_client_message(&json!({"type":"hello","version":1}), None).unwrap();
        let reply = server.handle_client_bytes(&hello).unwrap();
        assert!(!reply.is_empty());
        let req = encode_client_message(
            &json!({"type":"request","id":"r1","request":{"command":"list"}}),
            None,
        )
        .unwrap();
        let reply = server.handle_client_bytes(&req).unwrap();
        assert!(!reply.is_empty());
        assert!(server.pending.contains_key("r1"));
    }

    #[test]
    fn protocol_commands_beyond_list_create() {
        let mut server = Server::default();
        let created = server
            .dispatch(
                &json!({"type":"request","id":"c","request":{"command":"create","cwd":"/tmp"}}),
            )
            .unwrap();
        let id = created["result"]["session"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let attach = server
            .dispatch(
                &json!({"type":"request","id":"a","request":{"command":"attach","sessionId": id}}),
            )
            .unwrap();
        assert_eq!(attach["ok"], true);
        let prompt = server
            .dispatch(&json!({"type":"request","id":"p","request":{"command":"prompt","sessionId": id, "text":"hi"}}))
            .unwrap();
        assert_eq!(prompt["result"]["command"], "prompt");
        let model = server
            .dispatch(&json!({"type":"request","id":"m","request":{"command":"set_model","sessionId": id, "model":{"provider":"google","id":"g"}}}))
            .unwrap();
        assert_eq!(model["ok"], true);
        let think = server
            .dispatch(&json!({"type":"request","id":"t","request":{"command":"set_thinking","sessionId": id, "thinkingLevel":"low"}}))
            .unwrap();
        assert_eq!(think["ok"], true);
        let abort = server
            .dispatch(
                &json!({"type":"request","id":"x","request":{"command":"abort","sessionId": id}}),
            )
            .unwrap();
        assert_eq!(abort["ok"], true);
        let detach = server
            .dispatch(
                &json!({"type":"request","id":"d","request":{"command":"detach","sessionId": id}}),
            )
            .unwrap();
        assert_eq!(detach["result"]["command"], "detach");
    }
}
