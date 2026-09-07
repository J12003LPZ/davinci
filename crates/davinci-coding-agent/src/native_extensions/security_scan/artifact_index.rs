//! Complete committed-artifact integrity index; no upstream TypeScript equivalent.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

const MAX_ENTRIES: usize = 32_768;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub(super) const MAX_SEAL_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    path: String,
    bytes: u64,
    sha256: String,
}

pub(super) fn capture(directory: &Path, generation: u64) -> Result<Vec<Entry>, String> {
    let root = directory
        .canonicalize()
        .map_err(|_| "cannot resolve artifact directory")?;
    let excluded = [
        "active.lock".to_string(),
        // Legacy v1 output is a read-only compatibility artifact. It is never
        // evidence for, or part of, a sealed native v2 report.
        "findings.json".to_string(),
        format!("seal-{generation}.json"),
        format!("report-{generation}.json"),
        format!("report-{generation}.md"),
        format!("results-{generation}.sarif"),
    ];
    let mut names = Vec::new();
    for (index, entry) in fs::read_dir(&root)
        .map_err(|_| "cannot enumerate evidence artifacts")?
        .enumerate()
    {
        if index >= MAX_ENTRIES {
            return Err("security artifact inventory exceeds limit".into());
        }
        let name = entry
            .map_err(|_| "cannot inspect evidence artifact")?
            .file_name()
            .into_string()
            .map_err(|_| "invalid evidence artifact name")?;
        if excluded.contains(&name) || is_temporary(&name) {
            continue;
        }
        names.push(name);
    }
    names.sort();
    let mut remaining = MAX_TOTAL_BYTES;
    names
        .into_iter()
        .map(|name| {
            let entry = digest(&root, name, remaining)?;
            remaining -= entry.bytes;
            Ok(entry)
        })
        .collect()
}

fn is_temporary(name: &str) -> bool {
    name.strip_prefix('.')
        .and_then(|s| s.strip_suffix(".tmp"))
        .and_then(|s| uuid::Uuid::parse_str(s).ok().map(|id| id.to_string() == s))
        .unwrap_or(false)
}

fn digest(root: &Path, name: String, remaining: u64) -> Result<Entry, String> {
    let file = super::snapshot::open_confined(root, Path::new(&name))
        .map_err(|_| "cannot open committed evidence artifact")?;
    let limit = super::config::ScanConfig::default()
        .checkpoint_byte_limit()
        .min(remaining);
    let mut reader = file.take(limit + 1);
    let mut buffer = [0u8; 64 * 1024];
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| "cannot hash evidence artifact")?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > limit {
            return Err("committed evidence exceeds byte limit".into());
        }
        hash.update(&buffer[..count]);
    }
    Ok(Entry {
        path: name,
        bytes,
        sha256: format!("{:x}", hash.finalize()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_artifact_index_is_sorted_bounded_and_ignores_uncommitted_temps() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("z.json"), b"last").unwrap();
        fs::write(root.path().join("a.json"), b"first").unwrap();
        fs::write(root.path().join("findings.json"), b"legacy").unwrap();
        fs::write(
            root.path().join(format!(".{}.tmp", uuid::Uuid::new_v4())),
            b"incomplete",
        )
        .unwrap();
        let entries = capture(root.path(), 1).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            ["a.json", "z.json"]
        );
        assert_eq!(entries[0].sha256, super::super::sha256_hex(b"first"));
        assert_eq!(entries[0].bytes, 5);
        let canonical = root.path().canonicalize().unwrap();
        assert!(digest(&canonical, "a.json".into(), 4).is_err());
        assert!(digest(&canonical, "a.json".into(), 5).is_ok());
        fs::create_dir(root.path().join("nested")).unwrap();
        assert!(capture(root.path(), 1).is_err());
    }
}
