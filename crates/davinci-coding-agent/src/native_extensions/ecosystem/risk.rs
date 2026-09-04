//! Deterministic change-risk classification over graph-owned file mutations.
//!
//! Evaluates changed paths and diff content against sensitive surfaces
//! (e.g. auth, cryptography, process execution, manifests) without model calls.

use serde::{Deserialize, Serialize};

use crate::native_extensions::graph::mutation::GraphMutation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeRisk {
    None,
    Low,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskReason {
    pub surface: &'static str,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskAssessment {
    pub level: ChangeRisk,
    pub reasons: Vec<RiskReason>,
}

fn is_doc_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".txt")
        || lower.ends_with(".rst")
        || lower.starts_with("docs/")
        || lower.contains("/docs/")
}

fn is_manifest_file(file_name: &str) -> bool {
    matches!(
        file_name,
        "cargo.toml"
            | "cargo.lock"
            | "package.json"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "requirements.txt"
            | "pipfile"
            | "pipfile.lock"
            | "poetry.lock"
            | "go.mod"
            | "go.sum"
            | "pom.xml"
            | "build.gradle"
    )
}

pub fn assess_change_risk(mutation: &GraphMutation) -> RiskAssessment {
    if mutation.files.is_empty() && mutation.patch_chunks.is_empty() {
        return RiskAssessment {
            level: ChangeRisk::None,
            reasons: Vec::new(),
        };
    }

    let mut reasons = Vec::new();

    for file in &mutation.files {
        let path_lower = file.path.to_ascii_lowercase();
        let file_name = path_lower.rsplit(['/', '\\']).next().unwrap_or(&path_lower);

        // 1. Dependency manifests / lockfiles
        if is_manifest_file(file_name) {
            reasons.push(RiskReason {
                surface: "dependency_manifest",
                path: file.path.clone(),
            });
            continue;
        }

        // Docs are exempt from path-based sensitive word matches
        if is_doc_path(&file.path) {
            continue;
        }

        // 2. Authentication & Authorization / Credentials
        if path_lower.contains("auth")
            || path_lower.contains("permission")
            || path_lower.contains("policy")
            || path_lower.contains("credential")
            || path_lower.contains("secret")
            || path_lower.contains("keychain")
            || path_lower.contains("oauth")
            || path_lower.contains("rbac")
            || path_lower.contains("iam")
        {
            reasons.push(RiskReason {
                surface: "authentication_authorization",
                path: file.path.clone(),
            });
            continue;
        }

        // 3. Cryptography & TLS
        if path_lower.contains("crypto")
            || path_lower.contains("cipher")
            || path_lower.contains("tls")
            || path_lower.contains("ssl")
            || path_lower.contains("cert")
            || path_lower.contains("signing")
        {
            reasons.push(RiskReason {
                surface: "cryptography",
                path: file.path.clone(),
            });
            continue;
        }

        // 4. Shell / Process execution
        if file_name == "process.rs"
            || file_name == "exec.rs"
            || file_name == "shell.rs"
            || file_name == "command.rs"
            || file_name == "spawn.rs"
            || path_lower.contains("/exec/")
            || path_lower.contains("/process/")
        {
            reasons.push(RiskReason {
                surface: "process_execution",
                path: file.path.clone(),
            });
            continue;
        }

        // 5. Network / Transport
        if path_lower.contains("network")
            || path_lower.contains("transport")
            || path_lower.contains("socket")
        {
            reasons.push(RiskReason {
                surface: "network_transport",
                path: file.path.clone(),
            });
            continue;
        }

        // 6. Extension / Plugin loading
        if path_lower.contains("plugin")
            || path_lower.contains("extension_runner")
            || path_lower.contains("addon")
        {
            reasons.push(RiskReason {
                surface: "extension_loading",
                path: file.path.clone(),
            });
            continue;
        }

        // 7. Protocol boundaries / Deserialization
        if path_lower.contains("protocol")
            || path_lower.contains("deserializ")
            || path_lower.contains("ipc")
            || path_lower.contains("wire")
        {
            reasons.push(RiskReason {
                surface: "protocol_boundary",
                path: file.path.clone(),
            });
            continue;
        }
    }

    // Inspect patch diffs for code-level dangerous primitives
    for chunk in &mutation.patch_chunks {
        if is_doc_path(&chunk.file) {
            continue;
        }
        let lower = chunk.patch.to_ascii_lowercase();

        if (lower.contains("command::new(")
            || lower.contains("process::command")
            || lower.contains("shell_command(")
            || lower.contains("spawn_child("))
            && !reasons
                .iter()
                .any(|r| r.path == chunk.file && r.surface == "process_execution")
        {
            reasons.push(RiskReason {
                surface: "process_execution",
                path: chunk.file.clone(),
            });
        }

        if (lower.contains("tar::archive")
            || lower.contains("zip_extract")
            || lower.contains("archive.unpack")
            || lower.contains("fs::remove_dir_all"))
            && !reasons
                .iter()
                .any(|r| r.path == chunk.file && r.surface == "filesystem_traversal")
        {
            reasons.push(RiskReason {
                surface: "filesystem_traversal",
                path: chunk.file.clone(),
            });
        }

        if (lower.contains("private_key")
            || lower.contains("secret_key")
            || lower.contains("api_key")
            || lower.contains("bearer "))
            && !reasons
                .iter()
                .any(|r| r.path == chunk.file && r.surface == "secret_handling")
        {
            reasons.push(RiskReason {
                surface: "secret_handling",
                path: chunk.file.clone(),
            });
        }
    }

    let level = if reasons.is_empty() {
        ChangeRisk::Low
    } else {
        ChangeRisk::High
    };

    RiskAssessment { level, reasons }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::mutation::{ChangedFile, PatchChunk};

    #[test]
    fn empty_mutation_is_none_risk() {
        let mutation = GraphMutation::default();
        let assessment = assess_change_risk(&mutation);
        assert_eq!(assessment.level, ChangeRisk::None);
        assert!(assessment.reasons.is_empty());
    }

    #[test]
    fn documentation_changes_avoid_false_positive_high_risk() {
        let mutation = GraphMutation {
            files: vec![ChangedFile::modified("docs/architecture/token_auth.md")],
            patch_chunks: vec![PatchChunk {
                file: "docs/architecture/token_auth.md".into(),
                patch: "+ This section describes token and password auth handling.".into(),
            }],
        };
        let assessment = assess_change_risk(&mutation);
        assert_eq!(assessment.level, ChangeRisk::Low);
        assert!(assessment.reasons.is_empty());
    }

    #[test]
    fn harmless_ui_change_is_low_risk() {
        let mutation = GraphMutation {
            files: vec![ChangedFile::modified("crates/ui/src/button.rs")],
            patch_chunks: vec![PatchChunk {
                file: "crates/ui/src/button.rs".into(),
                patch: "+ pub fn render_button() { println!(\"Button\"); }".into(),
            }],
        };
        let assessment = assess_change_risk(&mutation);
        assert_eq!(assessment.level, ChangeRisk::Low);
        assert!(assessment.reasons.is_empty());
    }

    #[test]
    fn high_risk_surfaces_detected_deterministically() {
        // 1. Dependency manifest
        let m1 = GraphMutation {
            files: vec![ChangedFile::modified("Cargo.lock")],
            patch_chunks: vec![],
        };
        let a1 = assess_change_risk(&m1);
        assert_eq!(a1.level, ChangeRisk::High);
        assert_eq!(a1.reasons[0].surface, "dependency_manifest");

        // 2. Auth module
        let m2 = GraphMutation {
            files: vec![ChangedFile::modified("src/auth/jwt.rs")],
            patch_chunks: vec![],
        };
        let a2 = assess_change_risk(&m2);
        assert_eq!(a2.level, ChangeRisk::High);
        assert_eq!(a2.reasons[0].surface, "authentication_authorization");

        // 3. Cryptography
        let m3 = GraphMutation {
            files: vec![ChangedFile::modified("src/crypto/signing.rs")],
            patch_chunks: vec![],
        };
        let a3 = assess_change_risk(&m3);
        assert_eq!(a3.level, ChangeRisk::High);
        assert_eq!(a3.reasons[0].surface, "cryptography");

        // 4. Shell / Process execution via diff
        let m4 = GraphMutation {
            files: vec![ChangedFile::modified("src/utils.rs")],
            patch_chunks: vec![PatchChunk {
                file: "src/utils.rs".into(),
                patch: "+ let output = std::process::Command::new(\"sh\").output();".into(),
            }],
        };
        let a4 = assess_change_risk(&m4);
        assert_eq!(a4.level, ChangeRisk::High);
        assert_eq!(a4.reasons[0].surface, "process_execution");

        // 5. Filesystem traversal / archive
        let m5 = GraphMutation {
            files: vec![ChangedFile::modified("src/extractor.rs")],
            patch_chunks: vec![PatchChunk {
                file: "src/extractor.rs".into(),
                patch: "+ archive.unpack(dest)?;".into(),
            }],
        };
        let a5 = assess_change_risk(&m5);
        assert_eq!(a5.level, ChangeRisk::High);
        assert_eq!(a5.reasons[0].surface, "filesystem_traversal");
    }
}
