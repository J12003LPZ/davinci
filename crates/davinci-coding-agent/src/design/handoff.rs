//! Exact accepted-source proposals handed to the existing plan and transaction owners.
use super::{
    acceptance::accepted_revision, admission::*, error::*, events::DesignChange, records::*,
    store::*, sync::*, types::*,
};
use davinci_agent::{runtime::transactions::TransactionSummary, Agent};
use davinci_ai::MessageContent;
use davinci_session::JsonlSession;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffRequest {
    pub artifact_id: ArtifactId,
    pub revision: RevisionId,
    pub patch: String,
    pub verification: Vec<String>,
    pub operation_id: OperationId,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationProposal {
    pub request: HandoffRequest,
    pub accepted: RevisionReference,
    pub acceptance_evidence_hash: String,
    pub target_snapshot: DesignSystemSnapshot,
    pub workspace_fingerprint: String,
    pub target_files: Vec<String>,
    pub preview_transaction: String,
    pub before_hashes: BTreeMap<String, Option<String>>,
    pub proposed_hashes: BTreeMap<String, Option<String>>,
    #[serde(with = "decimal_u64")]
    pub plan_revision: u64,
    pub plan_hash: String,
    pub step_id: String,
    pub brief: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedHandoff {
    pub proposal_hash: String,
    pub proposal: ImplementationProposal,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveHandoff {
    pub proposal_id: OperationId,
    pub proposal_hash: String,
    pub operation_id: OperationId,
}

/// Scope inspection only. Parsing and applying patches remain transaction-owned.
pub fn validate_candidate(patch: &str, verification: &[String]) -> DesignResult<Vec<String>> {
    validate_text(patch, 128 * 1024, "implementation patch")?;
    if verification.is_empty() || verification.len() > 16 {
        return Err(DesignError::InvalidInput(
            "handoff needs 1..16 target verification obligations".into(),
        ));
    }
    for check in verification {
        validate_text(check, 2048, "target verification")?;
    }
    let mut paths = BTreeSet::new();
    for line in patch.lines() {
        if line.starts_with("*** Delete File:") || line.starts_with("*** Move to:") {
            return Err(DesignError::Denied(
                "design handoff preserves existing routes; review deletions separately".into(),
            ));
        }
        let Some(path) = line
            .strip_prefix("*** Add File: ")
            .or_else(|| line.strip_prefix("*** Update File: "))
        else {
            continue;
        };
        super::types::validate_path(path)?;
        let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
        if path.split('/').any(|part| part.starts_with('.'))
            || name.contains("lock")
            || name == "package.json"
            || name.contains("config")
            || name.contains("secret")
            || name.contains("credential")
            || !["tsx", "jsx", "ts", "js", "css", "html", "json"]
                .contains(&name.rsplit('.').next().unwrap_or(""))
        {
            return Err(DesignError::Denied("handoff changes only application UI source; dependencies, configuration and credentials require separate review".into()));
        }
        if !paths.insert(path.to_owned()) {
            return Err(DesignError::InvalidInput("duplicate patch target".into()));
        }
    }
    if paths.is_empty() || paths.len() > 16 {
        return Err(DesignError::BudgetExceeded(
            "handoff requires 1..16 target files".into(),
        ));
    }
    Ok(paths.into_iter().collect())
}
fn session(agent: &mut Agent) -> DesignResult<&mut JsonlSession> {
    agent
        .session
        .as_mut()
        .ok_or_else(|| DesignError::MissingCapability("persistent session required".into()))
}
fn tool(
    agent: &mut Agent,
    cwd: &Path,
    id: String,
    name: &str,
    args: Value,
) -> DesignResult<String> {
    let result = agent
        .execute_host_handoff_tool(cwd, &id, name, args)
        .map_err(DesignError::Denied)?;
    let text = result
        .content
        .iter()
        .filter_map(|content| match content {
            MessageContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if result.is_error == Some(true) {
        return Err(DesignError::Denied(text));
    }
    Ok(text)
}
fn plan_hash(agent: &Agent) -> DesignResult<String> {
    digest(
        &*agent
            .tool_context
            .living_plan
            .lock()
            .map_err(|_| DesignError::Denied("plan unavailable".into()))?,
    )
}
fn target_identity(ctx: &AuthorizedDesignContext, files: &[String]) -> DesignResult<String> {
    // Git reads are also subject to the existing read policy. The worktree owner
    // performs the Git inspection; this module never creates or switches trees.
    ctx.check_static_read(&ctx.workspace().join(".git"))?;
    let identity = davinci_agent::runtime::worktree::handoff_snapshot(ctx.workspace(), files)
        .map_err(|e| DesignError::Conflict(e.to_string()))?;
    digest(&identity)
}
pub fn load_proposal(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &JsonlSession,
    id: OperationId,
) -> DesignResult<PreparedHandoff> {
    let reference = store
        .events(ctx, session)?
        .into_iter()
        .find_map(|event| {
            if event.operation_id != id {
                return None;
            }
            match event.change {
                DesignChange::HandoffProposed { proposal } => Some(proposal),
                _ => None,
            }
        })
        .ok_or_else(|| DesignError::NotFound("implementation proposal missing".into()))?;
    Ok(PreparedHandoff {
        proposal_hash: reference.sha256.clone(),
        proposal: serde_json::from_slice(&store.read_blob(ctx, &reference)?)?,
    })
}

pub fn prepare_handoff(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    request: HandoffRequest,
) -> DesignResult<PreparedHandoff> {
    let files = validate_candidate(&request.patch, &request.verification)?;
    let args = serde_json::to_value(&request)?;
    let ctx = AuthorizedDesignContext::for_operation(agent, workspace, "design_handoff", &args)?;
    store.prepare(&ctx, session(agent)?, "design_handoff", &request)?;
    let request_hash = digest(&request)?;
    if store
        .retry(&ctx, session(agent)?, request.operation_id, &request_hash)?
        .is_some()
    {
        return load_proposal(store, &ctx, session(agent)?, request.operation_id);
    }
    let accepted = accepted_revision(
        store,
        &ctx,
        session(agent)?,
        request.artifact_id,
        request.revision,
    )?;
    let evidence = store
        .events(&ctx, session(agent)?)?
        .into_iter()
        .rev()
        .find_map(|event| match event.change {
            DesignChange::Accepted {
                revision,
                evidence_hash,
                ..
            } if revision == accepted => Some(evidence_hash),
            _ => None,
        })
        .ok_or_else(|| DesignError::Denied("acceptance evidence missing".into()))?;
    let snapshot = extract_system(&ctx)?;
    let identity = target_identity(&ctx, &files)?;
    let preview: TransactionSummary = serde_json::from_str(&tool(
        agent,
        workspace,
        format!("design-preview-{}", request.operation_id),
        "patch_preview",
        json!({"input":request.patch}),
    )?)?;
    let mut actual = preview.affected_files.clone();
    actual.sort();
    if actual != files {
        return Err(DesignError::Denied(
            "transaction target scope differs from the reviewed candidate".into(),
        ));
    }
    let brief = format!("Implement accepted design {} revision {} (source {}, acceptance evidence {}). Preserve routes, factual content, authentication and existing backend behavior. No dependency/configuration changes, installation, commits or deployment. Prototype mocks and preview captures are not production integration evidence. Review RSC/client boundaries and use existing components. Validate the actual target build/tests and primary browser flow. Until those receipts exist, implementation remains unverified.", request.artifact_id, request.revision.0, accepted.source_hash, evidence);
    let step_id = format!("design-{}", request.operation_id);
    let revision = agent
        .tool_context
        .living_plan
        .lock()
        .map_err(|_| DesignError::Denied("plan unavailable".into()))?
        .revision;
    // The proposal retains and revalidates the complete snapshot. The native
    // plan's bounded evidence index prioritizes the existing patch targets.
    let evidence_paths: Vec<_> = files
        .iter()
        .filter(|path| snapshot.files.contains_key(*path))
        .chain(snapshot.files.keys().filter(|path| !files.contains(path)))
        .take(32)
        .collect();
    let evidence_inputs: Vec<_> = evidence_paths.into_iter().map(|path| json!({"path":path,"finding":"Pinned target evidence; full source/dependency snapshot is retained in the design proposal"})).collect();
    tool(
        agent,
        workspace,
        format!("design-plan-{}", request.operation_id),
        "propose_plan",
        json!({
            "expected_revision":revision,"goal":brief,"evidence":evidence_inputs,
            "steps":[{"id":step_id,"change":"Apply the exact reviewed design patch","files":files,"why":format!("Accepted source {}; candidate patch {}", accepted.source_hash, super::skills::byte_hash(request.patch.as_bytes())),"depends_on":[],"verify":request.verification}]
        }),
    )?;
    let plan_revision = agent
        .tool_context
        .living_plan
        .lock()
        .map_err(|_| DesignError::Denied("plan unavailable".into()))?
        .revision;
    let proposal = ImplementationProposal {
        request,
        accepted,
        acceptance_evidence_hash: evidence,
        target_snapshot: snapshot,
        workspace_fingerprint: identity,
        target_files: files,
        preview_transaction: preview.id,
        before_hashes: preview.before_hashes,
        proposed_hashes: preview.proposed_hashes,
        plan_revision,
        plan_hash: plan_hash(agent)?,
        step_id,
        brief,
    };
    let bytes = serde_json::to_vec(&proposal)?;
    if bytes.len() > 220 * 1024 {
        return Err(DesignError::BudgetExceeded(
            "implementation proposal size".into(),
        ));
    }
    let reference = store.retain(&ctx, "application/json", &bytes)?;
    store.publish(
        &ctx,
        session(agent)?,
        proposal.request.operation_id,
        request_hash,
        DesignChange::HandoffProposed {
            proposal: reference.clone(),
        },
    )?;
    Ok(PreparedHandoff {
        proposal_hash: reference.sha256,
        proposal,
    })
}

/// Called only by an explicit host/user action. It never changes permission mode.
/// A successful apply returns existing transaction evidence, never `Verified`.
pub fn approve_handoff(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    request: ApproveHandoff,
) -> DesignResult<Value> {
    if agent.is_plan_mode() {
        return Err(DesignError::Denied(
            "leave Plan Mode explicitly before applying a design".into(),
        ));
    }
    validate_hash(&request.proposal_hash)?;
    let ctx = AuthorizedDesignContext::for_operation(
        agent,
        workspace,
        "design_apply",
        &serde_json::to_value(&request)?,
    )?;
    store.prepare(&ctx, session(agent)?, "design_apply", &request)?;
    let request_hash = digest(&request)?;
    if let Some(DesignChange::HandoffApplied {
        transaction_id,
        proposal_hash,
    }) = store.retry(&ctx, session(agent)?, request.operation_id, &request_hash)?
    {
        return Ok(
            json!({"transaction_id":transaction_id,"proposal_hash":proposal_hash,"implementation":"pending_verification"}),
        );
    }
    let prepared = load_proposal(store, &ctx, session(agent)?, request.proposal_id)?;
    if prepared.proposal_hash != request.proposal_hash {
        return Err(DesignError::Conflict(
            "proposal changed; review the exact proposal hash".into(),
        ));
    }
    let proposal = prepared.proposal;
    accepted_revision(
        store,
        &ctx,
        session(agent)?,
        proposal.request.artifact_id,
        proposal.request.revision,
    )?;
    validate_current(&ctx, &proposal.target_snapshot)?;
    if target_identity(&ctx, &proposal.target_files)? != proposal.workspace_fingerprint
        || plan_hash(agent)? != proposal.plan_hash
    {
        return Err(DesignError::Conflict(
            "target workspace or plan changed; prepare a fresh proposal".into(),
        ));
    }
    // Per-step acceptance preserves the existing permission mode and restricts
    // the native contract to this proposal's files and verification obligations.
    agent
        .handle_plan_command(&format!("accept {}", proposal.step_id))
        .map_err(DesignError::Denied)?;
    let preview: TransactionSummary = serde_json::from_str(&tool(
        agent,
        workspace,
        format!("design-approved-preview-{}", request.operation_id),
        "patch_preview",
        json!({"input":proposal.request.patch}),
    )?)?;
    let mut actual = preview.affected_files.clone();
    actual.sort();
    if actual != proposal.target_files
        || preview.before_hashes != proposal.before_hashes
        || preview.proposed_hashes != proposal.proposed_hashes
    {
        return Err(DesignError::Denied(
            "approved transaction scope or source changed".into(),
        ));
    }
    ctx.check("design_apply", &serde_json::to_value(&request)?)?;
    let applied: TransactionSummary = serde_json::from_str(&tool(
        agent,
        workspace,
        format!("design-apply-{}", request.operation_id),
        "patch_apply",
        json!({"id":preview.id,"paths":preview.affected_files}),
    )?)?;
    store.publish(
        &ctx,
        session(agent)?,
        request.operation_id,
        request_hash,
        DesignChange::HandoffApplied {
            proposal_hash: request.proposal_hash.clone(),
            transaction_id: applied.id.clone(),
        },
    )?;
    Ok(
        json!({"transaction":applied,"proposal_hash":request.proposal_hash,"implementation":"pending_verification","verification":proposal.request.verification}),
    )
}
