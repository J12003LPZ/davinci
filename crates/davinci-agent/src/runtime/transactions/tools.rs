//! Adapter shared by ordinary built-in mutations. Trusted hosts install live authority.
use super::{
    ProposedChange, SourceSnapshot, TransactionCoordinator, TransactionOwner, TransactionSummary,
};
use crate::tools::{ToolContext, ToolError};
use std::path::{Path, PathBuf};
use std::sync::Arc;

type Check = dyn Fn(&Path) -> Result<(), String> + Send + Sync;
#[derive(Clone)]
pub struct MutationAuthority {
    source: Arc<Check>,
    git: Option<Arc<Check>>,
}
impl std::fmt::Debug for MutationAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MutationAuthority(..)")
    }
}
impl MutationAuthority {
    pub(crate) fn new(check: impl Fn(&Path) -> Result<(), String> + Send + Sync + 'static) -> Self {
        Self {
            source: Arc::new(check),
            git: None,
        }
    }
    pub fn check(&self, path: &Path) -> Result<(), String> {
        (self.source)(path)
    }

    pub(crate) fn for_dispatch(
        cwd: &Path,
        name: &str,
        args: &serde_json::Value,
        policy: Arc<crate::PermissionState>,
        contract: Arc<std::sync::Mutex<Option<crate::runtime::contracts::TaskContract>>>,
        permit: Option<Arc<crate::approval::DispatchPermit>>,
    ) -> Self {
        let cwd = cwd.to_path_buf();
        let canonical_cwd = cwd.canonicalize().ok();
        let name = name.to_owned();
        let args = args.clone();
        let targets = crate::runtime::contracts::extract_tool_targets(&name, &args);
        let consumed = std::sync::atomic::AtomicBool::new(false);
        let git_policy = policy.clone();
        let git_root = cwd.clone();
        let source = Self::new(move |path| {
            let normalized = crate::permission::normalize_lexically(
                &crate::permission::strip_verbatim_prefix(path),
            );
            let in_call = targets.iter().any(|target| {
                let target = Path::new(target);
                let absolute = if target.is_absolute() {
                    target.to_path_buf()
                } else {
                    cwd.join(target)
                };
                let lexical = crate::permission::normalize_lexically(
                    &crate::permission::strip_verbatim_prefix(&absolute),
                );
                if lexical == normalized {
                    return true;
                }
                // Only substitute the workspace prefix, never resolve a target
                // symlink into an additional authorized file.
                let Some(root) = canonical_cwd.as_ref() else {
                    return false;
                };
                if cwd.canonicalize().ok().as_ref() != Some(root) {
                    return false;
                }
                let requested = crate::permission::normalize_lexically(
                    &crate::permission::strip_verbatim_prefix(&cwd),
                );
                lexical.strip_prefix(&requested).is_ok_and(|relative| {
                    crate::permission::normalize_lexically(
                        &crate::permission::strip_verbatim_prefix(&root.join(relative)),
                    ) == normalized
                })
            });
            if !in_call {
                return Err("transaction target was not authorized by this dispatch".into());
            }
            let contract = contract.lock().map_err(|_| "task contract lock poisoned")?;
            if let Some(contract) = contract.as_ref() {
                contract
                    .check_call(&cwd, &name, &args)
                    .map_err(|e| e.to_string())?;
            }
            let current = policy
                .lock()
                .map_err(|_| "permission policy lock poisoned")?;
            // Preimages are compound reads; an explicit read denial is binding.
            if let crate::PermissionVerdict::Deny { reason } = current.decide(
                "transaction-read",
                "read",
                &serde_json::json!({"path":path}),
                &cwd,
            ) {
                return Err(reason);
            }
            match current.decide("transaction-mutation", &name, &args, &cwd) {
                crate::PermissionVerdict::Allow => Ok(()),
                crate::PermissionVerdict::Deny { reason } => Err(reason),
                crate::PermissionVerdict::Ask(_) => {
                    let permit = permit
                        .as_ref()
                        .ok_or("transaction requires current approval")?;
                    if consumed.load(std::sync::atomic::Ordering::Acquire) {
                        permit
                            .recheck(&policy, current.revision(), &cwd, &name, &args)
                            .map_err(str::to_owned)
                    } else {
                        permit
                            .consume(&policy, current.revision(), &cwd, &name, &args)
                            .map_err(str::to_owned)?;
                        consumed.store(true, std::sync::atomic::Ordering::Release);
                        Ok(())
                    }
                }
            }
        });
        Self {
            git: Some(Arc::new(move |_| {
                let current = git_policy
                    .lock()
                    .map_err(|_| "permission policy lock poisoned")?;
                let args = serde_json::json!({"id":"base-observation", "paths":[".git"], "observe_commit":true});
                match current.decide("transaction-base", "patch_status", &args, &git_root) {
                    crate::PermissionVerdict::Allow => Ok(()),
                    _ => Err("base revision requires current Git metadata read authority".into()),
                }
            })),
            ..source
        }
    }
}

/// Host-only coordinator construction using the same trusted owner/session/task
/// derivation as mutation tools. Model JSON never supplies these identity fields.
pub fn coordinator_for_context(
    cwd: &Path,
    context: &ToolContext,
) -> Result<TransactionCoordinator, ToolError> {
    ToolTransaction::new(cwd, context).map(|transaction| transaction.coordinator())
}

pub(crate) struct ToolTransaction<'a> {
    root: PathBuf,
    requested_root: PathBuf,
    coordinator: TransactionCoordinator,
    context: &'a ToolContext,
}
impl<'a> ToolTransaction<'a> {
    pub(crate) fn coordinator(&self) -> TransactionCoordinator {
        self.coordinator.clone()
    }
    pub fn new(cwd: &Path, context: &'a ToolContext) -> Result<Self, ToolError> {
        let root = super::files::root(cwd).map_err(ToolError::Failed)?;
        let owner = if let Some(runtime) = &context.runtime {
            TransactionOwner {
                agent_id: runtime.agent_id,
                parent_agent_id: runtime.parent_agent_id,
                session_id: runtime.session_id.clone(),
                task_id: context
                    .active_contract
                    .lock()
                    .map_err(|_| ToolError::Failed("task contract lock poisoned".into()))?
                    .as_ref()
                    .map(|c| c.task_id),
                graph_node: context.transaction_owner.graph_node.clone(),
            }
        } else {
            context.transaction_owner.clone()
        };
        let mut coordinator =
            TransactionCoordinator::new(&root, owner).map_err(ToolError::Failed)?;
        // Only a trusted host-attached root session can recover across agent restarts.
        coordinator.allow_session_recovery = context.runtime.as_ref().is_some_and(|runtime| {
            runtime.parent_agent_id.is_none() && runtime.session_id.is_some()
        });
        Ok(Self {
            root,
            requested_root: cwd.to_path_buf(),
            coordinator,
            context,
        })
    }
    pub fn snapshot(&self, path: &Path) -> Result<SourceSnapshot, ToolError> {
        self.authorize(path).map_err(ToolError::Failed)?;
        let path = crate::permission::strip_verbatim_prefix(path);
        let root = crate::permission::strip_verbatim_prefix(&self.root);
        let requested_root = crate::permission::strip_verbatim_prefix(&self.requested_root);
        // Preserve the host's workspace spelling (case on Windows, /var aliases
        // on macOS), without canonicalizing a target and following its symlinks.
        if super::files::root(&self.requested_root).map_err(ToolError::Failed)? != self.root {
            return Err(ToolError::Failed(
                "transaction workspace alias changed".into(),
            ));
        }
        let relative = path
            .strip_prefix(&root)
            .or_else(|_| path.strip_prefix(&requested_root))
            .map_err(|_| ToolError::Failed("transaction target is outside the workspace".into()))?;
        let relative = relative
            .to_str()
            .ok_or_else(|| ToolError::Failed("transaction path is not UTF-8".into()))?;
        self.coordinator
            .snapshot(relative)
            .map_err(ToolError::Failed)
    }
    pub fn preview(&self, changes: Vec<ProposedChange>) -> Result<TransactionSummary, ToolError> {
        for change in &changes {
            self.authorize(&self.root.join(&change.path))
                .map_err(ToolError::Failed)?;
        }
        let mut coordinator = self.coordinator.clone();
        if let Some(check) = self
            .context
            .mutation_authority
            .as_ref()
            .and_then(|authority| authority.git.as_ref())
        {
            if check(&self.root).is_ok() {
                if let Ok(revision) = coordinator.observed_revision() {
                    if check(&self.root).is_ok() {
                        coordinator = coordinator.with_base_revision(Some(revision));
                    }
                }
            }
        }
        // Optional Git observation may take time; recheck source authority
        // immediately before the coordinator captures preimages.
        for change in &changes {
            self.authorize(&self.root.join(&change.path))
                .map_err(ToolError::Failed)?;
        }
        coordinator.preview(changes).map_err(ToolError::Failed)
    }
    pub fn status(&self, id: &str, paths: &[String]) -> Result<TransactionSummary, ToolError> {
        let summary = self
            .coordinator
            .metadata_status(id)
            .map_err(ToolError::Failed)?;
        let supplied = paths
            .iter()
            .map(|p| super::files::normalize(&self.root, p))
            .collect::<Result<std::collections::BTreeSet<_>, _>>()
            .map_err(ToolError::Failed)?;
        let recorded: std::collections::BTreeSet<_> =
            summary.affected_files.iter().cloned().collect();
        if supplied != recorded || paths.len() != recorded.len() {
            return Err(ToolError::Failed(
                "transaction paths must exactly match the preview".into(),
            ));
        }
        for path in &summary.affected_files {
            self.authorize(&self.root.join(path))
                .map_err(ToolError::Failed)?;
        }
        self.coordinator.status(id).map_err(ToolError::Failed)
    }
    pub fn apply(&self, changes: Vec<ProposedChange>) -> Result<TransactionSummary, ToolError> {
        if let Some(runtime) = &self.context.runtime {
            for change in &changes {
                let target_path = self.root.join(&change.path);
                let bytes = change.bytes.as_ref().map(|b| b.len()).unwrap_or(0);
                let event = crate::runtime::RuntimeEvent::BeforeWrite {
                    path: target_path,
                    bytes,
                };
                if let Err(reason) = runtime.emit_decision(event) {
                    return Err(ToolError::Failed(format!(
                        "before-write hook blocked: {reason}"
                    )));
                }
            }
        }
        let changes_clone = changes.clone();
        let preview_res = self.preview(changes);
        let preview = match preview_res {
            Ok(p) => p,
            Err(e) => {
                if let Some(runtime) = &self.context.runtime {
                    for change in &changes_clone {
                        runtime.emit_observe(crate::runtime::RuntimeEvent::AfterWrite {
                            path: self.root.join(&change.path),
                            bytes: change.bytes.as_ref().map(|b| b.len()).unwrap_or(0),
                            is_error: true,
                        });
                    }
                }
                return Err(e);
            }
        };
        let mutate_res = self.mutate(&preview.id, false);
        if let Some(runtime) = &self.context.runtime {
            let is_error = mutate_res.is_err();
            for change in &changes_clone {
                runtime.emit_observe(crate::runtime::RuntimeEvent::AfterWrite {
                    path: self.root.join(&change.path),
                    bytes: change.bytes.as_ref().map(|b| b.len()).unwrap_or(0),
                    is_error,
                });
            }
        }
        mutate_res
    }
    pub fn observe_commit(&self, id: &str) -> Result<TransactionSummary, ToolError> {
        self.coordinator
            .observe_commit(id, &|path| self.authorize(path))
            .map_err(ToolError::Failed)
    }
    pub fn mutate(&self, id: &str, rollback: bool) -> Result<TransactionSummary, ToolError> {
        // Reserve the existing checkpoint capacity before any source mutation.
        let reports = if let Some(runtime) = &self.context.runtime {
            let reports = self
                .coordinator
                .effect_reports(id)
                .map_err(ToolError::Failed)?;
            for report in &reports {
                for bytes in [
                    report.before_bytes.as_deref(),
                    report.after_bytes.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    runtime
                        .blob_store
                        .store_blob_for_task(report.effect.task_id, bytes)
                        .map_err(|e| {
                            ToolError::Failed(format!(
                                "checkpoint unavailable before mutation: {e}"
                            ))
                        })?;
                }
            }
            Some(reports)
        } else {
            None
        };
        self.context
            .mutation_attempted
            .store(true, std::sync::atomic::Ordering::Release);
        let authority = |path: &Path| self.authorize(path);
        let result = if rollback {
            self.coordinator
                .rollback(id, &authority, self.context.abort.as_deref())
        } else {
            self.coordinator
                .apply(id, &authority, self.context.abort.as_deref())
        };
        let applied = result.map_err(|e| ToolError::Failed(format!("{e}; transaction {id}")))?;
        let reports = if rollback && reports.is_some() {
            Some(
                self.coordinator
                    .rollback_effect_reports(id)
                    .map_err(ToolError::Failed)?,
            )
        } else {
            reports
        };
        let mut handoff_error = None;
        if let (Some(runtime), Some(reports)) = (&self.context.runtime, reports) {
            match runtime.effect_ledger.write() {
                Ok(mut ledger) => {
                    for mut report in reports {
                        report.effect.owner_generation = applied.sequence;
                        if let Some(path) = std::env::var_os("PI_GRAPH_EFFECT_REPORT") {
                            if let Err(error) = super::super::effects::append_effect_report(
                                Path::new(&path),
                                &report.effect,
                                report.before_bytes.as_deref(),
                                report.after_bytes.as_deref(),
                            ) {
                                handoff_error.get_or_insert(error);
                            }
                        }
                        ledger.push(report.effect);
                    }
                }
                Err(_) => {
                    handoff_error = Some("runtime effect ledger unavailable".into());
                }
            }
        }
        if let Some(error) = handoff_error {
            return Err(ToolError::Durability(format!("Effect handoff failed after mutation; durable transaction {} retained: {error}. Reconcile before retrying", applied.id)));
        }
        Ok(applied)
    }
    fn authorize(&self, path: &Path) -> Result<(), String> {
        if self.context.is_aborted() {
            return Err("transaction cancelled".into());
        }
        if let Some(authority) = &self.context.mutation_authority {
            authority.check(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PermissionMode, PermissionPolicy, PermissionState};

    #[test]
    fn mutation_rechecks_policy_between_preflight_and_first_write() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), "before").unwrap();
        let policy = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::Edits,
        )));
        let args = serde_json::json!({"path":"a.txt","content":"after"});
        let mut context = ToolContext::default();
        let authority = MutationAuthority::for_dispatch(
            root.path(),
            "write",
            &args,
            policy.clone(),
            context.active_contract.clone(),
            None,
        );
        let count = std::sync::atomic::AtomicUsize::new(0);
        context.mutation_authority = Some(MutationAuthority::new(move |path| {
            if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 3 {
                policy.lock().unwrap().mode = PermissionMode::ReadOnly;
            }
            authority.check(path)
        }));
        assert!(crate::tools::execute_tool_with(root.path(), "write", &args, &context).is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "before"
        );
    }

    #[test]
    fn mutation_authority_never_grants_a_different_target() {
        let root = tempfile::tempdir().unwrap();
        let context = ToolContext::default();
        let authority = MutationAuthority::for_dispatch(
            root.path(),
            "write",
            &serde_json::json!({"path":"allowed.txt","content":"after"}),
            Arc::new(PermissionState::new(PermissionPolicy::new(
                PermissionMode::AlwaysApprove,
            ))),
            context.active_contract.clone(),
            None,
        );
        assert!(authority.check(&root.path().join("other.txt")).is_err());
    }

    #[test]
    fn transaction_snapshot_accepts_workspace_alias_without_following_target_links() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("Workspace");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("a.txt"), "before").unwrap();
        #[cfg(windows)]
        let alias = root.path().join("workspace");
        #[cfg(unix)]
        let alias = {
            let alias = root.path().join("alias");
            std::os::unix::fs::symlink(&real, &alias).unwrap();
            alias
        };
        let mut context = ToolContext::default();
        context.mutation_authority = Some(MutationAuthority::for_dispatch(
            &alias,
            "write",
            &serde_json::json!({"path":"a.txt","content":"after"}),
            Arc::new(PermissionState::new(PermissionPolicy::new(
                PermissionMode::AlwaysApprove,
            ))),
            context.active_contract.clone(),
            None,
        ));
        let transaction = ToolTransaction::new(&alias, &context).unwrap();
        let snapshot = transaction.snapshot(&alias.join("a.txt")).unwrap();
        assert_eq!(snapshot.bytes().unwrap(), b"before");
        transaction
            .apply(vec![snapshot.change(Some(b"after".to_vec()))])
            .unwrap();
        assert_eq!(std::fs::read(real.join("a.txt")).unwrap(), b"after");
        assert!(transaction
            .snapshot(&root.path().join("outside.txt"))
            .is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(real.join("a.txt"), real.join("link.txt")).unwrap();
            assert!(transaction.snapshot(&alias.join("link.txt")).is_err());
        }
    }
}
