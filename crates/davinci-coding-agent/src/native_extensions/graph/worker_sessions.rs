//! Private, immutable launch bindings for recoverable worker conversations.
use super::{store, types::WorkerSpec, GraphWorkerContext};
use davinci_agent::{AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_session::JsonlSession;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[cfg(windows)]
#[path = "worker_sessions_windows.rs"]
mod windows;

pub const SESSION_ENV: &str = "DAVINCI_GRAPH_WORKER_SESSION";

#[cfg(test)]
#[path = "worker_sessions_tests.rs"]
pub(crate) mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionBinding {
    pub version: u32,
    pub graph_run: String,
    pub task_id: String,
    pub attempt: u32,
    pub graph_revision: u64,
    pub cwd: PathBuf,
    pub role: super::types::Role,
    pub expect: super::types::ArtifactKind,
    pub runtime_run: RunId,
    pub parent: AgentId,
    pub agent: AgentId,
    pub session_id: String,
    pub session_path: PathBuf,
    pub input_hash: String,
    pub authorized_tools: Vec<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub initial_tools: Vec<String>,
    pub extensions: Vec<String>,
    pub project_trusted: bool,
    pub briefing_hash: String,
    pub system_hash: String,
    pub contract_digest: Option<String>,
}

fn ordinary_directory(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("worker session directory is redirected or is not a directory".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("worker session directory is a reparse point".into());
        }
    }
    Ok(())
}

fn worker_lease_available(session_path: &Path) -> Result<bool, String> {
    let path = session_path.with_extension("worker.lock");
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("worker conversation lease is not an ordinary file".into());
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|error| error.to_string())?;
        // SAFETY: file owns this descriptor for the duration of the probe.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            // SAFETY: the successful probe owns the advisory lock.
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            Ok(false)
        } else {
            Err(error.to_string())
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("worker conversation lease is a reparse point".into());
        }
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .custom_flags(0x00200000)
            .open(&path)
        {
            Ok(_) => Ok(true),
            Err(error) if error.raw_os_error() == Some(32) => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err("worker conversation ownership is unsupported on this platform".into())
    }
}

fn private_directory(path: &Path, create: bool) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        if create {
            match std::fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        ordinary_directory(path)?;
        let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        // SAFETY: geteuid has no arguments or memory access requirements.
        if metadata.permissions().mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err("worker session directory is not private to this user".into());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        windows::private_directory(path, create)?;
        ordinary_directory(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (path, create);
        Err("private worker sessions are unsupported on this platform".into())
    }
}

fn session_root(cwd: &Path, run_id: &str, agent: AgentId, create: bool) -> Result<PathBuf, String> {
    if !store::is_safe_run_id(run_id) {
        return Err("invalid worker graph run identity".into());
    }
    let run = store::run_dir(cwd, run_id);
    let mut current = cwd.to_path_buf();
    for component in run
        .strip_prefix(cwd)
        .map_err(|error| error.to_string())?
        .components()
    {
        current.push(component);
        ordinary_directory(&current)?;
    }
    let private = run.join("workers");
    private_directory(&private, create)?;
    let attempt = private.join(agent.to_string());
    private_directory(&attempt, create)?;
    Ok(attempt)
}

fn binding_path(root: &Path, attempt: u32) -> PathBuf {
    root.join(format!("binding.attempt_{attempt}.json"))
}

fn input_hash(spec: &WorkerSpec) -> Result<String, String> {
    let identity = serde_json::json!({
        "briefing": spec.briefing, "system": spec.system_prompt,
        "role": spec.role, "expect": spec.expect,
        "model": spec.model, "thinking": spec.thinking_level,
        "authorized": spec.authorized_tools, "initial": spec.initially_exposed_tools,
        "extensions": spec.extra_extensions, "trusted": spec.project_trusted,
        "contract": spec.task_contract,
    });
    Ok(super::replay::compute_input_hash(
        &serde_json::to_string(&identity).map_err(|error| error.to_string())?,
    ))
}

impl WorkerSessionBinding {
    pub fn create(
        spec: &WorkerSpec,
        run_id: &str,
        revision: u64,
        attempt: u32,
        parent: Option<&RuntimeHandle>,
        previous: Option<&Self>,
    ) -> Result<Self, String> {
        let cwd = spec.cwd.canonicalize().map_err(|error| error.to_string())?;
        let agent = spec
            .runtime_agent_id
            .ok_or("worker agent identity is missing")?;
        let root = session_root(&cwd, run_id, agent, true)?;
        let manifest = binding_path(&root, attempt);
        if manifest.try_exists().map_err(|error| error.to_string())? {
            return Err("worker attempt identity already has a conversation binding".into());
        }
        let (session_id, session_path, runtime_run, parent_id) = match previous {
            Some(previous) => {
                previous.validate(&cwd)?;
                if previous.graph_run != run_id
                    || previous.task_id != spec.task_id
                    || previous.role != spec.role
                    || previous.expect != spec.expect
                    || previous.agent != agent
                    || previous.attempt.checked_add(1) != Some(attempt)
                {
                    return Err(
                        "worker retry does not continue the previous conversation lineage".into(),
                    );
                }
                if let Some(runtime) = parent {
                    if previous.runtime_run != runtime.run_id || previous.parent != runtime.agent_id
                    {
                        return Err("worker retry parent authority changed".into());
                    }
                }
                let session = JsonlSession::open(&previous.session_path)
                    .map_err(|error| error.to_string())?;
                (
                    session.header.id,
                    session.path,
                    previous.runtime_run,
                    previous.parent,
                )
            }
            None => {
                let session =
                    JsonlSession::create_in_directory(&root, &cwd.to_string_lossy(), None)
                        .map_err(|error| error.to_string())?;
                (
                    session.header.id,
                    session.path,
                    parent.map_or_else(RunId::new, |runtime| runtime.run_id),
                    parent.map_or_else(AgentId::new, |runtime| runtime.agent_id),
                )
            }
        };
        let binding = Self {
            version: 1,
            graph_run: run_id.into(),
            task_id: spec.task_id.clone(),
            attempt,
            graph_revision: revision,
            cwd,
            role: spec.role,
            expect: spec.expect,
            runtime_run,
            parent: parent_id,
            agent,
            session_id,
            session_path,
            input_hash: input_hash(spec)?,
            authorized_tools: spec.authorized_tools.clone(),
            model: spec.model.clone(),
            thinking: spec.thinking_level.clone(),
            initial_tools: spec.initially_exposed_tools.clone(),
            extensions: spec
                .extra_extensions
                .iter()
                .filter(|_| spec.project_trusted)
                .cloned()
                .collect(),
            project_trusted: spec.project_trusted,
            briefing_hash: super::replay::compute_input_hash(&spec.briefing),
            system_hash: super::replay::compute_input_hash(&format!(
                "{}\n\n{}",
                spec.system_prompt,
                super::validate::artifact_contract(spec.expect)
            )),
            contract_digest: spec
                .task_contract
                .as_ref()
                .map(|contract| contract.digest.clone()),
        };
        store::atomic_write(
            &manifest,
            &serde_json::to_vec_pretty(&binding).map_err(|e| e.to_string())?,
        )
        .map_err(|error| format!("worker session binding could not be saved: {error}"))?;
        Ok(binding)
    }

    pub fn validate(&self, cwd: &Path) -> Result<(), String> {
        let cwd = cwd.canonicalize().map_err(|error| error.to_string())?;
        if self.version != 1
            || self.cwd != cwd
            || self.attempt == 0
            || !store::is_safe_run_id(&self.task_id)
            || uuid::Uuid::parse_str(&self.session_id).is_err()
        {
            return Err("worker session binding has an incompatible identity".into());
        }
        let root = session_root(&cwd, &self.graph_run, self.agent, false)?;
        let expected = root.join(format!("{}.jsonl", self.session_id));
        if self.session_path != expected {
            return Err("worker conversation path does not match its attempt".into());
        }
        ordinary_directory(expected.parent().ok_or("worker session has no directory")?)?;
        for path in [binding_path(&root, self.attempt), expected.clone()] {
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err("worker session file is redirected or is not a file".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("worker session file is a reparse point".into());
                }
            }
        }
        let saved: Self = serde_json::from_slice(
            &std::fs::read(binding_path(&root, self.attempt)).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        if saved != *self {
            return Err("worker session manifest disagrees with the checkpoint".into());
        }
        let session = JsonlSession::open(&expected).map_err(|error| error.to_string())?;
        if session.header.id != self.session_id || Path::new(&session.header.cwd) != cwd {
            return Err("worker conversation header disagrees with its binding".into());
        }
        Ok(())
    }

    /// Refuse an automatic retry while another process owns this conversation
    /// or while the durable tool ledger contains a mutation whose outcome is
    /// not represented in the persisted conversation.
    pub fn validate_retry_safety(&self) -> Result<(), String> {
        self.validate(&self.cwd)?;
        if !worker_lease_available(&self.session_path)? {
            return Err("worker conversation is still owned by a live process".into());
        }
        let ledger_path = self.session_path.with_extension("tool-ledger.json");
        let metadata = match std::fs::symlink_metadata(&ledger_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("worker tool ledger is redirected or is not a file".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("worker tool ledger is a reparse point".into());
            }
        }
        let ledger = davinci_agent::tool_ledger::ToolCallLedger::load_bound(
            &ledger_path,
            &self.session_id,
        )
        .map_err(|error| format!("worker tool ledger could not be loaded: {error}"))?;
        let session = JsonlSession::open(&self.session_path).map_err(|error| error.to_string())?;
        let persisted_results: std::collections::HashSet<String> = session
            .entries
            .iter()
            .filter_map(|entry| entry.message.as_ref())
            .filter(|message| {
                message.get("role").and_then(serde_json::Value::as_str) == Some("toolResult")
            })
            .filter_map(|message| {
                message
                    .get("toolCallId")
                    .or_else(|| message.get("tool_call_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        for record in ledger.records().values() {
            use davinci_agent::tool_ledger::{AttemptOutcome, ToolExecutionStatus, ToolSideEffect};
            let uncertain = record.outcome == AttemptOutcome::StartedUnknown
                || record.status == ToolExecutionStatus::Executing;
            if uncertain
                && record.replay_policy != davinci_agent::runtime::ReplayPolicy::SafeToReplay
            {
                return Err(format!(
                    "tool '{}' may have produced an uncertain side effect",
                    record.tool_name
                ));
            }
            if record.status == ToolExecutionStatus::Blocked {
                return Err(format!(
                    "tool '{}' requires reconciliation before retry",
                    record.tool_name
                ));
            }
            if record.side_effect == ToolSideEffect::Mutating
                && matches!(
                    record.outcome,
                    AttemptOutcome::Succeeded | AttemptOutcome::Failed
                )
                && !persisted_results.contains(&record.call_id)
            {
                return Err(format!(
                    "tool '{}' completed without a persisted tool result",
                    record.tool_name
                ));
            }
        }
        Ok(())
    }

    pub fn validate_spec(&self, spec: &WorkerSpec) -> Result<(), String> {
        self.validate(&spec.cwd)?;
        if spec.runtime_agent_id != Some(self.agent)
            || spec.task_id != self.task_id
            || input_hash(spec)? != self.input_hash
        {
            return Err("worker launch inputs disagree with the saved conversation binding".into());
        }
        Ok(())
    }

    pub fn runtime(&self) -> RuntimeHandle {
        RuntimeHandle::new(self.runtime_run, self.agent, RuntimeBus::new()).with_parent(self.parent)
    }

    fn validate_args(&self, args: &crate::args::Args) -> Result<(), String> {
        if args.no_session
            || args.session.as_deref().map(Path::new) != Some(self.session_path.as_path())
            || args.session_dir.as_deref().map(Path::new) != self.session_path.parent()
            || args.session_id.is_some()
            || args.fork.is_some()
            || args.continue_session
            || args.resume
            || args.model != self.model
            || args.provider.is_some()
            || args.thinking.map(|level| level.as_str()) != self.thinking.as_deref()
            || args.tools != self.initial_tools
            || !args.exclude_tools.is_empty()
            || args.no_tools
            || args.no_builtin_tools
            || args.extensions != self.extensions
            || (args.project_trust_override == Some(true)) != self.project_trusted
            || args.permission_mode != Some(davinci_agent::PermissionMode::AlwaysApprove)
            || args.system_prompt.is_some()
            || args.append_system_prompt.len() != 1
            || args.file_args.len() != 1
            || !args.messages.is_empty()
            || !args.no_extensions
            || !args.no_skills
            || !args.no_prompt_templates
        {
            return Err("worker CLI inputs disagree with the saved conversation binding".into());
        }
        let briefing =
            std::fs::read_to_string(&args.file_args[0]).map_err(|error| error.to_string())?;
        let system = std::fs::read_to_string(&args.append_system_prompt[0])
            .map_err(|error| error.to_string())?;
        if super::replay::compute_input_hash(&briefing) != self.briefing_hash
            || super::replay::compute_input_hash(&system) != self.system_hash
        {
            return Err(
                "worker briefing or role prompt changed after checkpoint publication".into(),
            );
        }
        Ok(())
    }
}

/// Validate the parent-supplied binding before the host activates a session.
#[allow(dead_code)] // Used by the CLI host; graph is also compiled as a library.
pub fn runtime_from_env(
    args: &crate::args::Args,
    cwd: &Path,
    context: Option<&GraphWorkerContext>,
) -> Result<Option<RuntimeHandle>, String> {
    let Some(raw) = std::env::var_os(SESSION_ENV) else {
        return Ok(None);
    };
    let binding: WorkerSessionBinding =
        serde_json::from_str(raw.to_str().ok_or("invalid worker session environment")?)
            .map_err(|error| error.to_string())?;
    binding.validate(cwd)?;
    binding.validate_args(args)?;
    let context = context.ok_or("bound worker conversation requires a graph context")?;
    if context.node_id != binding.task_id
        || context.role != binding.role
        || context.expect != binding.expect
        || context
            .task_contract
            .as_ref()
            .map(|contract| &contract.digest)
            != binding.contract_digest.as_ref()
        || context.artifact_path != store::artifact_path(cwd, &binding.graph_run, &binding.task_id)
        || std::env::var("DAVINCI_AGENT_ID").ok().as_deref() != Some(&binding.agent.to_string())
        || std::env::var("PI_GRAPH_AUTHORIZED_TOOLS").ok().as_deref()
            != Some(&binding.authorized_tools.join(","))
    {
        return Err("worker context disagrees with its private conversation binding".into());
    }
    Ok(Some(binding.runtime()))
}
