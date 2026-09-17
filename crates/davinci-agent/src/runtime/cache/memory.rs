use super::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct CacheConfig {
    pub enabled: bool,
    pub memory_enabled: bool,
    pub persistent_enabled: bool,
    pub max_entries: usize,
    pub memory_max_bytes: usize,
    pub persistent_max_bytes: u64,
    pub max_object_bytes: usize,
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            memory_enabled: true,
            persistent_enabled: true,
            max_entries: 2048,
            memory_max_bytes: 64 * 1024 * 1024,
            persistent_max_bytes: 512 * 1024 * 1024,
            max_object_bytes: 8 * 1024 * 1024,
        }
    }
}
struct Entry {
    value: Arc<dyn Any + Send + Sync>,
    dependencies: Vec<CacheDependency>,
    namespace: CacheNamespace,
    weight: usize,
    access: u64,
    expires: Option<Instant>,
    negative: bool,
}
#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    bytes: usize,
    sequence: u64,
    stats: CacheStats,
    revoked: HashSet<String>,
    revocation_capacity_reached: bool,
}
struct Inner {
    config: CacheConfig,
    state: Mutex<State>,
    flight: SingleFlight,
    // Host-selected state location, never model input. Opened lazily.
    agent_dir: Option<PathBuf>,
}
#[derive(Clone)]
pub struct CacheRuntime {
    inner: Arc<Inner>,
}
impl std::fmt::Debug for CacheRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheRuntime")
            .field("config", &self.inner.config)
            .finish()
    }
}
impl Default for CacheRuntime {
    fn default() -> Self {
        Self::new(CacheConfig::default(), None)
    }
}
impl CacheRuntime {
    /// Share mechanics across hosts in this process without retaining dead runtimes.
    pub fn shared(config: CacheConfig, agent_dir: PathBuf) -> Self {
        type Registry = Mutex<HashMap<String, std::sync::Weak<Inner>>>;
        static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
        let id = digest(
            &serde_json::to_vec(&(&agent_dir, &config)).expect("cache config serialization"),
        );
        let mut registry = REGISTRY
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        registry.retain(|_, value| value.strong_count() > 0);
        if let Some(inner) = registry.get(&id).and_then(std::sync::Weak::upgrade) {
            return Self { inner };
        }
        let cache = Self::new(config, Some(agent_dir));
        if registry.len() < 64 {
            registry.insert(id, Arc::downgrade(&cache.inner));
        }
        cache
    }
    pub fn new(mut config: CacheConfig, agent_dir: Option<PathBuf>) -> Self {
        config.max_entries = config.max_entries.min(65536);
        config.memory_max_bytes = config.memory_max_bytes.min(1024 * 1024 * 1024);
        config.max_object_bytes = config.max_object_bytes.min(64 * 1024 * 1024);
        config.persistent_max_bytes = config.persistent_max_bytes.min(16 * 1024 * 1024 * 1024);
        Self {
            inner: Arc::new(Inner {
                config,
                agent_dir,
                state: Mutex::default(),
                flight: SingleFlight::default(),
            }),
        }
    }
    pub fn config(&self) -> &CacheConfig {
        &self.inner.config
    }
    /// Process-local ownership identity for live-resource registries only.
    /// Never use this value in an immutable or provider cache key.
    pub fn resource_scope(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }
    pub fn agent_dir(&self) -> Option<&std::path::Path> {
        self.inner.agent_dir.as_deref()
    }
    fn revoked(&self, request: &CacheRequest) -> bool {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.revocation_capacity_reached
            || (!state.revoked.is_empty()
                && request
                    .key
                    .dependencies()
                    .iter()
                    .any(|dep| state.revoked.contains(&dependency_digest(dep))))
    }
    pub fn get<T: DeserializeOwned + Send + Sync + 'static>(
        &self,
        request: &CacheRequest,
        authorize: impl Fn() -> Result<(), CacheError>,
    ) -> Result<Option<Arc<T>>, CacheError> {
        authorize()?;
        let result = self.lookup(request, true);
        authorize()?;
        Ok(if self.revoked(request) { None } else { result })
    }
    fn lookup<T: DeserializeOwned + Send + Sync + 'static>(
        &self,
        request: &CacheRequest,
        record_miss: bool,
    ) -> Option<Arc<T>> {
        if !self.inner.config.enabled || !request.cacheable() || self.revoked(request) {
            return None;
        }
        let id = request.identity::<T>();
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.sequence = state.sequence.saturating_add(1);
        let sequence = state.sequence;
        let expired = state
            .entries
            .get(&id)
            .is_some_and(|e| e.expires.is_some_and(|t| Instant::now() >= t));
        if expired {
            remove(&mut state, &id);
        }
        if let Some(entry) = state.entries.get_mut(&id) {
            entry.access = sequence;
            let negative = entry.negative;
            if let Ok(value) = entry.value.clone().downcast::<T>() {
                let stats = state
                    .stats
                    .namespaces
                    .entry(request.key.namespace())
                    .or_default();
                stats.memory_hits += 1;
                stats.negative_hits += u64::from(negative);
                return Some(value);
            }
        }
        drop(state);
        let mut miss_reason = if expired {
            LocalMissReason::Expired
        } else {
            LocalMissReason::NotFound
        };
        if request.policy.persistent() && self.inner.config.persistent_enabled {
            if let Some(agent) = self.agent_dir() {
                match super::persistent::read(
                    agent,
                    &id,
                    request,
                    self.inner.config.max_object_bytes,
                ) {
                    Ok(bytes) => match serde_json::from_slice::<T>(&bytes) {
                        Ok(value) => {
                            let value = Arc::new(value);
                            let mut state =
                                self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                            let stats = state
                                .stats
                                .namespaces
                                .entry(request.key.namespace())
                                .or_default();
                            stats.persistent_hits += 1;
                            stats.bytes_read += bytes.len() as u64;
                            drop(state);
                            self.insert_memory(request, value.clone(), bytes.len());
                            return Some(value);
                        }
                        Err(_) => {
                            miss_reason = LocalMissReason::Corrupt;
                        }
                    },
                    Err(reason) => {
                        miss_reason = reason;
                    }
                }
            }
        }
        if !record_miss {
            return None;
        }
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let stats = state
            .stats
            .namespaces
            .entry(request.key.namespace())
            .or_default();
        stats.misses += 1;
        stats.corrupt_entries += u64::from(miss_reason == LocalMissReason::Corrupt);
        *stats.miss_reasons.entry(miss_reason).or_default() += 1;
        None
    }
    pub fn get_or_compute<T: Serialize + DeserializeOwned + Send + Sync + 'static>(
        &self,
        request: &CacheRequest,
        authorize: impl Fn() -> Result<(), CacheError>,
        cancel: Option<&crate::CancellationToken>,
        compute: impl FnOnce() -> Result<T, CacheError>,
    ) -> Result<Arc<T>, CacheError> {
        authorize()?;
        if self.revoked(request) {
            return Err(CacheError::Invalidated);
        }
        if cancel.is_some_and(crate::CancellationToken::is_cancelled) {
            return Err(CacheError::Cancelled);
        }
        if let Some(value) = self.get(request, &authorize)? {
            return Ok(value);
        }
        if !self.inner.config.enabled || !request.cacheable() || self.revoked(request) {
            let value = compute()?;
            authorize()?;
            return Ok(Arc::new(value));
        }
        // The inner result is an Arc so the leader and waiters share the same allocation.
        let (value, leader) = self.inner.flight.run(
            &request.identity::<T>(),
            Duration::from_secs(10),
            cancel,
            || {
                if let Some(value) = self.lookup::<T>(request, false) {
                    return Ok(value);
                }
                self.inner
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .stats
                    .namespaces
                    .entry(request.key.namespace())
                    .or_default()
                    .computations += 1;
                let value = Arc::new(compute()?);
                authorize()?;
                if self.revoked(request) {
                    return Err(CacheError::Invalidated);
                }
                if cancel.is_some_and(crate::CancellationToken::is_cancelled) {
                    return Err(CacheError::Cancelled);
                }
                self.insert(request, value.clone());
                Ok(value)
            },
        )?;
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let stats = state
            .stats
            .namespaces
            .entry(request.key.namespace())
            .or_default();
        if !leader {
            stats.waiter_reuse += 1;
        }
        drop(state);
        authorize()?;
        if self.revoked(request) {
            return Err(CacheError::Invalidated);
        }
        Ok((*value).clone())
    }
    pub fn put<T: Serialize + Send + Sync + 'static>(
        &self,
        request: &CacheRequest,
        value: T,
        authorize: impl Fn() -> Result<(), CacheError>,
    ) -> Result<(), CacheError> {
        authorize()?;
        self.insert(request, Arc::new(value));
        Ok(())
    }
    fn insert<T: Serialize + Send + Sync + 'static>(&self, request: &CacheRequest, value: Arc<T>) {
        if !self.inner.config.enabled || !request.cacheable() || self.revoked(request) {
            return;
        }
        let mut writer = BoundedWriter {
            bytes: Vec::new(),
            limit: self.inner.config.max_object_bytes,
        };
        if serde_json::to_writer(&mut writer, value.as_ref()).is_err() {
            return;
        }
        let bytes = writer.bytes;
        if bytes.len() > self.inner.config.max_object_bytes {
            return;
        }
        if request.policy.persistent() && self.inner.config.persistent_enabled {
            if let Some(agent) = self.agent_dir() {
                match super::persistent::write(
                    agent,
                    &request.identity::<T>(),
                    request,
                    bytes.clone(),
                    &self.inner.config,
                ) {
                    Ok(disk) => {
                        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                        let stats = state
                            .stats
                            .namespaces
                            .entry(request.key.namespace())
                            .or_default();
                        stats.bytes_written += bytes.len() as u64;
                        stats.evictions += disk.evictions;
                        for ns in state.stats.namespaces.values_mut() {
                            ns.disk_bytes = 0;
                            ns.disk_objects = 0;
                        }
                        for (namespace, (objects, bytes)) in disk.namespaces {
                            let ns = state.stats.namespaces.entry(namespace).or_default();
                            ns.disk_bytes = bytes;
                            ns.disk_objects = objects;
                        }
                    }
                    Err(_) => {
                        self.miss_reason(request.key.namespace(), LocalMissReason::Unavailable)
                    }
                }
            }
        }
        self.insert_memory(request, value, bytes.len());
    }
    fn insert_memory<T: Send + Sync + 'static>(
        &self,
        request: &CacheRequest,
        value: Arc<T>,
        serialized_bytes: usize,
    ) {
        let weight = serialized_bytes
            .saturating_add(std::mem::size_of::<T>())
            .saturating_add(serde_json::to_vec(&request.key).map_or(0, |bytes| bytes.len()))
            .saturating_add(std::mem::size_of::<Entry>() + 64);
        if !self.inner.config.memory_enabled
            || weight > self.inner.config.memory_max_bytes
            || self.inner.config.max_entries == 0
        {
            return;
        }
        let id = request.identity::<T>();
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.revocation_capacity_reached
            || request.key.dependencies().iter().any(|dep| {
                !state.revoked.is_empty() && state.revoked.contains(&dependency_digest(dep))
            })
        {
            return;
        }
        remove(&mut state, &id);
        while state.entries.len() >= self.inner.config.max_entries
            || state.bytes.saturating_add(weight) > self.inner.config.memory_max_bytes
        {
            let oldest = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.access)
                .map(|(id, _)| id.clone());
            let Some(oldest) = oldest else {
                break;
            };
            if let Some(ns) = remove(&mut state, &oldest) {
                let stats = state.stats.namespaces.entry(ns).or_default();
                stats.evictions += 1;
                *stats
                    .miss_reasons
                    .entry(LocalMissReason::Evicted)
                    .or_default() += 1;
            }
        }
        state.sequence = state.sequence.saturating_add(1);
        let access = state.sequence;
        state.bytes += weight;
        state.entries.insert(
            id,
            Entry {
                value,
                dependencies: request.key.dependencies().to_vec(),
                namespace: request.key.namespace(),
                weight,
                access,
                expires: request
                    .policy
                    .ttl()
                    .and_then(|ttl| Instant::now().checked_add(ttl)),
                negative: matches!(request.policy, CachePolicy::Negative { .. }),
            },
        );
        state
            .stats
            .namespaces
            .entry(request.key.namespace())
            .or_default()
            .writes += 1;
    }
    pub fn invalidate(&self, dependency: &CacheDependency, reason: LocalMissReason) {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        // Version tokens are revoked for this runtime's lifetime. Consumers must
        // present a new version after invalidation; old disk objects cannot revive it.
        if state.revoked.len() < 65536 {
            state.revoked.insert(dependency_digest(dependency));
        } else {
            state.revocation_capacity_reached = true;
        }
        let ids: Vec<_> = state
            .entries
            .iter()
            .filter(|(_, e)| e.dependencies.contains(dependency))
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            if let Some(ns) = remove(&mut state, &id) {
                let stats = state.stats.namespaces.entry(ns).or_default();
                stats.invalidations += 1;
                *stats.miss_reasons.entry(reason).or_default() += 1;
            }
        }
    }
    pub fn clear_memory(&self, namespace: Option<CacheNamespace>) {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let ids: Vec<_> = state
            .entries
            .iter()
            .filter(|(_, e)| namespace.is_none_or(|n| n == e.namespace))
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            if let Some(ns) = remove(&mut state, &id) {
                *state
                    .stats
                    .namespaces
                    .entry(ns)
                    .or_default()
                    .miss_reasons
                    .entry(LocalMissReason::ManualClear)
                    .or_default() += 1;
            }
        }
    }
    pub fn record_provider_usage(&self, input: u64, read: u64, write: u64) {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let provider = &mut state.stats.provider;
        provider.input_tokens = provider.input_tokens.saturating_add(input);
        provider.cache_read_tokens = provider.cache_read_tokens.saturating_add(read);
        provider.cache_write_tokens = provider.cache_write_tokens.saturating_add(write);
    }
    /// The resource owner retains and closes handles; the cache records only counters.
    pub fn record_resource(&self, started: bool, live: usize) {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.stats.live_resources = live as u64;
        state.stats.resource_cold_starts += u64::from(started);
        state.stats.resource_reuses += u64::from(!started);
    }
    pub fn record_source_read(&self, namespace: CacheNamespace, bytes: usize) {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let stats = state.stats.namespaces.entry(namespace).or_default();
        stats.source_reads += 1;
        stats.source_bytes_read += bytes as u64;
    }
    fn miss_reason(&self, namespace: CacheNamespace, reason: LocalMissReason) {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let stats = state.stats.namespaces.entry(namespace).or_default();
        *stats.miss_reasons.entry(reason).or_default() += 1;
        stats.corrupt_entries += u64::from(reason == LocalMissReason::Corrupt);
    }
    pub fn sweep(&self) -> std::io::Result<SweepStats> {
        match self.agent_dir() {
            Some(agent) => super::persistent::sweep(agent, self.inner.config.persistent_max_bytes),
            None => Ok(SweepStats::default()),
        }
    }
    pub fn stats(&self) -> CacheStats {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut stats = state.stats.clone();
        for entry in state.entries.values() {
            let ns = stats.namespaces.entry(entry.namespace).or_default();
            ns.entries += 1;
            ns.memory_bytes += entry.weight as u64;
        }
        stats
    }
}
struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
fn dependency_digest(dependency: &CacheDependency) -> String {
    digest(&serde_json::to_vec(dependency).expect("dependency serialization"))
}
impl std::io::Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("cache object exceeds size limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn remove(state: &mut State, id: &str) -> Option<CacheNamespace> {
    let entry = state.entries.remove(id)?;
    state.bytes = state.bytes.saturating_sub(entry.weight);
    Some(entry.namespace)
}
