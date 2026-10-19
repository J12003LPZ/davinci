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
    /// Revise only this concept of the current revision. Every other concept
    /// and every file it uses must come back unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_variant: Option<VariantId>,
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
    let mut strategies = std::collections::BTreeSet::new();
    let mut titles = std::collections::BTreeSet::new();
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
        // Different words for the same idea are not a different concept, but
        // the same words certainly are not.
        let normalize = |text: &str| {
            text.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        };
        if !strategies.insert(normalize(&direction.content_strategy))
            || !titles.insert(normalize(&direction.title))
        {
            return Err(DesignError::InvalidInput(
                "concepts must differ in content strategy and title, not only layout".into(),
            ));
        }
    }
    Ok(())
}

/// Read a model's concept plan. Accepts the documented array, the same array
/// in a code fence or wrapped as `{"directions": [...]}`, and a bare object
/// when exactly one concept was asked for. Anything else is rejected with the
/// shape that was expected.
pub fn parse_directions(text: &str, count: u32) -> DesignResult<Vec<ConceptDirection>> {
    let value: serde_json::Value = serde_json::from_str(unfenced(text))
        .map_err(|error| DesignError::InvalidInput(format!("the plan is not JSON: {error}")))?;
    let list = match value {
        serde_json::Value::Array(list) => list,
        serde_json::Value::Object(mut object) => {
            if let Some(serde_json::Value::Array(list)) = object
                .remove("directions")
                .or_else(|| object.remove("concepts"))
            {
                list
            } else if count == 1 {
                vec![serde_json::Value::Object(object)]
            } else {
                return Err(DesignError::InvalidInput(
                    "the plan is an object; an array of concept directions was expected".into(),
                ));
            }
        }
        _ => {
            return Err(DesignError::InvalidInput(
                "the plan is not an array of concept directions".into(),
            ))
        }
    };
    list.into_iter()
        .enumerate()
        .map(|(index, item)| {
            serde_json::from_value::<ConceptDirection>(item)
                .map_err(|error| DesignError::InvalidInput(format!("direction {index}: {error}")))
        })
        .collect()
}

/// What a phase's reply must look like, for an error a person can act on.
pub fn expected_schema(phase: RunPhase, variants: u32) -> String {
    match phase {
        RunPhase::Directions => format!(
            "a JSON array of exactly {variants} objects {{title, layout: editorial|split|modular, typography: serif|sans|mono, content_strategy}}; layouts, typographies, titles and content strategies all different"
        ),
        RunPhase::Generate => "one design_create or design_patch tool call: {files: {name: html}, entry_points: [name], variants: [{id, title, artboards: [{id, title, entry_point}]}], bindings: []}; variants is always an array".into(),
        RunPhase::Render => "source that compiles and renders in the confined browser".into(),
        RunPhase::Review => "JSON {\"findings\": [\"concrete actionable defect\"]} with at most 32 findings".into(),
        RunPhase::Complete | RunPhase::Blocked => String::new(),
    }
}

/// A model reply cut down for an error message: control characters gone,
/// token-like strings replaced, and a hard length bound. Never the whole reply.
pub fn sanitize_snippet(text: &str, limit: usize) -> String {
    let mut out = String::new();
    // The word after a scheme name is the credential itself.
    let mut after_scheme = false;
    for word in text.split_whitespace() {
        let secret = word.len() >= 24
            && word.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'+' | b'/' | b'=')
            })
            && word.bytes().any(|b| b.is_ascii_digit());
        let scheme = word.eq_ignore_ascii_case("bearer") || word.eq_ignore_ascii_case("basic");
        let word = if secret || after_scheme || scheme || word.starts_with("sk-") {
            "[redacted]"
        } else {
            word
        };
        after_scheme = scheme;
        if !out.is_empty() {
            out.push(' ');
        }
        out.extend(word.chars().filter(|c| !c.is_control()));
        if out.chars().count() >= limit {
            break;
        }
    }
    let mut out: String = out.chars().take(limit).collect();
    if out.chars().count() == limit {
        out.push_str(" [...]");
    }
    out
}
/// Why a run stopped, kept with the state so the stop is not a guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignFailureReceipt {
    /// The phase that failed. The run itself is `Blocked`.
    pub phase: RunPhase,
    pub code: String,
    pub message: String,
    pub artifact: ArtifactId,
    /// The last committed revision; it stays readable.
    pub revision: RevisionId,
    /// Whether `resume` can try the phase again.
    pub retryable: bool,
    pub repair_count: u32,
    pub repairs_remaining: u32,
    pub model_route: String,
    pub expected_schema: String,
    /// A bounded, sanitized piece of the reply that failed.
    pub snippet: String,
    /// The run to resume: this operation's checkpoint.
    pub checkpoint: OperationId,
    /// The retained model reply, when there is one.
    pub draft: Option<ArtifactRef>,
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
    /// Spend one repair to ask the same phase again (a plan or a critique that
    /// did not parse). Shares the two-round allowance with source repairs.
    pub fn retry_phase(&mut self) -> DesignResult<()> {
        if self.repairs >= 2 {
            return Err(DesignError::BudgetExceeded(
                "two repair rounds exhausted; last revision retained".into(),
            ));
        }
        self.repairs += 1;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<DesignFailureReceipt>,
    /// The revision a targeted run started from; the concepts it must not
    /// change are read from it, however many repairs the run takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<RevisionId>,
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
/// A single planned direction sometimes arrives as one Variant object instead
/// of a one-element array. Canonicalize only that exact shape. All other
/// malformed structures still reach the ordinary repair path.
fn parse_generated_source(
    mut arguments: serde_json::Value,
    directions: usize,
) -> DesignResult<GeneratedSource> {
    let variants = arguments
        .get_mut("variants")
        .ok_or_else(|| DesignError::InvalidInput("variants must be an array".into()))?;
    if variants.is_object() && directions == 1 {
        let object = variants.as_object().expect("checked object");
        if object.len() == 3
            && ["id", "title", "artboards"]
                .iter()
                .all(|key| object.contains_key(*key))
        {
            let single = variants.take();
            *variants = serde_json::Value::Array(vec![single]);
        }
    }
    if !variants.is_array() {
        return Err(DesignError::InvalidInput(
            "variants must be an array, even for one concept".into(),
        ));
    }
    let source: GeneratedSource = serde_json::from_value(arguments)?;
    if source.variants.len() != directions {
        return Err(DesignError::InvalidInput(
            "source must include every planned concept".into(),
        ));
    }
    Ok(source)
}

/// A targeted revision may change the chosen concept and the files only it
/// uses. Every other concept must be present as it was, and every other file
/// byte for byte; anything else is the model's to fix.
fn only_target_changed(
    before: &DesignRevision,
    files: &BTreeMap<String, String>,
    after: &GeneratedSource,
    target: VariantId,
) -> DesignResult<()> {
    let changed = |what: String| {
        Err(DesignError::InvalidInput(format!(
            "only the selected concept may change, but {what}"
        )))
    };
    let mut target_files = std::collections::BTreeSet::new();
    for variant in &before.variants {
        if variant.id == target {
            target_files.extend(variant.artboards.iter().map(|b| b.entry_point.clone()));
            continue;
        }
        match after.variants.iter().find(|v| v.id == variant.id) {
            Some(same) if same == variant => {}
            Some(_) => return changed(format!("concept {} was altered", variant.id)),
            None => return changed(format!("concept {} is missing", variant.id)),
        }
    }
    if !after.variants.iter().any(|variant| variant.id == target) {
        return changed("the selected concept is missing".into());
    }
    // A file the other concepts use is not the target's to rewrite, even when
    // the target uses it too.
    let shared: std::collections::BTreeSet<&String> = before
        .variants
        .iter()
        .filter(|variant| variant.id != target)
        .flat_map(|variant| variant.artboards.iter().map(|b| &b.entry_point))
        .collect();
    for (path, text) in files {
        if target_files.contains(path) && !shared.contains(path) {
            continue;
        }
        match after.files.get(path) {
            Some(now) if now == text => {}
            Some(_) => return changed(format!("{path} was edited")),
            None => return changed(format!("{path} was removed")),
        }
    }
    Ok(())
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
            "bindings":{"type":"array","items":binding_schema()}
        },"required":["files","entry_points","variants","bindings"]}), constrained_sampling:None }]
}

/// One editable binding, as `EditableBinding` parses it. `source_hash` is
/// optional: the host computes it from the file's bytes.
fn binding_schema() -> serde_json::Value {
    let constraint = |kind: &str, fields: serde_json::Value, required: &[&str]| {
        let mut properties = json!({"type":{"const":kind}});
        for (name, schema) in fields.as_object().unwrap() {
            properties[name] = schema.clone();
        }
        let mut required: Vec<&str> = required.to_vec();
        required.insert(0, "type");
        json!({"type":"object","additionalProperties":false,"properties":properties,"required":required})
    };
    json!({"type":"object","additionalProperties":false,"properties":{
        "node_id":{"type":"string","description":"UUID of the bound node"},
        "artboard_id":{"type":"string","description":"UUID of an artboard in variants"},
        "source_file":{"type":"string","description":"the JSON file in files holding the value"},
        "source_hash":{"type":"string","description":"optional; the host computes it"},
        "pointer":{"type":"string","description":"JSON Pointer to the value, e.g. /hero/title"},
        "constraint":{"oneOf":[
            constraint("text", json!({"max_bytes":{"type":"integer","minimum":1}}), &["max_bytes"]),
            constraint("enum", json!({"values":{"type":"array","items":{"type":"string"}}}), &["values"]),
            constraint("number", json!({"min":{"type":"integer"},"max":{"type":"integer"}}), &["min","max"]),
            constraint("color", json!({}), &[]),
            constraint("local_asset", json!({"paths":{"type":"array","items":{"type":"string"}}}), &["paths"]),
        ]},
        "affected_nodes":{"type":"array","items":{"type":"string"},"description":"node_id of every binding with this same source_file and pointer, this one included"}
    },"required":["node_id","artboard_id","source_file","pointer","constraint","affected_nodes"]})
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
        RunPhase::Directions => {
            messages.push(ChatMessage::text("user", format!("Return only a JSON array of exactly {} concept directions. Each has title, layout (editorial/split/modular), typography (serif/sans/mono), content_strategy. Each direction must have a unique layout, typography, title AND content_strategy: a different idea for what leads the page and how dense it is, not the same idea restyled. Describe how each meets the brief. Do not generate source yet.", manifest.requested_variants)));
            if !state.findings.is_empty() {
                messages.push(ChatMessage::text(
                    "user",
                    format!(
                        "The previous plan was rejected: {}. Return the corrected JSON array only.",
                        state.findings.join("; ")
                    ),
                ));
            }
        }
        RunPhase::Generate | RunPhase::Review => {
            let files = if state.last_revision.0 == 0 {
                BTreeMap::new()
            } else {
                store.read_sources(ctx, session(agent)?, manifest.id, state.last_revision)?
            };
            let assets = if state.last_revision.0 == 0 {
                vec![]
            } else {
                store
                    .read_revision(ctx, session(agent)?, manifest.id, state.last_revision)?
                    .assets
            };
            messages.push(ChatMessage::text("user", serde_json::to_string(&json!({"directions":state.directions,"current_source":files,"retained_assets":assets,"findings":state.findings,"quality":state.quality}))?));
            if let (RunPhase::Generate, Some(target)) =
                (state.progress.phase, state.request.target_variant)
            {
                messages.push(ChatMessage::text("user", format!("Revise ONLY the concept with id {target} according to the requested revision. Call the single active source tool exactly once with every variant. Return every other variant exactly as it is in current_source: the same id, title and artboards. Only the entry point files of the revised concept may change; every other existing file must come back byte-identical. Add new files for anything new. The variants field MUST be an array of variant objects. Keep all node bindings backed by JSON source; the host computes source_hash from supplied JSON bytes. Never invent real backend integrations. Repairs must address the supplied findings.")));
            } else if state.progress.phase == RunPhase::Generate {
                messages.push(ChatMessage::text("user", "Call the single active source tool exactly once with every variant and complete source. The variants field MUST be an array of variant objects, including when there is only one concept; never send a bare object. Preserve factual content, responsive behavior, accessible names and keyboard interactions. Keep all node bindings backed by JSON source; the host computes source_hash from supplied JSON bytes. Never invent real backend integrations. Repairs must address the supplied findings without changing the selected directions."));
            } else {
                if images {
                    if let Some(report) = &state.quality {
                        use base64::Engine;
                        for reference in report
                            .evidence
                            .iter()
                            .filter(|r| r.media_type == "image/png")
                        {
                            let bytes = store.read_blob(ctx, reference)?;
                            if bytes.len() > 4 * 1024 * 1024 {
                                return Err(DesignError::BudgetExceeded(
                                    "review image size".into(),
                                ));
                            }
                            messages.push(ChatMessage {
                                role: "user".into(),
                                content: vec![MessageContent::Image {
                                    data: base64::engine::general_purpose::STANDARD.encode(bytes),
                                    mime_type: "image/png".into(),
                                }],
                                ..Default::default()
                            });
                        }
                    }
                }
                messages.push(ChatMessage::text("user", "Critique the actual supplied source and evidence against the brief and every planned concept. Return only JSON {\"findings\":[\"concrete actionable defect\"]}. Empty means no defect found in supplied evidence, not acceptance or verified implementation. Do not infer visual quality without images, interaction without action evidence, or production integration from mocks."));
            }
        }
        _ => {
            return Err(DesignError::Conflict(
                "no provider dispatch for this phase".into(),
            ))
        }
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
    drive(
        store, agent, workspace, request, model, renderer, cancel, false,
    )
}

/// Try the failed phase of a blocked run once more, from its checkpoint. The
/// original request, route, effort, ledger, deadline and repair count all
/// stay: this is one more attempt inside the same allowance, not a new budget.
/// A run whose provider outcome is unknown is never resumed.
pub fn resume(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    request: GenerationRequest,
    model: &mut dyn DesignModel,
    renderer: &mut dyn DesignRenderer,
    cancel: Option<Arc<AtomicBool>>,
) -> DesignResult<DesignRunState> {
    drive(
        store, agent, workspace, request, model, renderer, cancel, true,
    )
}

/// The design a checkpoint belongs to, read without running anything. `resume`
/// checks it before the model or the renderer is built, so a checkpoint of
/// another design never reaches a provider.
pub fn checkpoint_artifact(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    operation_id: OperationId,
) -> DesignResult<ArtifactId> {
    let ctx = AuthorizedDesignContext::from_agent(agent, workspace)?;
    latest_run(store, &ctx, session(agent)?, operation_id)?
        .map(|state| state.request.artifact_id)
        .ok_or_else(|| DesignError::NotFound("no design run has that checkpoint".into()))
}

/// Resume a blocked run by its checkpoint (the operation id its failure
/// receipt names). The stored request is used as it was; nothing is re-typed.
pub fn resume_operation(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    operation_id: OperationId,
    model: &mut dyn DesignModel,
    renderer: &mut dyn DesignRenderer,
    cancel: Option<Arc<AtomicBool>>,
) -> DesignResult<DesignRunState> {
    let ctx = AuthorizedDesignContext::from_agent(agent, workspace)?;
    let state = latest_run(store, &ctx, session(agent)?, operation_id)?
        .ok_or_else(|| DesignError::NotFound("no design run has that checkpoint".into()))?;
    resume(
        store,
        agent,
        workspace,
        state.request,
        model,
        renderer,
        cancel,
    )
}

#[allow(clippy::too_many_arguments)]
fn drive(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    request: GenerationRequest,
    model: &mut dyn DesignModel,
    renderer: &mut dyn DesignRenderer,
    cancel: Option<Arc<AtomicBool>>,
    resuming: bool,
) -> DesignResult<DesignRunState> {
    if agent.root_budget().is_some() {
        return run_accounted(
            store, agent, workspace, request, model, renderer, cancel, resuming,
        );
    }
    // No host budget: this operation's own, attached for its length only.
    model.validate(agent)?;
    let budget = super::budget::for_operation(
        agent,
        &request.operation_id.to_string(),
        super::budget::GENERATION,
    )?;
    agent
        .with_operation_budget(budget, |agent| {
            run_accounted(
                store, agent, workspace, request, model, renderer, cancel, resuming,
            )
        })
        .map_err(DesignError::MissingCapability)?
}

#[allow(clippy::too_many_arguments)]
fn run_accounted(
    store: &DesignStore,
    agent: &mut Agent,
    workspace: &Path,
    request: GenerationRequest,
    model: &mut dyn DesignModel,
    renderer: &mut dyn DesignRenderer,
    cancel: Option<Arc<AtomicBool>>,
    resuming: bool,
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
    let budget = super::budget::for_operation(
        agent,
        &request.operation_id.to_string(),
        super::budget::GENERATION,
    )?;
    let request_hash = digest(&request)?;
    let effort = format!("{:?}", agent.request_thinking_level());
    let mut resumed = false;
    let mut state = if let Some(mut state) =
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
        let resumable = resuming
            && state.progress.phase == RunPhase::Blocked
            && !state.progress.in_flight
            && state
                .failure
                .as_ref()
                .is_some_and(|failure| failure.retryable);
        if resumable {
            resumed = true;
            let failure = state.failure.take().expect("checked above");
            state.progress.phase = failure.phase;
            // A reply that failed validation is asked for again; a reply that
            // was fine and failed later (a write) is used as it is.
            if failure.code == "invalid_input" {
                state.reply = None;
            }
            state.findings.push(format!(
                "resumed from checkpoint {} at {:?}",
                failure.checkpoint, failure.phase
            ));
        } else if matches!(state.progress.phase, RunPhase::Complete | RunPhase::Blocked) {
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
        let baseline = match request.target_variant {
            Some(target) => {
                if manifest.revision.0 == 0 {
                    return Err(DesignError::InvalidInput(
                        "there is no concept to revise yet".into(),
                    ));
                }
                let current = store.read_revision(
                    &ctx,
                    session(agent)?,
                    request.artifact_id,
                    manifest.revision,
                )?;
                if !current.variants.iter().any(|variant| variant.id == target) {
                    return Err(DesignError::NotFound(
                        "the selected concept is not in the current revision".into(),
                    ));
                }
                Some(manifest.revision)
            }
            None => None,
        };
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
                // A targeted revision keeps the plan it already has.
                phase: if baseline.is_some() {
                    RunPhase::Generate
                } else {
                    RunPhase::Directions
                },
                in_flight: false,
                repairs: 0,
                deadline_ms: davinci_session::now_ms().saturating_add(600_000),
            },
            reply: None,
            last_revision: manifest.revision,
            findings: vec![],
            quality: None,
            failure: None,
            baseline,
        };
        checkpoint(store, &ctx, agent, &state)?;
        state
    };
    let operation = request.operation_id.to_string();
    let opened = if resumed {
        budget.resume_operation(&operation, 12, state.progress.deadline_ms)
    } else {
        budget.begin_operation(&operation, 12, state.progress.deadline_ms)
    };
    opened.map_err(DesignError::BudgetExceeded)?;
    let result = run_phases(store, &ctx, agent, model, renderer, &mut state);
    if let Err(error) = result {
        // If authority was lost, publication also fails; the previous durable state remains.
        let receipt = failure_receipt(store, &ctx, agent, &state, &error);
        state.progress.phase = RunPhase::Blocked;
        state.findings.push(error.to_string());
        state.failure = Some(receipt);
    }
    let publication = checkpoint(store, &ctx, agent, &state);
    budget
        .finish_operation(&request.operation_id.to_string())
        .map_err(DesignError::BudgetExceeded)?;
    publication?;
    Ok(state)
}

fn failure_receipt(
    store: &DesignStore,
    ctx: &AuthorizedDesignContext,
    agent: &mut Agent,
    state: &DesignRunState,
    error: &DesignError,
) -> DesignFailureReceipt {
    let phase = state.progress.phase;
    let (code, message) = match error {
        DesignError::Cancelled => ("cancelled".to_string(), "the run was cancelled".to_string()),
        other => {
            let value = serde_json::to_value(other).unwrap_or_default();
            (
                value["code"].as_str().unwrap_or("error").to_string(),
                value["message"].as_str().unwrap_or_default().to_string(),
            )
        }
    };
    let snippet = state
        .reply
        .as_ref()
        .and_then(|reference| store.read_blob(ctx, reference).ok())
        .and_then(|data| serde_json::from_slice::<ChatMessage>(&data).ok())
        .map(|reply| {
            let mut text = davinci_ai::content_text(&reply.content);
            for content in &reply.content {
                if let MessageContent::ToolCall { arguments, .. } = content {
                    text.push(' ');
                    text.push_str(&arguments.to_string());
                }
            }
            sanitize_snippet(&text, 240)
        })
        .unwrap_or_default();
    let variants = session(agent)
        .ok()
        .and_then(|session| store.manifest(ctx, session, state.request.artifact_id).ok())
        .map(|manifest| manifest.requested_variants)
        .unwrap_or(state.directions.len() as u32);
    DesignFailureReceipt {
        phase,
        retryable: matches!(
            error,
            DesignError::InvalidInput(_)
                | DesignError::BudgetExceeded(_)
                | DesignError::IoFailure(_)
                | DesignError::Cancelled
        ) && !state.progress.in_flight,
        code,
        message: sanitize_snippet(&message, 400),
        artifact: state.request.artifact_id,
        revision: state.last_revision,
        repair_count: state.progress.repairs,
        repairs_remaining: 2u32.saturating_sub(state.progress.repairs),
        model_route: format!("{}/{}", state.provider, state.model),
        expected_schema: expected_schema(phase, variants),
        snippet,
        checkpoint: state.request.operation_id,
        draft: state.reply.clone(),
    }
}

/// Spend one repair, or stop with the message that needed it. The caller's
/// message, not "allowance exhausted", is what a person has to act on.
fn repair_with(state: &mut DesignRunState, message: &str, in_place: bool) -> DesignResult<()> {
    let spent = if in_place {
        state.progress.retry_phase()
    } else {
        state.progress.begin_repair()
    };
    spent.map_err(|_| {
        DesignError::InvalidInput(format!(
            "{message} (two repair rounds exhausted; last revision retained)"
        ))
    })
}

/// Reply text without a surrounding code fence.
fn unfenced(text: &str) -> &str {
    let trimmed = text.trim();
    trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.trim_end().strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim()
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
                    repair_with(state, &message, false)?;
                    state.findings = vec![message];
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
                let planned = parse_directions(
                    &davinci_ai::content_text(&reply.content),
                    head.requested_variants,
                )
                .and_then(|directions| {
                    validate_directions(&directions, head.requested_variants)?;
                    Ok(directions)
                });
                match planned {
                    Ok(directions) => {
                        state.directions = directions;
                        state.findings.clear();
                        state.progress.phase = RunPhase::Generate;
                    }
                    // A plan that does not parse or differ is the model's to
                    // fix: ask again, inside the same allowance.
                    Err(DesignError::InvalidInput(message)) => {
                        repair_with(
                            state,
                            &format!("the concept plan was rejected: {message}"),
                            true,
                        )?;
                        state.findings = vec![format!("the concept plan was rejected: {message}")];
                        state.reply = None;
                        checkpoint(store, ctx, agent, state)?;
                        continue;
                    }
                    Err(error) => return Err(error),
                }
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
                let expected_variants = match state.baseline {
                    Some(baseline) => store
                        .read_revision(ctx, session(agent)?, head.id, baseline)?
                        .variants
                        .len(),
                    None => state.directions.len(),
                };
                let mut source = (|| -> DesignResult<GeneratedSource> {
                    let mut source = parse_generated_source(calls[0].1.clone(), expected_variants)?;
                    // Hashing is deterministic host work, never a model arithmetic obligation.
                    for binding in &mut source.bindings {
                        let file = source.files.get(&binding.source_file).ok_or_else(|| {
                            DesignError::InvalidInput(format!(
                                "binding source {} is not one of the files",
                                binding.source_file
                            ))
                        })?;
                        binding.source_hash = super::skills::byte_hash(file.as_bytes());
                    }
                    Ok(source)
                })();
                // Only the chosen concept may change in a targeted revision.
                if let (Ok(generated), Some(target), Some(baseline)) =
                    (&mut source, state.request.target_variant, state.baseline)
                {
                    let before = store.read_revision(ctx, session(agent)?, head.id, baseline)?;
                    let files = store.read_sources(ctx, session(agent)?, head.id, baseline)?;
                    if let Err(error) = only_target_changed(&before, &files, generated, target) {
                        source = Err(error);
                    }
                }
                // A malformed call is the model's to fix, inside the repair
                // allowance, like a compile or render failure.
                let source = match source {
                    Ok(source) => source,
                    Err(DesignError::InvalidInput(message)) => {
                        let message = format!("the source tool call was rejected: {message}");
                        repair_with(state, &message, false)?;
                        state.findings = vec![message];
                        state.reply = None;
                        checkpoint(store, ctx, agent, state)?;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let sources = match store.store_sources(
                    ctx,
                    session(agent)?,
                    &source.files,
                    source.entry_points,
                ) {
                    Ok(sources) => sources,
                    Err(DesignError::InvalidInput(message)) => {
                        let message = format!("the source files were rejected: {message}");
                        repair_with(state, &message, false)?;
                        state.findings = vec![message];
                        state.reply = None;
                        checkpoint(store, ctx, agent, state)?;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let assets = if state.last_revision.0 == 0 {
                    vec![]
                } else {
                    store
                        .read_revision(ctx, session(agent)?, head.id, state.last_revision)?
                        .assets
                };
                let revision = match store.commit_revision(
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
                ) {
                    Ok(revision) => revision,
                    // Bindings and sources the store refuses are the model's
                    // to fix, like a malformed call.
                    Err(DesignError::InvalidInput(message)) => {
                        let message = format!("the source revision was rejected: {message}");
                        repair_with(state, &message, false)?;
                        state.findings = vec![message];
                        state.reply = None;
                        checkpoint(store, ctx, agent, state)?;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
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
                    match serde_json::from_str(unfenced(&davinci_ai::content_text(&reply.content)))
                    {
                        Ok(critique) => critique,
                        Err(error) => {
                            let message =
                                format!("the critique was not valid JSON findings: {error}");
                            repair_with(state, &message, true)?;
                            state.reply = None;
                            checkpoint(store, ctx, agent, state)?;
                            continue;
                        }
                    };
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

#[cfg(test)]
mod source_shape_evals {
    use super::*;

    #[test]
    fn recorded_single_concept_shape_and_neighboring_malformed_shapes() {
        let corpus: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../docs/evals/design/fixtures/source-shapes.json"
        ))
        .unwrap();
        assert_eq!(corpus.len(), 8);
        for case in corpus {
            let variant = json!({
                "id":"00000000-0000-4000-8000-000000000001",
                "title":"Concept",
                "artboards":[{"id":"00000000-0000-4000-8000-000000000002","title":"Desktop","entry_point":"index.html"}]
            });
            let shape = case["shape"].as_str().unwrap();
            let variants = match shape {
                "single" => variant,
                "array" => json!([variant]),
                "keyed" => json!({"concept":variant}),
                "extra" => {
                    let mut value = variant;
                    value["unrecognized"] = json!(true);
                    value
                }
                "missing" => {
                    let mut value = variant;
                    value.as_object_mut().unwrap().remove("artboards");
                    value
                }
                "bad-id" => {
                    let mut value = variant;
                    value["id"] = json!("not-a-uuid");
                    value
                }
                _ => panic!("unknown eval shape"),
            };
            let source = json!({"files":{"index.html":"<h1>Hello</h1>"},"entry_points":["index.html"],"variants":variants,"bindings":[]});
            let parsed =
                parse_generated_source(source, case["directions"].as_u64().unwrap() as usize);
            assert_eq!(
                parsed.is_ok(),
                case["accepted"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
}
