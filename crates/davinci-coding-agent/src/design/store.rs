use super::blob::Blobs;
use super::{admission::AuthorizedDesignContext, error::*, events::*, records::*, types::*};
use davinci_session::{branch_entries, custom_entry, JsonlSession};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone)]
pub struct DesignStore {
    root: PathBuf,
    limits: DesignLimits,
}
pub fn digest(value: &impl Serialize) -> DesignResult<String> {
    Ok(format!("{:x}", davinci_sys::hex::Lower(&Sha256::digest(serde_json::to_vec(value)?))))
}

impl DesignStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            limits: DesignLimits::default(),
        }
    }
    pub fn blob_directory(&self, ctx: &AuthorizedDesignContext) -> PathBuf {
        self.root
            .join(ctx.workspace_id())
            .join(ctx.session_id())
            .join("blobs")
    }
    fn blobs(&self, ctx: &AuthorizedDesignContext) -> Blobs {
        Blobs::new(self.blob_directory(ctx))
    }
    pub(crate) fn events(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &JsonlSession,
    ) -> DesignResult<Vec<DesignEvent>> {
        ctx.check_session(session)?;
        let mut events = Vec::new();
        for entry in branch_entries(&session.entries, session.leaf_id.as_deref()) {
            if entry.custom_type.as_deref() != Some(CUSTOM_TYPE) {
                continue;
            }
            let event: DesignEvent = serde_json::from_value(
                entry
                    .extra
                    .get("data")
                    .cloned()
                    .ok_or_else(|| DesignError::CorruptArtifact("missing event data".into()))?,
            )
            .map_err(|_| DesignError::CorruptArtifact("invalid design event".into()))?;
            if event.owner_session != ctx.session_id() || event.workspace != ctx.workspace_id() {
                return Err(DesignError::Denied(
                    "design event owner mismatch; fork explicitly before reuse".into(),
                ));
            }
            events.push(event);
        }
        Ok(events)
    }
    pub(crate) fn prepare(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        operation: &str,
        args: &impl Serialize,
    ) -> DesignResult<()> {
        ctx.check_session(session)?;
        ctx.check(operation, &serde_json::to_value(args)?)?;
        session
            .prepare_mutation()
            .map_err(|e| DesignError::Conflict(e.to_string()))?;
        ctx.check_session(session)
    }
    pub(crate) fn retry(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &JsonlSession,
        op: OperationId,
        input_digest: &str,
    ) -> DesignResult<Option<DesignChange>> {
        for event in self.events(ctx, session)? {
            if event.operation_id == op {
                return if event.payload_digest == input_digest {
                    Ok(Some(event.change))
                } else {
                    Err(DesignError::Conflict(
                        "operation ID reused with a different payload".into(),
                    ))
                };
            }
        }
        Ok(None)
    }
    pub(crate) fn publish(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        op: OperationId,
        input_digest: String,
        change: DesignChange,
    ) -> DesignResult<()> {
        ctx.check_session(session)?;
        super::inventory::check_artifact(
            self,
            ctx,
            session,
            &change,
            self.limits.max_artifact_bytes,
        )?;
        let event = DesignEvent {
            schema_version: SchemaVersion,
            owner_session: ctx.session_id().into(),
            workspace: ctx.workspace_id().into(),
            operation_id: op,
            payload_digest: input_digest,
            change,
        };
        let id = op.to_string();
        let parent = session.leaf_id.clone();
        #[cfg(test)]
        super::publication_tests::checkpoint("before_event")?;
        session
            .append_entry_once(
                &id,
                parent.as_deref(),
                custom_entry(&id, CUSTOM_TYPE, serde_json::to_value(event)?),
            )
            .map_err(|e| DesignError::IoFailure(e.to_string()))?;
        #[cfg(test)]
        super::publication_tests::checkpoint("after_event")?;
        Ok(())
    }
    fn quota(&self, ctx: &AuthorizedDesignContext, additional: u64) -> DesignResult<()> {
        let dir = self.blob_directory(ctx);
        let mut used = 0u64;
        if dir.exists() {
            super::runtime::no_links(&dir)?;
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let metadata = entry.path().symlink_metadata()?;
                super::runtime::no_links(&entry.path())?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(DesignError::CorruptArtifact(
                        "unexpected blob store entry".into(),
                    ));
                }
                used = used
                    .checked_add(metadata.len())
                    .ok_or_else(|| DesignError::BudgetExceeded("storage overflow".into()))?;
            }
        }
        if used.saturating_add(additional) > self.limits.max_task_bytes {
            return Err(DesignError::BudgetExceeded(
                "task artifact storage limit".into(),
            ));
        }
        Ok(())
    }
    pub fn create(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        input: CreateDesign,
    ) -> DesignResult<ArtifactManifest> {
        self.prepare(ctx, session, "design_create", &input)?;
        validate_text(&input.title, 256, "title")?;
        validate_text(&input.brief, 64 * 1024, "brief")?;
        if input.variants == 0 || input.variants > self.limits.max_variants {
            return Err(DesignError::InvalidInput("variants must be 1..3".into()));
        }
        let input_digest = digest(&input)?;
        if let Some(change) = self.retry(ctx, session, input.operation_id, &input_digest)? {
            return match change {
                DesignChange::Created { manifest } => Ok(manifest),
                _ => Err(DesignError::Conflict("operation type mismatch".into())),
            };
        }
        self.quota(ctx, input.brief.len() as u64)?;
        let brief = self
            .blobs(ctx)
            .store_artifact("text/plain", input.brief.as_bytes())
            .map_err(DesignError::IoFailure)?;
        let manifest = ArtifactManifest {
            schema_version: SchemaVersion,
            id: ArtifactId::new(),
            owner_session: ctx.session_id().into(),
            workspace: ctx.workspace_id().into(),
            branch_entry: session.leaf_id.clone(),
            title: input.title,
            kind: input.kind,
            brief,
            requested_variants: input.variants,
            created_at_ms: davinci_session::now_ms(),
            revision: RevisionId(0),
            ancestry: None,
        };
        ctx.check(
            "design_create",
            &serde_json::json!({"artifact_id":manifest.id}),
        )?;
        self.publish(
            ctx,
            session,
            input.operation_id,
            input_digest,
            DesignChange::Created {
                manifest: manifest.clone(),
            },
        )?;
        Ok(manifest)
    }
    pub fn list(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &JsonlSession,
    ) -> DesignResult<Vec<ArtifactManifest>> {
        ctx.check("design_read", &serde_json::json!({}))?;
        let mut manifests = BTreeMap::<ArtifactId, ArtifactManifest>::new();
        for event in self.events(ctx, session)? {
            match event.change {
                DesignChange::Created { manifest } | DesignChange::Forked { manifest, .. } => {
                    if manifests.insert(manifest.id, manifest).is_some() {
                        return Err(DesignError::CorruptArtifact(
                            "duplicate artifact creation".into(),
                        ));
                    }
                }
                DesignChange::Revision {
                    artifact_id,
                    revision,
                    ..
                } => {
                    let head = manifests.get_mut(&artifact_id).ok_or_else(|| {
                        DesignError::CorruptArtifact("revision without artifact".into())
                    })?;
                    if head.revision.next()? != revision {
                        return Err(DesignError::CorruptArtifact(
                            "nonconsecutive revision".into(),
                        ));
                    }
                    head.revision = revision;
                }
                _ => {}
            }
        }
        Ok(manifests.into_values().collect())
    }
    pub fn manifest(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &JsonlSession,
        id: ArtifactId,
    ) -> DesignResult<ArtifactManifest> {
        self.list(ctx, session)?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| DesignError::NotFound("artifact is not on this session branch".into()))
    }
    /// Store bounded text files only after admission. These blobs do not publish a revision.
    pub fn store_sources(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        files: &BTreeMap<String, String>,
        entry_points: Vec<String>,
    ) -> DesignResult<SourceBundle> {
        self.prepare(
            ctx,
            session,
            "design_patch",
            &serde_json::json!({"paths":files.keys().collect::<Vec<_>>()}),
        )?;
        let refs = files
            .iter()
            .map(|(path, text)| {
                let sha256 = format!("{:x}", davinci_sys::hex::Lower(&Sha256::digest(text.as_bytes())));
                (
                    path.clone(),
                    ArtifactRef {
                        id: format!("artifact_{}", &sha256[..12]),
                        relative_store_path: format!("{sha256}.bin"),
                        sha256,
                        media_type: "text/plain".into(),
                        size: text.len() as u64,
                        redaction: None,
                    },
                )
            })
            .collect();
        let bundle = SourceBundle {
            files: refs,
            entry_points,
        };
        validate_bundle(&bundle, &self.limits)?;
        self.quota(ctx, bundle.files.values().map(|r| r.size).sum())?;
        for source in files.values() {
            self.blobs(ctx)
                .store_artifact("text/plain", source.as_bytes())
                .map_err(DesignError::IoFailure)?;
        }
        Ok(bundle)
    }
    fn verify_sources(
        &self,
        ctx: &AuthorizedDesignContext,
        sources: &SourceBundle,
    ) -> DesignResult<()> {
        validate_bundle(sources, &self.limits)?;
        for source in sources.files.values() {
            self.blobs(ctx)
                .get_artifact(source)
                .map_err(DesignError::CorruptArtifact)?;
        }
        Ok(())
    }
    pub fn commit_revision(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        write: RevisionWrite,
    ) -> DesignResult<DesignRevision> {
        self.commit(ctx, session, write, None)
    }
    fn commit(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        write: RevisionWrite,
        restored_from: Option<RevisionId>,
    ) -> DesignResult<DesignRevision> {
        self.prepare(ctx, session, "design_patch", &write)?;
        let input_digest = digest(&(&write, restored_from))?;
        if let Some(change) = self.retry(ctx, session, write.operation_id, &input_digest)? {
            return match change {
                DesignChange::Revision {
                    artifact_id,
                    revision,
                    ..
                } => self.read_revision(ctx, session, artifact_id, revision),
                _ => Err(DesignError::Conflict("operation type mismatch".into())),
            };
        }
        let head = self.manifest(ctx, session, write.artifact_id)?;
        if head.revision != write.expected_revision {
            return Err(DesignError::Conflict(format!(
                "current revision is {}",
                head.revision
            )));
        }
        self.verify_sources(ctx, &write.sources)?;
        validate_variants(&write.variants, &write.sources, &self.limits)?;
        super::edits::validate_bindings(&write)?;
        for binding in &write.bindings {
            let bytes = self.read_blob(ctx, &write.sources.files[&binding.source_file])?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            let selected = value.pointer(&binding.pointer).ok_or_else(|| {
                DesignError::InvalidInput("declared binding pointer is missing".into())
            })?;
            super::edits::validate_value(binding, selected)?;
        }
        if write.assets.len() > 20 || write.profile_refs.len() > 16 || write.bindings.len() > 1024 {
            return Err(DesignError::BudgetExceeded(
                "revision metadata limit".into(),
            ));
        }
        let mut asset_bytes = 0u64;
        let mut asset_paths = std::collections::BTreeSet::new();
        for asset in &write.assets {
            validate_path(&asset.path)?;
            if !asset_paths.insert(asset.path.to_ascii_lowercase())
                || write
                    .sources
                    .files
                    .keys()
                    .any(|path| path.eq_ignore_ascii_case(&asset.path))
            {
                return Err(DesignError::InvalidInput(
                    "asset path collides with source or another asset".into(),
                ));
            }
            validate_text(&asset.provenance, 4096, "asset provenance")?;
            validate_text(&asset.rights, 4096, "asset rights")?;
            if asset.source.size > 10 * 1024 * 1024
                || u64::from(asset.width) * u64::from(asset.height) > 16_000_000
            {
                return Err(DesignError::BudgetExceeded("asset size limit".into()));
            }
            asset_bytes = asset_bytes.saturating_add(asset.source.size);
            let bytes = self
                .blobs(ctx)
                .get_artifact(&asset.source)
                .map_err(DesignError::CorruptArtifact)?;
            super::assets::validate_image(
                &bytes,
                &asset.source.media_type,
                asset.width,
                asset.height,
            )?;
        }
        if asset_bytes > self.limits.max_artifact_bytes {
            return Err(DesignError::BudgetExceeded("artifact asset budget".into()));
        }
        for reference in write
            .profile_refs
            .iter()
            .chain(write.system_snapshot.iter())
        {
            if reference.size > self.limits.max_source_bytes {
                return Err(DesignError::BudgetExceeded("snapshot size".into()));
            }
            self.blobs(ctx)
                .get_artifact(reference)
                .map_err(DesignError::CorruptArtifact)?;
        }
        let revision = DesignRevision {
            schema_version: SchemaVersion,
            artifact_id: write.artifact_id,
            revision: head.revision.next()?,
            parent: head.revision,
            operation_id: write.operation_id,
            source_hash: super::assets::source_digest(&write.sources, &write.assets)?,
            sources: write.sources,
            variants: write.variants,
            bindings: write.bindings,
            assets: write.assets,
            profile_refs: write.profile_refs,
            system_snapshot: write.system_snapshot,
            restored_from,
        };
        let data = serde_json::to_vec(&revision)?;
        self.quota(ctx, data.len() as u64)?;
        let manifest = self
            .blobs(ctx)
            .store_artifact("application/json", &data)
            .map_err(DesignError::IoFailure)?;
        ctx.check(
            "design_patch",
            &serde_json::json!({"artifact_id":revision.artifact_id}),
        )?;
        self.publish(
            ctx,
            session,
            revision.operation_id,
            input_digest,
            DesignChange::Revision {
                artifact_id: revision.artifact_id,
                revision: revision.revision,
                manifest,
            },
        )?;
        Ok(revision)
    }
    pub fn read_revision(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &JsonlSession,
        artifact_id: ArtifactId,
        revision: RevisionId,
    ) -> DesignResult<DesignRevision> {
        self.manifest(ctx, session, artifact_id)?;
        for event in self.events(ctx, session)? {
            let stored = match event.change {
                DesignChange::Revision {
                    artifact_id,
                    revision,
                    manifest,
                } => Some((artifact_id, revision, manifest)),
                DesignChange::Forked {
                    manifest,
                    revision_manifest,
                } => Some((manifest.id, manifest.revision, revision_manifest)),
                _ => None,
            };
            if let Some((id, rev, manifest)) = stored {
                if id != artifact_id || rev != revision {
                    continue;
                }
                if manifest.size > self.limits.max_source_bytes {
                    return Err(DesignError::CorruptArtifact(
                        "oversize revision metadata".into(),
                    ));
                }
                let bytes = self
                    .blobs(ctx)
                    .get_artifact(&manifest)
                    .map_err(DesignError::CorruptArtifact)?;
                let value: DesignRevision = serde_json::from_slice(&bytes)
                    .map_err(|e| DesignError::CorruptArtifact(e.to_string()))?;
                if value.artifact_id != artifact_id
                    || value.revision != revision
                    || super::assets::source_digest(&value.sources, &value.assets)?
                        != value.source_hash
                {
                    return Err(DesignError::CorruptArtifact(
                        "revision metadata mismatch".into(),
                    ));
                }
                self.verify_sources(ctx, &value.sources)?;
                return Ok(value);
            }
        }
        Err(DesignError::NotFound(
            "revision is not committed on this branch".into(),
        ))
    }
    pub fn read_sources(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &JsonlSession,
        id: ArtifactId,
        revision: RevisionId,
    ) -> DesignResult<BTreeMap<String, String>> {
        let revision = self.read_revision(ctx, session, id, revision)?;
        revision
            .sources
            .files
            .into_iter()
            .map(|(path, reference)| {
                let bytes = self
                    .blobs(ctx)
                    .get_artifact(&reference)
                    .map_err(DesignError::CorruptArtifact)?;
                let text = String::from_utf8(bytes)
                    .map_err(|_| DesignError::CorruptArtifact("source is not UTF-8".into()))?;
                Ok((path, text))
            })
            .collect()
    }
    pub fn restore(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        id: ArtifactId,
        expected: RevisionId,
        from: RevisionId,
        operation_id: OperationId,
    ) -> DesignResult<DesignRevision> {
        let old = self.read_revision(ctx, session, id, from)?;
        self.commit(
            ctx,
            session,
            RevisionWrite {
                artifact_id: id,
                expected_revision: expected,
                operation_id,
                sources: old.sources,
                variants: old.variants,
                bindings: old.bindings,
                assets: old.assets,
                profile_refs: old.profile_refs,
                system_snapshot: old.system_snapshot,
            },
            Some(from),
        )
    }
    pub fn fork(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        id: ArtifactId,
        from: RevisionId,
        operation_id: OperationId,
    ) -> DesignResult<ArtifactManifest> {
        let input = serde_json::json!({"artifact_id":id,"revision":from,"operation_id":operation_id,"operation":"fork"});
        self.prepare(ctx, session, "design_create", &input)?;
        let input_digest = digest(&input)?;
        if let Some(change) = self.retry(ctx, session, operation_id, &input_digest)? {
            return match change {
                DesignChange::Forked { manifest, .. } => Ok(manifest),
                _ => Err(DesignError::Conflict("operation type mismatch".into())),
            };
        }
        let original = self.manifest(ctx, session, id)?;
        let old = self.read_revision(ctx, session, id, from)?;
        let new_id = ArtifactId::new();
        let manifest = ArtifactManifest {
            id: new_id,
            branch_entry: session.leaf_id.clone(),
            revision: RevisionId(1),
            title: format!(
                "{} (fork)",
                original.title.chars().take(240).collect::<String>()
            ),
            created_at_ms: davinci_session::now_ms(),
            ancestry: Some(RevisionReference {
                artifact_id: id,
                revision: from,
                source_hash: old.source_hash.clone(),
            }),
            ..original
        };
        let revision = DesignRevision {
            artifact_id: new_id,
            revision: RevisionId(1),
            parent: RevisionId(0),
            operation_id,
            restored_from: None,
            ..old
        };
        let data = serde_json::to_vec(&revision)?;
        self.quota(ctx, data.len() as u64)?;
        let revision_manifest = self
            .blobs(ctx)
            .store_artifact("application/json", &data)
            .map_err(DesignError::IoFailure)?;
        self.publish(
            ctx,
            session,
            operation_id,
            input_digest,
            DesignChange::Forked {
                manifest: manifest.clone(),
                revision_manifest,
            },
        )?;
        Ok(manifest)
    }
    pub fn add_comment(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        comment: DesignComment,
        operation_id: OperationId,
    ) -> DesignResult<()> {
        self.prepare(ctx, session, "design_patch", &comment)?;
        let input_digest = digest(&comment)?;
        if self
            .retry(ctx, session, operation_id, &input_digest)?
            .is_some()
        {
            return Ok(());
        }
        validate_text(&comment.text, 8192, "comment")?;
        if comment.x_milli > 1_000_000 || comment.y_milli > 1_000_000 {
            return Err(DesignError::InvalidInput(
                "comment anchor outside artboard".into(),
            ));
        }
        let revision = self.read_revision(ctx, session, comment.artifact_id, comment.revision)?;
        if !revision
            .variants
            .iter()
            .flat_map(|v| &v.artboards)
            .any(|b| b.id == comment.artboard_id)
            || comment.node_id.is_some_and(|id| {
                !revision
                    .bindings
                    .iter()
                    .any(|b| b.node_id == id && b.artboard_id == comment.artboard_id)
            })
        {
            return Err(DesignError::InvalidInput(
                "comment anchor not present in revision".into(),
            ));
        }
        if self.events(ctx, session)?.iter().any(|event| matches!(&event.change, DesignChange::Comment { comment: old } if old.id == comment.id)) {
            return Err(DesignError::Conflict("comment ID already exists".into()));
        }
        self.publish(
            ctx,
            session,
            operation_id,
            input_digest,
            DesignChange::Comment { comment },
        )
    }
    pub fn comments(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &JsonlSession,
        id: ArtifactId,
    ) -> DesignResult<Vec<(DesignComment, bool)>> {
        let head = self.manifest(ctx, session, id)?;
        let revision = self.read_revision(ctx, session, id, head.revision)?;
        Ok(self
            .events(ctx, session)?
            .into_iter()
            .filter_map(|e| match e.change {
                DesignChange::Comment { comment } if comment.artifact_id == id => {
                    let orphaned = !revision
                        .variants
                        .iter()
                        .flat_map(|v| &v.artboards)
                        .any(|b| b.id == comment.artboard_id)
                        || comment.node_id.is_some_and(|node| {
                            !revision
                                .bindings
                                .iter()
                                .any(|b| b.node_id == node && b.artboard_id == comment.artboard_id)
                        });
                    Some((comment, orphaned))
                }
                _ => None,
            })
            .collect())
    }
    pub(crate) fn retain(
        &self,
        ctx: &AuthorizedDesignContext,
        media: &str,
        bytes: &[u8],
    ) -> DesignResult<ArtifactRef> {
        ctx.check(
            "design_render",
            &serde_json::json!({"media_type":media,"size":bytes.len()}),
        )?;
        self.quota(ctx, bytes.len() as u64)?;
        self.blobs(ctx)
            .store_artifact(media, bytes)
            .map_err(DesignError::IoFailure)
    }
    /// Explicit cleanup only, while holding the existing exclusive session writer.
    /// Every branch and its accepted/history/checkpoint references remain pinned.
    pub fn prune_unreferenced(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
    ) -> DesignResult<u64> {
        self.prepare(
            ctx,
            session,
            "design_patch",
            &serde_json::json!({"cleanup":"unreferenced_blobs"}),
        )?;
        let values = super::inventory::all_changes(ctx, session)?
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        let keep = super::inventory::closure(self, ctx, &values, self.limits.max_task_bytes)?;
        let directory = self.blob_directory(ctx);
        if !directory.exists() {
            return Ok(0);
        }
        super::runtime::no_links(&directory)?;
        let mut removed = 0u64;
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            super::runtime::no_links(&path)?;
            if !path.is_file() {
                return Err(DesignError::CorruptArtifact(
                    "unexpected blob store entry".into(),
                ));
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| DesignError::CorruptArtifact("invalid blob name".into()))?;
            let hash = name
                .strip_suffix(".bin")
                .ok_or_else(|| DesignError::CorruptArtifact("unknown blob file".into()))?;
            validate_hash(hash)?;
            if !keep.contains_key(hash) {
                ctx.check(
                    "design_patch",
                    &serde_json::json!({"cleanup":"unreferenced_blobs"}),
                )?;
                removed = removed
                    .checked_add(path.metadata()?.len())
                    .ok_or_else(|| DesignError::BudgetExceeded("cleanup size overflow".into()))?;
                std::fs::remove_file(path)?;
            }
        }
        Ok(removed)
    }
    pub(crate) fn read_blob(
        &self,
        ctx: &AuthorizedDesignContext,
        reference: &ArtifactRef,
    ) -> DesignResult<Vec<u8>> {
        self.blobs(ctx)
            .get_artifact(reference)
            .map_err(DesignError::CorruptArtifact)
    }
    pub fn receipts(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &JsonlSession,
        id: ArtifactId,
        revision: RevisionId,
    ) -> DesignResult<Vec<RenderReceipt>> {
        self.read_revision(ctx, session, id, revision)?;
        let mut receipts = BTreeMap::new();
        for event in self.events(ctx, session)? {
            if let DesignChange::Rendered { receipt } = event.change {
                if receipt.request.artifact_id == id && receipt.request.revision == revision {
                    self.read_blob(ctx, &receipt.screenshot)?;
                    self.read_blob(ctx, &receipt.geometry)?;
                    receipts.insert(digest(&receipt.request)?, receipt);
                }
            }
        }
        Ok(receipts.into_values().collect())
    }
    pub(crate) fn record_render(
        &self,
        ctx: &AuthorizedDesignContext,
        session: &mut JsonlSession,
        receipt: RenderReceipt,
        operation_id: OperationId,
    ) -> DesignResult<()> {
        self.prepare(ctx, session, "design_render", &receipt.request)?;
        let input_digest = digest(&receipt)?;
        if self
            .retry(ctx, session, operation_id, &input_digest)?
            .is_some()
        {
            return Ok(());
        }
        let head = self.manifest(ctx, session, receipt.request.artifact_id)?;
        if head.revision != receipt.request.revision {
            return Err(DesignError::Conflict(
                "design changed during rendering".into(),
            ));
        }
        let revision = self.read_revision(ctx, session, head.id, head.revision)?;
        if receipt.source_hash != revision.source_hash {
            return Err(DesignError::CorruptArtifact(
                "render source mismatch".into(),
            ));
        }
        for hash in [&receipt.runtime_hash, &receipt.policy_hash] {
            validate_hash(hash)?;
        }
        receipt.request.viewport.validate()?;
        let png = self.read_blob(ctx, &receipt.screenshot)?;
        if png.len() < 24
            || &png[..8] != b"\x89PNG\r\n\x1a\n"
            || u32::from_be_bytes(png[16..20].try_into().expect("PNG width"))
                != receipt.request.viewport.width
            || u32::from_be_bytes(png[20..24].try_into().expect("PNG height"))
                != receipt.request.viewport.height
        {
            return Err(DesignError::CorruptArtifact(
                "capture dimensions mismatch".into(),
            ));
        }
        self.read_blob(ctx, &receipt.geometry)?;
        self.publish(
            ctx,
            session,
            operation_id,
            input_digest,
            DesignChange::Rendered { receipt },
        )
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use davinci_agent::{Agent, PermissionMode};
    #[test]
    fn cumulative_artifact_quota_deduplicates_and_preserves_last_commit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut agent = Agent::new("quota fixture");
        agent.set_permission_mode(PermissionMode::AlwaysApprove);
        agent.session = Some(
            JsonlSession::create_in_directory(
                &root.join("sessions"),
                &root.to_string_lossy(),
                None,
            )
            .unwrap(),
        );
        let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
        let mut store = DesignStore::new(root.join("design"));
        store.limits.max_artifact_bytes = 6 * 1024;
        let s = agent.session.as_mut().unwrap();
        let created = store
            .create(
                &ctx,
                s,
                CreateDesign {
                    title: "Limit".into(),
                    brief: "Tiny fixture budget".into(),
                    kind: DesignKind::Product,
                    variants: 1,
                    operation_id: OperationId::new(),
                },
            )
            .unwrap();
        let make = |sources, expected| RevisionWrite {
            artifact_id: created.id,
            expected_revision: RevisionId(expected),
            operation_id: OperationId::new(),
            sources,
            variants: vec![Variant {
                id: VariantId::new(),
                title: "One".into(),
                artboards: vec![Artboard {
                    id: ArtboardId::new(),
                    title: "Home".into(),
                    entry_point: "index.html".into(),
                }],
            }],
            bindings: vec![],
            assets: vec![],
            profile_refs: vec![],
            system_snapshot: None,
        };
        let source = store
            .store_sources(
                &ctx,
                s,
                &BTreeMap::from([("index.html".into(), "a".repeat(2048))]),
                vec!["index.html".into()],
            )
            .unwrap();
        store
            .commit_revision(&ctx, s, make(source.clone(), 0))
            .unwrap();
        store.commit_revision(&ctx, s, make(source, 1)).unwrap();
        let source = store
            .store_sources(
                &ctx,
                s,
                &BTreeMap::from([("index.html".into(), "b".repeat(2048))]),
                vec!["index.html".into()],
            )
            .unwrap();
        assert!(matches!(
            store.commit_revision(&ctx, s, make(source, 2)),
            Err(DesignError::BudgetExceeded(_))
        ));
        assert_eq!(
            store.manifest(&ctx, s, created.id).unwrap().revision,
            RevisionId(2)
        );
        assert_eq!(
            store
                .read_sources(&ctx, s, created.id, RevisionId(2))
                .unwrap()["index.html"],
            "a".repeat(2048)
        );
        assert!(store.prune_unreferenced(&ctx, s).unwrap() >= 2048);
    }
    #[test]
    fn task_quota_counts_unreferenced_bytes_before_allocation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut agent = Agent::new("quota fixture");
        agent.set_permission_mode(PermissionMode::AlwaysApprove);
        agent.session = Some(
            JsonlSession::create_in_directory(
                &root.join("sessions"),
                &root.to_string_lossy(),
                None,
            )
            .unwrap(),
        );
        let ctx = AuthorizedDesignContext::from_agent(&agent, &root).unwrap();
        let mut store = DesignStore::new(root.join("design"));
        store.limits.max_task_bytes = 32;
        let s = agent.session.as_mut().unwrap();
        store
            .store_sources(
                &ctx,
                s,
                &BTreeMap::from([("index.html".into(), "a".repeat(24))]),
                vec!["index.html".into()],
            )
            .unwrap();
        assert!(matches!(
            store.store_sources(
                &ctx,
                s,
                &BTreeMap::from([("index.html".into(), "b".repeat(24))]),
                vec!["index.html".into()]
            ),
            Err(DesignError::BudgetExceeded(_))
        ));
        assert_eq!(store.prune_unreferenced(&ctx, s).unwrap(), 24);
    }
}
