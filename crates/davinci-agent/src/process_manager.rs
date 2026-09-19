//! Authorized session adapter for JobBook's supervised ownership primitives.
mod command;
pub(crate) use command::direct as resolve_native_executable;
mod schemas;
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

impl ProcessManager {
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
