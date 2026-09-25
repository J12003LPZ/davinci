//! Bounded server-to-client LSP callbacks.
use super::protocol::{IntelligenceError, RequestBudget, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub(super) struct ClientRequestState {
    workspace: PathBuf,
    workspace_folders: Arc<Vec<Value>>,
    configuration: Arc<Value>,
    registrations: Arc<Mutex<BTreeMap<String, Registration>>>,
    progress: Arc<Mutex<BTreeMap<String, Value>>>,
    refresh_generation: Arc<AtomicU64>,
    messages: Arc<Mutex<Vec<String>>>,
    /// Last `experimental/serverStatus` (rust-analyzer). `None` until reported.
    server_status: Arc<(Mutex<Option<Value>>, Condvar)>,
    started: Instant,
}

#[derive(Debug, Clone)]
struct Registration {
    method: String,
    // Retained for diagnostics of dynamic registrations; routing uses `method`.
    #[allow(dead_code)]
    selectors: BTreeSet<String>,
}

impl ClientRequestState {
    pub fn new(workspace: PathBuf, configuration: Value) -> Result<Self> {
        let uri = super::documents::file_uri(&workspace)?;
        Ok(Self {
            workspace,
            workspace_folders: Arc::new(vec![json!({"uri":uri,"name":"project"})]),
            configuration: Arc::new(configuration),
            registrations: Arc::new(Mutex::new(BTreeMap::new())),
            progress: Arc::new(Mutex::new(BTreeMap::new())),
            refresh_generation: Arc::new(AtomicU64::new(0)),
            messages: Arc::new(Mutex::new(Vec::new())),
            server_status: Arc::new((Mutex::new(None), Condvar::new())),
            started: Instant::now(),
        })
    }

    pub fn handle_request(
        &self,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Value, (i64, &'static str)> {
        match method {
            "workspace/configuration" => self
                .configuration_response(params)
                .map_err(|_| (-32602, "Invalid configuration request")),
            "workspace/workspaceFolders" => Ok(Value::Array((*self.workspace_folders).clone())),
            "client/registerCapability" => self.register(params),
            "client/unregisterCapability" => self.unregister(params),
            "workspace/diagnostic/refresh" => {
                self.refresh_generation.fetch_add(1, Ordering::AcqRel);
                Ok(Value::Null)
            }
            "window/workDoneProgress/create" => {
                let token = params
                    .get("token")
                    .cloned()
                    .ok_or((-32602, "Missing progress token"))?;
                let key = compact_token(&token).ok_or((-32602, "Invalid progress token"))?;
                let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
                if progress.len() >= 64 && !progress.contains_key(&key) {
                    return Err((-32602, "Progress capacity exceeded"));
                }
                progress.insert(key, Value::Null);
                Ok(Value::Null)
            }
            "window/showMessageRequest" => Ok(Value::Null),
            "workspace/applyEdit" => Ok(
                json!({"applied":false,"failureReason":"DaVinci Language Intelligence is read-only"}),
            ),
            _ => Err((-32601, "Unsupported client method")),
        }
    }

    pub fn observe_notification(&self, method: &str, params: &Value) {
        match method {
            "$/progress" => {
                if let Some(key) = params.get("token").and_then(compact_token) {
                    let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
                    if progress.len() < 64 || progress.contains_key(&key) {
                        let mut value = params.get("value").cloned().unwrap_or(Value::Null);
                        truncate_json_strings(&mut value, 1024);
                        progress.insert(key, value);
                    }
                }
            }
            "experimental/serverStatus" => {
                let status = json!({
                    "health": params.get("health").and_then(Value::as_str),
                    "quiescent": params.get("quiescent").and_then(Value::as_bool),
                    "message": params
                        .get("message")
                        .and_then(Value::as_str)
                        .map(|message| super::normalize::compact(message, 1024)),
                });
                let (lock, changed) = &*self.server_status;
                *lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(status);
                changed.notify_all();
            }
            "window/logMessage" | "window/showMessage" => {
                if let Some(message) = params.get("message").and_then(Value::as_str) {
                    let mut messages = self.messages.lock().unwrap_or_else(|e| e.into_inner());
                    if messages.len() >= 32 {
                        messages.remove(0);
                    }
                    messages.push(super::normalize::compact(message, 1024));
                }
            }
            _ => {}
        }
    }

    /// Wait, within `budget`, until the server reports it has finished loading
    /// and indexing. Returns `None` when the server never reports readiness
    /// (servers without `experimental/serverStatus`), otherwise the last
    /// observed `quiescent` value.
    pub fn wait_quiescent(&self, budget: &RequestBudget) -> Option<bool> {
        // A server that supports the extension reports its first status right
        // after `initialized`; one that has not by then is treated as silent.
        let first_status_grace = self.started + Duration::from_secs(1);
        let (lock, changed) = &*self.server_status;
        let mut status = lock.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let quiescent = status
                .as_ref()
                .map(|status| status["quiescent"].as_bool() == Some(true));
            if quiescent == Some(true)
                || (quiescent.is_none() && Instant::now() >= first_status_grace)
            {
                return quiescent;
            }
            let Ok(remaining) = budget.remaining() else {
                return quiescent;
            };
            status = changed
                .wait_timeout(status, remaining.min(Duration::from_millis(50)))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    pub fn has_document_diagnostics(&self) -> bool {
        self.registrations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|registration| registration.method == "textDocument/diagnostic")
    }

    pub fn refresh_generation(&self) -> u64 {
        self.refresh_generation.load(Ordering::Acquire)
    }

    pub fn status(&self) -> Value {
        let registrations = self.registrations.lock().unwrap_or_else(|e| e.into_inner());
        let progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        let messages = self.messages.lock().unwrap_or_else(|e| e.into_inner());
        json!({
            "dynamicRegistrations": registrations.len(),
            "diagnosticRefreshGeneration": self.refresh_generation(),
            "progressEntries": progress.len(),
            "messages": messages.clone(),
            "serverStatus": self.server_status.0.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        })
    }

    fn configuration_response(&self, params: &Value) -> Result<Value> {
        let items = params
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                IntelligenceError::new(
                    "protocol_error",
                    "workspace/configuration items are missing",
                )
            })?;
        if items.len() > 64 {
            return Err(IntelligenceError::new(
                "protocol_error",
                "workspace/configuration exceeds the bounded item count",
            ));
        }
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            if let Some(scope) = item.get("scopeUri").and_then(Value::as_str) {
                let allowed = url::Url::parse(scope)
                    .ok()
                    .and_then(|uri| uri.to_file_path().ok())
                    .and_then(|path| path.canonicalize().ok().or(Some(path)))
                    .is_some_and(|path| path.starts_with(&self.workspace));
                if !allowed {
                    out.push(Value::Null);
                    continue;
                }
            }
            let Some(section) = item.get("section").and_then(Value::as_str) else {
                out.push(Value::Null);
                continue;
            };
            out.push(section_value(&self.configuration, section).unwrap_or(Value::Null));
        }
        Ok(Value::Array(out))
    }

    fn register(&self, params: &Value) -> std::result::Result<Value, (i64, &'static str)> {
        let registrations = params
            .get("registrations")
            .and_then(Value::as_array)
            .ok_or((-32602, "Invalid registrations"))?;
        if registrations.len() > 32 {
            return Err((-32602, "Registration capacity exceeded"));
        }
        let mut state = self.registrations.lock().unwrap_or_else(|e| e.into_inner());
        for registration in registrations {
            let id = registration
                .get("id")
                .and_then(Value::as_str)
                .ok_or((-32602, "Registration id missing"))?;
            let method = registration
                .get("method")
                .and_then(Value::as_str)
                .ok_or((-32602, "Registration method missing"))?;
            if method != "textDocument/diagnostic" {
                return Err((-32602, "Unsupported dynamic registration"));
            }
            if state.contains_key(id) {
                return Err((-32602, "Duplicate registration id"));
            }
            let selectors = registration
                .pointer("/registerOptions/documentSelector")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            item.get("language")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default();
            state.insert(
                id.to_string(),
                Registration {
                    method: method.into(),
                    selectors,
                },
            );
        }
        Ok(Value::Null)
    }

    fn unregister(&self, params: &Value) -> std::result::Result<Value, (i64, &'static str)> {
        let items = params
            .get("unregisterations")
            .or_else(|| params.get("unregistrations"))
            .and_then(Value::as_array)
            .ok_or((-32602, "Invalid unregistrations"))?;
        let mut state = self.registrations.lock().unwrap_or_else(|e| e.into_inner());
        for item in items {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .ok_or((-32602, "Unregistration id missing"))?;
            state.remove(id);
        }
        Ok(Value::Null)
    }
}

fn section_value(root: &Value, section: &str) -> Option<Value> {
    let mut value = root;
    for component in section.split('.').filter(|v| !v.is_empty()) {
        value = value.get(component)?;
    }
    Some(value.clone())
}

fn compact_token(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if value.len() <= 256 => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn truncate_json_strings(value: &mut Value, max: usize) {
    match value {
        Value::String(text) => *text = super::normalize::compact(text, max),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| truncate_json_strings(item, max)),
        Value::Object(map) => map
            .values_mut()
            .for_each(|item| truncate_json_strings(item, max)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (tempfile::TempDir, ClientRequestState) {
        let dir = tempfile::tempdir().unwrap();
        let state = ClientRequestState::new(dir.path().to_path_buf(), Value::Null).unwrap();
        (dir, state)
    }

    fn budget(millis: u64) -> RequestBudget {
        RequestBudget::from_timeout(Duration::from_millis(millis))
    }

    #[test]
    fn readiness_waits_for_quiescent_server_status() {
        let (_dir, state) = state();
        state.observe_notification(
            "experimental/serverStatus",
            &json!({"health":"ok","quiescent":false}),
        );
        let observer = state.clone();
        let reporter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            observer.observe_notification(
                "experimental/serverStatus",
                &json!({"health":"ok","quiescent":true}),
            );
        });
        let started = Instant::now();
        assert_eq!(state.wait_quiescent(&budget(5_000)), Some(true));
        assert!(started.elapsed() >= Duration::from_millis(90));
        reporter.join().unwrap();
        assert_eq!(state.status()["serverStatus"]["quiescent"], true);
    }

    #[test]
    fn readiness_reports_still_indexing_when_the_budget_ends() {
        let (_dir, state) = state();
        state.observe_notification("experimental/serverStatus", &json!({"quiescent":false}));
        assert_eq!(state.wait_quiescent(&budget(100)), Some(false));
    }

    #[test]
    fn a_server_without_status_notifications_is_not_waited_on_after_grace() {
        let (_dir, state) = state();
        std::thread::sleep(Duration::from_millis(1_050));
        let started = Instant::now();
        assert_eq!(state.wait_quiescent(&budget(5_000)), None);
        assert!(started.elapsed() < Duration::from_millis(200));
    }
}
