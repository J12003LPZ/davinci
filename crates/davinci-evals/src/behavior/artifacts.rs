//! Durable artifact roots used by behavior evaluations.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::{Component, Path, PathBuf};

const MANIFEST_FILE: &str = "manifest.json";
const HASHES_FILE: &str = "hashes.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvalRunManifest {
    pub run_id: String,
    pub davinci_commit: String,
    pub runner_version: String,
    pub suite_hash: String,
    pub provider: String,
    pub model: String,
    pub baseline_prompt_hash: String,
    pub candidate_prompt_hash: String,
    pub permission_mode: String,
    pub tool_surface_hash: String,
    pub repeats: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactHashes {
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ArtifactRoot {
    pub root: PathBuf,
}

impl ArtifactRoot {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn write_json<T: Serialize>(
        &self,
        relative: impl AsRef<Path>,
        value: &T,
    ) -> Result<(), String> {
        let relative = safe_relative_path(relative.as_ref())?;
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create artifact directory: {error}"))?;
        }
        let encoded = serde_json::to_vec_pretty(value)
            .map_err(|error| format!("failed to encode artifact: {error}"))?;
        fs::write(&path, encoded)
            .map_err(|error| format!("failed to write artifact {}: {error}", path.display()))
    }

    pub fn write_text(&self, relative: impl AsRef<Path>, contents: &str) -> Result<(), String> {
        let relative = safe_relative_path(relative.as_ref())?;
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create artifact directory: {error}"))?;
        }
        fs::write(&path, contents)
            .map_err(|error| format!("failed to write artifact {}: {error}", path.display()))
    }

    pub fn write_manifest(&self, manifest: &EvalRunManifest) -> Result<(), String> {
        validate_manifest(manifest)?;
        self.write_json(MANIFEST_FILE, manifest)
    }

    pub fn read_manifest(&self) -> Result<EvalRunManifest, String> {
        read_json(&self.root.join(MANIFEST_FILE), "manifest")
    }

    pub fn hash_artifacts(&self) -> Result<ArtifactHashes, String> {
        let mut files = BTreeMap::new();
        collect_hashes(&self.root, &self.root, &mut files)?;
        Ok(ArtifactHashes { files })
    }

    pub fn finalize_hashes(&self) -> Result<ArtifactHashes, String> {
        let hashes = self.hash_artifacts()?;
        self.write_json(HASHES_FILE, &hashes)?;
        Ok(hashes)
    }

    pub fn validate_complete(&self) -> Result<EvalRunManifest, String> {
        let manifest = self.read_manifest()?;
        validate_manifest(&manifest)?;
        for required in ["summary.md", "summary.json"] {
            if !self.root.join(required).is_file() {
                return Err(format!("artifact run is missing {required}"));
            }
        }
        let recorded: ArtifactHashes = read_json(&self.root.join(HASHES_FILE), "hashes")?;
        if recorded.files.is_empty() {
            return Err("artifact run has no content hashes".into());
        }
        let actual = self.hash_artifacts()?;
        if actual != recorded {
            return Err("artifact content hashes do not match hashes.json".into());
        }
        Ok(manifest)
    }
}

pub fn persist_eval_run<T: Serialize>(
    parent: impl AsRef<Path>,
    manifest: &EvalRunManifest,
    summary_markdown: &str,
    summary_json: &T,
) -> Result<(PathBuf, ArtifactHashes), String> {
    validate_manifest(manifest)?;
    let run_root = parent.as_ref().join(&manifest.run_id);
    if run_root.exists() {
        return Err(format!(
            "artifact run already exists: {}",
            run_root.display()
        ));
    }
    let artifacts = ArtifactRoot::new(&run_root);
    artifacts.write_manifest(manifest)?;
    artifacts.write_text("summary.md", summary_markdown)?;
    artifacts.write_json("summary.json", summary_json)?;
    let hashes = artifacts.finalize_hashes()?;
    artifacts.validate_complete()?;
    Ok((run_root, hashes))
}

fn validate_manifest(manifest: &EvalRunManifest) -> Result<(), String> {
    let fields = [
        ("run_id", manifest.run_id.as_str()),
        ("davinci_commit", manifest.davinci_commit.as_str()),
        ("runner_version", manifest.runner_version.as_str()),
        ("suite_hash", manifest.suite_hash.as_str()),
        ("provider", manifest.provider.as_str()),
        ("model", manifest.model.as_str()),
        (
            "baseline_prompt_hash",
            manifest.baseline_prompt_hash.as_str(),
        ),
        (
            "candidate_prompt_hash",
            manifest.candidate_prompt_hash.as_str(),
        ),
        ("permission_mode", manifest.permission_mode.as_str()),
        ("tool_surface_hash", manifest.tool_surface_hash.as_str()),
    ];
    for (name, value) in fields {
        if value.trim().is_empty() {
            return Err(format!("manifest field {name} is empty"));
        }
    }
    if manifest.run_id.contains('/') || manifest.run_id.contains('\\') {
        return Err("manifest run_id must be a single path component".into());
    }
    if manifest.baseline_prompt_hash == manifest.candidate_prompt_hash {
        return Err("baseline and candidate prompt hashes must differ".into());
    }
    if manifest.repeats == 0 {
        return Err("manifest repeats must be greater than zero".into());
    }
    Ok(())
}

fn safe_relative_path(path: &Path) -> Result<&Path, String> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(format!(
            "artifact path must stay below the artifact root: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn read_json<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|error| format!("failed to read {label}: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("failed to decode {label}: {error}"))
}

fn collect_hashes(
    root: &Path,
    current: &Path,
    hashes: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    if !current.is_dir() {
        return Err(format!(
            "artifact root is not a directory: {}",
            current.display()
        ));
    }
    for entry in
        fs::read_dir(current).map_err(|error| format!("failed to read artifacts: {error}"))?
    {
        let entry = entry.map_err(|error| format!("failed to read artifact entry: {error}"))?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| format!("failed to inspect artifact entry: {error}"))?
            .is_symlink()
        {
            return Err(format!(
                "symlinked artifact paths are not allowed: {}",
                path.display()
            ));
        }
        if path.is_dir() {
            collect_hashes(root, &path, hashes)?;
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("failed to relativize artifact path: {error}"))?;
        let relative_string = relative.to_string_lossy().replace('\\', "/");
        if relative_string == MANIFEST_FILE || relative_string == HASHES_FILE {
            continue;
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("failed to read artifact {}: {error}", path.display()))?;
        hashes.insert(relative_string, sha256_hex(&bytes));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest() -> EvalRunManifest {
        EvalRunManifest {
            run_id: "run-1".into(),
            davinci_commit: "commit".into(),
            runner_version: "runner".into(),
            suite_hash: "suite".into(),
            provider: "provider".into(),
            model: "model".into(),
            baseline_prompt_hash: "stable".into(),
            candidate_prompt_hash: "preview".into(),
            permission_mode: "default".into(),
            tool_surface_hash: "tools".into(),
            repeats: 3,
        }
    }

    #[test]
    fn persisted_run_has_layout_hashes_and_tamper_detection() {
        let root = tempfile::tempdir().unwrap();
        let (run_root, hashes) = persist_eval_run(
            root.path(),
            &manifest(),
            "# Summary\n",
            &json!({"passed": true}),
        )
        .unwrap();
        assert!(run_root.join(MANIFEST_FILE).is_file());
        assert!(run_root.join("summary.md").is_file());
        assert!(run_root.join("summary.json").is_file());
        assert!(run_root.join(HASHES_FILE).is_file());
        assert!(hashes.files.contains_key("summary.md"));
        assert!(ArtifactRoot::new(run_root.clone())
            .validate_complete()
            .is_ok());

        fs::write(run_root.join("summary.md"), "tampered").unwrap();
        assert!(ArtifactRoot::new(run_root).validate_complete().is_err());
    }

    #[test]
    fn incomplete_or_unsafe_manifests_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let artifacts = ArtifactRoot::new(root.path());
        let mut invalid = manifest();
        invalid.repeats = 0;
        assert!(artifacts.write_manifest(&invalid).is_err());
        assert!(artifacts.write_json("../escape.json", &json!({})).is_err());
    }
}
