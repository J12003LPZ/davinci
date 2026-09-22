mod compiler;
mod events;
mod fold;
mod metrics;
mod reducer;
mod retrieval;
mod shadow;
mod sources;
mod store;
mod types;

pub(crate) const CONTEXT_BUDGET_EXCEEDED: &str =
    "mandatory context exceeds the compilation token budget";

pub use compiler::{ContextCompileRequest, ContextCompiler};
pub use events::{
    events_from_messages, events_from_session_branch, ContextEvent, ContextEventKind,
};
pub(crate) use fold::fold_request;
pub use fold::{ContextFoldDecision, ContextFoldPolicy, FoldReason};
pub use metrics::ContextVmMetrics;
pub use reducer::{
    parse_checkpoint_proposal, CheckpointProposal, ContextStateReducer, ProposedStateValue,
    RetiredState, StateSlot, StateTransition, TransitionKind,
};
pub use retrieval::{retrieve_context_tool, RetrieveContextRequest, RetrieveContextResult};
pub use shadow::{compare_shadow_views, ShadowComparison};
pub use store::{
    ContextObject, ContextObjectStore, CONTEXT_OBJECT_ALGORITHM, CONTEXT_OBJECT_SCHEMA,
};
pub use types::{
    ArtifactRef, CheckpointState, ContextImage, ContextImageEntry, ContextPageKind, ContextPageRef,
    ContextRoot, ContextVmConfig, ContextVmMode, Episode, ProvenanceRef, StateDelta, StateValue,
};

use super::cache::{digest, CacheRuntime};
use super::context::ContextPacket;
use davinci_session::SessionEntry;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Default)]
pub struct ContextVmState {
    pub root: ContextRoot,
    pub last_source_seq: u64,
    pub last_fold_reason: Option<String>,
    pub last_fold_tokens: Option<(u64, u64)>,
    pub last_prefix_digest: Option<String>,
    pub stable_context_digest: Option<String>,
}

#[derive(Clone)]
pub struct ContextVmRuntime {
    config: ContextVmConfig,
    pub(crate) store: ContextObjectStore,
    pub(crate) state: Arc<RwLock<ContextVmState>>,
    pub(crate) events: Arc<RwLock<Vec<ContextEvent>>>,
    pub(crate) source_contents: Arc<RwLock<HashMap<String, String>>>,
    session_source: Arc<RwLock<Option<sources::SessionSource>>>,
    metrics: Arc<RwLock<ContextVmMetrics>>,
}

impl std::fmt::Debug for ContextVmRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextVmRuntime")
            .field("config", &self.config)
            .field("root", &self.root())
            .finish()
    }
}

impl ContextVmRuntime {
    pub fn new(config: ContextVmConfig, cache: CacheRuntime) -> Self {
        let metrics = Arc::new(RwLock::new(ContextVmMetrics::default()));
        Self {
            config,
            store: ContextObjectStore::with_metrics(cache, metrics.clone()),
            state: Arc::new(RwLock::new(ContextVmState::default())),
            events: Arc::new(RwLock::new(Vec::new())),
            source_contents: Arc::new(RwLock::new(HashMap::new())),
            session_source: Arc::new(RwLock::new(None)),
            metrics,
        }
    }

    pub fn config(&self) -> &ContextVmConfig {
        &self.config
    }

    pub fn set_mode(&mut self, mode: ContextVmMode) {
        self.config.mode = mode;
    }

    pub fn root(&self) -> ContextRoot {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .root
            .clone()
    }

    pub fn install_root(&self, mut root: ContextRoot, last_source_seq: u64) {
        if root.cache_namespace.is_empty() {
            let checkpoint = root
                .checkpoint
                .as_ref()
                .map(|page| page.content_hash.as_str())
                .unwrap_or("uninitialized");
            root.cache_namespace = digest(
                format!("ctxvm_cache_namespace_v2:{}:{checkpoint}", root.epoch).as_bytes(),
            );
        }
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        state.root = root;
        state.last_source_seq = last_source_seq;
    }

    pub fn events(&self) -> Vec<ContextEvent> {
        self.events
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn metrics(&self) -> ContextVmMetrics {
        self.metrics
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn last_fold_reason(&self) -> Option<String> {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .last_fold_reason
            .clone()
    }

    pub fn last_fold_tokens(&self) -> Option<(u64, u64)> {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .last_fold_tokens
    }

    pub fn prefix_digest(&self) -> Option<String> {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .last_prefix_digest
            .clone()
    }

    pub fn rebuild_from_events(&self, events: &[ContextEvent]) -> Result<ContextRoot, String> {
        self.bump_metrics(|metrics| {
            metrics.rebuild_attempts = metrics.rebuild_attempts.saturating_add(1)
        });
        let result = self.rebuild_inner(events);
        self.bump_metrics(|metrics| {
            if result.is_ok() {
                metrics.rebuilds = metrics.rebuilds.saturating_add(1);
                metrics.rebuild_successes = metrics.rebuild_successes.saturating_add(1);
            } else {
                metrics.rebuild_failures = metrics.rebuild_failures.saturating_add(1);
            }
        });
        result
    }

    fn rebuild_inner(&self, events: &[ContextEvent]) -> Result<ContextRoot, String> {
        self.record_events(events);
        let parent = CheckpointState::default();
        let delta = ContextStateReducer::deterministic_delta(&parent, events);
        let checkpoint = self
            .store
            .save(&ContextObject::Checkpoint(delta.checkpoint_patch))
            .map_err(|error| error.to_string())?;
        let previous = self.root();
        let root = ContextRoot {
            epoch: previous
                .epoch
                .saturating_add(u64::from(previous.checkpoint.is_some())),
            cache_namespace: String::new(),
            checkpoint: Some(checkpoint),
            deltas: Vec::new(),
            episodes: Vec::new(),
            hot_event_refs: self.hot_refs(events),
            evidence_refs: evidence_refs(events),
            updates_since_fold: 0,
        };
        let last_source_seq = events.iter().map(|event| event.seq).max().unwrap_or(0);
        self.install_root(root.clone(), last_source_seq);
        Ok(root)
    }

    pub fn compile(
        &self,
        events: &[ContextEvent],
        broker_packet: &ContextPacket,
        max_tokens: u64,
    ) -> Result<ContextImage, String> {
        let diverged = self.record_events(events);
        let current = self.root();
        if diverged
            || current.checkpoint.is_none()
            || current
                .hot_event_refs
                .iter()
                .any(|source| !events.iter().any(|event| &event.source_ref == source))
            || current
                .checkpoint
                .iter()
                .chain(&current.deltas)
                .chain(&current.episodes)
                .any(|page| self.store.load(page).is_err())
        {
            self.rebuild_from_events(events)?;
        }
        {
            let mut contents = self
                .source_contents
                .write()
                .unwrap_or_else(|e| e.into_inner());
            for item in &broker_packet.items {
                contents.insert(item.source.clone(), item.content.clone());
            }
        }
        let mut root = self.root();
        root.hot_event_refs = self.hot_refs(events);
        root.evidence_refs = evidence_refs(events);
        let last_source_seq = events.iter().map(|event| event.seq).max().unwrap_or(0);
        self.install_root(root.clone(), last_source_seq);
        let compiler = ContextCompiler::new(self.store.clone());
        let image = compiler.compile(ContextCompileRequest {
            root: &root,
            hot_events: events,
            broker_packet,
            max_tokens,
        })?;
        let stable_context_digest = stable_context_digest(&image);
        let prefix_changed = {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let changed = state
                .last_prefix_digest
                .as_deref()
                .is_some_and(|previous| previous != image.prefix_digest.as_str());
            state.last_prefix_digest = Some(image.prefix_digest.clone());
            state.stable_context_digest = Some(stable_context_digest);
            changed
        };
        if prefix_changed {
            self.bump_metrics(|metrics| {
                metrics.prefix_churn = metrics.prefix_churn.saturating_add(1)
            });
        }
        self.bump_metrics(|metrics| {
            metrics.images_compiled = metrics.images_compiled.saturating_add(1)
        });
        Ok(image)
    }

    pub fn append_delta(&self, events: &[ContextEvent]) -> Result<ContextRoot, String> {
        let diverged = self.record_events(events);
        if diverged || self.root().checkpoint.is_none() {
            self.rebuild_from_events(events)?;
        }
        let parent = match self.load_state_from_root() {
            Ok(state) => state,
            Err(_) => {
                self.rebuild_from_events(events)?;
                self.load_state_from_root()?
            }
        };
        let new_events = events
            .iter()
            .filter(|event| event.seq > parent.through_seq)
            .cloned()
            .collect::<Vec<_>>();
        let delta = ContextStateReducer::deterministic_delta(&parent, &new_events);
        let mut state_without_sequence = delta.checkpoint_patch.clone();
        state_without_sequence.through_seq = parent.through_seq;
        if state_without_sequence == parent {
            let mut root = self.root();
            root.hot_event_refs = self.hot_refs(events);
            root.evidence_refs = evidence_refs(events);
            self.install_root(
                root.clone(),
                events.iter().map(|event| event.seq).max().unwrap_or(0),
            );
            return Ok(root);
        }
        let page = self
            .store
            .save(&ContextObject::Checkpoint(delta.checkpoint_patch.clone()))
            .map_err(|error| error.to_string())?;
        let mut root = self.root();
        root.checkpoint = Some(page);
        root.deltas.clear();
        root.updates_since_fold = root.updates_since_fold.saturating_add(1);
        root.hot_event_refs = self.hot_refs(events);
        root.evidence_refs = evidence_refs(events);
        self.install_root(root.clone(), delta.through_seq);
        Ok(root)
    }

    pub fn fold(&self, reason: FoldReason, events: &[ContextEvent]) -> Result<ContextRoot, String> {
        self.fold_with_proposal(reason, events, None)
    }

    pub fn fold_with_proposal(
        &self,
        reason: FoldReason,
        events: &[ContextEvent],
        proposal: Option<CheckpointProposal>,
    ) -> Result<ContextRoot, String> {
        let diverged = self.record_events(events);
        if diverged || self.root().checkpoint.is_none() {
            self.rebuild_from_events(events)?;
        }
        let before = match self.load_state_from_root() {
            Ok(state) => state,
            Err(_) => {
                self.rebuild_from_events(events)?;
                self.load_state_from_root()?
            }
        };
        let new_events = events
            .iter()
            .filter(|event| event.seq > before.through_seq)
            .cloned()
            .collect::<Vec<_>>();
        let fallback =
            ContextStateReducer::deterministic_delta(&before, &new_events).checkpoint_patch;
        let state = proposal
            .map(|p| ContextStateReducer::validate_proposal(&fallback, events, p))
            .unwrap_or(fallback);
        let checkpoint = self
            .store
            .save(&ContextObject::Checkpoint(state.clone()))
            .map_err(|error| error.to_string())?;
        let old = self.root();
        let mut episodes = old.episodes;
        if !events.is_empty() {
            let episode = Episode {
                title: format!("Context fold ({})", reason.as_str()),
                outcome:
                    "Structured state retained; exact evidence remains retrievable by source_ref."
                        .into(),
                source_refs: events
                    .iter()
                    .rev()
                    .take(16)
                    .map(|event| event.source_ref.clone())
                    .collect(),
                artifact_refs: events
                    .iter()
                    .flat_map(|event| {
                        event
                            .artifact_refs
                            .iter()
                            .map(|artifact| artifact.uri.clone())
                    })
                    .fold(Vec::new(), |mut refs: Vec<String>, uri| {
                        if !refs.iter().any(|existing| existing == &uri) {
                            refs.push(uri);
                        }
                        refs
                    }),
            };
            let page = self
                .store
                .save(&ContextObject::Episode(episode))
                .map_err(|error| error.to_string())?;
            if !episodes.iter().any(|existing| existing.id == page.id) {
                episodes.push(page);
            }
        }
        let root = ContextRoot {
            epoch: old.epoch.saturating_add(1),
            cache_namespace: String::new(),
            checkpoint: Some(checkpoint),
            deltas: Vec::new(),
            episodes,
            hot_event_refs: self.hot_refs(events),
            evidence_refs: evidence_refs(events),
            updates_since_fold: 0,
        };
        let before_tokens = estimate_state_tokens(&before);
        let after_tokens = estimate_state_tokens(&state);
        self.install_root(root.clone(), state.through_seq);
        {
            let mut guard = self
                .state
                .write()
                .unwrap_or_else(|error| error.into_inner());
            guard.last_fold_reason = Some(reason.as_str().into());
            guard.last_fold_tokens = Some((before_tokens, after_tokens));
        }
        self.bump_metrics(|metrics| {
            metrics.folds = metrics.folds.saturating_add(1);
            metrics.tokens_before_fold = metrics.tokens_before_fold.saturating_add(before_tokens);
            metrics.tokens_after_fold = metrics.tokens_after_fold.saturating_add(after_tokens);
        });
        Ok(root)
    }

    pub fn retrieve(
        &self,
        request: &RetrieveContextRequest,
    ) -> Result<RetrieveContextResult, String> {
        retrieval::retrieve(self, request)
    }

    pub(crate) fn note_page_fault(&self, hit: bool) {
        self.bump_metrics(|metrics| {
            metrics.semantic_page_faults = metrics.semantic_page_faults.saturating_add(1);
            metrics.page_faults = metrics.page_faults.saturating_add(1);
            if hit {
                metrics.retrieval_hits = metrics.retrieval_hits.saturating_add(1);
                metrics.page_fault_hits = metrics.page_fault_hits.saturating_add(1);
            } else {
                metrics.retrieval_misses = metrics.retrieval_misses.saturating_add(1);
                metrics.page_fault_misses = metrics.page_fault_misses.saturating_add(1);
            }
        });
    }

    pub fn record_shadow_comparison(&self, comparison: &ShadowComparison) {
        self.bump_metrics(|metrics| {
            metrics.shadow_missing_user_refs = metrics
                .shadow_missing_user_refs
                .saturating_add(comparison.missing_user_refs.len() as u64);
            metrics.shadow_missing_tool_refs = metrics
                .shadow_missing_tool_refs
                .saturating_add(comparison.missing_tool_refs.len() as u64);
            metrics.prefix_churn = metrics
                .prefix_churn
                .saturating_add(u64::from(comparison.prefix_changed));
        });
    }

    pub fn cache_affinity(&self) -> String {
        let root = self.root();
        format!("ctxvm:v2:{}:{}", root.epoch, root.cache_namespace)
    }

    /// Ordered content fingerprint for diagnostics and prefix-change analysis.
    /// Unlike cache_affinity(), this is expected to change as stable provider
    /// content evolves within one routing/accounting namespace.
    pub fn content_fingerprint(&self) -> String {
        let state = self
            .state
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let root = &state.root;
        let checkpoint = root
            .checkpoint
            .as_ref()
            .map(|page| page.content_hash.as_str())
            .unwrap_or_default();
        let deltas = root
            .deltas
            .iter()
            .map(|page| page.content_hash.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let stable = state.stable_context_digest.as_deref().unwrap_or_default();
        digest(
            format!(
                "ctxvm_content_fingerprint_v1:{}:{checkpoint}:{deltas}:{stable}",
                root.epoch
            )
            .as_bytes(),
        )
    }

    pub fn manifest_entries(
        &self,
        image: &ContextImage,
    ) -> Vec<super::context_manifest::ContextManifestEntry> {
        ContextCompiler::new(self.store.clone()).manifest_entries(image)
    }

    pub fn load_state_from_root(&self) -> Result<CheckpointState, String> {
        let root = self.root();
        let Some(checkpoint) = root.checkpoint else {
            return Ok(CheckpointState::default());
        };
        let mut state = match self
            .store
            .load(&checkpoint)
            .map_err(|error| error.to_string())?
        {
            ContextObject::Checkpoint(state) => state,
            _ => return Err("context checkpoint page has the wrong kind".into()),
        };
        for page in root.deltas {
            let ContextObject::Delta(delta) =
                self.store.load(&page).map_err(|error| error.to_string())?
            else {
                return Err("context delta page has the wrong kind".into());
            };
            state = delta.checkpoint_patch;
        }
        Ok(state)
    }

    fn record_events(&self, events: &[ContextEvent]) -> bool {
        let mut old = self.events.write().unwrap_or_else(|e| e.into_inner());
        let diverged = !old.is_empty()
            && (events.len() < old.len()
                || old.iter().zip(events).any(|(a, b)| {
                    a.source_ref != b.source_ref
                        || a.content_hash != b.content_hash
                        || a.seq != b.seq
                }));
        *old = events
            .iter()
            .map(|event| ContextEvent {
                seq: event.seq,
                source_ref: event.source_ref.clone(),
                content_hash: event.content_hash.clone(),
                kind: event.kind,
                provenance_kind: event.provenance_kind,
                visible_text: String::new(),
                artifact_refs: event.artifact_refs.clone(),
            })
            .collect();
        drop(old);
        let mut contents = self
            .source_contents
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let disk_backed = self
            .session_source
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        contents.clear();
        for event in events {
            if !disk_backed || !event.source_ref.starts_with("session:") {
                contents.insert(event.source_ref.clone(), event.visible_text.clone());
            }
        }
        diverged
    }

    fn hot_refs(&self, events: &[ContextEvent]) -> Vec<String> {
        let mut tokens = 0u64;
        let mut refs = Vec::new();
        for event in events.iter().rev() {
            let estimate = event.visible_text.len().div_ceil(4).max(1) as u64;
            if !refs.is_empty() && tokens.saturating_add(estimate) > self.config.hot_event_tokens {
                break;
            }
            tokens = tokens.saturating_add(estimate);
            refs.push(event.source_ref.clone());
        }
        refs.reverse();
        refs
    }

    fn bump_metrics(&self, update: impl FnOnce(&mut ContextVmMetrics)) {
        update(
            &mut self
                .metrics
                .write()
                .unwrap_or_else(|error| error.into_inner()),
        );
    }
}

pub fn latest_persisted_root(
    entries: &[SessionEntry],
    leaf_id: Option<&str>,
) -> Option<(ContextRoot, u64)> {
    davinci_session::branch_entries(entries, leaf_id)
        .into_iter()
        .rev()
        .filter(|entry| entry.entry_type == "context_checkpoint")
        .find_map(|entry| {
            let root = serde_json::from_value(entry.extra.get("root")?.clone()).ok()?;
            let through_seq = entry
                .extra
                .get("throughSeq")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            Some((root, through_seq))
        })
}

pub fn context_checkpoint_entry(
    root: &ContextRoot,
    through_seq: u64,
    prefix_digest: &str,
    parent_id: Option<String>,
    seq: u64,
) -> SessionEntry {
    let mut extra = serde_json::Map::new();
    extra.insert("schema".into(), Value::from(1));
    extra.insert(
        "root".into(),
        serde_json::to_value(root).unwrap_or(Value::Null),
    );
    extra.insert("throughSeq".into(), Value::from(through_seq));
    extra.insert("prefixDigest".into(), Value::String(prefix_digest.into()));
    SessionEntry {
        id: format!("context-checkpoint-{}", uuid::Uuid::new_v4()),
        entry_type: "context_checkpoint".into(),
        parent_id,
        seq,
        timestamp: 0,
        message: None,
        custom_type: None,
        extra,
    }
}

fn evidence_refs(events: &[ContextEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.kind == ContextEventKind::ToolResult)
        .map(|event| event.source_ref.clone())
        .collect()
}

fn estimate_state_tokens(state: &CheckpointState) -> u64 {
    serde_json::to_vec(state)
        .map(|bytes| bytes.len().div_ceil(4).max(1) as u64)
        .unwrap_or(0)
}

fn stable_context_digest(image: &ContextImage) -> String {
    let mut stable = image
        .entries
        .iter()
        .filter(|entry| entry.stable_for_cache)
        .map(|entry| format!("{}:{}", entry.id, entry.content_hash))
        .collect::<Vec<_>>();
    stable.sort();
    digest(stable.join("\n").as_bytes())
}
