from pathlib import Path


def replace(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"expected block not found in {path}")
    p.write_text(text.replace(old, new, 1), encoding="utf-8")


# 1) Host-independent path normalization.
replace(
    "crates/davinci-agent/src/permission.rs",
    '''pub fn has_windows_drive_prefix(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}
''',
    '''pub fn has_windows_drive_prefix(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_windows_unc_path(s: &str) -> bool {
    s.starts_with(r"\\\\") || s.starts_with("//")
}

/// Normalize path syntax independently from the host operating system.
/// Both slash styles are treated as separators so persisted Windows paths
/// have the same identity when evaluated on Linux/macOS runners.
pub(crate) fn normalize_portable_path_text(raw: &str) -> String {
    let trailing_separator = raw.ends_with('/') || raw.ends_with('\\\\');
    let normalized = raw.replace('\\\\', "/");
    let bytes = normalized.as_bytes();
    let (prefix, rest, absolute) = if normalized.starts_with("//") {
        ("//".to_string(), normalized.trim_start_matches('/'), true)
    } else if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let after = &normalized[2..];
        if after.starts_with('/') {
            (format!("{drive}:/"), after.trim_start_matches('/'), true)
        } else {
            (format!("{drive}:"), after, false)
        }
    } else if normalized.starts_with('/') {
        ("/".to_string(), normalized.trim_start_matches('/'), true)
    } else {
        (String::new(), normalized.as_str(), false)
    };

    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            _ => parts.push(part),
        }
    }

    let body = parts.join("/");
    let mut result = match prefix.as_str() {
        "//" => format!("//{body}"),
        "/" => format!("/{body}"),
        _ if prefix.ends_with('/') => format!("{prefix}{body}"),
        _ => format!("{prefix}{body}"),
    };
    if trailing_separator && !result.is_empty() && !result.ends_with('/') {
        result.push('/');
    }
    result
}
'''
)

replace(
    "crates/davinci-agent/src/permission.rs",
    '''    let joined = if given.is_absolute() {
        given.to_path_buf()
    } else if has_windows_drive_prefix(raw_trimmed) {
        PathBuf::from(raw_trimmed)
    } else {
''',
    '''    let joined = if given.is_absolute()
        || has_windows_drive_prefix(raw_trimmed)
        || is_windows_unc_path(raw_trimmed)
    {
        PathBuf::from(raw_trimmed)
    } else {
'''
)

start = '''pub fn normalize_lexically(path: &Path) -> PathBuf {
    let mut prefix = None;
    let mut has_root = false;
    let mut normals: Vec<std::ffi::OsString> = Vec::new();
    let mut leading_parents = 0usize;

    for component in path.components() {
        match component {
            Component::Prefix(p) => {
                prefix = Some(p.as_os_str().to_os_string());
            }
            Component::RootDir => {
                has_root = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !normals.is_empty() {
                    normals.pop();
                } else if !has_root {
                    leading_parents += 1;
                }
            }
            Component::Normal(n) => {
                normals.push(n.to_os_string());
            }
        }
    }

    let mut out = PathBuf::new();
    if let Some(p) = prefix {
        out.push(p);
    }
    if has_root {
        out.push(std::path::MAIN_SEPARATOR.to_string());
    }
    for _ in 0..leading_parents {
        out.push("..");
    }
    for n in normals {
        out.push(n);
    }
    out
}
'''
replace(
    "crates/davinci-agent/src/permission.rs",
    start,
    '''pub fn normalize_lexically(path: &Path) -> PathBuf {
    PathBuf::from(normalize_portable_path_text(&path.to_string_lossy()))
}
'''
)
replace(
    "crates/davinci-agent/src/permission.rs",
    '''        let norm = normalize_lexically(Path::new("C:\\\\a\\\\..\\\\..\\\\b"));
        assert_eq!(norm, PathBuf::from("C:\\\\b"));
''',
    '''        let norm = normalize_lexically(Path::new("C:\\\\a\\\\..\\\\..\\\\b"));
        assert_eq!(slashes(&norm), "C:/b");
'''
)

replace(
    "crates/davinci-agent/src/runtime/contracts.rs",
    "use std::path::{Component, Path};",
    "use std::path::Path;",
)
old_normalize = '''pub fn normalize_relative_path(raw: &str) -> Result<String, ContractError> {
    if raw.len() > MAX_CONTRACT_PATH_LEN {
        return Err(ContractError::PathTooLong {
            actual: raw.len(),
            limit: MAX_CONTRACT_PATH_LEN,
        });
    }

    // Reject Windows Alternate Data Streams (e.g. file.txt:stream)
    if raw.contains(':') {
        return Err(ContractError::AlternateDataStream(raw.to_string()));
    }

    let p = Path::new(raw);
    let mut normalized_parts = Vec::new();

    for component in p.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                return Err(ContractError::AbsolutePath(raw.to_string()));
            }
            Component::ParentDir => {
                if normalized_parts.pop().is_none() {
                    return Err(ContractError::PathTraversal(raw.to_string()));
                }
            }
            Component::CurDir => continue,
            Component::Normal(part) => {
                let part_str = part.to_string_lossy();
                normalized_parts.push(part_str.to_string());
            }
        }
    }

    let mut result = normalized_parts.join("/");
    if (raw.ends_with('/') || raw.ends_with('\\\\')) && !result.is_empty() {
        result.push('/');
    }
    Ok(result)
}
'''
new_normalize = '''pub fn normalize_relative_path(raw: &str) -> Result<String, ContractError> {
    if raw.len() > MAX_CONTRACT_PATH_LEN {
        return Err(ContractError::PathTooLong {
            actual: raw.len(),
            limit: MAX_CONTRACT_PATH_LEN,
        });
    }

    if raw.starts_with('/')
        || raw.starts_with('\\\\')
        || crate::permission::has_windows_drive_prefix(raw)
    {
        return Err(ContractError::AbsolutePath(raw.to_string()));
    }
    if raw.contains(':') {
        return Err(ContractError::AlternateDataStream(raw.to_string()));
    }

    let result = crate::permission::normalize_portable_path_text(raw);
    if result == ".." || result.starts_with("../") {
        return Err(ContractError::PathTraversal(raw.to_string()));
    }
    Ok(result)
}
'''
replace("crates/davinci-agent/src/runtime/contracts.rs", old_normalize, new_normalize)

# 2) One authoritative lossless-output predicate shared with the Governor.
cap_path = Path("crates/davinci-agent/src/runtime/capabilities.rs")
cap = cap_path.read_text(encoding="utf-8")
fn_start = cap.index("pub fn default_execution_policies(")
fn_end = cap.index("/// Conservative fallback used", fn_start)
new_fn = '''pub fn output_policy_for_tool(name: &str, tool_class: ToolClass) -> OutputPolicy {
    if matches!(
        name,
        "read"
            | "edit"
            | "write"
            | "notebook_edit"
            | "batch"
            | "todo"
            | "agent"
            | "retrieve_output"
            | "memory_search"
            | "graph_submit"
    ) {
        return OutputPolicy::LosslessRequired;
    }
    if matches!(
        tool_class,
        ToolClass::Read | ToolClass::Edit | ToolClass::Shell | ToolClass::Network
    ) {
        OutputPolicy::Compressible
    } else {
        OutputPolicy::Normal
    }
}

/// Return conservative execution properties for a capability at registration
/// time. Unknown names remain serial and non-replayable; output policy is
/// derived from the same lossless contract used by the Token Governor.
pub fn default_execution_policies(
    name: &str,
    _source: CapabilitySource,
    read_only: bool,
    tool_class: ToolClass,
) -> (ConcurrencyPolicy, ReplayPolicy, OutputPolicy) {
    let output_policy = output_policy_for_tool(name, tool_class);
    let (concurrency, replay) = if name == "agent" {
        (ConcurrencyPolicy::ParallelSafe, ReplayPolicy::NeverAutoReplay)
    } else if matches!(name, "web_search" | "web_fetch") {
        (
            ConcurrencyPolicy::ParallelSafe,
            ReplayPolicy::ReconcileBeforeReplay,
        )
    } else if name == "tool_search" {
        (ConcurrencyPolicy::ParallelSafe, ReplayPolicy::SafeToReplay)
    } else if matches!(
        name,
        "bash"
            | "powershell"
            | "exec_command"
            | "write_stdin"
            | "write"
            | "edit"
            | "apply_patch"
            | "notebook_edit"
            | "todo"
            | "update_plan"
            | "batch"
            | "propose_plan"
            | "ask_user_question"
            | "job_kill"
            | "agent_message"
            | "agent_stop"
            | "task_create"
            | "task_update"
    ) {
        (
            ConcurrencyPolicy::SerialBarrier,
            if matches!(name, "write" | "edit" | "apply_patch" | "notebook_edit") {
                ReplayPolicy::ReconcileBeforeReplay
            } else {
                ReplayPolicy::NeverAutoReplay
            },
        )
    } else if matches!(
        name,
        "read" | "grep" | "find" | "ls" | "mcp_read" | "job_output"
    ) || (read_only && matches!(tool_class, ToolClass::Read | ToolClass::Network))
    {
        (ConcurrencyPolicy::ParallelSafe, ReplayPolicy::SafeToReplay)
    } else {
        (ConcurrencyPolicy::SerialBarrier, ReplayPolicy::NeverAutoReplay)
    };
    (concurrency, replay, output_policy)
}

'''
cap_path.write_text(cap[:fn_start] + new_fn + cap[fn_end:], encoding="utf-8")

replace(
    "crates/davinci-agent/src/runtime/mod.rs",
    '''    builtin_capabilities, compute_schema_hash, conservative_replay_policy,
    default_declared_effects, default_execution_policies, CapabilitySource, ConcurrencyPolicy,
''',
    '''    builtin_capabilities, compute_schema_hash, conservative_replay_policy,
    default_declared_effects, default_execution_policies, output_policy_for_tool, CapabilitySource,
    ConcurrencyPolicy,
'''
)

# Governor uses the central lossless decision rather than a second list.
tg = Path("crates/davinci-coding-agent/src/native_extensions/token_governor.rs")
text = tg.read_text(encoding="utf-8")
const_start = text.index("const LOSSLESS_TOOLS: &[&str] = &[")
const_end = text.index("/// Guarantees that if any tool", const_start)
replacement = '''/// Returns whether a tool's output may be compressed by the token governor.
pub fn tool_may_be_compressed(name: &str) -> bool {
    !matches!(
        davinci_agent::runtime::output_policy_for_tool(name, davinci_agent::tool_class(name)),
        davinci_agent::OutputPolicy::LosslessRequired
    )
}

'''
text = text[:const_start] + replacement + text[const_end:]
text = text.replace(
    "        if LOSSLESS_TOOLS.contains(&name) {\n            return result;\n        }",
    "        if !tool_may_be_compressed(name) {\n            return result;\n        }",
    1,
)
tg.write_text(text, encoding="utf-8")

# 3) Durable tool-attempt ledger and restart reconciliation.
ledger = Path("crates/davinci-agent/src/tool_ledger.rs")
text = ledger.read_text(encoding="utf-8")
text = text.replace(
    "use std::collections::HashMap;\nuse std::sync::{Arc, Condvar, Mutex};",
    "use std::collections::HashMap;\nuse std::path::{Path, PathBuf};\nuse std::sync::{Arc, Condvar, Mutex};",
    1,
)
text = text.replace(
    '''    #[serde(skip, default = "default_condvar")]
    condvar: Arc<Condvar>,
}''',
    '''    #[serde(skip, default = "default_condvar")]
    condvar: Arc<Condvar>,
    #[serde(skip, default)]
    persistence_path: Option<PathBuf>,
}''',
    1,
)
text = text.replace(
    '''            record_order: Vec::new(),
            condvar: default_condvar(),
        }''',
    '''            record_order: Vec::new(),
            condvar: default_condvar(),
            persistence_path: None,
        }''',
    1,
)
text = text.replace(
    '''            record_order: Vec::new(),
            condvar: default_condvar(),
        }
    }

    pub fn records''',
    '''            record_order: Vec::new(),
            condvar: default_condvar(),
            persistence_path: None,
        }
    }

    pub fn load_bound(path: &Path, session_id: &str) -> Result<Self, String> {
        let mut ledger = if path.is_file() {
            let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
            serde_json::from_slice::<Self>(&bytes)
                .map_err(|err| format!("tool ledger is corrupt: {err}"))?
        } else {
            Self::new(session_id, session_id)
        };
        if !ledger.session_id.is_empty() && ledger.session_id != session_id {
            return Err("tool ledger belongs to a different session".into());
        }
        ledger.session_id = session_id.to_string();
        if ledger.lineage_id.is_empty() {
            ledger.lineage_id = session_id.to_string();
        }
        ledger.persistence_path = Some(path.to_path_buf());
        ledger.reconcile_after_restart();
        ledger.persist()?;
        Ok(ledger)
    }

    fn reconcile_after_restart(&mut self) {
        let mut remove = Vec::new();
        for (id, record) in &mut self.records {
            if record.outcome == AttemptOutcome::NotStarted
                && record.status == ToolExecutionStatus::Pending
            {
                remove.push(id.clone());
                continue;
            }
            if record.outcome == AttemptOutcome::StartedUnknown
                || record.status == ToolExecutionStatus::Executing
            {
                match record.replay_policy {
                    ReplayPolicy::SafeToReplay => remove.push(id.clone()),
                    ReplayPolicy::ReconcileBeforeReplay | ReplayPolicy::NeverAutoReplay => {
                        record.status = ToolExecutionStatus::Blocked;
                        record.output = Some(replay_blocked_message(
                            &record.tool_name,
                            record.replay_policy,
                        ));
                        record.is_error = true;
                    }
                }
            }
        }
        for id in &remove {
            self.records.remove(id);
        }
        self.record_order.retain(|id| self.records.contains_key(id));
    }

    pub fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(self).map_err(|err| err.to_string())?;
        std::fs::write(path, bytes).map_err(|err| err.to_string())?;
        std::fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|err| err.to_string())
    }

    pub fn records''',
    1,
)
# Persist direct legacy/test starts. Live dispatch additionally checks persistence before execution.
text = text.replace(
    '''        }
    }

    pub fn record_completion(&mut self, call_id: &str, output: &str, is_error: bool) {''',
    '''        }
        let _ = self.persist();
    }

    pub fn record_completion(&mut self, call_id: &str, output: &str, is_error: bool) {''',
    1,
)
# Persist terminal outcomes conservatively. If this write fails, the durable state remains StartedUnknown.
for marker in ["record_completion", "record_failure", "record_blocked"]:
    pass
# Insert persistence at end of completion/failure/blocked methods using stable following-method boundaries.
text = text.replace(
    '''            self.condvar.notify_all();
        }
    }

    pub fn record_failure''',
    '''            self.condvar.notify_all();
        }
        let _ = self.persist();
    }

    pub fn record_failure''',
    1,
)
text = text.replace(
    '''            self.condvar.notify_all();
        }
    }

    pub fn record_blocked''',
    '''            self.condvar.notify_all();
        }
        let _ = self.persist();
    }

    pub fn record_blocked''',
    1,
)
# The blocked method is followed by tests/another method depending current source; add persistence before records if found.
needle = '''            self.condvar.notify_all();
        }
    }

    pub fn set_state_hashes'''
if needle in text:
    text = text.replace(
        needle,
        '''            self.condvar.notify_all();
        }
        let _ = self.persist();
    }

    pub fn set_state_hashes''',
        1,
    )
# Persist reservation creation. Returning Err prevents dispatch if the durable identity cannot be written.
text = text.replace(
    '''        self.records.insert(
            call_id.to_string(),
            ToolCallRecord {''',
    '''        self.records.insert(
            call_id.to_string(),
            ToolCallRecord {''',
    1,
)
old_reserved = '''        );
        Ok(ReservationOutcome::Reserved)
    }

    /// Mark an execution as beginning'''
new_reserved = '''        );
        self.persist()
            .map_err(|err| format!("tool ledger persistence failed: {err}"))?;
        Ok(ReservationOutcome::Reserved)
    }

    /// Mark an execution as beginning'''
if old_reserved not in text:
    raise SystemExit("reserve persistence insertion point not found")
text = text.replace(old_reserved, new_reserved, 1)
ledger.write_text(text, encoding="utf-8")

# Bind/restore ledger on session load and make StartedUnknown durable before dispatch.
lib = Path("crates/davinci-agent/src/lib.rs")
text = lib.read_text(encoding="utf-8")
old = '''        let candidate = match current {
            Some(runtime) => runtime.clone(),
            None => runtime::session::restore_session_runtime(
                RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new()),
                &session,
            )
            .map_err(|error| format!("Runtime recovery required: {error}"))?,
        };
        let messages = messages_from_session(&session);'''
new = '''        let candidate = match current {
            Some(runtime) => runtime.clone(),
            None => runtime::session::restore_session_runtime(
                RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new()),
                &session,
            )
            .map_err(|error| format!("Runtime recovery required: {error}"))?,
        };
        let ledger_path = session.path.with_extension("tool-ledger.json");
        let candidate_ledger = ToolCallLedger::load_bound(&ledger_path, &session.header.id)
            .map_err(|error| format!("Runtime recovery required: {error}"))?;
        let messages = messages_from_session(&session);'''
if old not in text:
    raise SystemExit("session candidate block not found")
text = text.replace(old, new, 1)
text = text.replace(
    '''        self.session = Some(session);
        self.set_runtime(candidate);''',
    '''        self.session = Some(session);
        self.tool_ledger = Arc::new(std::sync::Mutex::new(candidate_ledger));
        self.set_runtime(candidate);''',
    1,
)
lib.write_text(text, encoding="utf-8")

turn = Path("crates/davinci-agent/src/turn.rs")
text = turn.read_text(encoding="utf-8")
text = text.replace(
    '''                crate::tool_ledger::BeginOutcome::Execute => {
                    // Ready to execute tool as leader
                }
''',
    '''                crate::tool_ledger::BeginOutcome::Execute => {
                    // Persist StartedUnknown before dispatch. If this fails,
                    // do not execute a mutation whose restart state is ambiguous.
                    if let Err(error) = ledger.persist() {
                        return crate::ToolResult {
                            content: format!("Tool ledger persistence failed before dispatch: {error}"),
                            is_error: true,
                            details: Some(serde_json::json!({ "ledger_persistence": true })),
                        };
                    }
                }
''',
    1,
)
turn.write_text(text, encoding="utf-8")

print("reliability follow-up patch applied")
