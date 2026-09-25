//! Host-owned, lazy, bounded language-server sessions.
use super::config::LanguageIntelligenceConfig;
use super::identity::{LanguageFamily, ResolvedProject, SessionKey};
use super::metadata::{FsMetadataReader, ResolutionContext};
use super::protocol::{IntelligenceError, RequestBudget, Result};
use super::session::Session;
use super::{documents, normalize, servers, tools};
use davinci_agent::{PermissionState, ToolError, ToolResult};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Slot {
    session: Option<Session>,
    starts: usize,
    last_error: Option<IntelligenceError>,
    healthy_since: Option<Instant>,
    last_used: Instant,
    generation: u64,
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            session: None,
            starts: 0,
            last_error: None,
            healthy_since: None,
            last_used: Instant::now(),
            generation: 0,
        }
    }
}
impl Slot {
    fn refresh_budget(&mut self) {
        if self
            .healthy_since
            .is_some_and(|since| since.elapsed() >= Duration::from_secs(300))
        {
            self.starts = 0;
            self.healthy_since = Some(Instant::now());
        }
    }
}

#[derive(Debug, Clone)]
struct NegativeDiscovery {
    error: IntelligenceError,
    expires: Instant,
}

#[derive(Debug)]
struct Manager {
    workspace: PathBuf,
    config: LanguageIntelligenceConfig,
    slots: Arc<Mutex<BTreeMap<SessionKey, Arc<Mutex<Slot>>>>>,
    negative: Arc<Mutex<BTreeMap<SessionKey, NegativeDiscovery>>>,
    closed: Arc<AtomicBool>,
    permissions: Mutex<Option<Arc<PermissionState>>>,
    governor: Mutex<Option<crate::native_extensions::SharedTokenGovernor>>,
}

#[derive(Debug, Clone)]
pub struct LanguageIntelligence {
    inner: Arc<Manager>,
}

impl Default for LanguageIntelligence {
    fn default() -> Self {
        Self::new(Path::new("."), LanguageIntelligenceConfig::default())
    }
}

#[derive(Debug, Clone, Copy)]
struct Limits {
    warm_ms: u64,
    init_ms: u64,
    cold_ms: u64,
    references: usize,
    workspace_symbols: usize,
    diagnostics: usize,
    family_sessions: usize,
}

impl LanguageIntelligence {
    pub fn new(workspace: &Path, config: LanguageIntelligenceConfig) -> Self {
        Self {
            inner: Arc::new(Manager {
                workspace: workspace.into(),
                config,
                slots: Arc::new(Mutex::new(BTreeMap::new())),
                negative: Arc::new(Mutex::new(BTreeMap::new())),
                closed: Arc::new(AtomicBool::new(false)),
                permissions: Mutex::new(None),
                governor: Mutex::new(None),
            }),
        }
    }

    pub fn set_permissions(&self, permissions: Option<Arc<PermissionState>>) {
        *self.inner.permissions.lock().unwrap_or_else(|e| e.into_inner()) = permissions;
    }

    /// A graph worker is rebound by its authenticated parent. Session storage is
    /// shared, but the workspace remains part of every key.
    pub fn for_workspace(&self, workspace: &Path) -> Self {
        Self {
            inner: Arc::new(Manager {
                workspace: workspace.into(),
                config: self.inner.config.clone(),
                slots: self.inner.slots.clone(),
                negative: self.inner.negative.clone(),
                closed: self.inner.closed.clone(),
                permissions: Mutex::new(
                    self.inner.permissions.lock().unwrap_or_else(|e| e.into_inner()).clone(),
                ),
                governor: Mutex::new(
                    self.inner.governor.lock().unwrap_or_else(|e| e.into_inner()).clone(),
                ),
            }),
        }
    }

    pub fn set_governor(&self, governor: crate::native_extensions::SharedTokenGovernor) {
        *self.inner.governor.lock().unwrap_or_else(|e| e.into_inner()) = Some(governor);
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.inner.config.typescript.request_timeout_ms.clamp(100, 30_000))
    }

    pub fn shutdown(&self) {
        let slots = {
            let mut slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
            self.inner.closed.store(true, Ordering::Release);
            std::mem::take(&mut *slots)
        };
        self.inner.negative.lock().unwrap_or_else(|e| e.into_inner()).clear();
        for slot in slots.into_values() {
            slot.lock().unwrap_or_else(|e| e.into_inner()).session.take();
        }
    }

    pub fn status(&self) -> Value {
        let mut sessions = Vec::new();
        {
            let slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
            for (key, slot) in slots.iter() {
                let Ok(slot) = slot.try_lock() else {
                    sessions.push(json!({
                        "workspace":key.workspace,
                        "project":key.project_root,
                        "language":key.family,
                        "session":"busy"
                    }));
                    continue;
                };
                if slot.session.is_none() && slot.last_error.is_none() {
                    continue;
                }
                let mut status = slot.session.as_ref().map(Session::status).unwrap_or_else(|| {
                    json!({
                        "workspace":key.workspace,
                        "project":key.project_root,
                        "language":key.family,
                        "session":"unavailable"
                    })
                });
                status["generation"] = json!(slot.generation);
                status["starts"] = json!(slot.starts);
                status["lastError"] = json!(slot.last_error);
                status["idleMs"] = json!(slot.last_used.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
                sessions.push(status);
            }
        }
        let negative = {
            let mut negative = self.inner.negative.lock().unwrap_or_else(|e| e.into_inner());
            negative.retain(|_, entry| entry.expires > Instant::now());
            negative.len()
        };
        json!({
            "enabled":self.inner.config.enabled,
            "closed":self.inner.closed.load(Ordering::Acquire),
            "configurationError":self.inner.config.configuration_error,
            "profileErrors":self.inner.config.profile_errors,
            "languages":["TypeScript","JavaScript","Rust","Python"],
            "backend":self.inner.config.typescript.backend,
            "profiles":[
                {"language":"typescript","enabled":self.inner.config.typescript.enabled,"backend":self.inner.config.typescript.backend,"maxSessions":self.inner.config.typescript.max_sessions},
                {"language":"rust","enabled":self.inner.config.rust.enabled,"backend":self.inner.config.rust.backend,"profile":self.inner.config.rust.profile,"maxSessions":self.inner.config.rust.max_sessions},
                {"language":"python","enabled":self.inner.config.python.enabled,"backend":self.inner.config.python.backend,"diagnosticMode":self.inner.config.python.diagnostic_mode,"maxSessions":self.inner.config.python.max_sessions}
            ],
            "maxSessions":self.inner.config.max_sessions,
            "negativeDiscoveryEntries":negative,
            "sessions":sessions,
            "startup":"lazy; discovery and status never execute a server or version probe",
            "verification":"Diagnostics are advisory; run repository compiler/type checker, lint and tests."
        })
    }

    fn request(&self, name: &str, args: &Value) -> Result<Value> {
        let parsed = tools::Arguments::parse(name, args)?;
        let family = family_from_arguments(&parsed)?;
        let limits = self.limits(family);
        let budget = RequestBudget::from_timeout(Duration::from_millis(limits.cold_ms));
        self.request_with_budget(name, args, parsed, family, budget)
    }

    fn request_with_budget(
        &self,
        name: &str,
        args: &Value,
        parsed: tools::Arguments,
        family: LanguageFamily,
        mut budget: RequestBudget,
    ) -> Result<Value> {
        self.ensure_open()?;
        if let Some(message) = &self.inner.config.configuration_error {
            return Err(IntelligenceError::new("invalid_settings", message));
        }
        if !self.inner.config.enabled || !self.family_enabled(family) {
            return Err(IntelligenceError::new("disabled", "Language intelligence is disabled for this language"));
        }
        budget.check()?;
        let workspace = self.inner.workspace.canonicalize().map_err(|_| {
            IntelligenceError::new("invalid_source_path", "Workspace is unavailable")
        })?;

        // Authorization is obtained before project metadata discovery.
        let permissions = lock_until(&self.inner.permissions, &budget)?
            .clone()
            .ok_or_else(launch_denied)?;
        {
            let policy = permissions.lock().map_err(|_| launch_denied())?;
            if !policy.project_trusted {
                return Err(launch_denied());
            }
            if !matches!(
                policy.decide("language-intelligence", name, args, &workspace),
                davinci_agent::PermissionVerdict::Allow
            ) {
                return Err(IntelligenceError::new(
                    "permission_denied",
                    "Semantic request is not allowed by current permissions",
                ));
            }
        }

        let source = parsed.path.as_deref().map(|path| documents::source_path(&workspace, path)).transpose()?;
        if let Some(source) = &source {
            let source_family = LanguageFamily::from_path(source).ok_or_else(|| {
                IntelligenceError::new("unsupported_language", "No language-intelligence adapter supports this source file")
            })?;
            if source_family != family {
                return Err(IntelligenceError::new("language_path_conflict", "Explicit language conflicts with the source path"));
            }
        }
        let context = self.resolution_context(workspace.clone(), budget.clone());
        let adapter = servers::adapter_for(family);
        let project = if let Some(source) = &source {
            adapter.resolve_project(&context, source, &self.inner.config)?
        } else {
            self.pathless_project(family, &context)?
        };
        budget.check()?;

        let profile_fingerprint = self.profile_fingerprint(family);
        let negative_key = SessionKey {
            workspace: workspace.clone(),
            project_root: project.root.clone(),
            family,
            profile_fingerprint: profile_fingerprint.clone(),
        };
        if let Some(error) = self.cached_negative(&negative_key) {
            return Err(error);
        }

        let search_path = std::env::var_os("PATH").unwrap_or_default();
        let commands = match adapter.discover(&context, &project, &self.inner.config, &search_path) {
            Ok(commands) => commands,
            Err(error) => {
                self.remember_negative(negative_key, error.clone());
                return Err(error);
            }
        };
        let limits = self.limits(family);
        let mut last_error = None;

        for command in commands {
            budget.check()?;
            let key = SessionKey {
                workspace: workspace.clone(),
                project_root: project.root.clone(),
                family,
                profile_fingerprint: format!(
                    "{}:{}",
                    command.profile_fingerprint,
                    command.invocation.executable_fingerprint
                ),
            };
            let (slot, evicted) = self.slot_for(&key, limits.family_sessions, &budget)?;
            drop(evicted);
            let mut slot = lock_until(&slot, &budget)?;
            self.ensure_open()?;
            slot.last_used = Instant::now();

            if slot.session.as_ref().is_some_and(|session| !session.is_alive()) {
                // Record healthy elapsed time before clearing process state.
                slot.refresh_budget();
                slot.session = None;
                slot.healthy_since = None;
            } else {
                slot.refresh_budget();
            }

            let warm = slot.session.is_some();
            if !warm {
                if slot.starts >= 2 {
                    last_error = Some(slot.last_error.clone().unwrap_or_else(|| {
                        IntelligenceError::new("server_exited", "Language-server restart budget exhausted for this profile")
                    }));
                    continue;
                }
                let rendered = crate::semantic::render_command_for_policy(
                    &crate::semantic::LanguageServerSpec {
                        program: command.program.to_string_lossy().into_owned(),
                        args: command.args.clone(),
                        language: family.as_str().into(),
                    },
                );
                let policy = permissions.lock().map_err(|_| launch_denied())?;
                if !policy.project_trusted
                    || !matches!(
                        policy.decide(
                            "language-server-launch",
                            "bash",
                            &json!({
                                "command":rendered,
                                "language":family.as_str(),
                                "analysisInterpreter":command.analysis_environment
                            }),
                            &project.root,
                        ),
                        davinci_agent::PermissionVerdict::Allow
                    )
                {
                    return Err(IntelligenceError::new(
                        "server_launch_denied",
                        &format!("Project trust and exact launch permission are required for: {rendered}"),
                    ));
                }
                drop(policy);
                slot.starts += 1;
                let init_deadline = budget
                    .deadline
                    .min(Instant::now() + Duration::from_millis(limits.init_ms));
                let init_budget = RequestBudget {
                    deadline: init_deadline,
                    cancelled: budget.cancelled.clone(),
                };
                match Session::start_with_budget(command, init_budget) {
                    Ok(session) => {
                        slot.generation = slot.generation.saturating_add(1);
                        slot.session = Some(session);
                        slot.healthy_since = Some(Instant::now());
                        slot.last_error = None;
                    }
                    Err(error) => {
                        slot.last_error = Some(error.clone());
                        last_error = Some(error);
                        continue;
                    }
                }
            }

            let operation_budget = if warm {
                RequestBudget {
                    deadline: budget
                        .deadline
                        .min(Instant::now() + Duration::from_millis(limits.warm_ms)),
                    cancelled: budget.cancelled.clone(),
                }
            } else {
                budget.clone()
            };
            let (method, capability) = tools::operation(name).expect("validated tool");
            let generation = slot.generation;
            let Some(session) = slot.session.as_mut() else { continue; };
            let response = session.execute_with_budget(
                method,
                capability,
                source.as_deref(),
                parsed.params(name),
                adapter.as_ref(),
                &operation_budget,
            );
            slot.last_used = Instant::now();
            let mut raw = match response {
                Ok(raw) => raw,
                Err(error) => {
                    slot.last_error = Some(error.clone());
                    if error.code == "resync_required"
                        || !slot.session.as_ref().is_some_and(Session::is_alive)
                    {
                        slot.refresh_budget();
                        slot.session = None;
                        slot.healthy_since = None;
                    }
                    if error.code == "resync_required" {
                        return Err(IntelligenceError::new(
                            "stale_result",
                            "Source synchronization became uncertain; the session was discarded and the query should be retried within a fresh budget",
                        ));
                    }
                    return Err(error);
                }
            };

            let mut freshness = None;
            let mut omitted = 0;
            let mut document_version = None;
            if name == "lsp_diagnostics" && raw.is_object() {
                freshness = raw.get("freshness").cloned();
                omitted = raw["omitted"].as_u64().unwrap_or(0);
                document_version = raw.get("documentVersion").cloned();
                raw = raw["items"].take();
            }
            if name == "lsp_diagnostics" {
                if let Some(severity) = parsed.severity.as_deref().filter(|value| *value != "all") {
                    let number = match severity {
                        "error" => 1,
                        "warning" => 2,
                        "information" => 3,
                        _ => 4,
                    };
                    if let Some(items) = raw.as_array_mut() {
                        items.retain(|item| item["severity"] == number);
                    }
                }
            }
            let cap = match name {
                "lsp_workspace_symbols" => limits.workspace_symbols,
                "lsp_diagnostics" => limits.diagnostics,
                _ => limits.references,
            };
            let uri = source.as_deref().map(documents::file_uri).transpose()?;
            let mut normalized = normalize::normalize_retained(
                name,
                raw,
                &workspace,
                uri.as_deref(),
                parsed.limit.unwrap_or(cap).min(cap),
                |full| {
                    let governor = self
                        .inner
                        .governor
                        .lock()
                        .ok()?
                        .as_ref()?
                        .clone();
                    governor
                        .lock()
                        .ok()?
                        .retain_native_output(name, args, full)
                        .ok()
                },
            )?;
            normalized["available"] = json!(true);
            normalized["language"] = json!(family);
            normalized["backend"] = json!(session.command.kind);
            normalized["generation"] = json!(generation);
            normalized["project"] = json!(relative_project(&workspace, &project.root));
            normalized["profileFingerprint"] = json!(session.command.profile_fingerprint);
            normalized["limitations"] = json!(session.command.limitations);
            if let Some(source) = &source {
                normalized["sourceHash"] = json!(source_hash(source));
            }
            if let Some(version) = document_version {
                normalized["documentVersion"] = version;
            }
            if name == "lsp_diagnostics" {
                normalized["freshness"] = freshness.unwrap_or(json!("diagnostics_pending"));
                normalized["omittedByTransport"] = json!(omitted);
                normalized["advisory"] = json!(true);
                normalized["workspaceCoverage"] = json!("partial");
            }
            return Ok(normalized);
        }
        Err(last_error.unwrap_or_else(|| IntelligenceError::new("server_not_installed", "No eligible installed language server could be started")))
    }

    pub fn execute_with_budget(
        &self,
        name: &str,
        args: &Value,
        budget: RequestBudget,
    ) -> std::result::Result<ToolResult, ToolError> {
        let (value, is_error) = match tools::Arguments::parse(name, args)
            .and_then(|parsed| {
                let family = family_from_arguments(&parsed)?;
                self.request_with_budget(name, args, parsed, family, budget)
            }) {
            Ok(value) => (value, false),
            Err(error) => (
                json!({
                    "available":false,
                    "error":error,
                    "fallback":"Use read/search and the repository compiler/type checker, lint and tests."
                }),
                true,
            ),
        };
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&value).expect("JSON value"),
            is_error,
            details: Some(value),
        })
    }

    pub fn execute(&self, name: &str, args: &Value) -> std::result::Result<ToolResult, ToolError> {
        let parsed = tools::Arguments::parse(name, args);
        let timeout = parsed
            .as_ref()
            .ok()
            .and_then(|parsed| family_from_arguments(parsed).ok())
            .map(|family| self.limits(family).cold_ms)
            .unwrap_or(60_000);
        self.execute_with_budget(name, args, RequestBudget::from_timeout(Duration::from_millis(timeout)))
    }

    fn ensure_open(&self) -> Result<()> {
        if self.inner.closed.load(Ordering::Acquire) {
            Err(IntelligenceError::new("session_closed", "Language intelligence has shut down"))
        } else {
            Ok(())
        }
    }

    fn family_enabled(&self, family: LanguageFamily) -> bool {
        match family {
            LanguageFamily::TypeScript => self.inner.config.typescript.enabled,
            LanguageFamily::Rust => self.inner.config.rust.enabled,
            LanguageFamily::Python => self.inner.config.python.enabled,
        }
    }

    fn limits(&self, family: LanguageFamily) -> Limits {
        match family {
            LanguageFamily::TypeScript => {
                let profile = &self.inner.config.typescript;
                Limits { warm_ms: profile.request_timeout_ms, init_ms: profile.initialization_timeout_ms, cold_ms: profile.cold_request_timeout_ms, references: profile.max_references, workspace_symbols: profile.max_workspace_symbols, diagnostics: profile.max_diagnostics, family_sessions: profile.max_sessions }
            }
            LanguageFamily::Rust => {
                let profile = &self.inner.config.rust;
                Limits { warm_ms: profile.request_timeout_ms, init_ms: profile.initialization_timeout_ms, cold_ms: profile.cold_request_timeout_ms, references: profile.max_references, workspace_symbols: profile.max_workspace_symbols, diagnostics: profile.max_diagnostics, family_sessions: profile.max_sessions }
            }
            LanguageFamily::Python => {
                let profile = &self.inner.config.python;
                Limits { warm_ms: profile.request_timeout_ms, init_ms: profile.initialization_timeout_ms, cold_ms: profile.cold_request_timeout_ms, references: profile.max_references, workspace_symbols: profile.max_workspace_symbols, diagnostics: profile.max_diagnostics, family_sessions: profile.max_sessions }
            }
        }
    }

    fn profile_fingerprint(&self, family: LanguageFamily) -> String {
        match family {
            LanguageFamily::TypeScript => self.inner.config.profile_fingerprint(&self.inner.config.typescript),
            LanguageFamily::Rust => self.inner.config.profile_fingerprint(&self.inner.config.rust),
            LanguageFamily::Python => self.inner.config.profile_fingerprint(&self.inner.config.python),
        }
    }

    fn resolution_context(&self, workspace: PathBuf, budget: RequestBudget) -> ResolutionContext {
        let mut roots: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .filter(|path| path.is_absolute())
            .filter_map(|path| path.canonicalize().ok())
            .collect();
        for path in [
            self.inner.config.typescript.server.as_ref().map(|v| v.program.as_path()),
            self.inner.config.rust.server.as_ref().map(|v| v.program.as_path()),
            self.inner.config.python.server.as_ref().map(|v| v.program.as_path()),
            self.inner.config.python.interpreter.as_deref(),
            self.inner.config.rust.toolchain_dir.as_deref(),
            self.inner.config.rust.sysroot.as_deref(),
            self.inner.config.rust.sysroot_src.as_deref(),
        ].into_iter().flatten() {
            let candidate = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
            if let Ok(candidate) = candidate.canonicalize() {
                roots.push(candidate);
            }
        }
        roots.sort();
        roots.dedup();
        ResolutionContext {
            workspace: workspace.clone(),
            reader: Arc::new(FsMetadataReader::new(workspace, roots)),
            budget,
        }
    }

    fn pathless_project(&self, family: LanguageFamily, context: &ResolutionContext) -> Result<ResolvedProject> {
        let explicit = match family {
            LanguageFamily::TypeScript => &[][..],
            LanguageFamily::Rust => self.inner.config.rust.project_roots.as_slice(),
            LanguageFamily::Python => self.inner.config.python.project_roots.as_slice(),
        };
        if explicit.len() == 1 {
            let root = context.workspace.join(&explicit[0]).canonicalize().map_err(|_| {
                IntelligenceError::new("project_root_required", "Configured project root is unavailable")
            })?;
            if !root.starts_with(&context.workspace) {
                return Err(IntelligenceError::new("outside_workspace", "Configured project root escapes the workspace"));
            }
            return Ok(ResolvedProject { workspace: context.workspace.clone(), root, family, analysis_environment: None, config_files: Vec::new(), limitations: Vec::new() });
        }
        let unambiguous = match family {
            LanguageFamily::TypeScript => true,
            LanguageFamily::Rust => context.workspace.join("Cargo.toml").is_file(),
            LanguageFamily::Python => context.workspace.join("pyrightconfig.json").is_file() || context.workspace.join("pyproject.toml").is_file(),
        };
        if !unambiguous {
            return Err(IntelligenceError::new("project_root_required", "A source path or one explicit project root is required for this workspace-symbol query"));
        }
        Ok(ResolvedProject { workspace: context.workspace.clone(), root: context.workspace.clone(), family, analysis_environment: None, config_files: Vec::new(), limitations: Vec::new() })
    }

    fn slot_for(
        &self,
        key: &SessionKey,
        family_cap: usize,
        budget: &RequestBudget,
    ) -> Result<(Arc<Mutex<Slot>>, Vec<Arc<Mutex<Slot>>>)> {
        let mut slots = lock_until(&self.inner.slots, budget)?;
        if let Some(slot) = slots.get(key) {
            return Ok((slot.clone(), Vec::new()));
        }
        let mut evicted = Vec::new();
        while live_count(&slots) >= self.inner.config.max_sessions
            || live_family_count(&slots, key.family) >= family_cap
        {
            let candidate = slots
                .iter()
                .filter(|(candidate, _)| {
                    if live_family_count(&slots, key.family) >= family_cap {
                        candidate.family == key.family
                    } else {
                        true
                    }
                })
                .filter_map(|(candidate, slot)| {
                    let guard = slot.try_lock().ok()?;
                    if guard.session.is_none() {
                        return Some((candidate.clone(), guard.last_used));
                    }
                    Some((candidate.clone(), guard.last_used))
                })
                .min_by_key(|(_, last_used)| *last_used)
                .map(|(candidate, _)| candidate);
            let Some(candidate) = candidate else {
                return Err(IntelligenceError::new("session_limit", "All language-server session slots are busy"));
            };
            if let Some(slot) = slots.remove(&candidate) {
                evicted.push(slot);
            }
        }
        let slot = Arc::new(Mutex::new(Slot::default()));
        slots.insert(key.clone(), slot.clone());
        Ok((slot, evicted))
    }

    fn remember_negative(&self, key: SessionKey, error: IntelligenceError) {
        let mut negative = self.inner.negative.lock().unwrap_or_else(|e| e.into_inner());
        negative.retain(|_, entry| entry.expires > Instant::now());
        if negative.len() >= 32 && !negative.contains_key(&key) {
            if let Some(oldest) = negative.keys().next().cloned() {
                negative.remove(&oldest);
            }
        }
        negative.insert(key, NegativeDiscovery { error, expires: Instant::now() + Duration::from_secs(30) });
    }

    fn cached_negative(&self, key: &SessionKey) -> Option<IntelligenceError> {
        let mut negative = self.inner.negative.lock().unwrap_or_else(|e| e.into_inner());
        if negative.get(key).is_some_and(|entry| entry.expires <= Instant::now()) {
            negative.remove(key);
        }
        negative.get(key).map(|entry| entry.error.clone())
    }
}

fn family_from_arguments(parsed: &tools::Arguments) -> Result<LanguageFamily> {
    if let Some(path) = parsed.path.as_deref() {
        let family = LanguageFamily::from_path(Path::new(path)).ok_or_else(|| {
            IntelligenceError::new("unsupported_language", "Source extension is not supported by language intelligence")
        })?;
        if let Some(language) = parsed.language.as_deref() {
            let explicit = family_from_selector(language)?;
            if explicit != family {
                return Err(IntelligenceError::new("language_path_conflict", "Explicit language conflicts with the source path"));
            }
        }
        return Ok(family);
    }
    parsed.language.as_deref().map(family_from_selector).transpose()?.map_or(Ok(LanguageFamily::TypeScript), Ok)
}

fn family_from_selector(language: &str) -> Result<LanguageFamily> {
    match language {
        "typescript" | "javascript" => Ok(LanguageFamily::TypeScript),
        "rust" => Ok(LanguageFamily::Rust),
        "python" => Ok(LanguageFamily::Python),
        _ => Err(IntelligenceError::new("invalid_arguments", "Unknown language selector")),
    }
}

fn relative_project(workspace: &Path, project: &Path) -> String {
    project
        .strip_prefix(workspace)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| ".".into())
}

fn source_hash(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > documents::MAX_SOURCE_BYTES {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Some(format!("{:x}", hasher.finalize()))
}

fn live_count(slots: &BTreeMap<SessionKey, Arc<Mutex<Slot>>>) -> usize {
    slots.values().filter(|slot| {
        slot.try_lock().map(|slot| slot.session.is_some()).unwrap_or(true)
    }).count()
}

fn live_family_count(slots: &BTreeMap<SessionKey, Arc<Mutex<Slot>>>, family: LanguageFamily) -> usize {
    slots.iter().filter(|(key, slot)| {
        key.family == family && slot.try_lock().map(|slot| slot.session.is_some()).unwrap_or(true)
    }).count()
}

fn launch_denied() -> IntelligenceError {
    IntelligenceError::new(
        "server_launch_denied",
        "Language servers require project trust and an existing permission grant for the exact discovered command",
    )
}

impl davinci_agent::runtime::task_transport::CoordinatorToolHandler for LanguageIntelligence {
    fn handles(&self, tool: &str) -> bool {
        tools::TOOL_NAMES.contains(&tool) || tool == "retrieve_output"
    }

    fn execute(&self, tool: &str, args: &Value) -> std::result::Result<ToolResult, ToolError> {
        if tool == "retrieve_output" {
            let governor = self
                .inner
                .governor
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .cloned()
                .ok_or_else(|| ToolError::Failed("Parent output store unavailable".into()))?;
            return governor
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retrieve(args);
        }
        LanguageIntelligence::execute(self, tool, args)
    }
}

fn lock_until<'a, T>(mutex: &'a Mutex<T>, budget: &RequestBudget) -> Result<MutexGuard<'a, T>> {
    loop {
        budget.check()?;
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => {
                return Err(IntelligenceError::new("session_unavailable", "Language-intelligence state unavailable"))
            }
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::language_intelligence::test_support::TestWorkspace;

    #[test]
    fn restart_budget_recovers_after_a_healthy_period() {
        let mut slot = Slot {
            starts: 2,
            healthy_since: Some(Instant::now() - Duration::from_secs(301)),
            ..Default::default()
        };
        slot.refresh_budget();
        assert_eq!(slot.starts, 0);
    }

    #[test]
    fn same_root_rust_python_are_distinct() {
        let base = SessionKey {
            workspace: PathBuf::from("/work"),
            project_root: PathBuf::from("/work"),
            family: LanguageFamily::Rust,
            profile_fingerprint: "same".into(),
        };
        let python = SessionKey { family: LanguageFamily::Python, ..base.clone() };
        assert_ne!(base, python);
    }

    #[test]
    fn fixture_routes_rust_python_and_typescript_without_collision() {
        for (language, path) in [
            ("typescript", "a.ts"),
            ("rust", "src/lib.rs"),
            ("python", "app.py"),
        ] {
            let workspace = TestWorkspace::new(language, "capture");
            let result = workspace.manager.execute("lsp_hover", &json!({"path":path,"line":1,"column":1})).unwrap();
            assert!(!result.is_error, "{language}: {}", result.content);
            assert_eq!(result.details.unwrap()["language"], json!(LanguageFamily::from_path(Path::new(path)).unwrap()));
        }
    }

    #[test]
    fn disabled_and_untrusted_requests_never_launch() {
        let workspace = TestWorkspace::new("typescript", "capture");
        workspace.manager.set_permissions(None);
        let result = workspace.manager.execute("lsp_hover", &json!({"path":"a.ts","line":1,"column":1})).unwrap();
        assert!(result.is_error);
        assert_eq!(result.details.unwrap()["error"]["code"], "server_launch_denied");
    }

    #[test]
    fn status_is_nonexecuting_and_lists_all_profiles() {
        let workspace = TestWorkspace::new("rust", "capture");
        let status = workspace.manager.status();
        assert!(status["sessions"].as_array().unwrap().is_empty());
        assert_eq!(status["profiles"].as_array().unwrap().len(), 3);
        assert!(workspace.events().is_empty());
    }
}
