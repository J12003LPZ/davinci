//! Qdrant over its REST API, as a [`VectorProjection`].
//!
//! Safety rules this file keeps:
//! - One collection per repository and embedding model, named from both, so a
//!   different repo or a different model never shares vectors. The old global
//!   `pi_memory_v1` is not used and is never touched.
//! - A collection that exists with a different vector size or distance is
//!   refused, not repaired, not deleted. It may belong to someone else.
//! - DaVinci writes a marker point into every collection it creates. A
//!   collection of the derived name without that marker was not made by
//!   DaVinci: it is refused, never adopted, and nothing is written to it or
//!   deleted from it.
//! - Points carry ids, numbers and a repo id. No memory text leaves the
//!   machine through this file.
//! - A Qdrant that is not on this machine is refused unless the user allowed
//!   it. The API key goes in a header and nowhere else, and is never printed;
//!   it is never sent over plain http to a host that is not this machine, and
//!   redirects are not followed, so the header cannot be forwarded elsewhere.

use super::{
    ProjectionConfig, ProjectionError, ProjectionHit, ProjectionPoint, Secret, VectorProjection,
};
use crate::native_extensions::service_supervisor::is_local_url;
use crate::native_extensions::vector_memory::{
    hash_to_uuid, sha256_hex, EmbeddingIdentity, VectorMemoryConfig,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const DISTANCE: &str = "Cosine";

/// The point DaVinci writes when it creates a collection. It has no `repo_id`,
/// so no query, delete or namespace clear ever matches it.
pub const MARKER_POINT_ID: &str = "da71c100-0000-4000-8000-000000000001";
pub const MARKER_VALUE: &str = "davinci-memory";

/// The largest batch one request carries, whatever the configuration says.
pub const MAX_BATCH: usize = 512;

/// `davinci-<12 hex of repo>-<8 hex of model identity>`. Pure and stable, so a
/// restart finds the same collection and a model change starts a fresh one.
pub fn collection_name(prefix: &str, repo_id: &str, identity: &EmbeddingIdentity) -> String {
    let prefix: String = prefix
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(24)
        .collect();
    let prefix = if prefix.trim_matches('-').is_empty() {
        "davinci".to_string()
    } else {
        prefix
    };
    let repo = &sha256_hex(repo_id.as_bytes())[..12];
    let model = &sha256_hex(
        format!(
            "{}\0{}\0{}",
            identity.model, identity.dimensions, identity.prefix_revision
        )
        .as_bytes(),
    )[..8];
    format!("{prefix}-{repo}-{model}")
}

/// Qdrant point ids must be unsigned integers or UUIDs. Record ids already are
/// UUIDs; anything else maps to a stable one, and the record id travels in the
/// payload so a hit can be resolved back.
pub fn point_id(record_id: &str) -> String {
    let looks_uuid = record_id.len() == 36
        && record_id
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() || ch == '-');
    if looks_uuid {
        record_id.to_ascii_lowercase()
    } else {
        hash_to_uuid(&sha256_hex(
            format!("davinci-point\0{record_id}").as_bytes(),
        ))
    }
}

pub struct QdrantProjection {
    base: String,
    collection: String,
    dimensions: usize,
    api_key: Option<Secret>,
    batch_size: usize,
    agent: ureq::Agent,
    /// Set once `/readyz` and the collection check passed; cleared by any
    /// failure so the next call checks again (and recreates a lost collection).
    ready: AtomicBool,
}

impl QdrantProjection {
    pub fn new(config: &VectorMemoryConfig, repo_id: &str) -> Result<Self, ProjectionError> {
        let projection: &ProjectionConfig = &config.projection;
        if !is_local_url(&config.qdrant_url) && !projection.allow_remote {
            return Err(ProjectionError::Refused(
                "qdrant URL is not on this machine; set projection.allowRemote to send vectors there"
                    .into(),
            ));
        }
        if projection.api_key.is_some()
            && !config
                .qdrant_url
                .trim()
                .to_ascii_lowercase()
                .starts_with("https://")
            && !is_local_url(&config.qdrant_url)
        {
            return Err(ProjectionError::Refused(
                "the Qdrant API key would travel over plain http to a host that is not this \
                 machine; use an https:// URL, or remove the API key"
                    .into(),
            ));
        }
        let identity = EmbeddingIdentity::from_config(config);
        let timeout = Duration::from_secs(config.request_timeout_seconds.max(1));
        Ok(Self {
            base: config.qdrant_url.trim_end_matches('/').to_string(),
            collection: collection_name(&projection.collection_prefix, repo_id, &identity),
            dimensions: config.embedding_dimensions,
            api_key: projection.api_key.clone(),
            batch_size: projection.batch_size.clamp(1, MAX_BATCH),
            // No redirects: the api-key header must not follow one elsewhere.
            agent: ureq::AgentBuilder::new()
                .timeout(timeout)
                .redirects(0)
                .build(),
            ready: AtomicBool::new(false),
        })
    }

    pub fn collection(&self) -> &str {
        &self.collection
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, CallError> {
        let url = format!("{}{path}", self.base);
        let mut request = self.agent.request(method, &url);
        if let Some(key) = &self.api_key {
            request = request.set("api-key", key.expose());
        }
        let outcome = match body {
            Some(body) => request.send_json(body),
            None => request.call(),
        };
        match outcome {
            Ok(response) => Ok(response.into_json::<Value>().unwrap_or(Value::Null)),
            Err(ureq::Error::Status(status, _)) => Err(CallError::Status(status)),
            Err(ureq::Error::Transport(error)) => Err(CallError::Transport(error.to_string())),
        }
    }

    fn fail(&self, error: CallError, what: &str) -> ProjectionError {
        self.ready.store(false, Ordering::SeqCst);
        match error {
            CallError::Transport(detail) => {
                ProjectionError::Offline(format!("qdrant {what} failed: {detail}"))
            }
            CallError::Status(401 | 403) => ProjectionError::Refused(format!(
                "qdrant refused the request ({what}): check the API key"
            )),
            CallError::Status(400 | 422) => {
                ProjectionError::Refused(format!("qdrant rejected {what} as malformed"))
            }
            CallError::Status(status) => {
                ProjectionError::Offline(format!("qdrant {what} returned HTTP {status}"))
            }
        }
    }

    fn validate_collection(&self, info: &Value) -> Result<(), ProjectionError> {
        let vectors = &info["result"]["config"]["params"]["vectors"];
        let size = vectors["size"].as_u64().map(|size| size as usize);
        let distance = vectors["distance"].as_str().unwrap_or_default();
        if size != Some(self.dimensions) || !distance.eq_ignore_ascii_case(DISTANCE) {
            return Err(ProjectionError::Refused(format!(
                "collection {} exists with vector size {:?} and distance {distance:?}, \
                 expected {} and {DISTANCE}; it was left untouched",
                self.collection, size, self.dimensions
            )));
        }
        Ok(())
    }
}

impl QdrantProjection {
    /// Does the collection carry DaVinci's marker point?
    fn has_marker(&self) -> Result<bool, ProjectionError> {
        let path = format!("/collections/{}/points/{MARKER_POINT_ID}", self.collection);
        match self.call("GET", &path, None) {
            Ok(reply) => {
                Ok(reply["result"]["payload"]["davinci_marker"].as_str() == Some(MARKER_VALUE))
            }
            Err(CallError::Status(404)) => Ok(false),
            Err(error) => Err(self.fail(error, "collection marker check")),
        }
    }

    fn write_marker(&self) -> Result<(), CallError> {
        let mut vector = vec![0.0_f32; self.dimensions];
        if let Some(first) = vector.first_mut() {
            *first = 1.0;
        }
        let path = format!("/collections/{}/points?wait=true", self.collection);
        self.call(
            "PUT",
            &path,
            Some(json!({"points": [{
                "id": MARKER_POINT_ID,
                "vector": vector,
                "payload": {"davinci_marker": MARKER_VALUE},
            }]})),
        )
        .map(|_| ())
    }

    fn refuse_unmarked(&self) -> ProjectionError {
        ProjectionError::Refused(format!(
            "collection {} exists but DaVinci did not create it (no ownership marker); it was \
             left untouched. Delete it yourself, or set projection.collectionPrefix to another name",
            self.collection
        ))
    }
}

enum CallError {
    Transport(String),
    Status(u16),
}

impl VectorProjection for QdrantProjection {
    fn ensure_ready(&self) -> Result<(), ProjectionError> {
        if self.ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.call("GET", "/readyz", None)
            .map_err(|error| self.fail(error, "readiness check"))?;
        let path = format!("/collections/{}", self.collection);
        match self.call("GET", &path, None) {
            Ok(info) => {
                self.validate_collection(&info)?;
                if !self.has_marker()? {
                    return Err(self.refuse_unmarked());
                }
            }
            Err(CallError::Status(404)) => {
                let created = self.call(
                    "PUT",
                    &path,
                    Some(json!({"vectors": {"size": self.dimensions, "distance": DISTANCE}})),
                );
                match created {
                    Ok(_) => {
                        // Ours: mark it. If the marker cannot be written, take the
                        // empty collection back so it is not left unmarked.
                        if let Err(error) = self.write_marker() {
                            let _ = self.call("DELETE", &path, None);
                            return Err(self.fail(error, "collection marker write"));
                        }
                    }
                    // Someone created it between our GET and PUT: validate theirs.
                    // Their marker may not be written yet, so a missing one is a
                    // retry, not a refusal.
                    Err(CallError::Status(409)) => {
                        let info = self
                            .call("GET", &path, None)
                            .map_err(|error| self.fail(error, "collection check"))?;
                        self.validate_collection(&info)?;
                        if !self.has_marker()? {
                            return Err(ProjectionError::Offline(format!(
                                "collection {} was just created by someone else; checking again later",
                                self.collection
                            )));
                        }
                    }
                    Err(error) => return Err(self.fail(error, "collection create")),
                }
            }
            Err(error) => return Err(self.fail(error, "collection check")),
        }
        self.ready.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn upsert(&self, points: &[ProjectionPoint]) -> Result<(), ProjectionError> {
        self.ensure_ready()?;
        for batch in points.chunks(self.batch_size) {
            let body: Vec<Value> = batch
                .iter()
                .map(|point| {
                    json!({
                        "id": point_id(&point.id),
                        "vector": point.vector,
                        "payload": {"repo_id": point.repo_id, "record_id": point.id},
                    })
                })
                .collect();
            let path = format!("/collections/{}/points?wait=true", self.collection);
            self.call("PUT", &path, Some(json!({"points": body})))
                .map_err(|error| self.fail(error, "upsert"))?;
        }
        Ok(())
    }

    fn query(
        &self,
        vector: &[f32],
        repo_id: &str,
        limit: usize,
    ) -> Result<Vec<ProjectionHit>, ProjectionError> {
        self.ensure_ready()?;
        let path = format!("/collections/{}/points/query", self.collection);
        let reply = self
            .call(
                "POST",
                &path,
                Some(json!({
                    "query": vector,
                    "limit": limit,
                    "filter": {"must": [{"key": "repo_id", "match": {"value": repo_id}}]},
                    "with_payload": {"include": ["record_id"]},
                })),
            )
            .map_err(|error| self.fail(error, "query"))?;
        let points = reply["result"]["points"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(points
            .iter()
            .filter_map(|point| {
                let id = point["payload"]["record_id"]
                    .as_str()
                    .or_else(|| point["id"].as_str())?;
                Some(ProjectionHit {
                    id: id.to_string(),
                    score: point["score"].as_f64()? as f32,
                })
            })
            .collect())
    }

    fn delete(&self, ids: &[String], repo_id: &str) -> Result<(), ProjectionError> {
        if ids.is_empty() {
            return Ok(());
        }
        self.ensure_ready()?;
        for batch in ids.chunks(self.batch_size) {
            let point_ids: Vec<String> = batch.iter().map(|id| point_id(id)).collect();
            let path = format!("/collections/{}/points/delete?wait=true", self.collection);
            self.call(
                "POST",
                &path,
                Some(json!({"filter": {"must": [
                    {"has_id": point_ids},
                    {"key": "repo_id", "match": {"value": repo_id}},
                ]}})),
            )
            .map_err(|error| self.fail(error, "delete"))?;
        }
        Ok(())
    }

    fn clear_namespace(&self, repo_id: &str) -> Result<(), ProjectionError> {
        self.ensure_ready()?;
        let path = format!("/collections/{}/points/delete?wait=true", self.collection);
        self.call(
            "POST",
            &path,
            Some(json!({"filter": {"must": [
                {"key": "repo_id", "match": {"value": repo_id}},
            ]}})),
        )
        .map_err(|error| self.fail(error, "namespace clear"))?;
        Ok(())
    }

    fn count(&self, repo_id: &str) -> Result<usize, ProjectionError> {
        self.ensure_ready()?;
        let path = format!("/collections/{}/points/count", self.collection);
        let body = json!({
            "exact": true,
            "filter": {"must": [{"key": "repo_id", "match": {"value": repo_id}}]},
        });
        match self.call("POST", &path, Some(body)) {
            Ok(reply) => Ok(reply["result"]["count"].as_u64().unwrap_or(0) as usize),
            // Deleted since the last check: it holds nothing, and the next call
            // checks readiness again, which recreates and marks it.
            Err(CallError::Status(404)) => {
                self.ready.store(false, Ordering::SeqCst);
                Ok(0)
            }
            Err(error) => Err(self.fail(error, "count")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(model: &str) -> EmbeddingIdentity {
        EmbeddingIdentity {
            model: model.into(),
            dimensions: 768,
            prefix_revision: 1,
        }
    }

    #[test]
    fn collection_name_is_namespaced_stable_and_model_specific() {
        let a = collection_name("davinci", "repo-a", &identity("m1"));
        assert_eq!(a, collection_name("davinci", "repo-a", &identity("m1")));
        assert_ne!(a, collection_name("davinci", "repo-b", &identity("m1")));
        assert_ne!(a, collection_name("davinci", "repo-a", &identity("m2")));
        assert!(a.starts_with("davinci-") && a.len() == "davinci-".len() + 12 + 1 + 8);
        assert_ne!(a, "pi_memory_v1");
        let odd = collection_name("my team/../x", "r", &identity("m"));
        assert!(odd
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
        assert!(collection_name("///", "r", &identity("m")).starts_with("davinci-"));
    }

    #[test]
    fn point_ids_are_uuids_and_stable() {
        let uuid = "0a1b2c3d-0000-4000-8000-0123456789ab";
        assert_eq!(point_id(uuid), uuid);
        let mapped = point_id("mem-1");
        assert_eq!(mapped, point_id("mem-1"));
        assert_eq!(mapped.len(), 36);
        assert_ne!(mapped, point_id("mem-2"));
    }

    #[test]
    fn a_remote_url_is_refused_without_the_opt_in() {
        let mut config = VectorMemoryConfig {
            qdrant_url: "https://qdrant.example.com".into(),
            ..VectorMemoryConfig::default()
        };
        assert!(matches!(
            QdrantProjection::new(&config, "r"),
            Err(ProjectionError::Refused(_))
        ));
        config.projection.allow_remote = true;
        assert!(QdrantProjection::new(&config, "r").is_ok());
    }
}
