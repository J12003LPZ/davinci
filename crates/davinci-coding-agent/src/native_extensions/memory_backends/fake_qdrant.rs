//! An in-process Qdrant for tests: a `TcpListener` thread that speaks the few
//! REST calls the projection makes and logs every request. No Docker, no
//! network beyond loopback. Compiled only for tests and the `test-fixtures`
//! feature.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// One request the fake saw.
#[derive(Debug, Clone)]
pub struct LoggedRequest {
    pub method: String,
    pub path: String,
    pub body: Value,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredPoint {
    pub vector: Vec<f32>,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct Collection {
    pub size: usize,
    pub distance: String,
    pub points: BTreeMap<String, StoredPoint>,
}

#[derive(Default)]
struct State {
    collections: BTreeMap<String, Collection>,
    log: Vec<LoggedRequest>,
    not_ready: bool,
    required_key: Option<String>,
    /// Answer every non-readiness call with this status, if set.
    fail_status: Option<u16>,
    /// Delay before answering a points request.
    delay: Duration,
}

struct Shared {
    state: Mutex<State>,
    /// Close the connection without answering: the way a stopped service looks.
    down: AtomicBool,
    stop: AtomicBool,
    /// Misbehaviors for tests that prove a gate can fail.
    ignore_filters: AtomicBool,
    ignore_deletes: AtomicBool,
}

impl std::fmt::Debug for FakeQdrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FakeQdrant({})", self.address)
    }
}

pub struct FakeQdrant {
    address: SocketAddr,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl FakeQdrant {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake qdrant");
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            down: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            ignore_filters: AtomicBool::new(false),
            ignore_deletes: AtomicBool::new(false),
        });
        let worker = shared.clone();
        let thread = std::thread::spawn(move || {
            while !worker.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => handle(stream, &worker),
                    Err(_) => std::thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        Self {
            address,
            shared,
            thread: Some(thread),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub fn requests(&self) -> Vec<LoggedRequest> {
        self.shared.state.lock().unwrap().log.clone()
    }

    pub fn request_count(&self) -> usize {
        self.shared.state.lock().unwrap().log.len()
    }

    /// Requests whose method and path match, e.g. `("PUT", "/points")` matches
    /// any upsert regardless of collection.
    pub fn count_matching(&self, method: &str, path_contains: &str) -> usize {
        self.requests()
            .iter()
            .filter(|request| request.method == method && request.path.contains(path_contains))
            .count()
    }

    /// While down, a connection is accepted and dropped with no answer.
    pub fn set_down(&self, down: bool) {
        self.shared.down.store(down, Ordering::SeqCst);
    }

    /// A broken Qdrant that answers queries without applying the filter.
    pub fn ignore_filters(&self, on: bool) {
        self.shared.ignore_filters.store(on, Ordering::SeqCst);
    }

    /// A broken Qdrant that acknowledges deletes and keeps the points.
    pub fn ignore_deletes(&self, on: bool) {
        self.shared.ignore_deletes.store(on, Ordering::SeqCst);
    }

    pub fn set_not_ready(&self, not_ready: bool) {
        self.shared.state.lock().unwrap().not_ready = not_ready;
    }

    pub fn require_api_key(&self, key: &str) {
        self.shared.state.lock().unwrap().required_key = Some(key.to_string());
    }

    pub fn fail_with(&self, status: Option<u16>) {
        self.shared.state.lock().unwrap().fail_status = status;
    }

    pub fn set_delay(&self, delay: Duration) {
        self.shared.state.lock().unwrap().delay = delay;
    }

    /// A collection someone else made, so tests can prove it is left alone.
    pub fn preload_collection(&self, name: &str, size: usize, distance: &str) {
        self.shared.state.lock().unwrap().collections.insert(
            name.to_string(),
            Collection {
                size,
                distance: distance.to_string(),
                points: BTreeMap::new(),
            },
        );
    }

    pub fn collection(&self, name: &str) -> Option<Collection> {
        self.shared
            .state
            .lock()
            .unwrap()
            .collections
            .get(name)
            .cloned()
    }

    pub fn collection_names(&self) -> Vec<String> {
        self.shared
            .state
            .lock()
            .unwrap()
            .collections
            .keys()
            .cloned()
            .collect()
    }

    /// Drop every point of every collection: the remote lost its data.
    pub fn wipe_points(&self) {
        for collection in self.shared.state.lock().unwrap().collections.values_mut() {
            collection.points.clear();
        }
    }
}

impl Drop for FakeQdrant {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn read_request(
    stream: &mut TcpStream,
) -> Option<(String, String, Option<String>, Value)> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let mut length = 0usize;
    let mut api_key = None;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("api-key:") {
            api_key = Some(line["api-key:".len()..].trim().to_string());
        }
    }
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
    Some((method, path, api_key, body))
}

pub(super) fn respond(stream: &mut TcpStream, status: u16, body: &Value) {
    let text = body.to_string();
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denominator = norm(a) * norm(b);
    if denominator == 0.0 {
        0.0
    } else {
        dot / denominator
    }
}

/// `must` conditions: `{"key": k, "match": {"value": v}}` and `{"has_id": [..]}`.
fn filter_matches(filter: &Value, id: &str, payload: &Value) -> bool {
    filter["must"].as_array().is_none_or(|conditions| {
        conditions.iter().all(|condition| {
            if let Some(ids) = condition["has_id"].as_array() {
                return ids.iter().any(|candidate| candidate.as_str() == Some(id));
            }
            let key = condition["key"].as_str().unwrap_or_default();
            payload.get(key) == condition["match"].get("value")
        })
    })
}

fn handle(mut stream: TcpStream, shared: &Shared) {
    let Some((method, path, api_key, body)) = read_request(&mut stream) else {
        return;
    };
    let mut state = shared.state.lock().unwrap();
    state.log.push(LoggedRequest {
        method: method.clone(),
        path: path.clone(),
        body: body.clone(),
        api_key: api_key.clone(),
    });
    if shared.down.load(Ordering::SeqCst) {
        return;
    }
    let delay = state.delay;
    if path == "/readyz" {
        let status = if state.not_ready { 503 } else { 200 };
        return respond(&mut stream, status, &json!({"title": "qdrant"}));
    }
    if let Some(required) = &state.required_key {
        if api_key.as_ref() != Some(required) {
            return respond(
                &mut stream,
                401,
                &json!({"status": {"error": "unauthorized"}}),
            );
        }
    }
    if let Some(status) = state.fail_status {
        return respond(
            &mut stream,
            status,
            &json!({"status": {"error": "injected"}}),
        );
    }
    if !delay.is_zero() {
        drop(state);
        std::thread::sleep(delay);
        state = shared.state.lock().unwrap();
    }
    let path_only = path.split('?').next().unwrap_or(&path).to_string();
    let parts: Vec<&str> = path_only.trim_matches('/').split('/').collect();
    match (method.as_str(), parts.as_slice()) {
        ("GET", ["collections", name]) => match state.collections.get(*name) {
            Some(collection) => respond(
                &mut stream,
                200,
                &json!({"result": {
                    "status": "green",
                    "points_count": collection.points.len(),
                    "config": {"params": {"vectors": {
                        "size": collection.size,
                        "distance": collection.distance,
                    }}}
                }}),
            ),
            None => respond(&mut stream, 404, &json!({"status": {"error": "Not found"}})),
        },
        ("PUT", ["collections", name]) => {
            let size = body["vectors"]["size"].as_u64().unwrap_or(0) as usize;
            let distance = body["vectors"]["distance"]
                .as_str()
                .unwrap_or("Cosine")
                .to_string();
            if state.collections.contains_key(*name) {
                return respond(&mut stream, 409, &json!({"status": {"error": "exists"}}));
            }
            state.collections.insert(
                (*name).to_string(),
                Collection {
                    size,
                    distance,
                    points: BTreeMap::new(),
                },
            );
            respond(&mut stream, 200, &json!({"result": true}))
        }
        ("PUT", ["collections", name, "points"]) => {
            let Some(collection) = state.collections.get_mut(*name) else {
                return respond(&mut stream, 404, &json!({"status": {"error": "Not found"}}));
            };
            for point in body["points"].as_array().cloned().unwrap_or_default() {
                let id = point["id"].as_str().unwrap_or_default().to_string();
                let vector: Vec<f32> = point["vector"]
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .map(|value| value.as_f64().unwrap_or(0.0) as f32)
                            .collect()
                    })
                    .unwrap_or_default();
                if vector.len() != collection.size {
                    return respond(
                        &mut stream,
                        400,
                        &json!({"status": {"error": "Wrong input: vector dimension"}}),
                    );
                }
                collection.points.insert(
                    id,
                    StoredPoint {
                        vector,
                        payload: point["payload"].clone(),
                    },
                );
            }
            respond(
                &mut stream,
                200,
                &json!({"result": {"status": "completed"}}),
            )
        }
        ("POST", ["collections", name, "points", "query"]) => {
            let Some(collection) = state.collections.get(*name) else {
                return respond(&mut stream, 404, &json!({"status": {"error": "Not found"}}));
            };
            let query: Vec<f32> = body["query"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .map(|value| value.as_f64().unwrap_or(0.0) as f32)
                        .collect()
                })
                .unwrap_or_default();
            let limit = body["limit"].as_u64().unwrap_or(10) as usize;
            let mut scored: Vec<(f32, &String, &StoredPoint)> = collection
                .points
                .iter()
                .filter(|(id, point)| {
                    shared.ignore_filters.load(Ordering::SeqCst)
                        || filter_matches(&body["filter"], id, &point.payload)
                })
                .map(|(id, point)| (cosine(&query, &point.vector), id, point))
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
            let points: Vec<Value> = scored
                .into_iter()
                .take(limit)
                .map(|(score, id, point)| {
                    json!({"id": id, "score": score, "payload": point.payload})
                })
                .collect();
            respond(&mut stream, 200, &json!({"result": {"points": points}}))
        }
        ("POST", ["collections", name, "points", "delete"]) => {
            let Some(collection) = state.collections.get_mut(*name) else {
                return respond(&mut stream, 404, &json!({"status": {"error": "Not found"}}));
            };
            let filter = body["filter"].clone();
            if filter.is_null() {
                return respond(
                    &mut stream,
                    400,
                    &json!({"status": {"error": "empty delete"}}),
                );
            }
            if !shared.ignore_deletes.load(Ordering::SeqCst) {
                collection
                    .points
                    .retain(|id, point| !filter_matches(&filter, id, &point.payload));
            }
            respond(
                &mut stream,
                200,
                &json!({"result": {"status": "completed"}}),
            )
        }
        _ => respond(
            &mut stream,
            404,
            &json!({"status": {"error": "unknown route"}}),
        ),
    }
}
