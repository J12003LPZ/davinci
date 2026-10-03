//! Immutable, separately identified Git index evidence; no TypeScript equivalent.

use super::{
    config::ScanConfig,
    snapshot::{ConflictSource, ConflictStage, Snapshot, SourceFile},
};
use std::path::Path;

pub(super) fn capture(
    snapshot: &mut Snapshot,
    root: &Path,
    stages: Vec<ConflictStage>,
    config: &ScanConfig,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let mut identities = std::collections::BTreeMap::new();
    for stage in &stages {
        stage.validate()?;
        if identities
            .insert((&stage.path, stage.stage), stage)
            .is_some()
        {
            return Err("duplicate index conflict identity".into());
        }
    }
    if snapshot
        .conflicts
        .iter()
        .any(|old| identities.get(&(&old.path, old.stage)).copied() != Some(old))
    {
        return Err("index conflicts changed during source capture".into());
    }
    let captured: std::collections::BTreeSet<_> = snapshot
        .conflicts
        .iter()
        .map(|stage| (stage.path.clone(), stage.stage))
        .collect();
    let mut paths: std::collections::BTreeSet<_> = snapshot
        .conflicts
        .iter()
        .map(|stage| stage.path.clone())
        .collect();
    for stage in stages {
        if cancelled() {
            return Err("security Git capture cancelled".into());
        }
        if captured.contains(&(stage.path.clone(), stage.stage)) {
            continue;
        }
        if paths.insert(stage.path.clone()) {
            snapshot.skip(
                &stage.path,
                "unresolved merge; index stages require separate review",
            );
        }
        let result = source(root, &stage, snapshot, config, cancelled)?;
        match result {
            Ok(source) => {
                snapshot.bytes += source.text.len() as u64;
                snapshot.conflict_sources.push(ConflictSource {
                    identity: stage.clone(),
                    source,
                    supporting: false,
                });
            }
            Err(reason) => snapshot.skip(&stage.path, &format!("{}: {reason}", stage.side())),
        }
        snapshot.conflicts.push(stage);
    }
    snapshot
        .conflicts
        .sort_by(|a, b| (&a.path, a.stage).cmp(&(&b.path, b.stage)));
    snapshot.conflict_sources.sort_by(|a, b| {
        (&a.identity.path, a.identity.stage).cmp(&(&b.identity.path, b.identity.stage))
    });
    Ok(())
}

// Outer error aborts capture; inner error records an explicit source exclusion.
fn source(
    root: &Path,
    stage: &ConflictStage,
    snapshot: &Snapshot,
    config: &ScanConfig,
    cancelled: &dyn Fn() -> bool,
) -> Result<Result<SourceFile, &'static str>, String> {
    let path = match super::snapshot::relative_scope(&stage.path) {
        Ok(path) => path,
        Err(_) => return Ok(Err("excluded conflict source")),
    };
    if super::snapshot::denied(&path)
        || path
            .components()
            .any(|part| super::snapshot::excluded_dir(&part.as_os_str().to_string_lossy()))
    {
        return Ok(Err("excluded conflict source"));
    }
    if !["100644", "100755"].contains(&stage.mode.as_str()) {
        return Ok(Err("non-regular conflict source"));
    }
    if snapshot.source_count() >= config.max_inventory_entries {
        return Ok(Err("conflict source inventory limit"));
    }
    let run = |args: &[&str]| super::git::run_interruptible(root, args, cancelled);
    let size = run(&["cat-file", "-s", &stage.object_id])?;
    let size: u64 = std::str::from_utf8(&size)
        .map_err(|_| "invalid conflict object size")?
        .trim()
        .parse()
        .map_err(|_| "invalid conflict object size")?;
    let limit = if path.file_name().is_some_and(|name| name == "SECURITY.md") {
        config.max_file_bytes.min(config.max_policy_bytes)
    } else {
        config.max_file_bytes
    };
    // Match the bounded Git transport ceiling before asking for blob contents.
    if size > limit.min(16 * 1024 * 1024)
        || snapshot.bytes.saturating_add(size) > config.max_snapshot_bytes
    {
        return Ok(Err("conflict source exceeds byte limit"));
    }
    let bytes = run(&["cat-file", "blob", &stage.object_id])?;
    if bytes.len() as u64 != size {
        return Err("conflict object size changed".into());
    }
    let text = match String::from_utf8(bytes) {
        Ok(text)
            if !(text.contains('\0')
                || text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----")) =>
        {
            text
        }
        _ => return Ok(Err("binary, unsupported encoding, or private key material")),
    };
    Ok(Ok(SourceFile {
        hash: super::sha256_hex(text.as_bytes()),
        text,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(path: &str) -> ConflictStage {
        ConflictStage {
            path: path.into(),
            stage: 1,
            object_id: "a".repeat(40),
            mode: "100644".into(),
        }
    }

    #[test]
    fn security_conflict_capture_rejects_denied_links_and_cancellation_before_git() {
        let dir = tempfile::tempdir().unwrap();
        let absent = dir.path().join("not-a-repository");
        for path in [
            ".env",
            ".git/config",
            "target/source.rs",
            "nested/auth.json",
            "key.pem",
        ] {
            let mut snapshot = Snapshot::default();
            capture(
                &mut snapshot,
                &absent,
                vec![stage(path)],
                &ScanConfig::default(),
                &|| false,
            )
            .unwrap();
            assert!(snapshot.conflict_sources.is_empty());
            assert!(snapshot
                .skipped
                .iter()
                .any(|row| row.reason.contains("excluded conflict source")));
        }
        for mode in ["120000", "160000"] {
            let mut snapshot = Snapshot::default();
            let identity = ConflictStage {
                mode: mode.into(),
                ..stage("source.rs")
            };
            capture(
                &mut snapshot,
                &absent,
                vec![identity],
                &ScanConfig::default(),
                &|| false,
            )
            .unwrap();
            assert!(snapshot.conflict_sources.is_empty());
            assert!(snapshot
                .skipped
                .iter()
                .any(|row| row.reason.contains("non-regular")));
        }
        let mut snapshot = Snapshot::default();
        assert!(capture(
            &mut snapshot,
            &absent,
            vec![stage("source.rs")],
            &ScanConfig::default(),
            &|| true
        )
        .is_err());
        assert!(snapshot.conflicts.is_empty());
        assert!(capture(
            &mut snapshot,
            &absent,
            vec![stage("../source.rs")],
            &ScanConfig::default(),
            &|| false
        )
        .is_err());
    }

    #[test]
    fn security_conflict_capture_bounds_blob_inventory_bytes_and_content() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            super::super::git::run_interruptible(dir.path(), args, &|| false).unwrap()
        };
        run(&["init", "-q"]);
        let blob = |text: &[u8]| {
            std::fs::write(dir.path().join("fixture"), text).unwrap();
            String::from_utf8(run(&["hash-object", "-w", "--", "fixture"]))
                .unwrap()
                .trim()
                .to_owned()
        };
        let object_id = blob(b"source\n");
        let stages: Vec<_> = (1..=3)
            .map(|index| ConflictStage {
                stage: index,
                object_id: object_id.clone(),
                ..stage("source.rs")
            })
            .collect();
        let config = ScanConfig {
            max_file_bytes: 7,
            max_snapshot_bytes: 14,
            supporting_reads: false,
            ..ScanConfig::default()
        };
        let mut snapshot = Snapshot::default();
        capture(&mut snapshot, dir.path(), stages.clone(), &config, &|| {
            false
        })
        .unwrap();
        assert_eq!(snapshot.bytes, 14);
        assert_eq!(snapshot.source_count(), 2);
        assert!(snapshot.file("source.rs", "index-theirs").is_err());
        assert!(snapshot
            .skipped
            .iter()
            .any(|row| row.reason == "index-theirs: conflict source exceeds byte limit"));
        config.validate_snapshot(&snapshot).unwrap();
        capture(&mut snapshot, dir.path(), stages.clone(), &config, &|| {
            false
        })
        .unwrap();
        assert_eq!(
            snapshot.bytes, 14,
            "a repeated index inventory must not duplicate evidence"
        );
        let mut changed = stages.clone();
        changed[0].object_id = "b".repeat(40);
        assert!(capture(&mut snapshot, dir.path(), changed, &config, &|| false).is_err());
        let mut limited = Snapshot::default();
        capture(
            &mut limited,
            dir.path(),
            stages,
            &ScanConfig {
                max_inventory_entries: 1,
                ..config.clone()
            },
            &|| false,
        )
        .unwrap();
        assert_eq!(limited.source_count(), 1);
        assert!(limited
            .skipped
            .iter()
            .any(|row| row.reason.contains("inventory limit")));
        for (path, bytes, config) in [
            ("source.rs", b"too long".as_slice(), config.clone()),
            (
                "SECURITY.md",
                b"source\n".as_slice(),
                ScanConfig {
                    max_policy_bytes: 2,
                    ..config.clone()
                },
            ),
            ("source.rs", b"nul\0".as_slice(), config.clone()),
            ("source.rs", b"\xff".as_slice(), config.clone()),
            (
                "source.rs",
                b"-----BEGIN PRIVATE KEY-----".as_slice(),
                ScanConfig::default(),
            ),
        ] {
            let identity = ConflictStage {
                object_id: blob(bytes),
                ..stage(path)
            };
            let mut snapshot = Snapshot::default();
            capture(&mut snapshot, dir.path(), vec![identity], &config, &|| {
                false
            })
            .unwrap();
            assert!(snapshot.conflict_sources.is_empty(), "{path}");
            assert_eq!(snapshot.bytes, 0);
            assert_eq!(snapshot.skipped.len(), 2);
        }
    }
}
