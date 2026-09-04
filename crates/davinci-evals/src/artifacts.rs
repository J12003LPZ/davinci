//! TS `vendor/pi/packages/evals/src/vitest-evals/artifacts.ts`.

use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::path::{Path, PathBuf};

pub const PI_SESSION_SNAPSHOT_ARTIFACT: &str = "piSessionJsonl";
pub const SESSION_ARTIFACT_TYPE: &str = "@earendil-works/pi-evals:session";
pub const SOURCE_ARTIFACT_TYPE: &str = "@earendil-works/pi-evals:source";
pub const INVALID_SESSION_METADATA: &str = "Pi eval session artifact metadata is invalid.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalAttachment {
    pub name: String,
    pub content_type: String,
    pub body: String,
    pub body_encoding: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalArtifact {
    pub artifact_type: String,
    pub run_id: String,
    pub attachments: Vec<EvalAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReference {
    pub name: String,
    pub path: String,
}

pub fn record_eval_session_artifact(
    run_id: Option<&str>,
    session: Option<&str>,
) -> Result<Option<EvalArtifact>, String> {
    let Some(session) = session else {
        return Ok(None);
    };
    let Some(run_id) = run_id else {
        return Err(INVALID_SESSION_METADATA.into());
    };
    Ok(Some(EvalArtifact {
        artifact_type: SESSION_ARTIFACT_TYPE.into(),
        run_id: run_id.into(),
        attachments: vec![EvalAttachment {
            name: "session.jsonl".into(),
            content_type: "application/jsonl".into(),
            body: session.into(),
            body_encoding: "utf-8".into(),
        }],
    }))
}

pub fn record_eval_source_artifact(run_id: &str, attachment: EvalAttachment) -> EvalArtifact {
    EvalArtifact {
        artifact_type: SOURCE_ARTIFACT_TYPE.into(),
        run_id: run_id.into(),
        attachments: vec![attachment],
    }
}

pub(crate) fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

pub fn persist_eval_artifact_references(
    artifacts: &[EvalArtifact],
    run_id: &str,
    artifact_directory: &Path,
) -> Result<Vec<ArtifactReference>, String> {
    let mut references = Vec::new();
    for artifact in artifacts {
        if artifact.artifact_type != SESSION_ARTIFACT_TYPE
            && artifact.artifact_type != SOURCE_ARTIFACT_TYPE
        {
            continue;
        }
        if artifact.run_id != run_id {
            continue;
        }
        let category = if artifact.artifact_type == SESSION_ARTIFACT_TYPE {
            "sessions"
        } else {
            "sources"
        };
        for attachment in &artifact.attachments {
            let name = Path::new(&attachment.name)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if name != attachment.name {
                return Err(format!("Invalid eval artifact name: {}", attachment.name));
            }
            let directory = artifact_directory.join(category).join(sha256_hex(run_id));
            std::fs::create_dir_all(&directory).map_err(|err| err.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700));
            }
            let path = directory.join(name);
            std::fs::write(&path, &attachment.body).map_err(|err| err.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            let relative = PathBuf::from(category).join(sha256_hex(run_id)).join(name);
            references.push(ArtifactReference {
                name: name.to_string(),
                path: relative.to_string_lossy().replace('\\', "/"),
            });
        }
    }
    Ok(references)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_artifact_metadata_and_persist_lock_ts() {
        assert!(record_eval_session_artifact(Some("run-1"), None)
            .unwrap()
            .is_none());
        assert_eq!(
            record_eval_session_artifact(None, Some("{\"type\":\"session\"}\n")).unwrap_err(),
            INVALID_SESSION_METADATA
        );
        let session = record_eval_session_artifact(Some("run-1"), Some("{\"type\":\"session\"}\n"))
            .unwrap()
            .expect("session");
        assert_eq!(session.artifact_type, SESSION_ARTIFACT_TYPE);
        assert_eq!(session.attachments[0].name, "session.jsonl");
        assert_eq!(session.attachments[0].content_type, "application/jsonl");
        assert_eq!(session.attachments[0].body_encoding, "utf-8");

        let source = record_eval_source_artifact(
            "run-1",
            EvalAttachment {
                name: "hello.ts".into(),
                content_type: "text/typescript".into(),
                body: "export default function () {}\n".into(),
                body_encoding: "utf-8".into(),
            },
        );
        let root = tempfile::tempdir().unwrap();
        let references = persist_eval_artifact_references(
            &[
                session,
                EvalArtifact {
                    artifact_type: SESSION_ARTIFACT_TYPE.into(),
                    run_id: "run-2".into(),
                    attachments: Vec::new(),
                },
                source,
                EvalArtifact {
                    artifact_type: "internal:annotation".into(),
                    run_id: "run-1".into(),
                    attachments: Vec::new(),
                },
            ],
            "run-1",
            root.path(),
        )
        .unwrap();
        assert_eq!(references.len(), 2);
        assert_eq!(references[0].name, "session.jsonl");
        assert!(
            references[0].path.starts_with("sessions/")
                && references[0].path.ends_with("/session.jsonl")
        );
        assert_eq!(references[1].name, "hello.ts");
        assert!(
            references[1].path.starts_with("sources/") && references[1].path.ends_with("/hello.ts")
        );
        for item in &references {
            let expected = if item.name == "session.jsonl" {
                "{\"type\":\"session\"}\n"
            } else {
                "export default function () {}\n"
            };
            assert_eq!(
                std::fs::read_to_string(root.path().join(&item.path)).unwrap(),
                expected
            );
        }
        assert_eq!(
            persist_eval_artifact_references(
                &[EvalArtifact {
                    artifact_type: SOURCE_ARTIFACT_TYPE.into(),
                    run_id: "run-1".into(),
                    attachments: vec![EvalAttachment {
                        name: "nested/hello.ts".into(),
                        content_type: "text/typescript".into(),
                        body: "x".into(),
                        body_encoding: "utf-8".into(),
                    }],
                }],
                "run-1",
                root.path(),
            )
            .unwrap_err(),
            "Invalid eval artifact name: nested/hello.ts"
        );
    }
}
