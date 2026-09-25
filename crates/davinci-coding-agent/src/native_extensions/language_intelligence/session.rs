//! One initialized server and its synchronized documents.
use super::client_requests::ClientRequestState;
use super::documents::{self, Document};
use super::protocol::{IntelligenceError, RequestBudget, Result};
use super::servers::{ServerAdapter, ServerCommand};
use super::transport::Transport;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a push-diagnostics server must stay quiet before its latest
/// publication for a document version is taken as complete.
const DIAGNOSTIC_SETTLE: Duration = Duration::from_millis(400);

#[derive(Debug)]
pub(super) struct Session {
    pub command: ServerCommand,
    transport: Transport,
    capabilities: Value,
    documents: BTreeMap<PathBuf, Document>,
    diagnostic_floor: BTreeMap<String, u64>,
    diagnostic_result_ids: BTreeMap<String, String>,
    diagnostic_pull_items: BTreeMap<String, Vec<Value>>,
    diagnostic_refresh_generation: u64,
    needs_resync: bool,
}

impl Session {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn start(command: ServerCommand, deadline: Instant) -> Result<Self> {
        Self::start_with_budget(
            command,
            RequestBudget {
                deadline,
                cancelled: None,
            },
        )
    }

    pub fn start_with_budget(command: ServerCommand, budget: RequestBudget) -> Result<Self> {
        let client = ClientRequestState::new(
            command.workspace.clone(),
            command.client_configuration.clone(),
        )?;
        let transport = Transport::spawn_with_client(&mut command.command(), client)?;
        let uri = documents::file_uri(&command.workspace)?;
        let result = transport.request_with_budget("initialize", json!({
            "processId": std::process::id(), "clientInfo":{"name":"DaVinci"},
            "rootUri":uri, "workspaceFolders":[{"uri":uri,"name":"project"}],
            "capabilities": {
                "general":{"positionEncodings":["utf-16"]},
                "window":{"workDoneProgress":true},
                "workspace":{
                    "applyEdit":false,
                    "configuration":true,
                    "workspaceFolders":true,
                    "workspaceEdit":{"documentChanges":false},
                    "diagnostics":{"refreshSupport":true}
                },
                "textDocument": {
                    "synchronization":{"dynamicRegistration":false,"didSave":true},
                    "hover":{"contentFormat":["plaintext","markdown"]},
                    "definition":{"linkSupport":true}, "typeDefinition":{"linkSupport":true},
                    "implementation":{"linkSupport":true},
                    "documentSymbol":{"hierarchicalDocumentSymbolSupport":true},
                    "publishDiagnostics":{"versionSupport":true},
                    "diagnostic":{"dynamicRegistration":true,"relatedDocumentSupport":false}
                }
            }, "initializationOptions":command.initialization_options
        }), &budget).map_err(|e| IntelligenceError::new("initialization_failed", &format!("Language-server initialization failed ({})", e.code)))?;
        let capabilities = result
            .get("capabilities")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or_else(|| {
                IntelligenceError::new("initialization_failed", "Server returned no capabilities")
            })?;
        if capabilities
            .get("positionEncoding")
            .is_some_and(|v| v != "utf-16")
        {
            return Err(IntelligenceError::new(
                "initialization_failed",
                "Server selected an unsupported position encoding",
            ));
        }
        transport.notify_with_budget("initialized", json!({}), &budget)?;
        let diagnostic_refresh_generation = transport.client_status()
            ["diagnosticRefreshGeneration"]
            .as_u64()
            .unwrap_or(0);
        Ok(Self {
            command,
            transport,
            capabilities,
            documents: BTreeMap::new(),
            diagnostic_floor: BTreeMap::new(),
            diagnostic_result_ids: BTreeMap::new(),
            diagnostic_pull_items: BTreeMap::new(),
            diagnostic_refresh_generation,
            needs_resync: false,
        })
    }

    pub fn is_alive(&self) -> bool {
        self.transport.is_alive()
    }
    pub fn status(&self) -> Value {
        json!({
            "workspace":self.command.workspace,
            "backend":self.command.kind,
            "language":self.command.family,
            "serverVersion":self.command.version,
            "projectTypeScript":self.command.typescript_version,
            "profileFingerprint":self.command.profile_fingerprint,
            "analysisEnvironment":self.command.analysis_environment,
            "limitations":self.command.limitations,
            "session":if self.is_alive() {"running"} else {"stopped"},
            "pid":self.transport.pid(),
            "rssBytes":self.transport.rss_bytes(),
            "documents":self.documents.len(),
            "client":self.transport.client_status()
        })
    }

    fn supports(&self, capability: &str) -> bool {
        self.capabilities
            .get(capability)
            .is_some_and(|v| v == true || v.is_object())
    }

    fn synchronize(
        &mut self,
        source: Option<&Path>,
        adapter: &dyn ServerAdapter,
        budget: &RequestBudget,
    ) -> Result<()> {
        if self.needs_resync {
            return Err(IntelligenceError::new(
                "resync_required",
                "Document delivery became uncertain; restart this language-server session before using semantic evidence",
            ));
        }
        let sync = &self.capabilities["textDocumentSync"];
        let kind = sync
            .as_u64()
            .or_else(|| sync.get("change").and_then(Value::as_u64))
            .unwrap_or(0);
        if !matches!(kind, 1 | 2) || sync.get("openClose") == Some(&json!(false)) {
            return Err(IntelligenceError::new(
                "unsupported_document_sync",
                "Server cannot synchronize current source text",
            ));
        }
        let save = match sync.get("save") {
            Some(Value::Bool(true)) => Some(false),
            Some(Value::Object(v)) => Some(v.get("includeText") == Some(&json!(true))),
            _ => None,
        };
        let mut paths: Vec<PathBuf> = self.documents.keys().cloned().collect();
        if let Some(path) = source {
            if !self.documents.contains_key(path) {
                paths.push(path.into());
            }
        }
        let mut updates = Vec::new();
        let mut changed = false;
        for path in paths {
            budget.check()?;
            if !path.exists() && Some(path.as_path()) != source {
                if let Some(old) = self.documents.remove(&path) {
                    self.transport.notify_with_budget(
                        "textDocument/didClose",
                        json!({"textDocument":{"uri":old.uri}}),
                        budget,
                    )?;
                    self.transport.unwatch_document(&old.uri);
                    self.diagnostic_floor.remove(&old.uri);
                    self.diagnostic_result_ids.remove(&old.uri);
                    self.diagnostic_pull_items.remove(&old.uri);
                    changed = true;
                }
                continue;
            }
            // A symlink replacing a previously opened path must not read outside the session workspace.
            let canonical = path.canonicalize().map_err(|_| {
                IntelligenceError::new("invalid_source_path", "Open source file is unavailable")
            })?;
            if canonical != path {
                return Err(IntelligenceError::new(
                    "invalid_source_path",
                    "Open source identity changed; restart language intelligence",
                ));
            }
            let text = documents::read_source(&path)?;
            let (document, events) = if let Some(old) = self.documents.get(&path) {
                old.refreshed(text, kind, save)?
            } else {
                let language = adapter.language_id(&path).ok_or_else(|| {
                    IntelligenceError::new("unsupported_language", "No adapter for source file")
                })?;
                Document::open(path, language, text)?
            };
            self.transport.watch_document(&document.uri)?;
            changed |= !events.is_empty();
            updates.push((document, events));
        }
        // Changes to dependencies invalidate cached diagnostics for every open document.
        if changed {
            self.diagnostic_result_ids.clear();
            self.diagnostic_pull_items.clear();
            for (document, _) in &updates {
                self.diagnostic_floor.insert(
                    document.uri.clone(),
                    self.transport
                        .diagnostics(&document.uri)
                        .map_or(0, |d| d.sequence),
                );
            }
        }
        for (document, events) in updates {
            let mut sent = 0usize;
            for event in events {
                match self.transport.notify_with_budget(
                    event["method"].as_str().expect("internal event"),
                    event["params"].clone(),
                    budget,
                ) {
                    Ok(()) => sent += 1,
                    Err(error) => {
                        if sent > 0 {
                            self.needs_resync = true;
                            return Err(IntelligenceError::new(
                                "resync_required",
                                "A document notification batch was only partially delivered; semantic state must be recreated",
                            ));
                        }
                        return Err(error);
                    }
                }
            }
            self.documents.insert(document.path.clone(), document);
        }
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn execute(
        &mut self,
        method: &str,
        capability: &str,
        source: Option<&Path>,
        params: Value,
        adapter: &dyn ServerAdapter,
        deadline: Instant,
    ) -> Result<Value> {
        self.execute_with_budget(
            method,
            capability,
            source,
            params,
            adapter,
            &RequestBudget {
                deadline,
                cancelled: None,
            },
        )
    }

    pub fn execute_with_budget(
        &mut self,
        method: &str,
        capability: &str,
        source: Option<&Path>,
        mut params: Value,
        adapter: &dyn ServerAdapter,
        budget: &RequestBudget,
    ) -> Result<Value> {
        if method != "textDocument/diagnostic" && !self.supports(capability) {
            return Err(IntelligenceError::new(
                "unsupported_method",
                "Selected server does not advertise this semantic operation",
            ));
        }
        self.synchronize(source, adapter, budget)?;
        let refresh = self.transport.client_status()["diagnosticRefreshGeneration"]
            .as_u64()
            .unwrap_or(0);
        if refresh != self.diagnostic_refresh_generation {
            self.diagnostic_refresh_generation = refresh;
            self.diagnostic_result_ids.clear();
            self.diagnostic_pull_items.clear();
        }
        if let Some(path) = source {
            let document = &self.documents[path];
            params["textDocument"] = json!({"uri":document.uri});
            if let (Some(line), Some(column)) = (
                params.get("line").and_then(Value::as_u64),
                params.get("column").and_then(Value::as_u64),
            ) {
                params["position"] = documents::position(&document.text, line, column)?;
                params
                    .as_object_mut()
                    .expect("internal params")
                    .remove("line");
                params
                    .as_object_mut()
                    .expect("internal params")
                    .remove("column");
            }
        }
        if method == "textDocument/diagnostic" {
            if self.supports("diagnosticProvider") || self.transport.has_dynamic_diagnostics() {
                let path = source.ok_or_else(|| {
                    IntelligenceError::new(
                        "invalid_source_path",
                        "Diagnostics require a source file",
                    )
                })?;
                let document = &self.documents[path];
                if let Some(previous) = self.diagnostic_result_ids.get(&document.uri) {
                    params["previousResultId"] = json!(previous);
                }
                let response = self.transport.request_with_budget(method, params, budget)?;
                match response.get("kind").and_then(Value::as_str) {
                    Some("full") => {
                        let items = response
                            .get("items")
                            .and_then(Value::as_array)
                            .cloned()
                            .ok_or_else(|| {
                                IntelligenceError::new(
                                    "protocol_error",
                                    "Expected diagnostic items",
                                )
                            })?;
                        if let Some(result_id) = response.get("resultId").and_then(Value::as_str) {
                            self.diagnostic_result_ids
                                .insert(document.uri.clone(), result_id.into());
                        } else {
                            self.diagnostic_result_ids.remove(&document.uri);
                        }
                        self.diagnostic_pull_items
                            .insert(document.uri.clone(), items.clone());
                        return Ok(
                            json!({"items":items,"omitted":0,"freshness":"pull-response","documentVersion":document.version}),
                        );
                    }
                    Some("unchanged") => {
                        let result_id = response.get("resultId").and_then(Value::as_str);
                        if result_id.is_some()
                            && result_id
                                == self
                                    .diagnostic_result_ids
                                    .get(&document.uri)
                                    .map(String::as_str)
                        {
                            let items = self
                                .diagnostic_pull_items
                                .get(&document.uri)
                                .cloned()
                                .ok_or_else(|| IntelligenceError::new(
                                    "diagnostics_pending",
                                    "Unchanged diagnostic report has no retained current full report",
                                ))?;
                            return Ok(
                                json!({"items":items,"omitted":0,"freshness":"pull-response","unchanged":true,"documentVersion":document.version}),
                            );
                        }
                        return Err(IntelligenceError::new("diagnostics_pending", "Unchanged diagnostic report did not match the current provider result id"));
                    }
                    _ => {
                        return Err(IntelligenceError::new(
                            "protocol_error",
                            "Expected a full or unchanged document diagnostic report",
                        ))
                    }
                }
            }
            let path = source.ok_or_else(|| {
                IntelligenceError::new("invalid_source_path", "Diagnostics require a source file")
            })?;
            let document = &self.documents[path];
            let mut floor = *self.diagnostic_floor.get(&document.uri).unwrap_or(&0);
            loop {
                let mut snapshot =
                    self.transport
                        .wait_diagnostics(&document.uri, floor, budget.remaining()?)?;
                if snapshot
                    .version
                    .is_some_and(|version| version != document.version)
                {
                    floor = snapshot.sequence;
                    continue;
                }
                // A push server may publish one version more than once:
                // typescript-language-server sends syntax diagnostics first
                // and type errors later. Keep the newest publication until
                // the server has been quiet for the settle window, so a
                // file with type errors is not reported clean.
                loop {
                    let window = budget
                        .remaining()
                        .map(|left| left.min(DIAGNOSTIC_SETTLE))
                        .unwrap_or_default();
                    if window.is_zero() {
                        break;
                    }
                    match self
                        .transport
                        .wait_diagnostics(&document.uri, snapshot.sequence, window)
                    {
                        Ok(newer)
                            if newer
                                .version
                                .is_none_or(|version| version == document.version) =>
                        {
                            snapshot = newer
                        }
                        Ok(_) => break,
                        Err(error) if error.code == "diagnostics_pending" => break,
                        Err(error) => return Err(error),
                    }
                }
                return Ok(json!({"items":snapshot.items,"omitted":snapshot.omitted,
                    "freshness":if snapshot.version.is_some() {"versioned-publication"} else {"unversioned-publication"},
                    "documentVersion":document.version}));
            }
        }
        self.transport.request_with_budget(method, params, budget)
    }
}

#[allow(dead_code)]
pub(super) fn remaining(deadline: Instant) -> Result<std::time::Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|v| !v.is_zero())
        .ok_or_else(|| {
            IntelligenceError::new("request_timeout", "Language-intelligence deadline exceeded")
        })
}

#[cfg(test)]
mod tests {
    use super::super::servers::TypeScriptAdapter;
    use super::*;
    use std::time::Duration;

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(3)
    }
    fn fixture(root: &Path, mode: &str) -> ServerCommand {
        use super::super::identity::{LanguageFamily, ServerInvocation};
        use super::super::servers::ServerBackend;
        use std::collections::BTreeMap;
        let program = davinci_sys::process::resolve_program("node");
        let program = program.canonicalize().unwrap_or(program);
        let args = vec![
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/language-server.cjs"
            )
            .into(),
            mode.into(),
        ];
        let invocation = ServerInvocation::new(program.clone(), args.clone()).unwrap();
        ServerCommand {
            kind: ServerBackend::TypeScriptLanguageServer,
            backend: ServerBackend::TypeScriptLanguageServer,
            program,
            args,
            invocation,
            workspace: root.into(),
            version: Some("fixture".into()),
            typescript_version: None,
            initialization_options: json!({}),
            client_configuration: Value::Null,
            family: LanguageFamily::TypeScript,
            profile_fingerprint: "fixture".into(),
            analysis_environment: None,
            limitations: Vec::new(),
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn initialized_session_reuses_documents_and_syncs_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a.ts");
        let b = root.join("b.js");
        std::fs::write(&a, "first").unwrap();
        std::fs::write(&b, "second").unwrap();
        let mut session = Session::start(fixture(&root, "session"), deadline()).unwrap();
        for source in [&a, &b, &a] {
            session
                .execute(
                    "textDocument/hover",
                    "hoverProvider",
                    Some(source),
                    json!({"position":{"line":0,"character":0}}),
                    &TypeScriptAdapter,
                    deadline(),
                )
                .unwrap();
        }
        std::fs::write(&a, "updated").unwrap();
        let result = session
            .execute(
                "textDocument/hover",
                "hoverProvider",
                Some(&b),
                json!({}),
                &TypeScriptAdapter,
                deadline(),
            )
            .unwrap();
        assert_eq!(result["fixture"]["opens"], 2);
        assert_eq!(result["fixture"]["changes"], 1);
        assert_eq!(
            result["fixture"]["texts"][documents::file_uri(&a).unwrap()],
            "updated"
        );
        assert_eq!(session.documents[&a].version, 2);
        assert_eq!(session.documents.len(), 2);
        assert!(session.transport.is_alive());
    }

    #[test]
    fn missing_capability_and_wrong_encoding_are_structured() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut session = Session::start(fixture(&root, "no-capabilities"), deadline()).unwrap();
        assert_eq!(
            session
                .execute(
                    "workspace/symbol",
                    "workspaceSymbolProvider",
                    None,
                    json!({"query":"x"}),
                    &TypeScriptAdapter,
                    deadline()
                )
                .unwrap_err()
                .code,
            "unsupported_method"
        );
        assert_eq!(
            Session::start(fixture(&root, "utf8"), deadline())
                .unwrap_err()
                .code,
            "initialization_failed"
        );
    }

    #[test]
    fn pull_diagnostics_follow_disk_edits_and_deleted_documents_close() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a.ts");
        std::fs::write(&a, "bad").unwrap();
        let mut session = Session::start(fixture(&root, "session"), deadline()).unwrap();
        let first = session
            .execute(
                "textDocument/diagnostic",
                "diagnosticProvider",
                Some(&a),
                json!({}),
                &TypeScriptAdapter,
                deadline(),
            )
            .unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        std::fs::write(&a, "good").unwrap();
        let next = session
            .execute(
                "textDocument/diagnostic",
                "diagnosticProvider",
                Some(&a),
                json!({}),
                &TypeScriptAdapter,
                deadline(),
            )
            .unwrap();
        assert_eq!(next["items"], json!([]));
        std::fs::remove_file(&a).unwrap();
        session
            .execute(
                "workspace/symbol",
                "workspaceSymbolProvider",
                None,
                json!({"query":""}),
                &TypeScriptAdapter,
                deadline(),
            )
            .unwrap();
        assert!(session.documents.is_empty());
    }

    #[test]
    fn partial_notification_delivery_marks_session_for_resynchronization() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("a.ts");
        std::fs::write(&path, "first").unwrap();
        let mut session = Session::start(fixture(&root, "normal"), deadline()).unwrap();
        session
            .execute(
                "textDocument/hover",
                "hoverProvider",
                Some(&path),
                json!({}),
                &TypeScriptAdapter,
                deadline(),
            )
            .unwrap();
        // This assertion locks the fail-closed state transition itself; the
        // transport queue-failure fixture exercises the actual partial batch.
        session.needs_resync = true;
        assert_eq!(
            session
                .execute(
                    "textDocument/hover",
                    "hoverProvider",
                    Some(&path),
                    json!({}),
                    &TypeScriptAdapter,
                    deadline(),
                )
                .unwrap_err()
                .code,
            "resync_required"
        );
    }

    #[test]
    fn push_diagnostics_wait_for_the_semantic_publication() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("a.ts");
        std::fs::write(&path, "bad").unwrap();
        let mut session = Session::start(fixture(&root, "push-two-phase"), deadline()).unwrap();
        let result = session
            .execute(
                "textDocument/diagnostic",
                "diagnosticProvider",
                Some(&path),
                json!({}),
                &TypeScriptAdapter,
                deadline(),
            )
            .unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1, "{result}");
    }

    #[test]
    fn push_diagnostics_reject_old_versions_after_edits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("a.ts");
        let mut session =
            Session::start(fixture(&root, "push-stale-diagnostics"), deadline()).unwrap();
        for (text, count) in [("bad", 1), ("good", 0)] {
            std::fs::write(&path, text).unwrap();
            let result = session
                .execute(
                    "textDocument/diagnostic",
                    "diagnosticProvider",
                    Some(&path),
                    json!({}),
                    &TypeScriptAdapter,
                    deadline(),
                )
                .unwrap();
            assert_eq!(result["items"].as_array().unwrap().len(), count);
            assert_eq!(result["freshness"], "versioned-publication");
        }
    }
}
