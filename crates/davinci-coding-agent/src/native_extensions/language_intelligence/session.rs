//! One initialized server and its synchronized documents.
use super::client_requests::ClientRequestConfig;
use super::documents::{self, Document};
use super::protocol::{IntelligenceError, Result};
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
}

impl Session {
    pub fn start(command: ServerCommand, deadline: Instant) -> Result<Self> {
        let uri = documents::file_uri(&command.workspace)?;
        let client = ClientRequestConfig::for_command(&command, &uri);
        let transport = Transport::spawn_with_client(&mut command.command(), client)?;
        let result = transport.request("initialize", json!({
            "processId": std::process::id(), "clientInfo":{"name":"DaVinci"},
            "rootUri":uri, "workspaceFolders":[{"uri":uri,"name":"project"}],
            "capabilities": {
                "general":{"positionEncodings":["utf-16"]},
                "workspace":{
                    "applyEdit":false,
                    "workspaceEdit":{"documentChanges":false},
                    "configuration":true,
                    "workspaceFolders":true,
                    "diagnostics":{"refreshSupport":true}
                },
                "window":{"workDoneProgress":true},
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
        }), remaining(deadline)?).map_err(|e| IntelligenceError::new("initialization_failed", &format!("Language-server initialization failed ({})", e.code)))?;
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
        transport.notify("initialized", json!({}))?;
        Ok(Self {
            command,
            transport,
            capabilities,
            documents: BTreeMap::new(),
            diagnostic_floor: BTreeMap::new(),
        })
    }

    pub fn is_alive(&self) -> bool {
        self.transport.is_alive()
    }
    pub fn status(&self) -> Value {
        json!({"workspace":self.command.workspace,"backend":self.command.kind,
            "serverVersion":self.command.version,"projectTypeScript":self.command.typescript_version,
            "session":if self.is_alive() {"running"} else {"stopped"},
            "pid":self.transport.pid(),"documents":self.documents.len()})
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
        deadline: Instant,
    ) -> Result<()> {
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
            remaining(deadline)?;
            if !path.exists() && Some(path.as_path()) != source {
                if let Some(old) = self.documents.remove(&path) {
                    self.transport.notify(
                        "textDocument/didClose",
                        json!({"textDocument":{"uri":old.uri}}),
                    )?;
                    self.transport.unwatch_document(&old.uri);
                    self.diagnostic_floor.remove(&old.uri);
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
            for event in events {
                self.transport.notify(
                    event["method"].as_str().expect("internal event"),
                    event["params"].clone(),
                )?;
            }
            self.documents.insert(document.path.clone(), document);
        }
        Ok(())
    }

    pub fn execute(
        &mut self,
        method: &str,
        capability: &str,
        source: Option<&Path>,
        mut params: Value,
        adapter: &dyn ServerAdapter,
        deadline: Instant,
    ) -> Result<Value> {
        if method != "textDocument/diagnostic" && !self.supports(capability) {
            return Err(IntelligenceError::new(
                "unsupported_method",
                "Selected server does not advertise this semantic operation",
            ));
        }
        self.synchronize(source, adapter, deadline)?;
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
            if self.supports("diagnosticProvider") {
                let response = self
                    .transport
                    .request(method, params, remaining(deadline)?)?;
                return response
                    .get("items")
                    .filter(|v| v.is_array())
                    .cloned()
                    .ok_or_else(|| {
                        IntelligenceError::new(
                            "protocol_error",
                            "Expected a full document diagnostic report",
                        )
                    });
            }
            let path = source.ok_or_else(|| {
                IntelligenceError::new("invalid_source_path", "Diagnostics require a source file")
            })?;
            let document = &self.documents[path];
            let mut floor = *self.diagnostic_floor.get(&document.uri).unwrap_or(&0);
            loop {
                let mut snapshot =
                    self.transport
                        .wait_diagnostics(&document.uri, floor, remaining(deadline)?)?;
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
                    let window = remaining(deadline)
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
        self.transport.request(method, params, remaining(deadline)?)
    }
}

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
    use super::super::servers::{ServerKind, TypeScriptAdapter};
    use super::*;
    use std::time::Duration;

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(3)
    }
    fn fixture(root: &Path, mode: &str) -> ServerCommand {
        ServerCommand {
            kind: ServerKind::TypeScriptLanguageServer,
            program: "node".into(),
            args: vec![
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/language-server.cjs"
                )
                .into(),
                mode.into(),
            ],
            workspace: root.into(),
            version: Some("fixture".into()),
            typescript_version: None,
            initialization_options: json!({}),
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
        assert_eq!(first.as_array().unwrap().len(), 1);
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
        assert_eq!(next, json!([]));
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
