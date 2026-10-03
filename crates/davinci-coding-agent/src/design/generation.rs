//! One serial, durable design run on the caller's existing agent and root ledger.
use super::{
    admission::*, context::*, error::*, events::*, model::*, records::*, store::*, types::*,
};
use davinci_agent::Agent;
use davinci_ai::{ChatMessage, MessageContent, ToolSpec};
use davinci_session::JsonlSession;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationRequest {
    pub artifact_id: ArtifactId,
    pub expected_revision: RevisionId,
    pub brief: String,
    pub operation_id: OperationId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptDirection {
    pub title: String,
    pub layout: String,
    pub typography: String,
    pub content_strategy: String,
}
pub fn validate_directions(directions: &[ConceptDirection], count: u32) -> DesignResult<()> {
    if !(1..=3).contains(&count) || directions.len() != count as usize {
        return Err(DesignError::InvalidInput(
            "concept count differs from requested variants".into(),
        ));
    }
    let mut layouts = std::collections::BTreeSet::new();
    let mut typography = std::collections::BTreeSet::new();
    for direction in directions {
        for text in [
            &direction.title,
            &direction.layout,
            &direction.typography,
            &direction.content_strategy,
        ] {
            validate_text(text, 2048, "concept direction")?;
        }
        if !["editorial", "split", "modular"].contains(&direction.layout.as_str())
            || !["serif", "sans", "mono"].contains(&direction.typography.as_str())
            || !layouts.insert(&direction.layout)
            || !typography.insert(&direction.typography)
        {
            return Err(DesignError::InvalidInput(
                "concepts must differ in layout and typography, not only palette".into(),
            ));
        }
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Directions,
    Generate,
    Render,
    Review,
    Complete,
    Blocked,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunProgress {
    pub phase: RunPhase,
    pub in_flight: bool,
    pub repairs: u32,
    #[serde(with = "decimal_u64")]
    pub deadline_ms: u64,
}
impl RunProgress {
    pub fn can_dispatch(&self, now_ms: u64) -> DesignResult<()> {
        if self.in_flight {
            return Err(DesignError::MissingCapability(
                "previous provider outcome is unknown; it will not be repeated".into(),
            ));
        }
        if now_ms >= self.deadline_ms {
            return Err(DesignError::BudgetExceeded(
                "design run exceeded its original ten-minute deadline".into(),
            ));
        }
        if matches!(self.phase, RunPhase::Complete | RunPhase::Blocked) {
            return Err(DesignError::Conflict("design run is terminal".into()));
        }
        Ok(())
    }
    pub fn begin_repair(&mut self) -> DesignResult<()> {
        if self.repairs >= 2 {
            return Err(DesignError::BudgetExceeded(
                "two repair rounds exhausted; last revision retained".into(),
            ));
        }
        self.repairs += 1;
        self.phase = RunPhase::Generate;
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignRunState {
    pub request: GenerationRequest,
    pub request_hash: String,
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub root_identity: String,
    pub context: ArtifactRef,
    pub profile: ArtifactRef,
    pub system_snapshot: Option<ArtifactRef>,
    pub directions: Vec<ConceptDirection>,
    pub progress: RunProgress,
    pub reply: Option<ArtifactRef>,
    pub last_revision: RevisionId,
    pub findings: Vec<String>,
    pub quality: Option<QualityReport>,
}
/// Trusted host injection, never constructible by a browser or generated tool call.
/// Implementations return measured evidence; they cannot publish acceptance.
pub trait DesignRenderer {
    fn render(
        &mut self,
        store: &DesignStore,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        revision: &DesignRevision,
    ) -> DesignResult<QualityReport>;
}
pub struct ConfinedRenderer {
    pub runtime: super::runtime::TrustedDesignRuntime,
}
impl DesignRenderer for ConfinedRenderer {
    fn render(
        &mut self,
        store: &DesignStore,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        revision: &DesignRevision,
    ) -> DesignResult<QualityReport> {
        let mut receipts = Vec::new();
        let mut budget = DesignLimits::default().max_render_bytes;
        for board in revision.variants.iter().flat_map(|v| &v.artboards) {
            for viewport in Viewport::defaults() {
                let request = RenderRequest {
                    artifact_id: revision.artifact_id,
                    revision: revision.revision,
                    artboard_id: board.id,
                    viewport,
                    theme: Theme::Light,
                    fixture: "default".into(),
                    reduced_motion: true,
                };
                let receipt = super::render::capture_with_budget(
                    store,
                    ctx,
                    session,
                    &self.runtime,
                    request,
                    OperationId::new(),
                    &mut budget,
                )?;
                receipts.push(receipt);
            }
        }
        super::quality::quality_report(
            revision,
            &receipts,
            self.runtime.fingerprint(),
            &super::render::policy_hash(),
        )
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedSource {
    files: BTreeMap<String, String>,
    entry_points: Vec<String>,
    variants: Vec<Variant>,
    bindings: Vec<EditableBinding>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Critique {
    findings: Vec<String>,
}

fn session(agent: &mut Agent) -> DesignResult<&mut JsonlSession> {
    agent
        .session
        .as_mut()
        .ok_or_else(|| DesignError::MissingCapability("persistent session required".into()))
}
pub fn latest_run(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    session: &JsonlSession,
    run_id: OperationId,
) -> DesignResult<Option<DesignRunState>> {
    for event in store.events(ctx, session)?.into_iter().rev() {
        if let DesignChange::RunCheckpoint { run_id: id, state } = event.change {
            if id == run_id {
                return Ok(Some(serde_json::from_slice(
                    &store.read_blob(ctx, &state)?,
                )?));
            }
        }
    }
    Ok(None)
}
fn checkpoint(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    agent: &mut Agent,
    state: &DesignRunState,
) -> DesignResult<()> {
    let data = serde_json::to_vec(state)?;
    let session = session(agent)?;
    store.prepare(ctx, session, "design_generate", &state.request)?;
    let reference = store.retain(ctx, "application/json", &data)?;
    store.publish(
        ctx,
        session,
        OperationId::new(),
        digest(state)?,
        DesignChange::RunCheckpoint {
            run_id: state.request.operation_id,
            state: reference,
        },
    )
}
fn revision_operation(state: &DesignRunState) -> DesignResult<OperationId> {
    let hash = digest(&(state.request.operation_id, state.progress.repairs, "source"))?;
    format!(
        "{}-{}-{}-{}-{}",
        &hash[..8],
        &hash[8..12],
        &hash[12..16],
        &hash[16..20],
        &hash[20..32]
    )
    .parse()
}
fn tools(phase: RunPhase, initial: bool) -> Vec<ToolSpec> {
    if phase != RunPhase::Generate {
        return vec![];
    }
    // No shell, filesystem, browser scripting, networking or ordinary coding tools.
    vec![ToolSpec { name:if initial { "design_create" } else { "design_patch" }.into(),
        description:"Publish a complete artifact-only virtual source revision. Use portable relative file names and UUID node/variant/artboard IDs. HTML or TSX entry points must consume the supplied JSON content/tokens for every editable binding. Sources must be self-contained; React/ReactDOM are the only external imports. bindings.source_hash is SHA-256 of the exact JSON text. No application edits or external assets.".into(),
        parameters:json!({"type":"object","additionalProperties":false,"properties":{
            "files":{"type":"object","additionalProperties":{"type":"string"}},
            "entry_points":{"type":"array","items":{"type":"string"}},
            "variants":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"title":{"type":"string"},"artboards":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"title":{"type":"string"},"entry_point":{"type":"string"}},"required":["id","title","entry_point"],"additionalProperties":false}}},"required":["id","title","artboards"],"additionalProperties":false}},
            "bindings":{"type":"array","items":{"type":"object"}}
        },"required":["files","entry_points","variants","bindings"]}), constrained_sampling:None }]
}
fn phase_messages(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    agent: &mut Agent,
    state: &DesignRunState,
    images: bool,
) -> DesignResult<Vec<ChatMessage>> {
    let mut messages = vec![ChatMessage::text(
        "user",
        String::from_utf8(store.read_blob(ctx, &state.context)?)
            .map_err(|_| DesignError::CorruptArtifact("context is not UTF-8".into()))?,
    )];
    let manifest = store.manifest(ctx, session(agent)?, state.request.artifact_id)?;
    match state.progress.phase {
        RunPhase::Directions => messages.push(ChatMessage::text("user", format!("Return only a JSON array of exactly {} concept directions. Each has title, layout (editorial/split/modular), typography (serif/sans/mono), content_strategy. Each direction must have a unique layout AND typography and describe how it meets the brief. Do not generate source yet.", manifest.requested_variants))),
        RunPhase::Generate | RunPhase::Review => {
            let files = if state.last_revision.0 == 0 { BTreeMap::new() } else { store.read_sources(ctx, session(agent)?, manifest.id, state.last_revision)? };
            let assets = if state.last_revision.0 == 0 { vec![] } else { store.read_revision(ctx, session(agent)?, manifest.id, state.last_revision)?.assets };
            messages.push(ChatMessage::text("user", serde_json::to_string(&json!({"directions":state.directions,"current_source":files,"retained_assets":assets,"findings":state.findings,"quality":state.quality}))?));
            if state.progress.phase == RunPhase::Generate {
                messages.push(ChatMessage::text("user", "Call the single active source tool exactly once with every variant and complete source. Preserve factual content, responsive behavior, accessible names and keyboard interactions. Keep all node bindings backed by JSON source; the host computes source_hash from supplied JSON bytes. Never invent real backend integrations. Repairs must address the supplied findings without changing the selected directions."));
            } else {
                if images {
                    if let Some(report) = &state.quality {
                        use base64::Engine;
                        for reference in report.evidence.iter().filter(|r| r.media_type == "image/png") {
                            let bytes = store.read_blob(ctx, reference)?;
                            if bytes.len() > 4 * 1024 * 1024 { return Err(DesignError::BudgetExceeded("review image size".into())); }
                            messages.push(ChatMessage { role:"user".into(), content:vec![MessageContent::Image { data:base64::engine::general_purpose::STANDARD.encode(bytes), mime_type:"image/png".into() }], ..Default::default() });
                        }
                    }
                }
                messages.push(ChatMessage::text("user", "Critique the actual supplied source and evidence against the brief and both concepts. Return only JSON {\"findings\":[\"concrete actionable defect\"]}. Empty means no defect found in supplied evidence, not acceptance or verified implementation. Do not infer visual quality without images, interaction without action evidence, or production integration from mocks."));
            }
        }
        _ => return Err(DesignError::Conflict("no provider dispatch for this phase".into())),
    }
    Ok(messages)
}

/// Execute or resume the exact operation. A recorded in-flight request is never retried.
pub fn run(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    request: GenerationRequest,
    model: &mut dyn DesignModel,
    renderer: &mut dyn DesignRenderer,
    cancel: Option<Arc<AtomicBool>>,
) -> DesignResult<DesignRunState> {
    model.validate(agent)?;
    validate_text(&request.brief, 64 * 1024, "revision brief")?;
    let args = serde_json::to_value(&request)?;
    let ctx = AuthorizedDesignContext::for_operation(agent, workspace, "design_generate", &args)?;
    let ctx = if let Some(cancel) = cancel {
        ctx.with_cancellation(cancel)
    } else {
        ctx
    };
    let budget = agent
        .root_budget()
        .ok_or_else(|| DesignError::MissingCapability("root admission unavailable".into()))?
        .clone();
    let request_hash = digest(&request)?;
    let effort = format!("{:?}", agent.request_thinking_level());
    let mut state = if let Some(state) =
        latest_run(store, &ctx, session(agent)?, request.operation_id)?
    {
        if state.request_hash != request_hash
            || state.provider != agent.provider
            || state.model != agent.model_id
            || state.effort != effort
            || state.root_identity != budget.binding_identity()
        {
            return Err(DesignError::Conflict(
                "resume requires the exact original request, route, effort and root ledger".into(),
            ));
        }
        if matches!(state.progress.phase, RunPhase::Complete | RunPhase::Blocked) {
            budget
                .finish_operation(&request.operation_id.to_string())
                .map_err(DesignError::BudgetExceeded)?;
            return Ok(state);
        }
        state.progress.can_dispatch(davinci_session::now_ms())?;
        state
    } else {
        let manifest = store.manifest(&ctx, session(agent)?, request.artifact_id)?;
        if manifest.revision != request.expected_revision {
            return Err(DesignError::Conflict(
                "artifact changed before generation".into(),
            ));
        }
        let selection = super::skills::select_profile(manifest.kind, &[])?;
        let snapshot = store
            .events(&ctx, session(agent)?)?
            .into_iter()
            .rev()
            .find_map(|event| match event.change {
                DesignChange::SystemSynced { snapshot, .. } => Some(snapshot),
                _ => None,
            });
        let facts = snapshot
            .as_ref()
            .map(|r| {
                store.read_blob(&ctx, r).and_then(|data| {
                    serde_json::from_slice::<super::sync::DesignSystemSnapshot>(&data)
                        .map_err(DesignError::from)
                })
            })
            .transpose()?;
        if let Some(facts) = &facts {
            super::sync::validate_current(&ctx, facts)?;
        }
        let original = String::from_utf8(store.read_blob(&ctx, &manifest.brief)?)
            .map_err(|_| DesignError::CorruptArtifact("brief is not UTF-8".into()))?;
        let context = compile_design_context(
            &selection,
            &format!(
                "Original brief:\n{original}\nRequested revision:\n{}",
                request.brief
            ),
            facts.as_ref(),
            12_000,
        )?;
        let state = DesignRunState {
            request: request.clone(),
            request_hash,
            provider: agent.provider.clone(),
            model: agent.model_id.clone(),
            effort,
            root_identity: budget.binding_identity(),
            context: store.retain(&ctx, "text/plain", context.text.as_bytes())?,
            profile: store.retain(&ctx, "application/json", &serde_json::to_vec(&selection)?)?,
            system_snapshot: snapshot,
            directions: vec![],
            progress: RunProgress {
                phase: RunPhase::Directions,
                in_flight: false,
                repairs: 0,
                deadline_ms: davinci_session::now_ms().saturating_add(600_000),
            },
            reply: None,
            last_revision: manifest.revision,
            findings: vec![],
            quality: None,
        };
        checkpoint(store, &ctx, agent, &state)?;
        state
    };
    budget
        .begin_operation(
            &request.operation_id.to_string(),
            12,
            state.progress.deadline_ms,
        )
        .map_err(DesignError::BudgetExceeded)?;
    let result = run_phases(store, &ctx, agent, model, renderer, &mut state);
    if let Err(error) = result {
        // If authority was lost, publication also fails; the previous durable state remains.
        state.progress.phase = RunPhase::Blocked;
        state.findings.push(error.to_string());
    }
    let publication = checkpoint(store, &ctx, agent, &state);
    budget
        .finish_operation(&request.operation_id.to_string())
        .map_err(DesignError::BudgetExceeded)?;
    publication?;
    Ok(state)
}

fn run_phases(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    agent: &mut Agent,
    model: &mut dyn DesignModel,
    renderer: &mut dyn DesignRenderer,
    state: &mut DesignRunState,
) -> DesignResult<()> {
    loop {
        ctx.check("design_generate", &serde_json::to_value(&state.request)?)?;
        state.progress.can_dispatch(davinci_session::now_ms())?;
        let head = store.manifest(ctx, session(agent)?, state.request.artifact_id)?;
        if head.revision != state.last_revision && state.reply.is_none() {
            return Err(DesignError::Conflict(
                "human edit changed the design during generation".into(),
            ));
        }
        if state.progress.phase == RunPhase::Render {
            let revision =
                store.read_revision(ctx, session(agent)?, head.id, state.last_revision)?;
            match renderer.render(store, ctx, session(agent)?, &revision) {
                Ok(report) => {
                    state.findings = [
                        &report.render,
                        &report.accessibility,
                        &report.interaction,
                        &report.assets,
                    ]
                    .iter()
                    .flat_map(|check| check.failures.clone())
                    .collect();
                    state.quality = Some(report);
                    state.progress.phase = RunPhase::Review;
                }
                Err(DesignError::InvalidInput(message)) => {
                    state.findings = vec![message];
                    state.progress.begin_repair()?;
                }
                Err(error) => return Err(error),
            }
            checkpoint(store, ctx, agent, state)?;
            continue;
        }
        let reply = if let Some(reference) = &state.reply {
            serde_json::from_slice::<ChatMessage>(&store.read_blob(ctx, reference)?)?
        } else {
            let request = DesignModelRequest {
                messages: phase_messages(store, ctx, agent, state, model.supports_images())?,
                tools: tools(state.progress.phase, state.last_revision.0 == 0),
                deadline_ms: state.progress.deadline_ms,
                abort: Arc::new(AtomicBool::new(false)),
            };
            state.progress.in_flight = true;
            checkpoint(store, ctx, agent, state)?;
            let result = super::model::complete_checked(agent, ctx, model, &request)?;
            let reply = davinci_ai::assistant_to_chat(&result.message);
            let data = serde_json::to_vec(&reply)?;
            if data.len() > 2 * 1024 * 1024 {
                return Err(DesignError::BudgetExceeded("model result size".into()));
            }
            state.reply = Some(store.retain(ctx, "application/json", &data)?);
            state.progress.in_flight = false;
            checkpoint(store, ctx, agent, state)?;
            reply
        };
        match state.progress.phase {
            RunPhase::Directions => {
                if reply
                    .content
                    .iter()
                    .any(|c| matches!(c, MessageContent::ToolCall { .. }))
                {
                    return Err(DesignError::Denied(
                        "tool calls are not admitted during concept planning".into(),
                    ));
                }
                state.directions = serde_json::from_str(&davinci_ai::content_text(&reply.content))?;
                validate_directions(&state.directions, head.requested_variants)?;
                state.progress.phase = RunPhase::Generate;
            }
            RunPhase::Generate => {
                let calls: Vec<_> = reply
                    .content
                    .iter()
                    .filter_map(|c| {
                        if let MessageContent::ToolCall {
                            name, arguments, ..
                        } = c
                        {
                            Some((name, arguments))
                        } else {
                            None
                        }
                    })
                    .collect();
                let expected = if state.last_revision.0 == 0 {
                    "design_create"
                } else {
                    "design_patch"
                };
                if calls.len() != 1 || calls[0].0 != expected {
                    return Err(DesignError::Denied(
                        "generation must call the active artifact source tool exactly once".into(),
                    ));
                }
                let mut source: GeneratedSource = serde_json::from_value(calls[0].1.clone())?;
                if source.variants.len() != state.directions.len() {
                    return Err(DesignError::InvalidInput(
                        "source must include every planned concept".into(),
                    ));
                }
                // Hashing is deterministic host work, never a model arithmetic obligation.
                for binding in &mut source.bindings {
                    let file = source.files.get(&binding.source_file).ok_or_else(|| {
                        DesignError::InvalidInput("binding source missing".into())
                    })?;
                    binding.source_hash = super::skills::byte_hash(file.as_bytes());
                }
                let sources = store.store_sources(
                    ctx,
                    session(agent)?,
                    &source.files,
                    source.entry_points,
                )?;
                let assets = if state.last_revision.0 == 0 {
                    vec![]
                } else {
                    store
                        .read_revision(ctx, session(agent)?, head.id, state.last_revision)?
                        .assets
                };
                let revision = store.commit_revision(
                    ctx,
                    session(agent)?,
                    RevisionWrite {
                        artifact_id: head.id,
                        expected_revision: state.last_revision,
                        operation_id: revision_operation(state)?,
                        sources,
                        variants: source.variants,
                        bindings: source.bindings,
                        assets,
                        profile_refs: vec![state.profile.clone()],
                        system_snapshot: state.system_snapshot.clone(),
                    },
                )?;
                state.last_revision = revision.revision;
                state.progress.phase = RunPhase::Render;
            }
            RunPhase::Review => {
                if reply
                    .content
                    .iter()
                    .any(|c| matches!(c, MessageContent::ToolCall { .. }))
                {
                    return Err(DesignError::Denied("review cannot mutate a design".into()));
                }
                let critique: Critique =
                    serde_json::from_str(&davinci_ai::content_text(&reply.content))?;
                if critique.findings.len() > 32 {
                    return Err(DesignError::BudgetExceeded("review findings limit".into()));
                }
                for finding in &critique.findings {
                    validate_text(finding, 2048, "review finding")?;
                }
                state.findings.extend(critique.findings);
                state.findings.sort();
                state.findings.dedup();
                if state.findings.is_empty() {
                    state.progress.phase = RunPhase::Complete;
                } else {
                    state.progress.begin_repair()?;
                }
            }
            _ => return Err(DesignError::Conflict("unexpected run phase".into())),
        }
        state.reply = None;
        checkpoint(store, ctx, agent, state)?;
        if state.progress.phase == RunPhase::Complete {
            return Ok(());
        }
    }
}
