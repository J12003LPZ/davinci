//! Shared native browser adapter over P2 authority and the supervised bridge.
use crate::interaction_testing::{
    artifacts::{ArtifactBudgetTracker, MAX_RUN_BYTES},
    browser_process::{BrowserProcess, BrowserProcessConfig},
    validate_receipt_provenance, BackendKind, InteractionReceipt,
};
use davinci_agent::{
    process_manager::{BrowserDevServerLease, BrowserRequest, ProcessManager},
    runtime::operations::{
        BrowserOperationAdapter, BrowserOperationDisposition, BrowserOperationHandle,
    },
    runtime::transactions::{coordinator_for_context, SourceObservation, TransactionCoordinator},
    ToolContext, ToolError, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const TOOL_NAMES: &[&str] = &[
    "browser_open",
    "browser_snapshot",
    "browser_click",
    "browser_type",
    "browser_select",
    "browser_console",
    "browser_network",
    "browser_accessibility",
    "browser_screenshot",
    "browser_close",
];

/// Executable and package pins are global host configuration only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserConfig {
    pub enabled: bool,
    pub node: PathBuf,
    pub package: PathBuf,
    pub version: String,
}

#[derive(Default)]
struct Store {
    engine: Option<Arc<BrowserProcess>>,
    starting: Option<Arc<Startup>>,
    opening: usize,
    resources: HashMap<Uuid, Arc<Resource>>,
    retained: HashMap<String, RetainedArtifact>,
}
#[derive(Default)]
struct Startup {
    result: Mutex<Option<Result<Arc<BrowserProcess>, String>>>,
    ready: Condvar,
}
impl Startup {
    fn wait(&self, context: &ToolContext) -> Result<Arc<BrowserProcess>, String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut result = self
            .result
            .lock()
            .map_err(|_| "browser startup unavailable")?;
        loop {
            if context.is_aborted() {
                return Err("browser request cancelled".into());
            }
            if let Some(result) = &*result {
                return result.clone();
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("browser startup wait timed out".into());
            }
            result = self
                .ready
                .wait_timeout(result, remaining.min(Duration::from_millis(10)))
                .map_err(|_| "browser startup unavailable")?
                .0;
        }
    }
    fn complete(&self, result: Result<Arc<BrowserProcess>, String>) {
        *self
            .result
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(result);
        self.ready.notify_all();
    }
}
struct SourceBinding {
    coordinator: TransactionCoordinator,
    observation: SourceObservation,
    workspace: PathBuf,
    contract: Arc<Mutex<Option<davinci_agent::runtime::TaskContract>>>,
}
impl SourceBinding {
    fn transaction_id(&self) -> &str {
        self.observation.transaction_id()
    }
    fn sequence(&self) -> u64 {
        self.observation.sequence()
    }
    fn workspace_identity(&self) -> &str {
        self.observation.workspace_identity()
    }
    fn source_digest(&self) -> &str {
        self.observation.source_digest()
    }
    fn affected_files(&self) -> &[String] {
        self.observation.affected_files()
    }
    fn check(&self, manager: &ProcessManager, context: &ToolContext) -> Result<(), String> {
        if context.is_aborted() {
            return Err("browser source observation cancelled".into());
        }
        self.coordinator
            .check_source_observation(&self.observation, &|path| {
                manager.check_current_source_read(&self.workspace, path, &self.contract)
            })
    }
    fn metadata(&self) -> Value {
        json!({
            "transaction_id": self.transaction_id(),
            "transaction_sequence": self.sequence(),
            "workspace_identity": self.workspace_identity(),
            "source_digest": self.source_digest(),
            "affected_paths": self.affected_files(),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserAssertionSpec {
    pub dom_contains: String,
    pub accessibility_contains: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserVerificationReceipt {
    pub transaction_id: String,
    pub transaction_sequence: u64,
    pub workspace_identity: String,
    pub source_digest: String,
    pub affected_paths: Vec<String>,
    pub action_sequences: Vec<u64>,
    pub screenshot_artifact: Option<String>,
    pub incomplete_coverage: Vec<String>,
    pub interaction: InteractionReceipt,
}

struct Resource {
    backend: u64,
    process_id: u32,
    port: u16,
    lease: BrowserDevServerLease,
    engine: Arc<BrowserProcess>,
    browser_version: String,
    source: Option<Arc<SourceBinding>>,
}
#[derive(Clone)]
struct RetainedArtifact {
    browser_id: Uuid,
    action_sequence: u64,
    lease: BrowserDevServerLease,
    workspace: PathBuf,
    source: Option<Arc<SourceBinding>>,
}
#[derive(Clone)]
pub struct BrowserController {
    config: BrowserConfig,
    workspace: PathBuf,
    host_workspace: PathBuf,
    store: Arc<Mutex<Store>>,
    artifacts: Arc<Mutex<ArtifactBudgetTracker>>,
}
impl std::fmt::Debug for BrowserController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserController")
            .field("enabled", &self.config.enabled)
            .finish_non_exhaustive()
    }
}
impl Default for BrowserController {
    fn default() -> Self {
        Self::new(Path::new(""), BrowserConfig::default())
    }
}

/// Parent-owned engine and trusted executable authority, never supplied by a worker.
#[derive(Debug, Clone)]
pub struct BrowserWorkerHost {
    pub controller: BrowserController,
    pub supervisor: davinci_agent::jobs::supervisor::SupervisorCommand,
}

/// One worker's contexts; physical engine and artifact budget remain shared.
pub(super) struct BrowserWorker {
    host: BrowserWorkerHost,
    context: ToolContext,
    owned: Mutex<std::collections::HashSet<Uuid>>,
}

impl BrowserWorkerHost {
    pub(super) fn for_worker(
        &self,
        workspace: &Path,
        processes: davinci_agent::process_manager::ProcessManager,
        abort: Arc<std::sync::atomic::AtomicBool>,
        transaction_owner: davinci_agent::runtime::transactions::TransactionOwner,
        task_contract: Option<davinci_agent::runtime::TaskContract>,
    ) -> Result<BrowserWorker, String> {
        let workspace = workspace
            .canonicalize()
            .map_err(|_| "browser workspace unavailable")?;
        if !workspace.is_dir() {
            return Err("browser workspace is not a directory".into());
        }
        if self.controller.config.enabled {
            for dependency in [
                &self.controller.config.node,
                &self.controller.config.package,
            ] {
                if dependency
                    .canonicalize()
                    .map_err(|_| "trusted browser dependency unavailable")?
                    .starts_with(&workspace)
                {
                    return Err("browser dependencies must be outside the worker workspace".into());
                }
            }
        }
        let mut host = self.clone();
        host.controller.workspace = workspace;
        Ok(BrowserWorker {
            host,
            context: ToolContext {
                processes: Some(processes),
                foreground_supervisor: Some(self.supervisor.clone()),
                abort: Some(abort),
                transaction_owner,
                active_contract: Arc::new(Mutex::new(task_contract)),
                ..Default::default()
            },
            owned: Mutex::new(Default::default()),
        })
    }
}

impl BrowserWorker {
    pub(super) fn execute(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
    ) -> Result<ToolResult, ToolError> {
        let request = parse(name, args).map_err(ToolError::Failed)?;
        if let Request::Action { id, .. } = &request {
            if !self
                .owned
                .lock()
                .map_err(|_| ToolError::Failed("worker browser state unavailable".into()))?
                .contains(id)
            {
                return Err(ToolError::Failed(
                    "worker browser resource unavailable".into(),
                ));
            }
        }
        let result = self.host.controller.execute(cwd, name, args, &self.context);
        // Backend failures can close contexts. Do not retain stale owner IDs.
        {
            let store = self
                .host
                .controller
                .store
                .lock()
                .map_err(|_| ToolError::Failed("browser state unavailable".into()))?;
            self.owned
                .lock()
                .map_err(|_| ToolError::Failed("worker browser state unavailable".into()))?
                .retain(|id| store.resources.contains_key(id));
        }
        let result = result?;
        if !result.is_error {
            let mut owned = self
                .owned
                .lock()
                .map_err(|_| ToolError::Failed("worker browser state unavailable".into()))?;
            if name == "browser_open" {
                if let Some(id) = result
                    .details
                    .as_ref()
                    .and_then(|v| v["browser_id"].as_str())
                    .and_then(|v| Uuid::parse_str(v).ok())
                {
                    owned.insert(id);
                }
            } else if name == "browser_close" {
                if let Request::Action { id, .. } = request {
                    owned.remove(&id);
                }
            }
        }
        Ok(result)
    }
}

impl Drop for BrowserWorker {
    fn drop(&mut self) {
        // Teardown is host cleanup, independent of revoked tool permission.
        if let Ok(owned) = self.owned.get_mut() {
            for id in owned.drain() {
                self.host.controller.close(id);
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Open {
    process_id: u32,
    port: u16,
    #[serde(default = "loopback_host")]
    host: String,
    #[serde(default = "root_path")]
    path: String,
    viewport: Option<Viewport>,
    transaction_id: Option<String>,
}
fn root_path() -> String {
    "/".into()
}
fn loopback_host() -> String {
    "127.0.0.1".into()
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Viewport {
    width: u16,
    height: u16,
}
enum Request {
    Open(Open),
    Action { id: Uuid, command: Value },
}

enum BrowserExecution {
    Live(Value),
    Replay(ToolResult),
}

fn parse(name: &str, args: &Value) -> Result<Request, String> {
    if !TOOL_NAMES.contains(&name) {
        return Err("unknown browser tool".into());
    }
    if serde_json::to_vec(args)
        .map_err(|_| "invalid browser arguments")?
        .len()
        > 12 * 1024
    {
        return Err("browser arguments exceed limit".into());
    }
    if name == "browser_open" {
        let request: Open =
            serde_json::from_value(args.clone()).map_err(|_| "invalid browser open arguments")?;
        if request.process_id == 0
            || request.port == 0
            || !matches!(request.host.as_str(), "127.0.0.1" | "::1")
            || request.path.len() > 8192
            || !request.path.starts_with('/')
            || request.path.starts_with("//")
            || request.path.contains('\\')
            || request.path.chars().any(char::is_control)
            || request.viewport.as_ref().is_some_and(|v| {
                !(128..=1920).contains(&v.width) || !(128..=1080).contains(&v.height)
            })
            || request
                .transaction_id
                .as_deref()
                .is_some_and(|id| Uuid::parse_str(id).is_err())
        {
            return Err("invalid browser open bounds".into());
        }
        return Ok(Request::Open(request));
    }
    let args = args
        .as_object()
        .ok_or("browser arguments must be an object")?;
    let action = name
        .strip_prefix("browser_")
        .ok_or("invalid browser action")?;
    let extra = match action {
        "click" => Some("selector"),
        "type" => Some("text"),
        "select" => Some("value"),
        _ => None,
    };
    let selecting = matches!(action, "click" | "type" | "select");
    if args.keys().any(|key| {
        key != "browser_id" && !(selecting && key == "selector") && Some(key.as_str()) != extra
    }) {
        return Err("unexpected browser argument".into());
    }
    let id = args
        .get("browser_id")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
        .ok_or("invalid browser resource ID")?;
    let mut command = json!({"action":action});
    if selecting {
        let selector = args
            .get("selector")
            .and_then(Value::as_object)
            .ok_or("browser selector required")?;
        let kind = selector
            .get("kind")
            .and_then(Value::as_str)
            .ok_or("invalid browser selector")?;
        let fields: &[&str] = match kind {
            "role" => &["kind", "role", "name"],
            "label" | "test_id" => &["kind", "value"],
            _ => return Err("unsupported browser selector".into()),
        };
        if selector.len() != fields.len()
            || fields.iter().any(|key| {
                !selector
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty() && text.len() <= 1024)
            })
        {
            return Err("invalid browser selector fields".into());
        }
        if kind == "role" && selector["role"].as_str().unwrap().len() > 64 {
            return Err("browser role exceeds limit".into());
        }
        command["selector"] = Value::Object(selector.clone());
    }
    if let Some(key @ ("text" | "value")) = extra {
        let text = args
            .get(key)
            .and_then(Value::as_str)
            .ok_or("browser text required")?;
        if text.len() > 4096 {
            return Err("browser text exceeds limit".into());
        }
        command[key] = json!(text);
    }
    Ok(Request::Action { id, command })
}

impl BrowserController {
    /// Frontend-only bounded binary retrieval; never added to model tool output.
    pub fn retrieve_artifact(
        &self,
        cwd: &Path,
        args: &Value,
        context: &ToolContext,
    ) -> Result<Value, String> {
        use base64::Engine;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Retrieval {
            browser_id: Uuid,
            artifact: String,
            offset: usize,
            limit: usize,
        }
        if serde_json::to_vec(args)
            .map_err(|_| "invalid browser artifact request")?
            .len()
            > 12 * 1024
        {
            return Err("browser artifact request exceeds limit".into());
        }
        let request: Retrieval =
            serde_json::from_value(args.clone()).map_err(|_| "invalid browser artifact request")?;
        if !self.config.enabled
            || request.artifact.len() > 128
            || !(1..=65536).contains(&request.limit)
            || cwd.canonicalize().map_err(|_| "browser cwd unavailable")? != self.workspace
        {
            return Err("invalid browser artifact bounds or workspace".into());
        }
        let manager = context
            .processes
            .as_ref()
            .ok_or("browser process manager unavailable")?;
        let retained = self
            .store
            .lock()
            .map_err(|_| "browser state unavailable")?
            .retained
            .get(&request.artifact)
            .cloned()
            .filter(|retained| retained.browser_id == request.browser_id)
            .ok_or("browser artifact unavailable for this context")?;
        manager.with_retained_browser_artifact(BrowserRequest {
            cwd: &retained.workspace, name: "browser_screenshot", args,
            abort: context.abort.as_deref(), permit: context.dispatch_permit.as_deref(),
            process_id: retained.lease.process_id(), port: retained.lease.port(), lease: Some(&retained.lease),
        }, || {
            if let Some(source) = &retained.source {
                source.check(manager, context)?;
            }
            let artifacts = self.artifacts.lock().map_err(|_| "browser artifacts unavailable")?;
            let bytes = artifacts.items.get(&request.artifact).ok_or("browser artifact unavailable")?;
            if request.offset > bytes.len() { return Err("browser artifact offset exceeds size".into()); }
            let end = request.offset.saturating_add(request.limit).min(bytes.len());
            let mut result = json!({"artifact":request.artifact,"mediaType":"image/png",
                "browser_id":retained.browser_id,"action_sequence":retained.action_sequence,
                "verification":"observations_only",
                "sha256":crate::interaction_testing::artifacts::compute_sha256(bytes),
                "size":bytes.len(),"offset":request.offset,"nextOffset":end,
                "eof":end == bytes.len(),
                "base64":base64::engine::general_purpose::STANDARD.encode(&bytes[request.offset..end])});
            drop(artifacts);
            if let Some(source) = &retained.source {
                source.check(manager, context)?;
                result["source_binding"] = source.metadata();
            }
            Ok(result)
        })
    }

    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn shutdown_backend_for_test(&self) {
        let engine = self.store.lock().unwrap().engine.clone().unwrap();
        engine
            .request(json!({"op":"shutdown"}), Duration::from_secs(5))
            .unwrap();
        // Confirm the transport has stopped accepting requests before returning.
        assert!(engine
            .request(
                json!({"op":"open","options":{"origins":["http://127.0.0.1:3000"]}}),
                Duration::from_secs(1)
            )
            .is_err());
    }

    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn context_count(&self) -> usize {
        self.store.lock().unwrap().resources.len()
    }
    pub fn new(workspace: &Path, config: BrowserConfig) -> Self {
        let workspace = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.into());
        Self {
            config,
            host_workspace: workspace.clone(),
            workspace,
            store: Arc::new(Mutex::new(Store::default())),
            artifacts: Arc::new(Mutex::new(ArtifactBudgetTracker::new(MAX_RUN_BYTES))),
        }
    }
    fn bind_source(
        &self,
        transaction_id: &str,
        manager: &ProcessManager,
        context: &ToolContext,
    ) -> Result<Arc<SourceBinding>, String> {
        let coordinator =
            coordinator_for_context(&self.workspace, context).map_err(|error| error.to_string())?;
        let contract = context.active_contract.clone();
        let observation = coordinator.observe_source(transaction_id, &|path| {
            manager.check_current_source_read(&self.workspace, path, &contract)
        })?;
        Ok(Arc::new(SourceBinding {
            coordinator,
            observation,
            workspace: self.workspace.clone(),
            contract,
        }))
    }

    /// Host-only deterministic browser verification. This composes the existing
    /// authorized native actions and returns RealBrowser evidence without changing
    /// transaction verification state.
    pub fn verify_host(
        &self,
        cwd: &Path,
        id: Uuid,
        spec: BrowserAssertionSpec,
        context: &ToolContext,
    ) -> Result<BrowserVerificationReceipt, String> {
        if spec.dom_contains.is_empty()
            || spec.dom_contains.len() > 4096
            || spec
                .accessibility_contains
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > 4096)
        {
            return Err("browser assertion bounds are invalid".into());
        }
        let resource = self
            .store
            .lock()
            .map_err(|_| "browser state unavailable")?
            .resources
            .get(&id)
            .cloned()
            .ok_or("browser resource unavailable")?;
        let source = resource
            .source
            .clone()
            .ok_or("browser verification requires a transaction source binding")?;
        let manager = context
            .processes
            .as_ref()
            .ok_or("browser process manager unavailable")?;
        source.check(manager, context)?;

        let mut action_sequences = Vec::new();
        let mut observe = |name: &str| -> Result<Value, String> {
            let result = self
                .execute(cwd, name, &json!({"browser_id":id}), context)
                .map_err(|error| error.to_string())?;
            if result.is_error {
                return Err(format!("{name} failed during browser verification"));
            }
            let details = result
                .details
                .ok_or("browser verification details unavailable")?;
            let sequence = details["result"]["action_sequence"]
                .as_u64()
                .filter(|value| *value > 0)
                .ok_or("browser verification action sequence unavailable")?;
            action_sequences.push(sequence);
            Ok(details)
        };

        let snapshot = observe("browser_snapshot")?;
        let accessibility = observe("browser_accessibility")?;
        let console = observe("browser_console")?;
        let network = observe("browser_network")?;
        let screenshot = observe("browser_screenshot")?;
        source.check(manager, context)?;

        let html = snapshot["result"]["html"]
            .as_str()
            .ok_or("browser snapshot missing HTML")?;
        let dom_passed = html.contains(&spec.dom_contains);
        let mut assertions = vec![format!(
            "{}: DOM contains {:?}",
            if dom_passed { "PASS" } else { "FAIL" },
            spec.dom_contains
        )];

        let tree = accessibility["result"]["tree"]
            .as_str()
            .ok_or("browser accessibility snapshot unavailable")?;
        let accessibility_passed = spec
            .accessibility_contains
            .as_ref()
            .map(|expected| {
                let passed = tree.contains(expected);
                assertions.push(format!(
                    "{}: accessibility contains {:?}",
                    if passed { "PASS" } else { "FAIL" },
                    expected
                ));
                passed
            })
            .unwrap_or(true);

        let console_events = console["result"]["events"]
            .as_array()
            .ok_or("browser console evidence unavailable")?;
        let console_clear = console_events.is_empty();
        assertions.push(format!(
            "{}: no console errors",
            if console_clear { "PASS" } else { "FAIL" }
        ));
        let console_errors = console_events
            .iter()
            .map(|event| {
                event["message"]
                    .as_str()
                    .unwrap_or("browser console error")
                    .chars()
                    .take(1024)
                    .collect()
            })
            .collect();

        let network_events = network["result"]["events"]
            .as_array()
            .ok_or("browser network evidence unavailable")?;
        let network_clear = network_events.is_empty();
        assertions.push(format!(
            "{}: no failed network requests",
            if network_clear { "PASS" } else { "FAIL" }
        ));
        let network_failures = network_events
            .iter()
            .map(|event| {
                format!(
                    "{} (status {})",
                    event["url"].as_str().unwrap_or("[unknown URL]"),
                    event["status"].as_u64().unwrap_or(0)
                )
            })
            .collect();

        let mut incomplete_coverage = Vec::new();
        if console["result"]["omitted"].as_bool() == Some(true) {
            incomplete_coverage.push("console evidence was truncated".into());
        }
        if network["result"]["omitted"].as_bool() == Some(true) {
            incomplete_coverage.push("network evidence was truncated".into());
        }
        let screenshot_artifact = screenshot["result"]["artifact"]
            .as_str()
            .map(str::to_owned)
            .ok_or("browser screenshot artifact unavailable")?;
        let passed = dom_passed
            && accessibility_passed
            && console_clear
            && network_clear
            && incomplete_coverage.is_empty();
        let interaction = InteractionReceipt {
            scenario_id: format!("browser-transaction-{}", source.transaction_id()),
            backend_kind: BackendKind::RealBrowser,
            backend_identity: format!("chromium-{}", resource.browser_version),
            source_manifest: Some(source.source_digest().to_owned()),
            assertions_passed: passed,
            assertions,
            frames_count: 3,
            event_log: vec![
                "host browser actions completed".into(),
                "accessibility snapshot captured".into(),
                "screenshot retained as artifact".into(),
                "transaction source remained unchanged".into(),
            ],
            console_errors,
            network_failures,
            trace_refs: vec![screenshot_artifact.clone()],
            exit_outcome: Some(if passed { 0 } else { 1 }),
        };
        validate_receipt_provenance(&interaction)?;

        Ok(BrowserVerificationReceipt {
            transaction_id: source.transaction_id().to_owned(),
            transaction_sequence: source.sequence(),
            workspace_identity: source.workspace_identity().to_owned(),
            source_digest: source.source_digest().to_owned(),
            affected_paths: source.affected_files().to_vec(),
            action_sequences,
            screenshot_artifact: Some(screenshot_artifact),
            incomplete_coverage,
            interaction,
        })
    }

    fn reconcile_backend(&self) -> Result<(), String> {
        let stale = {
            let mut store = self.store.lock().map_err(|_| "browser state unavailable")?;
            if store
                .engine
                .as_ref()
                .is_some_and(|engine| !engine.is_healthy())
            {
                Some((store.engine.take(), std::mem::take(&mut store.resources)))
            } else {
                None
            }
        };
        // Dropping the last backend reference may stop/wait for its process.
        // Never perform that cleanup while holding the controller's store lock.
        drop(stale);
        Ok(())
    }
    fn reconcile_servers(&self) -> Result<(), String> {
        let resources: Vec<_> = self
            .store
            .lock()
            .map_err(|_| "browser state unavailable")?
            .resources
            .iter()
            .map(|(id, resource)| (*id, resource.clone()))
            .collect();
        for (id, resource) in resources {
            if !resource.lease.is_live() {
                self.close(id);
            }
        }
        Ok(())
    }
    fn engine(&self, context: &ToolContext) -> Result<Arc<BrowserProcess>, String> {
        if context.is_aborted() {
            return Err("browser request cancelled".into());
        }
        self.reconcile_backend()?;
        let (startup, owner) = {
            let mut store = self.store.lock().map_err(|_| "browser state unavailable")?;
            if let Some(engine) = &store.engine {
                return Ok(engine.clone());
            }
            if let Some(startup) = &store.starting {
                (startup.clone(), false)
            } else {
                let startup = Arc::new(Startup::default());
                store.starting = Some(startup.clone());
                (startup, true)
            }
        };
        if !owner {
            return startup.wait(context);
        }
        // Only a small reservation spans startup; the store/native host locks do not.
        let result = context
            .foreground_supervisor
            .as_ref()
            .ok_or("browser supervisor unavailable")
            .and_then(|host| {
                BrowserProcess::start(
                    host,
                    BrowserProcessConfig {
                        node: &self.config.node,
                        package: &self.config.package,
                        version: &self.config.version,
                        workspace: &self.host_workspace,
                        environment: Default::default(),
                    },
                )
                .map_err(|_| "trusted browser backend unavailable")
            })
            .map(Arc::new)
            .map_err(str::to_owned);
        let published = self
            .store
            .lock()
            .map(|mut store| {
                if let Ok(engine) = &result {
                    store.engine = Some(engine.clone());
                }
                store.starting = None;
            })
            .is_ok();
        if !published {
            startup.complete(Err("browser state unavailable".into()));
            return Err("browser state unavailable".into());
        }
        // Waiters retain this attempt's result even if a later caller retries.
        startup.complete(result);
        startup.wait(context)
    }
    fn close(&self, id: Uuid) {
        let resource = self
            .store
            .lock()
            .ok()
            .and_then(|mut store| store.resources.remove(&id));
        if let Some(resource) = resource {
            let _ = resource.engine.request(
                json!({"op":"close","resource":resource.backend}),
                Duration::from_secs(10),
            );
        }
    }
    pub fn execute(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
        context: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let request = parse(name, args).map_err(ToolError::Failed)?;
        if context.is_aborted() {
            return Err(ToolError::Failed("browser request cancelled".into()));
        }
        if !self.config.enabled {
            return Ok(ToolResult {
                content: "Browser verification unavailable; use source and test verification."
                    .into(),
                is_error: true,
                details: Some(json!({"status":"unavailable","fallback":["source","tests"]})),
            });
        }
        if cwd
            .canonicalize()
            .map_err(|_| ToolError::Failed("browser workspace unavailable".into()))?
            != self.workspace
        {
            return Err(ToolError::Failed("browser workspace changed".into()));
        }
        let manager = context
            .processes
            .as_ref()
            .ok_or_else(|| ToolError::Failed("managed process context required".into()))?;
        self.reconcile_backend().map_err(ToolError::Failed)?;
        self.reconcile_servers().map_err(ToolError::Failed)?;
        let (process_id, port, expected, existing) = match &request {
            Request::Open(open) => (open.process_id, open.port, None, None),
            Request::Action { id, .. } => {
                let resource = self
                    .store
                    .lock()
                    .map_err(|_| ToolError::Failed("browser state unavailable".into()))?
                    .resources
                    .get(id)
                    .cloned()
                    .ok_or_else(|| ToolError::Failed("browser resource unavailable".into()))?;
                (
                    resource.process_id,
                    resource.port,
                    Some(resource.lease.clone()),
                    Some(resource),
                )
            }
        };
        let browser_id = match &request {
            Request::Open(_) => None,
            Request::Action { id, .. } => Some(id.to_string()),
        };
        let browser_operation = context
            .runtime
            .as_ref()
            .and_then(BrowserOperationAdapter::from_runtime);
        let browser_call_id = context.tool_call_id.clone();
        let mut created = None;
        let mut entered = false;
        let mut operation: Option<BrowserOperationHandle> = None;
        let result = manager.with_verified_browser_dev_server(BrowserRequest {
            cwd, name, args, abort: context.abort.as_deref(), permit: context.dispatch_permit.as_deref(),
            process_id, port, lease: expected.as_ref(),
        }, |lease| {
            if let Some(adapter) = &browser_operation {
                let call_id = browser_call_id
                    .as_deref()
                    .ok_or("browser operation requires a trusted tool call identity")?;
                let permission_revision = manager.permission_revision()?;
                let handle = adapter
                    .start(
                        call_id,
                        name,
                        args,
                        browser_id.clone(),
                        None,
                        lease.identity(),
                        permission_revision,
                    )
                    .map_err(|error| error.to_string())?;
                match handle.disposition() {
                    BrowserOperationDisposition::ExistingInFlight => {
                        return Err("browser operation is already in flight".into())
                    }
                    BrowserOperationDisposition::ExistingResult => {
                        return handle
                            .replay_result()
                            .map(BrowserExecution::Replay)
                            .map_err(|error| error.to_string())
                    }
                    BrowserOperationDisposition::New => {
                        handle
                            .begin(
                                context.is_aborted(),
                                || lease.verify_listening_socket(),
                                || (),
                            )
                            .map_err(|error| error.to_string())?;
                        operation = Some(handle);
                    }
                }
            }
            entered = true;
            match request {
                Request::Open(open) => {
                    {
                        let mut store = self.store.lock().map_err(|_| "browser state unavailable")?;
                        if store.resources.len() + store.opening >= 8 { return Err("browser context limit".into()); }
                        store.opening += 1;
                    }
                    let result = (|| {
                        let engine = self.engine(context)?;
                        // Waiting for shared startup cannot preserve stale authority.
                        manager.with_verified_browser_dev_server(BrowserRequest {
                            cwd, name, args, abort: context.abort.as_deref(), permit: context.dispatch_permit.as_deref(),
                            process_id, port, lease: Some(lease),
                        }, |_| Ok(()))?;
                        let source = open
                            .transaction_id
                            .as_deref()
                            .map(|id| self.bind_source(id, manager, context))
                            .transpose()?;
                        let origin = lease.origin();
                        let mut options = json!({"origins":[origin]});
                        if let Some(viewport) = open.viewport { options["viewport"] = json!(viewport); }
                        let opened = engine.request_with_abort(json!({"op":"open","options":options}), Duration::from_secs(30), context.abort.as_deref())?;
                        let backend = opened["resource"].as_u64().filter(|id| *id > 0).ok_or("invalid browser resource")?;
                        let id = Uuid::new_v4();
                        let browser_version = opened["browserVersion"]
                            .as_str()
                            .ok_or("invalid browser version")?
                            .to_owned();
                        let resource = Arc::new(Resource {
                            backend,
                            process_id,
                            port,
                            lease: lease.clone(),
                            engine: engine.clone(),
                            browser_version: browser_version.clone(),
                            source: source.clone(),
                        });
                        self.store.lock().map_err(|_| "browser state unavailable")?.resources.insert(id, resource);
                        created = Some(id);
                        let navigation = engine.request_with_abort(json!({"op":"execute","resource":backend,
                            "command":{"action":"navigate","url":format!("{origin}{}",open.path)}}), Duration::from_secs(10), context.abort.as_deref())?;
                        if let Some(source) = &source {
                            source.check(manager, context)?;
                        }
                        let mut result = json!({"status":"opened","browser_id":id,"origin":origin,
                            "browser_version":opened["browserVersion"],"navigation":navigation});
                        if let Some(source) = &source {
                            result["source_binding"] = source.metadata();
                        }
                        Ok(BrowserExecution::Live(result))
                    })();
                    self.store.lock().map_err(|_| "browser state unavailable")?.opening -= 1;
                    result
                }
                Request::Action { id, command } => {
                    let resource = existing.as_ref().ok_or("browser resource unavailable")?;
                    if name == "browser_close" {
                        let result = resource.engine.request(json!({"op":"close","resource":resource.backend}), Duration::from_secs(10))?;
                        self.store.lock().map_err(|_| "browser state unavailable")?.resources.remove(&id);
                        Ok(BrowserExecution::Live(result))
                    } else {
                        if let Some(source) = &resource.source {
                            source.check(manager, context)?;
                        }
                        let mut result = resource.engine.request_with_abort(json!({"op":"execute","resource":resource.backend,"command":command}), Duration::from_secs(10), context.abort.as_deref())?;
                        if let Some(source) = &resource.source {
                            source.check(manager, context)?;
                        }
                        if name == "browser_screenshot" {
                            let mut artifacts = self.artifacts.lock().map_err(|_| "browser artifacts unavailable")?;
                            result = resource.engine.retain_screenshot(&result, &mut artifacts)?;
                            let label = result["artifact"].as_str().ok_or("invalid retained artifact")?.to_owned();
                            let action_sequence = result["action_sequence"].as_u64().filter(|id| *id > 0).ok_or("invalid screenshot action sequence")?;
                            drop(artifacts);
                            self.store.lock().map_err(|_| "browser state unavailable")?.retained.insert(label,
                                RetainedArtifact {
                                    browser_id: id,
                                    action_sequence,
                                    lease: resource.lease.clone(),
                                    workspace: self.workspace.clone(),
                                    source: resource.source.clone(),
                                });
                        }
                        if let Some(source) = &resource.source {
                            result["source_binding"] = source.metadata();
                        }
                        Ok(BrowserExecution::Live(json!({"status":"observed","browser_id":id,"result":result})))
                    }
                }
            }
        });
        if result.is_err() && entered {
            if let Some(id) = created.or_else(|| {
                args["browser_id"]
                    .as_str()
                    .and_then(|id| Uuid::parse_str(id).ok())
            }) {
                self.close(id);
            }
        }
        let result_error = result.as_ref().err().cloned();
        let mut tool_result = match result {
            Ok(BrowserExecution::Replay(result)) => result,
            Ok(BrowserExecution::Live(mut result)) => {
                result["verification"] = json!("observations_only");
                ToolResult {
                    content: result.to_string(),
                    is_error: false,
                    details: Some(result),
                }
            }
            Err(error) => ToolResult {
                content: error,
                is_error: true,
                details: None,
            },
        };
        if let Some(handle) = operation.take() {
            let metadata = handle.metadata();
            let details = tool_result.details.get_or_insert_with(|| json!({}));
            if !details.is_object() {
                *details = json!({});
            }
            details["browser_operation"] = metadata.clone();
            if let Err(error) = handle.complete(&tool_result) {
                return Err(ToolError::Failed(format!(
                    "browser operation result could not be durably recorded: {error}; affected operation: {metadata}"
                )));
            }
        }
        if let Some(error) = result_error {
            return Err(ToolError::Failed(error));
        }
        Ok(tool_result)
    }
}

pub fn tool_spec(name: &str) -> Option<davinci_ai::ToolSpec> {
    if !TOOL_NAMES.contains(&name) {
        return None;
    }
    let selector = json!({"oneOf":[
        {"type":"object","properties":{"kind":{"const":"role"},"role":{"type":"string","minLength":1,"maxLength":64},"name":{"type":"string","minLength":1,"maxLength":1024}},"required":["kind","role","name"],"additionalProperties":false},
        {"type":"object","properties":{"kind":{"enum":["label","test_id"]},"value":{"type":"string","minLength":1,"maxLength":1024}},"required":["kind","value"],"additionalProperties":false}]});
    let mut parameters = json!({"type":"object","properties":{"browser_id":{"type":"string","format":"uuid"}},"required":["browser_id"],"additionalProperties":false});
    if name == "browser_open" {
        parameters = json!({"type":"object","properties":{"process_id":{"type":"integer","minimum":1},"port":{"type":"integer","minimum":1,"maximum":65535},"host":{"type":"string","enum":["127.0.0.1","::1"],"description":"Managed loopback address; defaults to 127.0.0.1."},"path":{"type":"string","maxLength":8192,"description":"Local path beginning with /; defaults to /."},"viewport":{"type":"object","properties":{"width":{"type":"integer","minimum":128,"maximum":1920},"height":{"type":"integer","minimum":128,"maximum":1080}},"required":["width","height"],"additionalProperties":false},"transaction_id":{"type":"string","format":"uuid","description":"Opaque transaction identifier supplied by a prior edit. Ownership and source binding come from trusted host state."}},"required":["process_id","port"],"additionalProperties":false});
    } else if matches!(name, "browser_click" | "browser_type" | "browser_select") {
        parameters["properties"]["selector"] = selector;
        parameters["required"]
            .as_array_mut()?
            .push(json!("selector"));
        let extra = match name {
            "browser_type" => Some("text"),
            "browser_select" => Some("value"),
            _ => None,
        };
        if let Some(extra) = extra {
            parameters["properties"][extra] = json!({"type":"string","maxLength":4096});
            parameters["required"].as_array_mut()?.push(json!(extra));
        }
    }
    Some(davinci_ai::ToolSpec { name: name.into(), description: format!("{} on an authorized managed local server. Returns browser observations, not completion proof.", name.replace('_', " ")),
        parameters, constrained_sampling: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_open_accepts_only_an_opaque_transaction_id() {
        let transaction_id = Uuid::new_v4().to_string();
        let request = parse(
            "browser_open",
            &json!({"process_id":1,"port":3000,"transaction_id":transaction_id}),
        )
        .expect("opaque transaction ID is accepted");
        let Request::Open(open) = request else {
            panic!("expected browser open request");
        };
        assert_eq!(
            open.transaction_id.as_deref(),
            Some(transaction_id.as_str())
        );
        assert!(parse(
            "browser_snapshot",
            &json!({"browser_id":Uuid::new_v4(),"transaction_id":transaction_id})
        )
        .is_err());
    }

    #[test]
    fn browser_source_binding_rechecks_live_transaction_source() {
        let root = tempfile::tempdir().unwrap();
        let permissions = Arc::new(davinci_agent::PermissionState::new(
            davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::AlwaysApprove),
        ));
        let manager = davinci_agent::process_manager::ProcessManager::new(
            root.path(),
            Arc::new(Mutex::new(davinci_agent::JobBook::default())),
            permissions,
            davinci_agent::jobs::supervisor::SupervisorCommand {
                executable: std::env::current_exe().unwrap(),
                argv: Vec::new(),
            },
        )
        .unwrap();
        let context = ToolContext {
            processes: Some(manager.clone()),
            ..Default::default()
        };
        let coordinator = davinci_agent::runtime::transactions::TransactionCoordinator::new(
            root.path(),
            context.transaction_owner.clone(),
        )
        .unwrap();
        let preview = coordinator
            .preview(vec![
                davinci_agent::runtime::transactions::ProposedChange::write(
                    "a.txt",
                    b"after".to_vec(),
                ),
            ])
            .unwrap();
        let applied = coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();

        let controller = BrowserController::new(root.path(), BrowserConfig::default());
        let binding = controller
            .bind_source(&preview.id, &manager, &context)
            .expect("current transaction source is observable");
        assert_eq!(binding.transaction_id(), preview.id);
        assert_eq!(binding.sequence(), applied.sequence);
        assert_eq!(binding.affected_files(), &["a.txt"]);
        binding
            .check(&manager, &context)
            .expect("unchanged source remains bound");

        std::fs::write(root.path().join("a.txt"), b"changed").unwrap();
        assert!(binding.check(&manager, &context).is_err());
    }

    #[test]
    fn graph_browser_worker_preserves_host_transaction_owner() {
        let root = tempfile::tempdir().unwrap();
        let permissions = Arc::new(davinci_agent::PermissionState::new(
            davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::AlwaysApprove),
        ));
        let manager = davinci_agent::process_manager::ProcessManager::new(
            root.path(),
            Arc::new(Mutex::new(davinci_agent::JobBook::default())),
            permissions,
            davinci_agent::jobs::supervisor::SupervisorCommand {
                executable: std::env::current_exe().unwrap(),
                argv: Vec::new(),
            },
        )
        .unwrap();
        let owner = davinci_agent::runtime::transactions::TransactionOwner {
            agent_id: davinci_agent::AgentId::new(),
            parent_agent_id: Some(davinci_agent::AgentId::new()),
            session_id: Some("graph-session".into()),
            task_id: None,
            graph_node: Some("writer-1".into()),
        };
        let host = BrowserWorkerHost {
            controller: BrowserController::default(),
            supervisor: davinci_agent::jobs::supervisor::SupervisorCommand {
                executable: std::env::current_exe().unwrap(),
                argv: Vec::new(),
            },
        };
        let worker = host
            .for_worker(
                root.path(),
                manager,
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                owner.clone(),
                None,
            )
            .unwrap();
        assert_eq!(worker.context.transaction_owner, owner);
    }

    #[test]
    fn browser_startup_waiter_cancellation_preserves_shared_attempt() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let controller = BrowserController::default();
        let startup = Arc::new(Startup::default());
        controller.store.lock().unwrap().starting = Some(startup.clone());
        let abort = Arc::new(AtomicBool::new(false));
        let context = ToolContext {
            abort: Some(abort.clone()),
            ..Default::default()
        };
        let waiting_controller = controller.clone();
        let waiter = std::thread::spawn(move || waiting_controller.engine(&context));
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            !waiter.is_finished(),
            "waiter must remain pending during startup"
        );
        let cancelled_at = Instant::now();
        abort.store(true, Ordering::SeqCst);
        assert_eq!(
            waiter.join().unwrap().err().as_deref(),
            Some("browser request cancelled")
        );
        assert!(cancelled_at.elapsed() < Duration::from_secs(1));
        assert!(Arc::ptr_eq(
            controller.store.lock().unwrap().starting.as_ref().unwrap(),
            &startup
        ));
        startup.complete(Err("original startup failed".into()));
        assert_eq!(
            controller.engine(&ToolContext::default()).err().as_deref(),
            Some("original startup failed")
        );
    }

    #[test]
    fn browser_startup_failure_is_shared_and_allows_fresh_attempt() {
        let controller = BrowserController::default();
        let old_attempt = Arc::new(Startup::default());
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let startup = old_attempt.clone();
                std::thread::spawn(move || startup.wait(&ToolContext::default()))
            })
            .collect();
        old_attempt.complete(Err("original startup failed".into()));
        // A new caller may retry, but previous waiters keep their original result.
        assert_eq!(
            controller.engine(&ToolContext::default()).err().as_deref(),
            Some("browser supervisor unavailable")
        );
        assert!(controller.store.lock().unwrap().starting.is_none());
        for thread in threads {
            assert_eq!(
                thread.join().unwrap().err().as_deref(),
                Some("original startup failed")
            );
        }
    }

    #[test]
    #[ignore = "requires explicitly configured trusted Node and Playwright installation"]
    fn browser_concurrent_startup_shares_one_supervised_backend() {
        let root = tempfile::tempdir().unwrap();
        let controller = BrowserController::new(
            root.path(),
            BrowserConfig {
                enabled: true,
                node: std::env::var("DAVINCI_TRUSTED_NODE_TEST_PATH")
                    .unwrap()
                    .into(),
                package: std::env::var("DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH")
                    .unwrap()
                    .into(),
                version: "1.62.0".into(),
            },
        );
        let context = ToolContext {
            foreground_supervisor: Some(davinci_agent::jobs::supervisor::SupervisorCommand {
                executable: std::env::current_exe().unwrap(),
                argv: vec![
                    "--exact".into(),
                    "native_extensions::graph::coordinator_handler::tests::helper_entry".into(),
                    "--nocapture".into(),
                ],
            }),
            ..Default::default()
        };
        let barrier = Arc::new(std::sync::Barrier::new(6));
        let threads: Vec<_> = (0..6)
            .map(|_| {
                let controller = controller.clone();
                let context = context.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    controller.engine(&context)
                })
            })
            .collect();
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let first = results[0].as_ref().unwrap();
        for result in &results {
            let engine = result
                .as_ref()
                .expect("concurrent callers must await shared startup");
            assert!(Arc::ptr_eq(first, engine));
        }
        let mut cancelled = context.clone();
        cancelled.abort = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        assert_eq!(
            controller.engine(&cancelled).err().as_deref(),
            Some("browser request cancelled")
        );
        let opened = first
            .request(
                json!({"op":"open","options":{"origins":["http://127.0.0.1:3000"]}}),
                Duration::from_secs(30),
            )
            .unwrap();
        assert!(opened["resource"].is_u64());
        first
            .request(json!({"op":"shutdown"}), Duration::from_secs(5))
            .unwrap();
    }

    #[test]
    #[ignore = "requires explicitly configured trusted Node and Playwright installation"]
    fn browser_dead_backend_is_replaced_without_replaying_actions() {
        let root = tempfile::tempdir().unwrap();
        let controller = BrowserController::new(
            root.path(),
            BrowserConfig {
                enabled: true,
                node: std::env::var("DAVINCI_TRUSTED_NODE_TEST_PATH")
                    .unwrap()
                    .into(),
                package: std::env::var("DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH")
                    .unwrap()
                    .into(),
                version: "1.62.0".into(),
            },
        );
        let context = ToolContext {
            foreground_supervisor: Some(davinci_agent::jobs::supervisor::SupervisorCommand {
                executable: std::env::current_exe().unwrap(),
                argv: vec![
                    "--exact".into(),
                    "native_extensions::graph::coordinator_handler::tests::helper_entry".into(),
                    "--nocapture".into(),
                ],
            }),
            ..Default::default()
        };
        let first = controller.engine(&context).unwrap();
        first
            .request(
                json!({"op":"open","options":{"origins":["http://127.0.0.1:3000"]}}),
                Duration::from_secs(30),
            )
            .unwrap();
        controller.shutdown_backend_for_test();
        let replacement = controller.engine(&context).unwrap();
        assert!(
            !Arc::ptr_eq(&first, &replacement),
            "dead backend must not remain cached"
        );
        let opened = replacement
            .request(
                json!({"op":"open","options":{"origins":["http://127.0.0.1:3000"]}}),
                Duration::from_secs(30),
            )
            .unwrap();
        assert!(opened["resource"].is_u64());
        replacement
            .request(json!({"op":"shutdown"}), Duration::from_secs(5))
            .unwrap();
    }

    #[test]
    fn browser_project_settings_cannot_enable_or_replace_host_pins() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".davinci")).unwrap();
        let global = state.path().join("settings.json");
        let project = root.path().join(".davinci/settings.json");
        let write_global = |enabled| {
            std::fs::write(&global, json!({"defaultProjectTrust":"always","browserVerification":{
                "enabled":enabled,"node":"host-node","package":"host-package","version":"host-version"
            }}).to_string()).unwrap();
        };
        let config = || {
            super::super::NativeExtensionHost::new_with_agent_dir(
                "browser-settings-fixture",
                root.path(),
                Some(state.path()),
            )
            .browser
            .config
        };
        write_global(false);
        std::fs::write(&project, json!({"browserVerification":{
            "enabled":true,"node":"project-node","package":"project-package","version":"project-version"
        }}).to_string()).unwrap();
        assert!(!config().enabled);
        write_global(true);
        let trusted = config();
        assert!(trusted.enabled);
        assert_eq!(trusted.node, PathBuf::from("host-node"));
        assert_eq!(trusted.package, PathBuf::from("host-package"));
        assert_eq!(trusted.version, "host-version");
        std::fs::write(&project, r#"{"browserVerification":{"enabled":false}}"#).unwrap();
        assert!(!config().enabled);
        std::fs::write(
            &project,
            r#"{"browserVerification":{"enabled":true,"unknown":true}}"#,
        )
        .unwrap();
        assert!(!config().enabled);
    }

    #[test]
    fn browser_disabled_and_cancelled_requests_do_not_start_processes() {
        let root = tempfile::tempdir().unwrap();
        let agent = davinci_agent::Agent::new("browser fallback fixture");
        let controller = BrowserController::new(root.path(), BrowserConfig::default());
        let args = json!({"process_id":1,"port":3000});
        let fallback = controller
            .execute(root.path(), "browser_open", &args, &agent.tool_context)
            .unwrap();
        assert!(fallback.is_error);
        assert_eq!(fallback.details.unwrap()["status"], "unavailable");
        let mut context = agent.tool_context.clone();
        context.abort = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        assert!(controller
            .execute(root.path(), "browser_open", &args, &context)
            .is_err());
        assert!(controller.store.lock().unwrap().engine.is_none());
        assert_eq!(context.jobs.lock().unwrap().running(), 0);
    }
    #[test]
    fn browser_parser_denies_protocols_extra_fields_and_unsafe_selectors() {
        for host in ["127.0.0.1", "::1"] {
            assert!(parse(
                "browser_open",
                &json!({"process_id":1,"port":3000,"host":host})
            )
            .is_ok());
        }
        for host in [
            json!("localhost"),
            json!("[::1]"),
            json!("external.example"),
            json!("::"),
            json!(null),
            json!(1),
        ] {
            assert!(parse(
                "browser_open",
                &json!({"process_id":1,"port":3000,"host":host})
            )
            .is_err());
        }
        for path in [
            "https://example.com/",
            "//example.com/",
            "/\\example.com/",
            "/\n",
        ] {
            assert!(parse(
                "browser_open",
                &json!({"process_id":1,"port":3000,"path":path})
            )
            .is_err());
        }
        assert!(parse(
            "browser_open",
            &json!({"process_id":1,"port":3000,"origins":["http://example.com"]})
        )
        .is_err());
        let id = Uuid::new_v4();
        for selector in [
            json!({"kind":"css","value":"body"}),
            json!({"kind":"role","role":"button","name":"Login","script":"evil"}),
        ] {
            assert!(parse(
                "browser_click",
                &json!({"browser_id":id,"selector":selector})
            )
            .is_err());
        }
        assert!(parse("browser_type", &json!({"browser_id":id,"selector":{"kind":"label","value":"Name"},"text":"x".repeat(4097)})).is_err());
        for name in TOOL_NAMES {
            assert!(tool_spec(name).is_some());
        }
        assert!(tool_spec("browser_evaluate").is_none());
    }
}
