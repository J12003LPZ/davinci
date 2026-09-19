use super::{CheckpointState, ContextPageKind, ContextPageRef, Episode, StateDelta};
use crate::runtime::cache::{
    digest, CacheDependency, CacheError, CacheKey, CacheNamespace, CachePolicy, CacheRequest,
    CacheRuntime,
};
use serde::{Deserialize, Serialize};

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
}

impl ContextObjectStore {
    pub fn new(cache: CacheRuntime) -> Self {
        Self { cache }
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
        self.cache.put(&request, object.clone(), || Ok(()))?;
        Ok(ContextPageRef {
            id: page_id,
            kind,
            content_hash,
            estimated_tokens: estimate_tokens(bytes.len()),
        })
    }

    pub fn load(&self, page: &ContextPageRef) -> Result<ContextObject, CacheError> {
        let request = request_for(&page.id, &page.content_hash);
        let Some(object) = self.cache.get::<ContextObject>(&request, || Ok(()))? else {
            return Err(CacheError::Compute(
                "context page unavailable; replay/rebuild required".into(),
            ));
        };
        let bytes = serde_json::to_vec(object.as_ref()).map_err(|error| {
            CacheError::Compute(format!("context object re-encode failed: {error}"))
        })?;
        if digest(&bytes) != page.content_hash || object.kind() != page.kind {
            return Err(CacheError::Compute(
                "context page integrity check failed".into(),
            ));
        }
        Ok((*object).clone())
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

fn estimate_tokens(bytes: usize) -> u64 {
    bytes.div_ceil(4).max(1) as u64
}
