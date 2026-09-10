//! Bounded, declarative graph exports.
//!
//! Exports are projected from a validated saved definition. Runtime state,
//! worker output, paths, credentials, and provenance are intentionally kept
//! out of the document.

use super::definitions::{
    project_graphs_dir, safe_graph_name, validate_graphs_dir, validate_saved_definition,
    SavedBudgets, SavedGraphDefinitionV1, SavedGraphTopology, SavedParameter, SavedStageBinding,
    SavedVerificationPolicy,
};
use super::replay::sha256_hex;
use super::store::atomic_write;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAX_EXPORT_BYTES: usize = 128 * 1024;

/// Public fields are deliberately enumerated instead of serializing GraphRun.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphExportDocument {
    pub schema_version: u32,
    pub name: String,
    pub description: String,
    pub topology: SavedGraphTopology,
    pub bindings: Vec<SavedStageBinding>,
    pub roles: Vec<ExportRole>,
    pub artifact_contracts: BTreeMap<String, u32>,
    pub budgets: Option<SavedBudgets>,
    pub verification_policy: Option<SavedVerificationPolicy>,
    pub parameters: Vec<ExportParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRole {
    pub node_id: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportParameter {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub param_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default)]
    pub allowed_values: Vec<String>,
}

#[allow(dead_code)]
pub fn export_field_allowed(field: &str) -> bool {
    matches!(
        field,
        "schema_version"
            | "name"
            | "description"
            | "topology"
            | "bindings"
            | "roles"
            | "artifact_contracts"
            | "budgets"
            | "verification_policy"
            | "parameters"
    )
}

pub fn export_definition(def: &SavedGraphDefinitionV1) -> Result<GraphExportDocument, String> {
    validate_saved_definition(def)?;
    if !safe_graph_name(&def.name) {
        return Err(format!("invalid graph name '{}'", def.name));
    }

    let parameters = def.parameters.iter().map(sanitize_parameter).collect();
    let roles = def
        .graph
        .nodes
        .iter()
        .map(|node| ExportRole {
            node_id: node.id.clone(),
            role: node.role.to_string(),
        })
        .collect();
    Ok(GraphExportDocument {
        schema_version: def.schema_version,
        name: def.name.clone(),
        description: redact_text(&def.description),
        topology: def.graph.clone(),
        bindings: def.bindings.clone(),
        roles,
        artifact_contracts: def.artifact_contract_versions.clone(),
        budgets: def.budgets.clone(),
        verification_policy: def.verification_policy.clone(),
        parameters,
    })
}

pub fn render_export_document(document: &GraphExportDocument) -> Result<String, String> {
    let bytes = serde_json::to_vec_pretty(document)
        .map_err(|error| format!("failed to serialize graph export: {error}"))?;
    if bytes.len() > MAX_EXPORT_BYTES {
        return Err(format!(
            "graph export exceeds maximum size of {MAX_EXPORT_BYTES} bytes"
        ));
    }
    String::from_utf8(bytes).map_err(|error| format!("graph export is not UTF-8: {error}"))
}

/// Return the stable checksum of the exact bounded bytes used for export.
pub fn export_checksum(rendered: &str) -> String {
    sha256_hex(rendered.as_bytes())
}

pub fn write_project_export(
    cwd: &Path,
    name: &str,
    document: &GraphExportDocument,
    overwrite: bool,
) -> Result<PathBuf, String> {
    if !safe_graph_name(name) {
        return Err(format!("invalid export name '{name}'"));
    }
    if document.name != name {
        return Err(format!(
            "export document name '{}' does not match destination '{name}'",
            document.name
        ));
    }
    let dir = project_graphs_dir(cwd);
    validate_graphs_dir(&dir, cwd)?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create graphs directory: {error}"))?;
    validate_graphs_dir(&dir, cwd)?;

    let path = dir.join(format!("{name}.export.json"));
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if metadata.file_type().is_symlink() {
            return Err("export destination must not be a symlink".into());
        }
        if !overwrite {
            return Err(format!(
                "export '{}' already exists; use --overwrite",
                path.display()
            ));
        }
    }
    let rendered = render_export_document(document)?;
    atomic_write(&path, rendered.as_bytes())
        .map_err(|error| format!("failed to write graph export: {error}"))?;
    Ok(path)
}

fn sanitize_parameter(parameter: &SavedParameter) -> ExportParameter {
    let sensitive_name = parameter.name.to_ascii_lowercase();
    let sensitive = ["secret", "token", "password", "api_key", "apikey"]
        .iter()
        .any(|marker| sensitive_name.contains(marker));
    ExportParameter {
        name: parameter.name.clone(),
        description: parameter.description.as_deref().map(redact_text),
        param_type: parameter.param_type.clone(),
        default: if sensitive {
            None
        } else {
            sanitize_value(parameter.default.as_deref())
        },
        allowed_values: if sensitive {
            Vec::new()
        } else {
            parameter
                .allowed_values
                .iter()
                .filter_map(|value| sanitize_value(Some(value)))
                .collect()
        },
    }
}

fn sanitize_value(value: Option<&str>) -> Option<String> {
    value.map(redact_text).filter(|value| value != "[REDACTED]")
}

fn redact_text(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let markers = [
        "api_key=",
        "apikey=",
        "token=",
        "password=",
        "secret=",
        "api-key:",
        "token:",
        "password:",
        "secret:",
    ];
    if markers.iter().any(|marker| lower.contains(marker)) {
        "[REDACTED]".into()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_definition(name: &str) -> SavedGraphDefinitionV1 {
        SavedGraphDefinitionV1 {
            schema_version: 1,
            name: name.into(),
            description: "token=do-not-export".into(),
            graph: SavedGraphTopology {
                graph_id: "graph-1".into(),
                version: 1,
                mode: super::super::topology::GraphMode::Simple,
                nodes: vec![super::super::topology::NodeDefinition {
                    id: "classify".into(),
                    role: super::super::types::Role::Classifier,
                    expect: super::super::types::ArtifactKind::Classification,
                    required: true,
                    allows_mutation: false,
                }],
                edges: vec![],
            },
            bindings: vec![SavedStageBinding {
                node_id: "classify".into(),
                stage: "classify".into(),
                input_artifacts: vec![],
            }],
            budgets: None,
            verification_policy: None,
            artifact_contract_versions: BTreeMap::new(),
            parameters: vec![SavedParameter {
                name: "api_token".into(),
                description: Some("token=private".into()),
                param_type: "string".into(),
                default: Some("private-value".into()),
                allowed_values: vec!["private-value".into()],
            }],
        }
    }

    #[test]
    fn f14_export_field_allowlist_is_explicit() {
        assert!(export_field_allowed("schema_version"));
        assert!(export_field_allowed("verification_policy"));
        assert!(!export_field_allowed("run_id"));
        assert!(!export_field_allowed("resource_snapshot"));
        assert!(!export_field_allowed("provenance"));
    }

    #[test]
    fn f14_export_redacts_secret_markers() {
        assert_eq!(redact_text("token=do-not-export"), "[REDACTED]");
        assert_eq!(redact_text("safe description"), "safe description");

        let parameter = SavedParameter {
            name: "api_token".into(),
            description: Some("token=private".into()),
            param_type: "string".into(),
            default: Some("private-value".into()),
            allowed_values: vec!["private-value".into()],
        };
        let sanitized = sanitize_parameter(&parameter);
        assert_eq!(sanitized.description.as_deref(), Some("[REDACTED]"));
        assert!(sanitized.default.is_none());
        assert!(sanitized.allowed_values.is_empty());

        let ordinary = SavedParameter {
            name: "description".into(),
            description: None,
            param_type: "string".into(),
            default: Some("token=also-private".into()),
            allowed_values: vec!["safe".into(), "password=also-private".into()],
        };
        let sanitized = sanitize_parameter(&ordinary);
        assert!(sanitized.default.is_none());
        assert_eq!(sanitized.allowed_values, vec!["safe"]);
    }

    #[test]
    fn f14_export_checksum_is_stable_for_exact_bytes() {
        let first = export_checksum("{\n  \"name\": \"graph\"\n}\n");
        let second = export_checksum("{\n  \"name\": \"graph\"\n}\n");
        assert_eq!(first, second);
        assert_ne!(first, export_checksum("{\n  \"name\": \"other\"\n}\n"));
    }

    #[test]
    fn f14_export_serializes_only_redacted_public_definition() {
        let definition = valid_definition("public-graph");
        let document = export_definition(&definition).unwrap();
        let value = serde_json::to_value(&document).unwrap();
        let object = value.as_object().unwrap();
        for private in [
            "runId",
            "resourceSnapshot",
            "provenance",
            "prompt",
            "transcript",
            "apiKey",
        ] {
            assert!(
                !object.contains_key(private),
                "private field leaked: {private}"
            );
        }
        assert_eq!(value["description"], "[REDACTED]");
        assert!(value["parameters"][0].get("default").is_none());
        assert!(value["parameters"][0]["allowedValues"]
            .as_array()
            .is_some_and(Vec::is_empty));
    }

    #[test]
    fn f14_export_rejects_destination_name_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let document = export_definition(&valid_definition("document-name")).unwrap();
        let error = write_project_export(dir.path(), "other-name", &document, false).unwrap_err();
        assert!(error.contains("does not match destination"));
    }
}
