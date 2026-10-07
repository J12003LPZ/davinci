//! One accounted, durable proposal request. Generated code remains a patch draft.
use super::{admission::*, error::*, events::*, handoff::*, model::*, store::*, types::*};
use davinci_agent::Agent;
use davinci_ai::{ChatMessage, MessageContent};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::Read,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftHandoff {
    pub artifact_id: ArtifactId,
    pub revision: RevisionId,
    pub target_files: Vec<String>,
    pub operation_id: OperationId,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftState {
    request: DraftHandoff,
    request_hash: String,
    context: ArtifactRef,
    target_snapshot: ArtifactRef,
    target_hashes: BTreeMap<String, Option<String>>,
    root_identity: String,
    provider: String,
    model: String,
    effort: String,
    deadline_ms: u64,
    in_flight: bool,
    reply: Option<ArtifactRef>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    patch: String,
    verification: Vec<String>,
}
fn read_target(ctx: &AuthorizedDesignContext, relative: &str) -> DesignResult<Option<String>> {
    let path = ctx.workspace().join(relative);
    ctx.check_static_read(&path)?;
    // Missing files are legitimate add targets; every existing ancestor still
    // goes through the same no-link check as an existing source file.
    let mut existing = path.as_path();
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => {
                super::runtime::no_links(existing)?;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .ok_or_else(|| DesignError::Denied("target has no workspace parent".into()))?;
                if !existing.starts_with(ctx.workspace()) {
                    return Err(DesignError::Denied("target escaped workspace".into()));
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    match std::fs::File::open(&path) {
        Ok(file) => {
            let mut content = String::new();
            file.take(128 * 1024 + 1).read_to_string(&mut content)?;
            if content.len() > 128 * 1024 {
                return Err(DesignError::BudgetExceeded(
                    "target source exceeds 128 KiB".into(),
                ));
            }
            Ok(Some(content))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn session(agent: &mut Agent) -> DesignResult<&mut davinci_session::JsonlSession> {
    agent
        .session
        .as_mut()
        .ok_or_else(|| DesignError::MissingCapability("persistent session required".into()))
}
fn checkpoint(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    agent: &mut Agent,
    state: &DraftState,
) -> DesignResult<()> {
    store.prepare(ctx, session(agent)?, "design_handoff", &state.request)?;
    let reference = store.retain(ctx, "application/json", &serde_json::to_vec(state)?)?;
    store.publish(
        ctx,
        session(agent)?,
        OperationId::new(),
        digest(state)?,
        DesignChange::HandoffDraft {
            run_id: state.request.operation_id,
            state: reference,
        },
    )
}

pub fn draft_handoff(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    request: DraftHandoff,
    model: &mut dyn DesignModel,
    cancel: Option<Arc<AtomicBool>>,
) -> DesignResult<PreparedHandoff> {
    if agent.root_budget().is_some() {
        return draft_accounted(store, agent, workspace, request, model, cancel);
    }
    model.validate(agent)?;
    let budget = super::budget::for_operation(
        agent,
        &request.operation_id.to_string(),
        super::budget::HANDOFF_DRAFT,
    )?;
    agent
        .with_operation_budget(budget, |agent| {
            draft_accounted(store, agent, workspace, request, model, cancel)
        })
        .map_err(DesignError::MissingCapability)?
}

fn draft_accounted(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    request: DraftHandoff,
    model: &mut dyn DesignModel,
    cancel: Option<Arc<AtomicBool>>,
) -> DesignResult<PreparedHandoff> {
    model.validate(agent)?;
    if request.target_files.is_empty() || request.target_files.len() > 16 {
        return Err(DesignError::InvalidInput(
            "choose 1..16 concrete UI source files for implementation".into(),
        ));
    }
    for path in &request.target_files {
        validate_path(path)?;
        validate_candidate(
            &format!("*** Add File: {path}"),
            &["Validate target".into()],
        )?;
        if !["tsx", "jsx", "ts", "js", "css", "html"]
            .contains(&path.rsplit('.').next().unwrap_or(""))
            || path.split('/').any(|p| p.starts_with('.'))
        {
            return Err(DesignError::Denied(
                "implementation context is limited to UI source files".into(),
            ));
        }
    }
    let ctx = AuthorizedDesignContext::for_operation(
        agent,
        workspace,
        "design_handoff",
        &serde_json::to_value(&request)?,
    )?;
    let ctx = if let Some(cancel) = cancel {
        ctx.with_cancellation(cancel)
    } else {
        ctx
    };
    super::acceptance::accepted_revision(
        store,
        &ctx,
        session(agent)?,
        request.artifact_id,
        request.revision,
    )?;
    ctx.check_static_read(&ctx.workspace().join(".git"))?;
    davinci_agent::runtime::worktree::handoff_snapshot(ctx.workspace(), &request.target_files)
        .map_err(|error| DesignError::Conflict(error.to_string()))?;
    let request_hash = digest(&request)?;
    let budget = super::budget::for_operation(
        agent,
        &request.operation_id.to_string(),
        super::budget::HANDOFF_DRAFT,
    )?;
    let effort = format!("{:?}", agent.request_thinking_level());
    let previous = store
        .events(&ctx, session(agent)?)?
        .into_iter()
        .rev()
        .find_map(|event| match event.change {
            DesignChange::HandoffDraft { run_id, state } if run_id == request.operation_id => {
                Some(state)
            }
            _ => None,
        });
    let mut state = if let Some(reference) = previous {
        let state: DraftState = serde_json::from_slice(&store.read_blob(&ctx, &reference)?)?;
        if state.request_hash != request_hash
            || state.root_identity != budget.binding_identity()
            || state.provider != agent.provider
            || state.model != agent.model_id
            || state.effort != effort
        {
            return Err(DesignError::Conflict(
                "handoff resume requires the same input, route and root ledger".into(),
            ));
        }
        if state.in_flight {
            return Err(DesignError::MissingCapability(
                "previous handoff request outcome is unknown; automatic repetition is blocked"
                    .into(),
            ));
        }
        state
    } else {
        let snapshot = super::sync::extract_system(&ctx)?;
        let mut files = BTreeMap::new();
        let mut hashes = BTreeMap::new();
        let mut bytes = 0usize;
        for relative in &request.target_files {
            let target = read_target(&ctx, relative)?;
            hashes.insert(
                relative.clone(),
                target
                    .as_ref()
                    .map(|text| super::skills::byte_hash(text.as_bytes())),
            );
            let content = target.unwrap_or_default();
            bytes = bytes
                .checked_add(content.len())
                .ok_or_else(|| DesignError::BudgetExceeded("target source byte overflow".into()))?;
            if bytes > 128 * 1024 {
                return Err(DesignError::BudgetExceeded(
                    "target implementation context exceeds 128 KiB".into(),
                ));
            }
            if files.insert(relative.clone(), content).is_some() {
                return Err(DesignError::InvalidInput("duplicate target source".into()));
            }
        }
        let source =
            store.read_sources(&ctx, session(agent)?, request.artifact_id, request.revision)?;
        let context = serde_json::to_vec(
            &json!({"accepted_design_source":source,"target_source":files,"target_facts":snapshot}),
        )?;
        if context.len() > 512 * 1024 {
            return Err(DesignError::BudgetExceeded(
                "implementation context exceeds 512 KiB; choose a smaller target scope".into(),
            ));
        }
        let state = DraftState {
            request: request.clone(),
            request_hash,
            context: store.retain(&ctx, "application/json", &context)?,
            target_snapshot: store.retain(
                &ctx,
                "application/json",
                &serde_json::to_vec(&snapshot)?,
            )?,
            target_hashes: hashes,
            root_identity: budget.binding_identity(),
            provider: agent.provider.clone(),
            model: agent.model_id.clone(),
            effort,
            deadline_ms: davinci_session::now_ms().saturating_add(600_000),
            in_flight: false,
            reply: None,
        };
        checkpoint(store, &ctx, agent, &state)?;
        state
    };
    if state.reply.is_none() {
        if davinci_session::now_ms() >= state.deadline_ms {
            return Err(DesignError::BudgetExceeded(
                "handoff original deadline expired".into(),
            ));
        }
        budget
            .begin_operation(&request.operation_id.to_string(), 1, state.deadline_ms)
            .map_err(DesignError::BudgetExceeded)?;
        let dispatched = (|| -> DesignResult<()> {
            state.in_flight = true;
            checkpoint(store, &ctx, agent, &state)?;
            let prompt = "Draft an implementation patch; do not execute anything. Return only JSON {\"patch\":\"*** Begin Patch...*** End Patch\",\"verification\":[\"concrete target checks\"]}. Adapt the accepted design to the supplied target components, stack and factual content. Change only listed target_source paths. Preserve routes, existing APIs, authentication and data behavior. Never present mock APIs as production integration. Use valid Next.js RSC/client boundaries and existing dependencies; no package/config changes, deletions or moves. The host will preview this exact Codex-format patch for separate human approval. Include exact runnable build and test commands as separate verification entries, supported by the supplied repository evidence; never invent scripts or installation steps. Verification obligations must cover the actual target build/tests and primary browser flow, not the design preview. If the requested target cannot preserve these requirements, return an empty patch with a clear verification blocker.";
            let model_request = DesignModelRequest {
                messages: vec![
                    ChatMessage::text("user", prompt),
                    ChatMessage::text(
                        "user",
                        String::from_utf8(store.read_blob(&ctx, &state.context)?).map_err(
                            |_| DesignError::CorruptArtifact("implementation context UTF-8".into()),
                        )?,
                    ),
                ],
                tools: vec![],
                deadline_ms: state.deadline_ms,
                abort: Arc::new(AtomicBool::new(false)),
            };
            let result = complete_checked(agent, &ctx, model, &model_request);
            let reply = davinci_ai::assistant_to_chat(&result?.message);
            let data = serde_json::to_vec(&reply)?;
            if data.len() > 192 * 1024 {
                return Err(DesignError::BudgetExceeded(
                    "handoff model response exceeds 192 KiB".into(),
                ));
            }
            state.reply = Some(store.retain(&ctx, "application/json", &data)?);
            state.in_flight = false;
            checkpoint(store, &ctx, agent, &state)?;
            Ok(())
        })();
        budget
            .finish_operation(&request.operation_id.to_string())
            .map_err(DesignError::BudgetExceeded)?;
        dispatched?;
    }
    let reply: ChatMessage =
        serde_json::from_slice(&store.read_blob(
            &ctx,
            state.reply.as_ref().ok_or_else(|| {
                DesignError::CorruptArtifact("implementation reply missing".into())
            })?,
        )?)?;
    if reply
        .content
        .iter()
        .any(|part| !matches!(part, MessageContent::Text { .. }))
    {
        return Err(DesignError::Denied(
            "implementation drafting cannot call tools".into(),
        ));
    }
    let candidate: Candidate = serde_json::from_str(&davinci_ai::content_text(&reply.content))?;
    let targets = validate_candidate(&candidate.patch, &candidate.verification)?;
    if targets
        .iter()
        .any(|path| !request.target_files.contains(path))
    {
        return Err(DesignError::Denied(
            "implementation proposal escaped the selected target files".into(),
        ));
    }
    let snapshot: super::sync::DesignSystemSnapshot =
        serde_json::from_slice(&store.read_blob(&ctx, &state.target_snapshot)?)?;
    super::sync::validate_current(&ctx, &snapshot)?;
    for (path, expected) in &state.target_hashes {
        let actual = read_target(&ctx, path)?.map(|text| super::skills::byte_hash(text.as_bytes()));
        if actual != *expected {
            return Err(DesignError::Conflict(
                "selected target changed while drafting; prepare a new proposal".into(),
            ));
        }
    }
    prepare_handoff(
        store,
        agent,
        workspace,
        HandoffRequest {
            artifact_id: request.artifact_id,
            revision: request.revision,
            patch: candidate.patch,
            verification: candidate.verification,
            operation_id: request.operation_id,
        },
    )
}
