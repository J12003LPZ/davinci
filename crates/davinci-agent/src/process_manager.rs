//! Authorized session adapter for JobBook's supervised ownership primitives.
mod command;
pub(crate) use command::direct as resolve_native_executable;
mod schemas;
mod socket_owner;
pub use schemas::tool_specs;

#[cfg(test)]
mod tests;

use crate::{
    approval::DispatchPermit,
    jobs::{managed::ManagedOwner, supervisor::SupervisorCommand, JobBook},
    PermissionState, PermissionVerdict, ToolResult,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone, Debug)]
pub struct ProcessManager {
    owner: ManagedOwner,
    workspace: PathBuf,
    permissions: Arc<PermissionState>,
    profile: crate::shell_policy::ShellPolicyProfile,
    counters: Arc<crate::SharedCounters>,
    provenance: crate::jobs::managed::Provenance,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    id: u32,
    cursor: Option<u64>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    id: u32,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {}

struct Authority<'a> {
    revision: u64,
    once: Option<&'a DispatchPermit>,
}

/// Trusted per-dispatch inputs; never deserialized from model JSON.
#[derive(Clone, Copy)]
pub struct BrowserRequest<'a> {
    pub cwd: &'a Path,
    pub name: &'a str,
    pub args: &'a Value,
    pub abort: Option<&'a AtomicBool>,
    pub permit: Option<&'a DispatchPermit>,
    pub process_id: u32,
    pub port: u16,
    pub lease: Option<&'a BrowserDevServerLease>,
}

/// Immutable binding to an active P2 process lifetime and declared local port.
/// Port metadata is request provenance, not proof that this PID owns a socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserDevServerLease {
    process_id: u32,
    owner: uuid::Uuid,
    session: uuid::Uuid,
    lifetime: uuid::Uuid,
    port: u16,
    pid: u32,
    pid_birth: Option<u64>,
    liveness: crate::jobs::managed::ManagedProcessLease,
}

impl BrowserDevServerLease {
    /// Cleanup observation only; permission must still be checked per request.
    pub fn is_live(&self) -> bool {
        self.liveness.is_live()
    }

    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Requires OS evidence that the local listener belongs to the managed
    /// child or a currently verifiable descendant. Declared ports are insufficient.
    pub fn verify_listening_socket(&self) -> Result<(), String> {
        let birth = self
            .pid_birth
            .ok_or("managed process identity unavailable")?;
        if socket_owner::identity(self.pid)? != birth {
            return Err("managed process identity changed".into());
        }
        socket_owner::verify(self.pid, self.port)?;
        if socket_owner::identity(self.pid)? != birth {
            return Err("managed process identity changed".into());
        }
        Ok(())
    }
}

impl ProcessManager {
    /// Browser I/O entry point: require current OS listener ownership both
    /// before the operation and before accepting its result, in addition to
    /// this request's live permission and managed-lifetime checks.
    pub fn with_verified_browser_dev_server<T>(
        &self,
        request: BrowserRequest<'_>,
        operation: impl FnOnce(&BrowserDevServerLease) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_browser_dev_server(request, |lease| {
            lease.verify_listening_socket()?;
            let result = operation(lease)?;
            lease.verify_listening_socket()?;
            Ok(result)
        })
    }

    /// Authorize the exact browser request and check its active process binding
    /// before and after host I/O. Cached bindings confer no permission.
    pub fn with_browser_dev_server<T>(
        &self,
        request: BrowserRequest<'_>,
        operation: impl FnOnce(&BrowserDevServerLease) -> Result<T, String>,
    ) -> Result<T, String> {
        let BrowserRequest {
            cwd,
            name,
            args,
            abort,
            permit,
            process_id,
            port,
            lease,
        } = request;
        if !matches!(
            name,
            "browser_open"
                | "browser_snapshot"
                | "browser_click"
                | "browser_type"
                | "browser_select"
                | "browser_console"
                | "browser_network"
                | "browser_accessibility"
                | "browser_screenshot"
                | "browser_close"
        ) {
            return Err("unknown browser request".into());
        }
        self.owner.ensure_open()?;
        let cancelled = || abort.is_some_and(|value| value.load(Ordering::SeqCst));
        if cancelled() {
            return Err("browser request cancelled".into());
        }
        if serde_json::to_vec(args)
            .map_err(|_| "invalid browser arguments")?
            .len()
            > 64 * 1024
        {
            return Err("browser arguments exceed 64 KiB".into());
        }
        if !cwd
            .canonicalize()
            .map_err(|_| "browser cwd unavailable")?
            .starts_with(&self.workspace)
        {
            return Err("browser request is outside its workspace".into());
        }
        let authority = self.authorize(cwd, name, args, permit)?;
        let current = self.browser_binding(process_id, port)?;
        if lease.is_some_and(|expected| expected != &current) {
            return Err("browser dev-server lifetime changed".into());
        }
        self.recheck(cwd, name, args, &authority)?;
        if cancelled() {
            return Err("browser request cancelled".into());
        }
        let result = operation(&current)?;
        self.recheck(cwd, name, args, &authority)?;
        if cancelled() {
            return Err("browser request cancelled".into());
        }
        if self.browser_binding(process_id, port)? != current {
            return Err("browser dev-server lifetime changed during the request".into());
        }
        Ok(result)
    }

    fn browser_binding(&self, process_id: u32, port: u16) -> Result<BrowserDevServerLease, String> {
        let (process, liveness) = self.owner.active_lease(process_id)?;
        if port == 0
            || !process
                .ports
                .iter()
                .any(|row| row["port"].as_u64() == Some(u64::from(port)))
        {
            return Err("browser port is not declared by its managed process".into());
        }
        Ok(BrowserDevServerLease {
            process_id,
            owner: process.owner,
            session: process.session,
            lifetime: process.lifetime,
            port,
            pid: process.pid,
            pid_birth: socket_owner::identity(process.pid).ok(),
            liveness,
        })
    }

    pub fn with_counters(mut self, counters: Arc<crate::SharedCounters>) -> Self {
        self.counters = counters;
        self
    }

    pub fn with_provenance(mut self, provenance: crate::jobs::managed::Provenance) -> Self {
        self.provenance = provenance;
        self
    }

    pub fn shutdown(&self) {
        self.owner.shutdown();
    }

    pub fn new_session(&self) -> Result<Self, String> {
        Ok(Self {
            owner: self.owner.new_session()?,
            ..self.clone()
        })
    }

    /// Host-only setup. No process is started by setup, schema, status or list.
    pub fn new(
        workspace: &Path,
        jobs: Arc<Mutex<JobBook>>,
        permissions: Arc<PermissionState>,
        host: SupervisorCommand,
    ) -> Result<Self, String> {
        Ok(Self {
            owner: ManagedOwner::new(workspace, jobs, host)?,
            workspace: workspace
                .canonicalize()
                .map_err(|_| "process workspace unavailable")?,
            permissions,
            profile: crate::shell_policy::ShellPolicyProfile::Permissive,
            counters: crate::SharedCounters::shared(),
            provenance: Default::default(),
        })
    }

    /// Explicit parent-owned sharing; the child's policy remains independent.
    pub fn child_lease(
        &self,
        permissions: Arc<PermissionState>,
        profile: crate::shell_policy::ShellPolicyProfile,
    ) -> Self {
        Self {
            owner: self.owner.child_lease(),
            workspace: self.workspace.clone(),
            permissions,
            profile,
            counters: self.counters.clone(),
            provenance: self.provenance.clone(),
        }
    }

    pub fn execute(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
        abort: Option<&AtomicBool>,
        permit: Option<&DispatchPermit>,
    ) -> Result<ToolResult, String> {
        let fallback_abort = AtomicBool::new(false);
        self.owner.ensure_open()?;
        let abort = abort.unwrap_or(&fallback_abort);
        if abort.load(Ordering::SeqCst) {
            return Err("process request cancelled".into());
        }
        if serde_json::to_vec(args)
            .map_err(|_| "invalid process arguments")?
            .len()
            > 64 * 1024
        {
            return Err("process arguments exceed 64 KiB".into());
        }
        if !cwd
            .canonicalize()
            .map_err(|_| "process request cwd unavailable")?
            .starts_with(&self.workspace)
        {
            return Err("process request is outside its workspace".into());
        }
        let authority = self.authorize(cwd, name, args, permit)?;
        let value = match name {
            "process_start" => {
                let request: command::Start = parse(args)?;
                let options = crate::jobs::managed::StartOptions {
                    restart: request.restart.clone(),
                    ports: request.ports.clone(),
                    counters: self.counters.clone(),
                    provenance: self.provenance.clone(),
                };
                options.validate()?;
                match crate::shell_policy::evaluate_argv(
                    self.profile,
                    &request.executable,
                    &request.argv,
                ) {
                    crate::shell_policy::ShellCommandDecision::Allowed => {}
                    crate::shell_policy::ShellCommandDecision::Denied { reason }
                    | crate::shell_policy::ShellCommandDecision::NeedsApproval { reason } => {
                        return Err(reason)
                    }
                }
                let config = command::resolve(&self.workspace, cwd, request)?;
                let permissions = self.permissions.clone();
                let restart_cwd = cwd.to_owned();
                let restart_args = args.clone();
                let revision = authority.revision;
                let restart_check = Arc::new(move || {
                    let policy = permissions.lock().unwrap_or_else(|e| e.into_inner());
                    if policy.revision() != Some(revision) {
                        return Err("process authorization changed before restart".into());
                    }
                    match policy.decide("managed-process-restart", "process_start", &restart_args, &restart_cwd) {
                        PermissionVerdict::Allow => Ok(()),
                        PermissionVerdict::Deny { reason } => Err(reason),
                        PermissionVerdict::Ask(_) => Err("automatic restart requires a continuing process_start grant; one-time approval is consumed".into()),
                    }
                });
                let (id, reused) = self.owner.start_with_options(
                    config,
                    authority.revision,
                    abort,
                    options,
                    || self.recheck(cwd, name, args, &authority),
                    restart_check,
                )?;
                json!({"process":self.owner.snapshot(id)?,"reused":reused})
            }
            "process_status" => {
                let request: Id = parse(args)?;
                json!(self.owner.snapshot(request.id)?)
            }
            "process_list" => {
                let _: List = parse(args)?;
                let rows = self
                    .owner
                    .ids()
                    .into_iter()
                    .map(|id| self.owner.snapshot(id))
                    .collect::<Result<Vec<_>, _>>()?;
                json!({"processes":rows, "remaining":0, "metrics":{
                    "startups":self.counters.process_startups.load(Ordering::Relaxed),
                    "reuses":self.counters.process_reuses.load(Ordering::Relaxed),
                    "restarts":self.counters.process_restarts.load(Ordering::Relaxed),
                }})
            }
            "process_output" => {
                let request: Output = parse(args)?;
                json!(self.owner.output(
                    request.id,
                    request.cursor,
                    request.limit.unwrap_or(8192)
                )?)
            }
            "process_write" => {
                let request: Write = parse(args)?;
                if self.profile != crate::shell_policy::ShellPolicyProfile::Permissive {
                    return Err(
                        "this role cannot write arbitrary stdin to managed processes".into(),
                    );
                }
                if request.text.len() > 16 * 1024 {
                    return Err("process stdin is limited to 16 KiB per call".into());
                }
                self.recheck(cwd, name, args, &authority)?;
                json!({"id":request.id, "written_bytes":self.owner.write(request.id, request.text.as_bytes())?})
            }
            "process_stop" => {
                let request: Id = parse(args)?;
                self.recheck(cwd, name, args, &authority)?;
                self.owner.release(request.id)?;
                json!({"process":self.owner.snapshot(request.id)?,"lease_released":true})
            }
            _ => return Err("unknown managed process tool".into()),
        };
        let content = if name == "process_output" {
            format!("Process {} output: earliest_cursor={}, next_cursor={}, total_bytes={}, remaining_bytes={}, truncated={}\n{}",
                args["id"], value["earliest_cursor"], value["next_cursor"], value["total_bytes"],
                value["remaining_bytes"], value["truncated"], value["text"].as_str().unwrap_or_default())
        } else {
            serde_json::to_string(&value).map_err(|_| "process result serialization failed")?
        };
        Ok(ToolResult {
            content,
            is_error: false,
            details: Some(value),
        })
    }

    fn authorize<'a>(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
        permit: Option<&'a DispatchPermit>,
    ) -> Result<Authority<'a>, String> {
        let policy = self.permissions.lock().unwrap_or_else(|e| e.into_inner());
        let revision = policy.revision().ok_or("permission revision exhausted")?;
        let once = match policy.decide("managed-process-boundary", name, args, cwd) {
            PermissionVerdict::Allow => None,
            PermissionVerdict::Deny { reason } => return Err(reason),
            PermissionVerdict::Ask(_) => {
                permit
                    .ok_or("managed process request requires current approval")?
                    .consume(&self.permissions, Some(revision), cwd, name, args)?;
                permit
            }
        };
        Ok(Authority { revision, once })
    }

    fn recheck(
        &self,
        cwd: &Path,
        name: &str,
        args: &Value,
        authority: &Authority<'_>,
    ) -> Result<(), String> {
        let policy = self.permissions.lock().unwrap_or_else(|e| e.into_inner());
        if policy.revision() != Some(authority.revision) {
            return Err("process permission changed during the request".into());
        }
        if let Some(permit) = authority.once {
            permit.recheck(&self.permissions, policy.revision(), cwd, name, args)?;
        }
        match policy.decide("managed-process-boundary", name, args, cwd) {
            PermissionVerdict::Allow => Ok(()),
            PermissionVerdict::Ask(_) if authority.once.is_some() => Ok(()),
            PermissionVerdict::Deny { reason } => Err(reason),
            PermissionVerdict::Ask(_) => {
                Err("managed process request requires current approval".into())
            }
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, String> {
    serde_json::from_value(args.clone()).map_err(|_| "invalid managed process arguments".into())
}
