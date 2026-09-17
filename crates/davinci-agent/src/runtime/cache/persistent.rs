use super::{
    digest, directory::Directory, CacheConfig, CacheKey, CacheNamespace, CacheRequest,
    LocalMissReason,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
    time::SystemTime,
};

const FORMAT: u32 = 1;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: u32,
    identity: String,
    key: CacheKey,
    checksum: String,
    payload: Vec<u8>,
}
#[derive(Debug, Default, Clone, Serialize)]
pub struct SweepStats {
    pub bytes: u64,
    pub objects: u64,
    pub evictions: u64,
    pub namespaces: BTreeMap<CacheNamespace, (u64, u64)>,
}
fn root(agent: &Path, create: bool) -> std::io::Result<Directory> {
    // Resolve only the host-selected base (e.g. macOS /var). Descendant cache
    // components are still opened without following links.
    let base = match agent.canonicalize() {
        Ok(base) => base,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => agent.to_path_buf(),
        Err(error) => return Err(error),
    };
    Directory::open(&base.join("cache-runtime/v1"), create)
}
fn name(id: &str, namespace: CacheNamespace) -> String {
    format!("{}-{id}.json", namespace_name(namespace))
}
fn namespace_name(namespace: CacheNamespace) -> &'static str {
    match namespace {
        CacheNamespace::Prompt => "prompt",
        CacheNamespace::File => "file",
        CacheNamespace::Ast => "ast",
        CacheNamespace::Repo => "repo",
        CacheNamespace::Query => "query",
        CacheNamespace::Lsp => "lsp",
        CacheNamespace::Package => "package",
        CacheNamespace::Git => "git",
        CacheNamespace::Build => "build",
        CacheNamespace::Test => "test",
    }
}
fn object_namespace(name: &str) -> Option<CacheNamespace> {
    let (prefix, suffix) = name.split_once('-')?;
    if !object_name(suffix) {
        return None;
    }
    [
        CacheNamespace::Prompt,
        CacheNamespace::File,
        CacheNamespace::Ast,
        CacheNamespace::Repo,
        CacheNamespace::Query,
        CacheNamespace::Lsp,
        CacheNamespace::Package,
        CacheNamespace::Git,
        CacheNamespace::Build,
        CacheNamespace::Test,
    ]
    .into_iter()
    .find(|&n| namespace_name(n) == prefix)
}
fn object_name(name: &str) -> bool {
    name.len() == 69
        && name.ends_with(".json")
        && name[..64]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub(super) fn read(
    agent: &Path,
    id: &str,
    request: &CacheRequest,
    limit: usize,
) -> Result<Vec<u8>, LocalMissReason> {
    let dir = root(agent, false).map_err(|_| LocalMissReason::NotFound)?;
    let file = dir
        .file(&name(id, request.key.namespace()), false)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                LocalMissReason::NotFound
            } else {
                LocalMissReason::Unavailable
            }
        })?;
    let bound = limit.saturating_mul(5).saturating_add(65536);
    if file
        .metadata()
        .map_err(|_| LocalMissReason::Unavailable)?
        .len()
        > bound as u64
    {
        return Err(LocalMissReason::Corrupt);
    }
    let mut bytes = Vec::new();
    file.take(bound as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| LocalMissReason::Unavailable)?;
    if bytes.len() > bound {
        return Err(LocalMissReason::Corrupt);
    }
    let value: Envelope = serde_json::from_slice(&bytes).map_err(|_| LocalMissReason::Corrupt)?;
    if value.format != FORMAT {
        return Err(LocalMissReason::SchemaChanged);
    }
    if value.identity != id
        || value.key != request.key
        || value.payload.len() > limit
        || digest(&value.payload) != value.checksum
    {
        return Err(LocalMissReason::Corrupt);
    }
    Ok(value.payload)
}
pub(super) fn write(
    agent: &Path,
    id: &str,
    request: &CacheRequest,
    payload: Vec<u8>,
    config: &CacheConfig,
) -> std::io::Result<SweepStats> {
    let bytes = serde_json::to_vec(&Envelope {
        format: FORMAT,
        identity: id.into(),
        key: request.key.clone(),
        checksum: digest(&payload),
        payload,
    })?;
    if bytes.len() as u64 > config.persistent_max_bytes {
        return Err(std::io::Error::other("object exceeds disk budget"));
    }
    let dir = root(agent, true)?;
    let _lease = dir.lease()?;
    // No compute or serialization under the lease. An existing invalid object is replaceable.
    if read(agent, id, request, config.max_object_bytes).is_ok() {
        return sweep_directory(&dir, config.persistent_max_bytes, 0);
    }
    let _ = dir.remove(&name(id, request.key.namespace()));
    let mut stats = sweep_directory(&dir, config.persistent_max_bytes, bytes.len() as u64)?;
    let temp = format!("{}.tmp", uuid::Uuid::new_v4());
    let result = (|| {
        let mut file = dir.file(&temp, true)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        dir.publish(&temp, &name(id, request.key.namespace()))
    })();
    let _ = dir.remove(&temp);
    result?;
    stats.bytes += bytes.len() as u64;
    stats.objects += 1;
    let ns = stats.namespaces.entry(request.key.namespace()).or_default();
    ns.0 += 1;
    ns.1 += bytes.len() as u64;
    Ok(stats)
}
pub(super) fn sweep(agent: &Path, budget: u64) -> std::io::Result<SweepStats> {
    let dir = match root(agent, false) {
        Ok(dir) => dir,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SweepStats::default()),
        Err(e) => return Err(e),
    };
    let _lease = dir.lease()?;
    sweep_directory(&dir, budget, 0)
}
fn sweep_directory(dir: &Directory, budget: u64, reserve: u64) -> std::io::Result<SweepStats> {
    let mut entries = Vec::new();
    let mut stats = SweepStats::default();
    for name in dir.names()? {
        // Interrupted writes are safe to discard while the exclusive lease is held.
        if name.ends_with(".tmp") && uuid::Uuid::parse_str(name.trim_end_matches(".tmp")).is_ok() {
            let _ = dir.remove(&name);
            continue;
        }
        let Some(namespace) = object_namespace(&name) else {
            continue;
        };
        let file = match dir.file(&name, false) {
            Ok(file) => file,
            Err(_) => {
                let _ = dir.remove(&name);
                continue;
            }
        };
        let metadata = file.metadata()?;
        stats.bytes = stats.bytes.saturating_add(metadata.len());
        entries.push((
            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            name,
            metadata.len(),
            namespace,
        ));
    }
    entries.sort();
    for (_, name, size, namespace) in entries {
        if stats.bytes.saturating_add(reserve) > budget {
            dir.remove(&name)?;
            stats.bytes = stats.bytes.saturating_sub(size);
            stats.evictions += 1;
        } else {
            stats.objects += 1;
            let ns = stats.namespaces.entry(namespace).or_default();
            ns.0 += 1;
            ns.1 += size;
        }
    }
    Ok(stats)
}
