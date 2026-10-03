//! Shared, synchronous host facade. All durable changes use the live session writer.
use super::{admission::*, commands::*, error::*, records::*, store::*, types::*};
use davinci_agent::Agent;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DesignRequest {
    List {},
    Create {
        input: CreateDesign,
    },
    Generate {
        request: super::generation::GenerationRequest,
    },
    DraftImplementation {
        request: super::handoff_draft::DraftHandoff,
    },
    PrepareImplementation {
        request: super::handoff::HandoffRequest,
    },
    Apply {
        request: super::handoff::ApproveHandoff,
    },
    VerifyImplementation {
        request: super::handoff_verification::VerifyHandoff,
    },
    Status {
        artifact_id: ArtifactId,
    },
    Read {
        artifact_id: ArtifactId,
        revision: RevisionId,
    },
    ReadBinding {
        artifact_id: ArtifactId,
        revision: RevisionId,
        node_id: NodeId,
    },
    Verify {
        artifact_id: ArtifactId,
    },
    Render {
        request: RenderRequest,
        operation_id: OperationId,
    },
    Interact {
        request: super::interaction::InteractionRequest,
    },
    Capture {
        artifact_id: ArtifactId,
        revision: RevisionId,
        artboard_id: ArtboardId,
        viewport: Viewport,
        offset: u32,
        interaction_id: Option<OperationId>,
    },
    Geometry {
        artifact_id: ArtifactId,
        revision: RevisionId,
        artboard_id: ArtboardId,
        viewport: Viewport,
        interaction_id: Option<OperationId>,
    },
    Edit {
        edit: DesignEdit,
    },
    Comment {
        comment: DesignComment,
        operation_id: OperationId,
    },
    Fork {
        artifact_id: ArtifactId,
        revision: RevisionId,
        operation_id: OperationId,
    },
    Restore {
        artifact_id: ArtifactId,
        expected_revision: RevisionId,
        revision: RevisionId,
        operation_id: OperationId,
    },
    Export {
        request: super::export::ExportRequest,
    },
    Accept {
        request: super::acceptance::AcceptRequest,
    },
    Sync {
        path: String,
        operation_id: OperationId,
    },
}
impl DesignRequest {
    fn authority(&self) -> &'static str {
        match self {
            Self::List {}
            | Self::Status { .. }
            | Self::Read { .. }
            | Self::ReadBinding { .. }
            | Self::Capture { .. }
            | Self::Geometry { .. } => "design_read",
            Self::Create { .. } | Self::Fork { .. } => "design_create",
            Self::Generate { .. } => "design_generate",
            Self::DraftImplementation { .. } | Self::PrepareImplementation { .. } => {
                "design_handoff"
            }
            Self::VerifyImplementation { .. } => "design_check",
            Self::Apply { .. } => "design_apply",
            Self::Edit { .. } | Self::Comment { .. } | Self::Restore { .. } => "design_patch",
            Self::Verify { .. } | Self::Render { .. } | Self::Interact { .. } => "design_render",
            Self::Export { .. } => "design_export",
            Self::Accept { .. } => "design_accept",
            Self::Sync { .. } => "design_sync",
        }
    }
}

#[derive(Clone)]
pub struct DesignController {
    pub store: DesignStore,
    enabled: bool,
}
impl DesignController {
    pub fn new(store: DesignStore, enabled: bool) -> Self {
        Self { store, enabled }
    }
    pub fn execute(
        &self,
        agent: &mut Agent,
        workspace: &Path,
        request: DesignRequest,
    ) -> DesignResult<Value> {
        self.execute_cancellable(agent, workspace, request, None)
    }
    fn execute_cancellable(
        &self,
        agent: &mut Agent,
        workspace: &Path,
        request: DesignRequest,
        cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> DesignResult<Value> {
        // RPC and CLI operations inherit the active host abort flag. A browser
        // operation may supply its own narrower cancellation scope explicitly.
        let cancel = cancel.or_else(|| agent.abort_signal.clone());
        if cancel
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
        {
            return Err(DesignError::Cancelled);
        }
        if !self.enabled
            && !matches!(
                &request,
                DesignRequest::List {}
                    | DesignRequest::Status { .. }
                    | DesignRequest::Read { .. }
                    | DesignRequest::Export {
                        request: super::export::ExportRequest {
                            format: ExportFormat::Source,
                            ..
                        }
                    }
            )
        {
            return Err(DesignError::MissingCapability(
                "design artifacts are disabled; existing source remains readable and exportable"
                    .into(),
            ));
        }
        if let DesignRequest::Generate { request } = request {
            let mut model = super::model::SubscriptionModel::configured(agent)?;
            let mut renderer = super::generation::ConfinedRenderer {
                runtime: super::runtime::TrustedDesignRuntime::configured(workspace)?,
            };
            return Ok(serde_json::to_value(super::generation::run(
                &self.store,
                agent,
                workspace,
                request,
                &mut model,
                &mut renderer,
                cancel,
            )?)?);
        }
        if let DesignRequest::DraftImplementation { request } = request {
            let mut model = super::model::SubscriptionModel::configured(agent)?;
            return Ok(serde_json::to_value(super::handoff_draft::draft_handoff(
                &self.store,
                agent,
                workspace,
                request,
                &mut model,
                cancel,
            )?)?);
        }
        if let DesignRequest::PrepareImplementation { request } = request {
            return Ok(serde_json::to_value(super::handoff::prepare_handoff(
                &self.store,
                agent,
                workspace,
                request,
            )?)?);
        }
        if let DesignRequest::Apply { request } = request {
            return super::handoff::approve_handoff(&self.store, agent, workspace, request);
        }
        if let DesignRequest::VerifyImplementation { request } = request {
            let directory = davinci_session::default_agent_dir();
            let mut config = crate::settings::load_settings(&directory)
                .browser_verification
                .unwrap_or_default();
            // Project settings may disable this owner, never select its executable.
            if crate::settings::load_merged_settings(&directory, workspace)
                .browser_verification
                .as_ref()
                .is_some_and(|settings| !settings.enabled)
            {
                config.enabled = false;
            }
            return super::handoff_verification::verify_handoff(
                &self.store,
                agent,
                workspace,
                request,
                config,
                cancel,
            );
        }
        let args = serde_json::to_value(&request)?;
        let ctx =
            AuthorizedDesignContext::for_operation(agent, workspace, request.authority(), &args)?;
        let ctx = if let Some(flag) = cancel {
            ctx.with_cancellation(flag)
        } else {
            ctx
        };
        ctx.check(request.authority(), &args)?;
        let session = agent
            .session
            .as_mut()
            .ok_or_else(|| DesignError::MissingCapability("persistent session required".into()))?;
        match request {
            DesignRequest::Generate { .. }
            | DesignRequest::DraftImplementation { .. }
            | DesignRequest::PrepareImplementation { .. }
            | DesignRequest::VerifyImplementation { .. }
            | DesignRequest::Apply { .. } => {
                unreachable!("agent operations handled before borrowing the session writer")
            }
            DesignRequest::List {} => Ok(serde_json::to_value(self.store.list(&ctx, session)?)?),
            DesignRequest::Create { input } => Ok(serde_json::to_value(
                self.store.create(&ctx, session, input)?,
            )?),
            DesignRequest::Read {
                artifact_id,
                revision,
            } => Ok(json!({
                "revision": self.store.read_revision(&ctx, session, artifact_id, revision)?,
                "files": self.store.read_sources(&ctx, session, artifact_id, revision)?,
            })),
            DesignRequest::ReadBinding {
                artifact_id,
                revision,
                node_id,
            } => {
                let revision = self
                    .store
                    .read_revision(&ctx, session, artifact_id, revision)?;
                let binding = revision
                    .bindings
                    .iter()
                    .find(|binding| binding.node_id == node_id)
                    .ok_or_else(|| DesignError::NotFound("editable node missing".into()))?;
                let files =
                    self.store
                        .read_sources(&ctx, session, artifact_id, revision.revision)?;
                let file: Value = serde_json::from_str(&files[&binding.source_file])?;
                let value = file
                    .pointer(&binding.pointer)
                    .ok_or_else(|| DesignError::Conflict("binding path missing".into()))?;
                Ok(
                    json!({"binding_hash":digest(binding)?,"affected_nodes":binding.affected_nodes,"value":value}),
                )
            }
            DesignRequest::Verify { artifact_id } => {
                let runtime = super::runtime::TrustedDesignRuntime::configured(workspace)?;
                let head = self.store.manifest(&ctx, session, artifact_id)?;
                let revision =
                    self.store
                        .read_revision(&ctx, session, artifact_id, head.revision)?;
                let mut receipts = Vec::new();
                let mut budget = DesignLimits::default().max_render_bytes;
                for board in revision
                    .variants
                    .iter()
                    .flat_map(|variant| &variant.artboards)
                {
                    for viewport in Viewport::defaults() {
                        receipts.push(super::render::capture_with_budget(
                            &self.store,
                            &ctx,
                            session,
                            &runtime,
                            RenderRequest {
                                artifact_id,
                                revision: head.revision,
                                artboard_id: board.id,
                                viewport,
                                theme: Theme::Light,
                                fixture: "default".into(),
                                reduced_motion: true,
                            },
                            OperationId::new(),
                            &mut budget,
                        )?);
                    }
                }
                Ok(serde_json::to_value(super::quality::quality_report(
                    &revision,
                    &receipts,
                    runtime.fingerprint(),
                    &super::render::policy_hash(),
                )?)?)
            }
            DesignRequest::Render {
                request,
                operation_id,
            } => {
                let runtime = super::runtime::TrustedDesignRuntime::configured(workspace)?;
                Ok(serde_json::to_value(super::render::capture(
                    &self.store,
                    &ctx,
                    session,
                    &runtime,
                    request,
                    operation_id,
                )?)?)
            }
            DesignRequest::Interact { request } => {
                let runtime = super::runtime::TrustedDesignRuntime::configured(workspace)?;
                Ok(serde_json::to_value(super::interaction::interact(
                    &self.store,
                    &ctx,
                    session,
                    &runtime,
                    request,
                )?)?)
            }
            DesignRequest::Capture {
                artifact_id,
                revision,
                artboard_id,
                viewport,
                offset,
                interaction_id,
            } => {
                use base64::Engine;
                let receipts = if let Some(id) = interaction_id {
                    vec![super::interaction::load(&self.store, &ctx, session, id)?.capture]
                } else {
                    self.store.receipts(&ctx, session, artifact_id, revision)?
                };
                let receipt = receipts
                    .iter()
                    .find(|receipt| {
                        receipt.request.artifact_id == artifact_id
                            && receipt.request.revision == revision
                            && receipt.request.artboard_id == artboard_id
                            && receipt.request.viewport == viewport
                    })
                    .ok_or_else(|| DesignError::NotFound("capture missing".into()))?;
                let bytes = self.store.read_blob(&ctx, &receipt.screenshot)?;
                let start = offset as usize;
                if start > bytes.len() || bytes.len() > 4 * 1024 * 1024 {
                    return Err(DesignError::InvalidInput("capture range".into()));
                }
                let end = start.saturating_add(64 * 1024).min(bytes.len());
                Ok(
                    json!({"bytes":base64::engine::general_purpose::STANDARD.encode(&bytes[start..end]),
                    "next":end,"total":bytes.len(),"sha256":receipt.screenshot.sha256}),
                )
            }
            DesignRequest::Geometry {
                artifact_id,
                revision,
                artboard_id,
                viewport,
                interaction_id,
            } => {
                let receipts = if let Some(id) = interaction_id {
                    vec![super::interaction::load(&self.store, &ctx, session, id)?.capture]
                } else {
                    self.store.receipts(&ctx, session, artifact_id, revision)?
                };
                let receipt = receipts
                    .iter()
                    .find(|receipt| {
                        receipt.request.artifact_id == artifact_id
                            && receipt.request.revision == revision
                            && receipt.request.artboard_id == artboard_id
                            && receipt.request.viewport == viewport
                    })
                    .ok_or_else(|| DesignError::NotFound("capture missing".into()))?;
                Ok(serde_json::from_slice(
                    &self.store.read_blob(&ctx, &receipt.geometry)?,
                )?)
            }
            DesignRequest::Status { artifact_id } => {
                let manifest = self.store.manifest(&ctx, session, artifact_id)?;
                let revision = if manifest.revision.0 == 0 {
                    None
                } else {
                    Some(
                        self.store
                            .read_revision(&ctx, session, artifact_id, manifest.revision)?,
                    )
                };
                let receipts = if revision.is_some() {
                    self.store
                        .receipts(&ctx, session, artifact_id, manifest.revision)?
                } else {
                    vec![]
                };
                let runtime_result = super::runtime::TrustedDesignRuntime::configured(workspace);
                let runtime_error = runtime_result.as_ref().err().map(ToString::to_string);
                let runtime = runtime_result.ok();
                let quality = revision
                    .as_ref()
                    .map(|r| {
                        super::quality::quality_report(
                            r,
                            &receipts,
                            runtime.as_ref().map_or("", |runtime| runtime.fingerprint()),
                            &super::render::policy_hash(),
                        )
                    })
                    .transpose()?;
                let comments = if revision.is_some() {
                    self.store.comments(&ctx, session, artifact_id)?
                } else {
                    vec![]
                };
                let captures: Vec<_> = receipts
                    .iter()
                    .map(|receipt| {
                        json!({"artboard_id":receipt.request.artboard_id,
                    "viewport":receipt.request.viewport,"sha256":receipt.screenshot.sha256})
                    })
                    .collect();
                let evidence_hash = quality.as_ref().map(digest).transpose()?;
                let acceptance_gaps = quality
                    .as_ref()
                    .and_then(|report| super::acceptance::acceptance_gaps(report).ok());
                let accepted = revision.as_ref().is_some_and(|revision| {
                    super::acceptance::accepted_revision(
                        &self.store,
                        &ctx,
                        session,
                        artifact_id,
                        revision.revision,
                    )
                    .is_ok()
                });
                Ok(
                    json!({"manifest":manifest,"revision":revision,"quality":quality,"comments":comments,"captures":captures,
                    "runtime_error":runtime_error,"evidence_hash":evidence_hash,"acceptance_gaps":acceptance_gaps,"accepted":accepted}),
                )
            }
            DesignRequest::Edit { edit } => Ok(serde_json::to_value(super::edits::apply_edit(
                &self.store,
                &ctx,
                session,
                edit,
            )?)?),
            DesignRequest::Comment {
                comment,
                operation_id,
            } => {
                self.store
                    .add_comment(&ctx, session, comment, operation_id)?;
                Ok(json!({"saved":true}))
            }
            DesignRequest::Fork {
                artifact_id,
                revision,
                operation_id,
            } => Ok(serde_json::to_value(self.store.fork(
                &ctx,
                session,
                artifact_id,
                revision,
                operation_id,
            )?)?),
            DesignRequest::Restore {
                artifact_id,
                expected_revision,
                revision,
                operation_id,
            } => Ok(serde_json::to_value(self.store.restore(
                &ctx,
                session,
                artifact_id,
                expected_revision,
                revision,
                operation_id,
            )?)?),
            DesignRequest::Export { request } => {
                let runtime = if request.format == ExportFormat::Source {
                    None
                } else {
                    Some(super::runtime::TrustedDesignRuntime::configured(workspace)?)
                };
                Ok(serde_json::to_value(super::export::export_revision(
                    &self.store,
                    &ctx,
                    session,
                    &request,
                    runtime.as_ref(),
                )?)?)
            }
            DesignRequest::Accept { request } => {
                let runtime = super::runtime::TrustedDesignRuntime::configured(workspace)?;
                Ok(serde_json::to_value(super::acceptance::accept_revision(
                    &self.store,
                    &ctx,
                    session,
                    &runtime,
                    request,
                )?)?)
            }
            DesignRequest::Sync { path, operation_id } => {
                self.store
                    .prepare(&ctx, session, "design_sync", &json!({"path":path}))?;
                let payload_hash = digest(&json!({"path":path}))?;
                if let Some(change) =
                    self.store
                        .retry(&ctx, session, operation_id, &payload_hash)?
                {
                    return match change {
                        super::events::DesignChange::SystemSynced { snapshot, .. } => Ok(
                            serde_json::from_slice(&self.store.read_blob(&ctx, &snapshot)?)?,
                        ),
                        _ => Err(DesignError::Conflict("operation type mismatch".into())),
                    };
                }
                let snapshot = super::sync::extract_system_at(&ctx, &path)?;
                let reference =
                    self.store
                        .retain(&ctx, "application/json", &serde_json::to_vec(&snapshot)?)?;
                self.store.publish(
                    &ctx,
                    session,
                    operation_id,
                    payload_hash,
                    super::events::DesignChange::SystemSynced {
                        snapshot: reference,
                        path,
                    },
                )?;
                Ok(serde_json::to_value(snapshot)?)
            }
        }
    }
    pub fn command(
        &self,
        agent: &mut Agent,
        workspace: &Path,
        command: DesignCommand,
    ) -> DesignResult<Value> {
        let request = match command {
            DesignCommand::List {} => DesignRequest::List {},
            DesignCommand::Create { input } => {
                // Validate the selected subscription route before creating a draft.
                super::model::SubscriptionModel::configured(agent)?;
                let brief = input.brief.clone();
                let manifest: ArtifactManifest = serde_json::from_value(self.execute(
                    agent,
                    workspace,
                    DesignRequest::Create { input },
                )?)?;
                DesignRequest::Generate {
                    request: super::generation::GenerationRequest {
                        artifact_id: manifest.id,
                        expected_revision: manifest.revision,
                        brief,
                        operation_id: OperationId::new(),
                    },
                }
            }
            DesignCommand::Revise {
                artifact_id,
                brief,
                operation_id,
            } => {
                let value =
                    self.execute(agent, workspace, DesignRequest::Status { artifact_id })?;
                let manifest: ArtifactManifest = serde_json::from_value(value["manifest"].clone())?;
                DesignRequest::Generate {
                    request: super::generation::GenerationRequest {
                        artifact_id,
                        expected_revision: manifest.revision,
                        brief,
                        operation_id,
                    },
                }
            }
            DesignCommand::Open { artifact_id } => {
                return self.serve(agent, workspace, artifact_id)
            }
            DesignCommand::Status { artifact_id } => DesignRequest::Status { artifact_id },
            DesignCommand::Verify { artifact_id } => DesignRequest::Verify { artifact_id },
            DesignCommand::Sync { path } => DesignRequest::Sync {
                path,
                operation_id: OperationId::new(),
            },
            DesignCommand::Export {
                artifact_id,
                revision,
                format,
                destination,
                artboard_id,
                viewport,
            } => DesignRequest::Export {
                request: super::export::ExportRequest {
                    artifact_id,
                    revision,
                    format,
                    destination,
                    artboard_id,
                    viewport,
                    operation_id: OperationId::new(),
                },
            },
            DesignCommand::Fork {
                artifact_id,
                revision,
                operation_id,
            } => DesignRequest::Fork {
                artifact_id,
                revision,
                operation_id,
            },
            DesignCommand::Restore {
                artifact_id,
                revision,
                operation_id,
            } => {
                let status =
                    self.execute(agent, workspace, DesignRequest::Status { artifact_id })?;
                let manifest: ArtifactManifest =
                    serde_json::from_value(status["manifest"].clone())?;
                DesignRequest::Restore {
                    artifact_id,
                    expected_revision: manifest.revision,
                    revision,
                    operation_id,
                }
            }
            DesignCommand::Apply {
                artifact_id,
                revision,
            } => {
                let ctx = AuthorizedDesignContext::for_operation(
                    agent,
                    workspace,
                    "design_handoff",
                    &json!({"artifact_id":artifact_id,"revision":revision}),
                )?;
                let session = agent.session.as_ref().ok_or_else(|| {
                    DesignError::MissingCapability("persistent session required".into())
                })?;
                super::acceptance::accepted_revision(
                    &self.store,
                    &ctx,
                    session,
                    artifact_id,
                    revision,
                )?;
                // Select concrete target paths and review the exact proposed patch
                // in the trusted workspace before the separate approval action.
                return self.serve(agent, workspace, artifact_id);
            }
        };
        self.execute(agent, workspace, request)
    }
    /// The foreground host owns the only session writer for the life of the workspace.
    pub fn serve(
        &self,
        agent: &mut Agent,
        workspace: &Path,
        artifact_id: ArtifactId,
    ) -> DesignResult<Value> {
        use std::time::{Duration, Instant};
        self.execute(agent, workspace, DesignRequest::Status { artifact_id })?;
        let ctx = AuthorizedDesignContext::for_operation(
            agent,
            workspace,
            "design_host",
            &json!({"artifact_id":artifact_id}),
        )?;
        let runtime = super::runtime::TrustedDesignRuntime::configured(workspace)?;
        let mut host = super::host::DesignHostLease::start(&ctx, &runtime)?;
        let handshake_deadline = Instant::now() + Duration::from_secs(15);
        let lease_deadline = Instant::now() + Duration::from_secs(8 * 60 * 60);
        let mut opened = false;
        loop {
            ctx.check("design_host", &json!({"artifact_id":artifact_id}))?;
            if Instant::now() >= lease_deadline {
                return Err(DesignError::Cancelled);
            }
            if !opened && Instant::now() >= handshake_deadline {
                return Err(DesignError::MissingCapability(
                    "design host did not start".into(),
                ));
            }
            let Some(message) = host.receive(Duration::from_millis(50))? else {
                continue;
            };
            let id = message["id"]
                .as_str()
                .ok_or_else(|| DesignError::InvalidInput("host ID missing".into()))?;
            if id == "ready" {
                let url = message["result"]["url"]
                    .as_str()
                    .ok_or_else(|| DesignError::InvalidInput("pairing URL missing".into()))?;
                // The capability is opened directly and never written to session, logs, or messages.
                if !url.starts_with("http://127.0.0.1:") || !url.contains("/#pair=") {
                    return Err(DesignError::Denied("invalid pairing URL".into()));
                }
                let _ = davinci_tui::open_browser(url);
                opened = true;
                continue;
            }
            if message["operation"] == "close" {
                host.respond(id, Ok(json!({"closed":true})))?;
                return Ok(json!({"closed":true}));
            }
            let request = serde_json::from_value::<DesignRequest>(
                json!({"operation":message["operation"],"payload":message["payload"]}),
            )
            .map_err(DesignError::from);
            let cancellation = host.cancellation(id)?;
            let result = request.and_then(|request| {
                self.execute_cancellable(agent, workspace, request, Some(cancellation))
            });
            host.respond(id, result)?;
        }
    }
}
