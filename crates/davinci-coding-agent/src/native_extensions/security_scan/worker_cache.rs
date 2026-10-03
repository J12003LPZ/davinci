//! Immutable completed worker evidence; no upstream TypeScript counterpart.
use super::{snapshot::Snapshot, store::Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub type ReadLedger = std::collections::BTreeMap<String, std::collections::BTreeSet<usize>>;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerResult {
    pub value: Value,
    pub reads: ReadLedger,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Artifact {
    schema_version: u32,
    key: String,
    digest: String,
    result: WorkerResult,
}

pub fn key(
    objective: &Value,
    snapshot: &Snapshot,
    provenance: &Value,
    turns: usize,
    tokens: u64,
) -> Result<String, String> {
    let input = serde_json::json!({"objective":objective,"snapshot":snapshot.id,
        "sources":snapshot.sources().map(|(side,path,file)| (side,path,&file.hash)).collect::<Vec<_>>(),
        "methodology":super::skills::manifest(),"tools":super::tools::specs(),
        "provenance":provenance,"turns":turns,"tokens":tokens,"workerContract":1});
    Ok(super::sha256_hex(
        &serde_json::to_vec(&input).map_err(|_| "cannot identify worker input")?,
    ))
}

pub fn load(store: &Store, key: &str) -> Result<Option<WorkerResult>, String> {
    validate_key(key)?;
    let Some(bytes) = store.read_optional(&format!("worker-{key}.json"), 8 * 1024 * 1024)? else {
        return Ok(None);
    };
    let artifact: Artifact =
        serde_json::from_slice(&bytes).map_err(|_| "invalid worker recovery artifact")?;
    let digest = super::sha256_hex(
        &serde_json::to_vec(&artifact.result)
            .map_err(|_| "cannot verify worker recovery artifact")?,
    );
    if artifact.schema_version != 1 || artifact.key != key || artifact.digest != digest {
        return Err("worker recovery identity or digest mismatch".into());
    }
    Ok(Some(artifact.result))
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.len() != 64
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("invalid worker recovery key".into());
    }
    Ok(())
}

pub fn save(store: &Store, key: &str, result: WorkerResult) -> Result<(), String> {
    validate_key(key)?;
    let digest = super::sha256_hex(
        &serde_json::to_vec(&result).map_err(|_| "cannot encode worker evidence")?,
    );
    let artifact = Artifact {
        schema_version: 1,
        key: key.into(),
        digest,
        result,
    };
    let bytes = serde_json::to_vec(&artifact).map_err(|_| "cannot encode worker artifact")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("worker recovery artifact exceeds limit".into());
    }
    store.publish(&format!("worker-{key}.json"), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn security_worker_recovery_rejects_corrupt_identity_and_digest() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let store = Store::open(
            agent.path(),
            root.path(),
            &uuid::Uuid::new_v4().to_string(),
            true,
        )
        .unwrap();
        let key = "a".repeat(64);
        let artifact = Artifact {
            schema_version: 1,
            key: key.clone(),
            digest: "0".repeat(64),
            result: WorkerResult {
                value: Value::Null,
                reads: Default::default(),
            },
        };
        store
            .publish(
                &format!("worker-{key}.json"),
                &serde_json::to_vec(&artifact).unwrap(),
            )
            .unwrap();
        assert!(load(&store, &key).is_err());
        assert!(load(&store, "../checkpoint").is_err());
        assert!(save(
            &store,
            "../checkpoint",
            WorkerResult {
                value: Value::Null,
                reads: Default::default()
            }
        )
        .is_err());
    }

    #[test]
    fn security_worker_recovery_key_separates_assignments_and_provider_context() {
        let snapshot = Snapshot::default();
        let original = key(
            &serde_json::json!({"audit":1}),
            &snapshot,
            &serde_json::json!({"model":"one"}),
            2,
            100,
        )
        .unwrap();
        for changed in [
            key(
                &serde_json::json!({"audit":2}),
                &snapshot,
                &serde_json::json!({"model":"one"}),
                2,
                100,
            )
            .unwrap(),
            key(
                &serde_json::json!({"audit":1}),
                &snapshot,
                &serde_json::json!({"model":"two"}),
                2,
                100,
            )
            .unwrap(),
            key(
                &serde_json::json!({"audit":1}),
                &snapshot,
                &serde_json::json!({"model":"one"}),
                3,
                100,
            )
            .unwrap(),
            key(
                &serde_json::json!({"audit":1}),
                &snapshot,
                &serde_json::json!({"model":"one"}),
                2,
                101,
            )
            .unwrap(),
        ] {
            assert_ne!(original, changed);
        }
    }
    #[test]
    fn security_completed_worker_evidence_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        let key = key(
            &serde_json::json!({"audit":1}),
            &Snapshot::default(),
            &Value::Null,
            2,
            100,
        )
        .unwrap();
        save(
            &store,
            &key,
            WorkerResult {
                value: serde_json::json!({"completed":true}),
                reads: Default::default(),
            },
        )
        .unwrap();
        drop(store);
        let store = Store::open(agent.path(), root.path(), &id, false).unwrap();
        assert_eq!(
            load(&store, &key).unwrap().unwrap().value["completed"],
            true
        );
    }
}
