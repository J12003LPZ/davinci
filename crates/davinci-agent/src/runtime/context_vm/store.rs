use super::{CheckpointState, ContextPageKind, ContextPageRef, ContextRoot, Episode, StateDelta};
use crate::runtime::cache::{
    digest, CacheDependency, CacheError, CacheKey, CacheNamespace, CachePolicy, CacheRequest,
    CacheRuntime, StoreOutcome,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub const CONTEXT_OBJECT_SCHEMA: u32 = 1;
pub const CONTEXT_OBJECT_ALGORITHM: &str = "context-object-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextObject {
    Checkpoint(CheckpointState),
    Delta(StateDelta),
    Episode(Episode),
}

#[derive(Debug, Clone)]
pub struct ContextObjectStore {
    cache: CacheRuntime,
    metrics: Arc<RwLock<super::ContextVmMetrics>>,
    /// Pages the live root references. The cache is best-effort (disabled,
    /// size-limited, evicting, failing disk), so a reference this process
    /// handed out must stay loadable without it.
    pinned: Arc<RwLock<HashMap<String, ContextObject>>>,
}

impl ContextObjectStore {
    pub fn new(cache: CacheRuntime) -> Self {
        Self::with_metrics(
            cache,
            Arc::new(RwLock::new(super::ContextVmMetrics::default())),
        )
    }

    pub(crate) fn with_metrics(
        cache: CacheRuntime,
        metrics: Arc<RwLock<super::ContextVmMetrics>>,
    ) -> Self {
        Self {
            cache,
            metrics,
            pinned: Arc::default(),
        }
    }

    pub fn cache(&self) -> &CacheRuntime {
        &self.cache
    }

    pub fn save(&self, object: &ContextObject) -> Result<ContextPageRef, CacheError> {
        let bytes = serde_json::to_vec(object).map_err(|error| {
            CacheError::Compute(format!("context object encode failed: {error}"))
        })?;
        let content_hash = digest(&bytes);
        let kind = object.kind();
        let page_id = format!("ctx:{}:{content_hash}", kind.as_str());
        let request = request_for(&page_id, &content_hash);
        let outcome = self
            .cache
            .put_reporting(&request, object.clone(), || Ok(()))?;
        self.pinned
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .insert(page_id.clone(), object.clone());
        if outcome != StoreOutcome::Durable {
            let mut metrics = self
                .metrics
                .write()
                .unwrap_or_else(|error| error.into_inner());
            metrics.page_writes_not_durable = metrics.page_writes_not_durable.saturating_add(1);
        }
        Ok(ContextPageRef {
            id: page_id,
            kind,
            content_hash,
            estimated_tokens: estimate_tokens(bytes.len()),
        })
    }

    pub fn load(&self, page: &ContextPageRef) -> Result<ContextObject, CacheError> {
        let result = self.load_inner(page);
        let mut metrics = self
            .metrics
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if result.is_ok() {
            metrics.page_lookup_hits = metrics.page_lookup_hits.saturating_add(1);
        } else {
            metrics.page_lookup_misses = metrics.page_lookup_misses.saturating_add(1);
        }
        result
    }

    pub(crate) fn pinned_pages(&self) -> HashMap<String, ContextObject> {
        self.pinned
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn restore_pinned_pages(&self, pages: HashMap<String, ContextObject>) {
        *self
            .pinned
            .write()
            .unwrap_or_else(|error| error.into_inner()) = pages;
    }

    /// Drop pins for pages the installed root no longer references.
    pub(crate) fn retain_pinned(&self, root: &ContextRoot) {
        let live = root
            .checkpoint
            .iter()
            .chain(&root.deltas)
            .chain(&root.episodes)
            .map(|page| page.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        self.pinned
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|id, _| live.contains(id.as_str()));
    }

    fn load_inner(&self, page: &ContextPageRef) -> Result<ContextObject, CacheError> {
        let request = request_for(&page.id, &page.content_hash);
        let cached = self.cache.get::<ContextObject>(&request, || Ok(()))?;
        let corrupt = match cached {
            Some(object) if verified(&object, page) => return Ok((*object).clone()),
            Some(_) => true,
            None => false,
        };
        let pinned = self
            .pinned
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(&page.id)
            .filter(|object| verified(object, page))
            .cloned();
        let Some(object) = pinned else {
            return Err(CacheError::Compute(if corrupt {
                "context page integrity check failed".into()
            } else {
                "context page unavailable; replay/rebuild required".into()
            }));
        };
        let mut metrics = self
            .metrics
            .write()
            .unwrap_or_else(|error| error.into_inner());
        metrics.pinned_page_loads = metrics.pinned_page_loads.saturating_add(1);
        Ok(object)
    }
}

impl ContextObject {
    pub fn kind(&self) -> ContextPageKind {
        match self {
            Self::Checkpoint(_) => ContextPageKind::Checkpoint,
            Self::Delta(_) => ContextPageKind::Delta,
            Self::Episode(_) => ContextPageKind::Episode,
        }
    }
}

impl ContextPageKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Checkpoint => "checkpoint",
            Self::Delta => "delta",
            Self::Episode => "episode",
        }
    }
}

fn request_for(page_id: &str, content_hash: &str) -> CacheRequest {
    CacheRequest::new(
        CacheKey::new(
            CacheNamespace::Context,
            page_id,
            CONTEXT_OBJECT_SCHEMA,
            CONTEXT_OBJECT_ALGORITHM,
            vec![CacheDependency::ContentHash(content_hash.to_string())],
        ),
        CachePolicy::PersistentImmutable,
    )
}

fn verified(object: &ContextObject, page: &ContextPageRef) -> bool {
    object.kind() == page.kind
        && serde_json::to_vec(object).is_ok_and(|bytes| digest(&bytes) == page.content_hash)
}

fn estimate_tokens(bytes: usize) -> u64 {
    bytes.div_ceil(4).max(1) as u64
}
