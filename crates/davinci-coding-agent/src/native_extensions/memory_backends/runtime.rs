//! The live state of the remote projection for one [`VectorMemory`]: the
//! client, whether Qdrant is currently considered down, and why.
//!
//! After a failure the projection is skipped for the configured backoff so a
//! stopped Qdrant costs one timeout, not one per prompt (the same shape as the
//! dense-embedding backoff). Callers fall back to local search and report the
//! state through [`ProjectionRuntime::health`].
//!
//! [`VectorMemory`]: crate::native_extensions::vector_memory::VectorMemory

use super::qdrant::QdrantProjection;
use super::{MemoryProjectionBackend, ProjectionError, VectorProjection};
use crate::native_extensions::vector_memory::{sha256_hex, VectorMemoryConfig};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Stable code shown when the remote could not be used and local search ran.
pub const QDRANT_OFFLINE: &str = "QDRANT_OFFLINE";
/// Stable code for a remote that answered but must not be used (wrong
/// collection shape, bad key, remote URL without consent).
pub const QDRANT_REFUSED: &str = "QDRANT_REFUSED";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionState {
    /// Local backend: the remote is not in use.
    Off,
    /// Chosen, not contacted yet.
    Unchecked,
    Ready,
    Offline,
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionHealth {
    pub state: ProjectionState,
    pub code: Option<&'static str>,
    pub detail: String,
}

struct Inner {
    injected: Option<Arc<dyn VectorProjection>>,
    cached: Option<(String, Arc<dyn VectorProjection>)>,
    backoff_until: Option<Instant>,
    health: ProjectionHealth,
}

pub struct ProjectionRuntime {
    inner: Mutex<Inner>,
}

impl Default for ProjectionRuntime {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                injected: None,
                cached: None,
                backoff_until: None,
                health: ProjectionHealth {
                    state: ProjectionState::Off,
                    code: None,
                    detail: String::new(),
                },
            }),
        }
    }
}

impl fmt::Debug for ProjectionRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProjectionRuntime")
            .field("health", &self.health_unchecked())
            .finish()
    }
}

impl ProjectionRuntime {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn health_unchecked(&self) -> ProjectionHealth {
        self.lock().health.clone()
    }

    /// Use this projection instead of building a Qdrant client. For tests.
    pub fn inject(&self, projection: Arc<dyn VectorProjection>) {
        self.lock().injected = Some(projection);
    }

    /// The health to show for this config: `Off` whenever the backend is local,
    /// whatever happened earlier.
    pub fn health(&self, config: &VectorMemoryConfig) -> ProjectionHealth {
        if config.projection.backend != MemoryProjectionBackend::Qdrant {
            return ProjectionHealth {
                state: ProjectionState::Off,
                code: None,
                detail: String::new(),
            };
        }
        let mut health = self.health_unchecked();
        if health.state == ProjectionState::Off {
            health.state = ProjectionState::Unchecked;
        }
        health
    }

    /// True while a recent failure keeps the remote out of use.
    pub fn backing_off(&self) -> bool {
        self.lock()
            .backoff_until
            .is_some_and(|until| Instant::now() < until)
    }

    /// The projection to use right now, or `None` for local-only: backend is
    /// local, or the remote failed recently, or its config is refused.
    pub fn acquire(
        &self,
        config: &VectorMemoryConfig,
        repo_id: &str,
    ) -> Option<Arc<dyn VectorProjection>> {
        if config.projection.backend != MemoryProjectionBackend::Qdrant || self.backing_off() {
            return None;
        }
        let mut inner = self.lock();
        if let Some(injected) = &inner.injected {
            return Some(injected.clone());
        }
        let key = cache_key(config, repo_id);
        if let Some((cached_key, client)) = &inner.cached {
            if *cached_key == key {
                return Some(client.clone());
            }
        }
        match QdrantProjection::new(config, repo_id) {
            Ok(client) => {
                let client: Arc<dyn VectorProjection> = Arc::new(client);
                inner.cached = Some((key, client.clone()));
                Some(client)
            }
            Err(error) => {
                drop(inner);
                self.record(config, Err(error));
                None
            }
        }
    }

    /// Note the outcome of a remote call: Ok clears the backoff, a failure
    /// starts one.
    pub fn record(&self, config: &VectorMemoryConfig, outcome: Result<(), ProjectionError>) {
        let backoff = Duration::from_secs(config.projection.offline_backoff_seconds);
        let mut inner = self.lock();
        match outcome {
            Ok(()) => {
                inner.backoff_until = None;
                inner.health = ProjectionHealth {
                    state: ProjectionState::Ready,
                    code: None,
                    detail: String::new(),
                };
            }
            Err(error) => {
                inner.backoff_until = Some(Instant::now() + backoff);
                let (state, code) = match error {
                    ProjectionError::Offline(_) => (ProjectionState::Offline, QDRANT_OFFLINE),
                    ProjectionError::Refused(_) => (ProjectionState::Refused, QDRANT_REFUSED),
                };
                inner.health = ProjectionHealth {
                    state,
                    code: Some(code),
                    detail: error.to_string(),
                };
            }
        }
    }
}

/// Everything a client depends on. The key is hashed so the cache key can be
/// compared without holding a second copy of the secret.
fn cache_key(config: &VectorMemoryConfig, repo_id: &str) -> String {
    let key_digest = config
        .projection
        .api_key
        .as_ref()
        .map(|key| sha256_hex(key.expose().as_bytes()))
        .unwrap_or_default();
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        config.qdrant_url,
        config.projection.collection_prefix,
        config.projection.allow_remote,
        config.projection.batch_size,
        config.request_timeout_seconds,
        config.embedding_model,
        config.embedding_dimensions,
        format!("{repo_id}{key_digest}")
    )
}
