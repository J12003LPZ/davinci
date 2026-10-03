use super::CacheNamespace;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalMissReason {
    NotFound,
    Expired,
    ContentChanged,
    WorkspaceGenerationChanged,
    DocumentChanged,
    ConfigChanged,
    SchemaChanged,
    AlgorithmChanged,
    ServerRestarted,
    PermissionChanged,
    DependencyChanged,
    Corrupt,
    Evicted,
    ManualClear,
    Unavailable,
    Unknown,
}
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceStats {
    pub source_reads: u64,
    pub source_bytes_read: u64,
    pub memory_hits: u64,
    pub persistent_hits: u64,
    pub misses: u64,
    pub writes: u64,
    pub evictions: u64,
    pub invalidations: u64,
    pub corrupt_entries: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub entries: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub disk_objects: u64,
    pub computations: u64,
    pub waiter_reuse: u64,
    pub negative_hits: u64,
    pub miss_reasons: BTreeMap<LocalMissReason, u64>,
}
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCacheStats {
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_cost_usd: f64,
}
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub namespaces: BTreeMap<CacheNamespace, NamespaceStats>,
    pub provider: ProviderCacheStats,
    pub live_resources: u64,
    pub resource_cold_starts: u64,
    pub resource_reuses: u64,
}

impl CacheStats {
    pub fn summary(&self) -> serde_json::Value {
        let sum =
            |field: fn(&NamespaceStats) -> u64| self.namespaces.values().map(field).sum::<u64>();
        let memory_hits = sum(|s| s.memory_hits);
        let persistent_hits = sum(|s| s.persistent_hits);
        let misses = sum(|s| s.misses);
        serde_json::json!({
            "memory": {"entries":sum(|s| s.entries),"bytes":sum(|s| s.memory_bytes),
                "hits":memory_hits,"hitRatio":memory_hits as f64 / (memory_hits + persistent_hits + misses).max(1) as f64,
                "evictions":sum(|s| s.evictions)},
            "persistent": {"observedObjects":sum(|s| s.disk_objects),"observedBytes":sum(|s| s.disk_bytes),
                "hits":persistent_hits,"corruptions":sum(|s| s.corrupt_entries)},
            "singleflight": {"computations":sum(|s| s.computations),"waitersReused":sum(|s| s.waiter_reuse)},
            "lsp": {"residentSessions":self.live_resources,"coldStarts":self.resource_cold_starts,"reuses":self.resource_reuses},
            "provider": self.provider,
        })
    }
}
