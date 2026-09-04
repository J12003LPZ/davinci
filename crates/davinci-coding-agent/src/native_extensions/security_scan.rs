//! Network-off-by-default security scanning primitives.
//!
//! The scanner treats all repository text as untrusted data, constrains scope
//! to the requested repository, and writes immutable scan artifacts outside
//! that repository. It is intentionally deterministic so a later deep worker
//! can consume the same manifest and candidate ledger.

use crate::native_extensions::ecosystem::verification::SecurityVerification;
use davinci_agent::{ToolError, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct SecurityVerifyRequest<'a> {
    pub cwd: &'a Path,
    pub changed_files: &'a [String],
    pub graph_run_id: &'a str,
}

const SECURITY_SCHEMA_VERSION: u32 = 1;
const SEALED_ARTIFACTS: [&str; 4] = [
    "findings.json",
    "coverage.json",
    "report.md",
    "results.sarif",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScanConfig {
    #[serde(default)]
    pub allow_network: bool,
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
    #[serde(default = "default_true")]
    pub include_hidden: bool,
}

fn default_true() -> bool {
    true
}
fn default_max_file_bytes() -> u64 {
    2 * 1024 * 1024
}

impl Default for SecurityScanConfig {
    fn default() -> Self {
        Self {
            allow_network: false,
            max_file_bytes: default_max_file_bytes(),
            include_hidden: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScanStatus {
    Started,
    Draft,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FindingSeverity {
    Critical,
    High,
    Medium,
    Low,
    Informational,
}

impl FindingSeverity {
    fn rank(self) -> u8 {
        match self {
            Self::Critical => 5,
            Self::High => 4,
            Self::Medium => 3,
            Self::Low => 2,
            Self::Informational => 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityFinding {
    pub id: String,
    pub rule_id: String,
    pub severity: FindingSeverity,
    pub file: String,
    pub line: usize,
    pub message: String,
    pub evidence: String,
    #[serde(default)]
    pub validated: bool,
    #[serde(default)]
    pub false_positive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityCandidate {
    pub id: String,
    pub rule_id: String,
    pub file: String,
    pub line: usize,
    pub reason: String,
    #[serde(default)]
    pub validated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack_path: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityCoverage {
    pub files_scanned: usize,
    pub files_skipped: usize,
    pub bytes_scanned: u64,
    pub candidate_count: usize,
    pub finding_count: usize,
    pub network_used: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityArtifactSeal {
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScanManifest {
    pub scan_id: String,
    pub repo_id: String,
    pub root: String,
    pub status: ScanStatus,
    pub started_at: u64,
    pub completed_at: Option<u64>,
    pub allow_network: bool,
    pub scope_digest: String,
    pub artifact_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<BTreeMap<String, SecurityArtifactSeal>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScan {
    pub manifest: SecurityScanManifest,
    pub coverage: SecurityCoverage,
    pub candidates: Vec<SecurityCandidate>,
    pub findings: Vec<SecurityFinding>,
}

#[derive(Debug, Clone)]
pub struct SecurityArtifactStore {
    root: PathBuf,
}

impl SecurityArtifactStore {
    pub fn new(repo_id: &str, scan_id: &str) -> Result<Self, ToolError> {
        if !safe_component(repo_id) || !safe_component(scan_id) {
            return Err(ToolError::Failed(
                "invalid security artifact identity".into(),
            ));
        }
        let root = std::env::temp_dir()
            .join("pi-security-scans")
            .join(repo_id)
            .join(scan_id);
        fs::create_dir_all(&root).map_err(|err| ToolError::Failed(err.to_string()))?;
        Ok(Self { root })
    }

    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn write_scan(&self, scan: &SecurityScan) -> Result<String, ToolError> {
        let findings_document = json!({
            "schemaVersion": SECURITY_SCHEMA_VERSION,
            "findings": scan.findings,
        });
        let candidates_document = json!({
            "schemaVersion": SECURITY_SCHEMA_VERSION,
            "candidates": scan.candidates,
        });
        let findings = serde_json::to_vec_pretty(&findings_document)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        let candidates = serde_json::to_vec_pretty(&candidates_document)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        let coverage = serde_json::to_vec_pretty(&scan.coverage)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        fs::write(self.root.join("findings.json"), &findings)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        fs::write(self.root.join("candidates.json"), &candidates)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        fs::write(self.root.join("coverage.json"), &coverage)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        let report = render_report(scan);
        fs::write(self.root.join("report.md"), &report)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        let sarif = render_sarif(scan);
        let sarif_bytes =
            serde_json::to_vec_pretty(&sarif).map_err(|err| ToolError::Failed(err.to_string()))?;
        fs::write(self.root.join("report.sarif"), &sarif_bytes)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        fs::write(self.root.join("results.sarif"), &sarif_bytes)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        let digest = self.combined_digest()?;
        fs::write(self.root.join("artifact.sha256"), &digest)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        self.write_manifest(scan)?;
        Ok(digest)
    }

    fn combined_digest(&self) -> Result<String, ToolError> {
        let mut bytes = Vec::new();
        for name in [
            "findings.json",
            "candidates.json",
            "coverage.json",
            "report.md",
            "results.sarif",
        ] {
            let content =
                fs::read(self.root.join(name)).map_err(|err| ToolError::Failed(err.to_string()))?;
            bytes.extend_from_slice(&content);
            bytes.push(b'\n');
        }
        Ok(sha256_hex(&bytes))
    }

    pub fn seal_artifacts(&self) -> Result<BTreeMap<String, SecurityArtifactSeal>, ToolError> {
        SEALED_ARTIFACTS
            .into_iter()
            .map(|name| {
                let bytes = fs::read(self.root.join(name))
                    .map_err(|err| ToolError::Failed(err.to_string()))?;
                Ok((
                    name.to_string(),
                    SecurityArtifactSeal {
                        sha256: sha256_hex(&bytes),
                        bytes: bytes.len() as u64,
                    },
                ))
            })
            .collect()
    }

    pub fn validate_manifest(&self, manifest: &SecurityScanManifest) -> Result<Value, ToolError> {
        if manifest.status != ScanStatus::Completed {
            return Err(ToolError::Failed("security scan is not completed".into()));
        }
        if manifest.sealed_at.is_none() {
            return Err(ToolError::Failed(
                "security scan has no artifact seal".into(),
            ));
        }
        let seals = manifest
            .artifacts
            .as_ref()
            .ok_or_else(|| ToolError::Failed("security scan has no artifact seal".into()))?;
        for name in SEALED_ARTIFACTS {
            let expected = seals
                .get(name)
                .ok_or_else(|| ToolError::Failed(format!("artifact seal missing {name}")))?;
            let bytes =
                fs::read(self.root.join(name)).map_err(|err| ToolError::Failed(err.to_string()))?;
            let actual = sha256_hex(&bytes);
            if expected.bytes != bytes.len() as u64 || expected.sha256 != actual {
                return Err(ToolError::Failed(format!(
                    "sealed artifact changed: {name}"
                )));
            }
        }
        let expected_digest = manifest
            .artifact_digest
            .as_deref()
            .ok_or_else(|| ToolError::Failed("security scan has no artifact digest".into()))?;
        let stored_digest = fs::read_to_string(self.root.join("artifact.sha256"))
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        if stored_digest.trim() != expected_digest || self.combined_digest()? != expected_digest {
            return Err(ToolError::Failed(
                "security artifact digest mismatch".into(),
            ));
        }
        let findings: Value = serde_json::from_slice(
            &fs::read(self.root.join("findings.json"))
                .map_err(|err| ToolError::Failed(err.to_string()))?,
        )
        .map_err(|err| ToolError::Failed(err.to_string()))?;
        let finding_ids = findings
            .get("findings")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(json!({
            "valid": true,
            "scanId": manifest.scan_id,
            "artifactDigest": expected_digest,
            "findingIds": finding_ids,
        }))
    }

    fn write_manifest(&self, scan: &SecurityScan) -> Result<(), ToolError> {
        let mut manifest = serde_json::to_value(&scan.manifest)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        if let Value::Object(fields) = &mut manifest {
            fields.insert("schemaVersion".into(), Value::from(SECURITY_SCHEMA_VERSION));
        }
        let content = serde_json::to_vec_pretty(&manifest)
            .map_err(|err| ToolError::Failed(err.to_string()))?;
        fs::write(self.root.join("scan-manifest.json"), content)
            .map_err(|err| ToolError::Failed(err.to_string()))
    }

    pub fn read_report(&self) -> Result<String, ToolError> {
        fs::read_to_string(self.root.join("report.md"))
            .map_err(|err| ToolError::Failed(err.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct SecurityScanController {
    cwd: PathBuf,
    pub config: SecurityScanConfig,
    current: Option<SecurityScan>,
    artifact: Option<SecurityArtifactStore>,
}

impl Default for SecurityScanController {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

impl SecurityScanController {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            config: SecurityScanConfig::default(),
            current: None,
            artifact: None,
        }
    }

    pub fn start(&mut self, scope: Option<&str>) -> Result<SecurityScan, ToolError> {
        let repo_id = repo_id(&self.cwd);
        let now = now_ms();
        let scan_id = format_scan_id(&repo_id, now, now_nanos());
        let files = enumerate_scope(&self.cwd, scope, &self.config)?;
        let scope_digest = sha256_hex(
            files
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("\n")
                .as_bytes(),
        );
        let mut scan = SecurityScan {
            manifest: SecurityScanManifest {
                scan_id: scan_id.clone(),
                repo_id: repo_id.clone(),
                root: self.cwd.to_string_lossy().into_owned(),
                status: ScanStatus::Started,
                started_at: now,
                completed_at: None,
                allow_network: self.config.allow_network,
                scope_digest,
                artifact_digest: None,
                sealed_at: None,
                artifacts: None,
            },
            coverage: SecurityCoverage {
                files_scanned: 0,
                files_skipped: 0,
                bytes_scanned: 0,
                candidate_count: 0,
                finding_count: 0,
                network_used: false,
            },
            candidates: Vec::new(),
            findings: Vec::new(),
        };
        for path in files {
            match scan_file(&self.cwd, &path, &self.config, &mut scan) {
                Ok(()) => scan.coverage.files_scanned += 1,
                Err(_) => scan.coverage.files_skipped += 1,
            }
        }
        scan.coverage.candidate_count = scan.candidates.len();
        scan.coverage.finding_count = scan.findings.len();
        scan.manifest.status = ScanStatus::Draft;
        let artifact = SecurityArtifactStore::new(&repo_id, &scan_id)?;
        let digest = artifact.write_scan(&scan)?;
        scan.manifest.artifact_digest = Some(digest);
        artifact.write_manifest(&scan)?;
        self.artifact = Some(artifact);
        self.current = Some(scan.clone());
        Ok(scan)
    }

    pub fn current(&self) -> Option<SecurityScan> {
        self.current.clone()
    }

    fn validate_candidates(&mut self, args: &Value) -> Result<SecurityScan, ToolError> {
        let candidate_id = args
            .get("candidateId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ToolError::Failed("candidateId is required".into()))?;
        let disposition = args
            .get("disposition")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::Failed("candidate disposition is required".into()))?;
        if !matches!(
            disposition,
            "reportable" | "not_reportable" | "duplicate" | "needs_review"
        ) {
            return Err(ToolError::Failed(
                "unsupported candidate disposition".into(),
            ));
        }
        let mut scan = self
            .current
            .clone()
            .ok_or_else(|| ToolError::Failed("no security scan is active".into()))?;
        ensure_draft(&scan)?;
        let candidate = scan
            .candidates
            .iter_mut()
            .find(|candidate| candidate.id == candidate_id)
            .ok_or_else(|| ToolError::Failed("candidate was not found".into()))?;
        candidate.disposition = Some(disposition.to_string());
        candidate.validation_reason = args
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        candidate.validated = disposition != "needs_review";
        if let Some(finding) = scan
            .findings
            .iter_mut()
            .find(|finding| finding.id == candidate_id)
        {
            finding.validated = candidate.validated;
            finding.false_positive = disposition == "not_reportable";
        }
        self.persist_scan(scan)
    }

    fn record_attack_path(&mut self, args: &Value) -> Result<SecurityScan, ToolError> {
        let candidate_id = args
            .get("candidateId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ToolError::Failed("candidateId is required".into()))?;
        let mut scan = self
            .current
            .clone()
            .ok_or_else(|| ToolError::Failed("no security scan is active".into()))?;
        ensure_draft(&scan)?;
        let candidate = scan
            .candidates
            .iter_mut()
            .find(|candidate| candidate.id == candidate_id)
            .ok_or_else(|| ToolError::Failed("candidate was not found".into()))?;
        let mut attack_path = args.clone();
        if let Value::Object(fields) = &mut attack_path {
            fields.remove("candidateId");
        }
        candidate.attack_path = Some(attack_path);
        self.persist_scan(scan)
    }

    fn deep_scan(&mut self, args: &Value) -> Result<SecurityScan, ToolError> {
        let mut scan = self
            .current
            .clone()
            .ok_or_else(|| ToolError::Failed("no security scan is active".into()))?;
        ensure_draft(&scan)?;
        let files = enumerate_scope(
            &self.cwd,
            args.get("scope").and_then(Value::as_str),
            &self.config,
        )?;
        for path in files {
            match scan_file(&self.cwd, &path, &self.config, &mut scan) {
                Ok(()) => scan.coverage.files_scanned += 1,
                Err(_) => scan.coverage.files_skipped += 1,
            }
        }
        scan.coverage.candidate_count = scan.candidates.len();
        scan.coverage.finding_count = scan.findings.len();
        self.persist_scan(scan)
    }

    fn persist_scan(&mut self, mut scan: SecurityScan) -> Result<SecurityScan, ToolError> {
        if let Some(artifact) = self.artifact.as_ref() {
            let digest = artifact.write_scan(&scan)?;
            scan.manifest.artifact_digest = Some(digest);
            artifact.write_manifest(&scan)?;
        }
        self.current = Some(scan.clone());
        Ok(scan)
    }

    pub fn complete(&mut self) -> Result<SecurityScan, ToolError> {
        let mut scan = self
            .current
            .clone()
            .ok_or_else(|| ToolError::Failed("no security scan is active".into()))?;
        scan.manifest.status = ScanStatus::Completed;
        scan.manifest.completed_at = Some(now_ms());
        if let Some(artifact) = &self.artifact {
            scan.manifest.artifact_digest = Some(artifact.write_scan(&scan)?);
            scan.manifest.sealed_at = Some(now_ms());
            scan.manifest.artifacts = Some(artifact.seal_artifacts()?);
            artifact.write_manifest(&scan)?;
        }
        self.current = Some(scan.clone());
        Ok(scan)
    }

    pub fn cancel(&mut self) -> Result<SecurityScan, ToolError> {
        let mut scan = self
            .current
            .clone()
            .ok_or_else(|| ToolError::Failed("no security scan is active".into()))?;
        scan.manifest.status = ScanStatus::Cancelled;
        scan.manifest.completed_at = Some(now_ms());
        if let Some(artifact) = &self.artifact {
            scan.manifest.artifact_digest = Some(artifact.write_scan(&scan)?);
            artifact.write_manifest(&scan)?;
        }
        self.current = Some(scan.clone());
        Ok(scan)
    }

    pub fn execute_tool(&mut self, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        let result = match name {
            "sec_scan_start" => self.start(args.get("scope").and_then(Value::as_str))?,
            "sec_scan_context" | "sec_scan_progress" | "sec_scan_draft" => self
                .current
                .clone()
                .ok_or_else(|| ToolError::Failed("no security scan is active".into()))?,
            "sec_scan_complete" => self.complete()?,
            "sec_scan_cancel" => self.cancel()?,
            "sec_candidates_record" => {
                let mut scan = self
                    .current
                    .clone()
                    .ok_or_else(|| ToolError::Failed("no security scan is active".into()))?;
                ensure_draft(&scan)?;
                let candidate = parse_candidate(args)?;
                if !scan.candidates.iter().any(|item| item.id == candidate.id) {
                    scan.candidates.push(candidate);
                }
                scan.coverage.candidate_count = scan.candidates.len();
                self.persist_scan(scan)?
            }
            "sec_candidates_list" => self
                .current
                .clone()
                .ok_or_else(|| ToolError::Failed("no security scan is active".into()))?,
            "sec_candidates_validate" => self.validate_candidates(args)?,
            "sec_candidates_attack_path" => self.record_attack_path(args)?,
            "sec_tracking_validate" => {
                let scan = self
                    .current
                    .clone()
                    .ok_or_else(|| ToolError::Failed("no security scan is active".into()))?;
                let artifact = self.artifact.as_ref().ok_or_else(|| {
                    ToolError::Failed("no security scan artifact is active".into())
                })?;
                let tracking = artifact.validate_manifest(&scan.manifest)?;
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&tracking).unwrap_or_default(),
                    is_error: false,
                    details: Some(json!({"securityTracking": tracking})),
                });
            }
            "sec_deep_scan" => self.deep_scan(args)?,
            "sec_scope_files" => {
                let files = enumerate_scope(
                    &self.cwd,
                    args.get("scope").and_then(Value::as_str),
                    &self.config,
                )?;
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&files).unwrap_or_default(),
                    is_error: false,
                    details: Some(json!({"files": files})),
                });
            }
            "sec_policy_resolve" => {
                return Ok(ToolResult {
                    content: serde_json::to_string_pretty(&self.config).unwrap_or_default(),
                    is_error: false,
                    details: Some(json!({"policy": self.config})),
                });
            }
            _ => return Err(ToolError::Unknown(name.to_string())),
        };
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&result).unwrap_or_default(),
            is_error: false,
            details: Some(json!({"securityScan": result})),
        })
    }

    pub fn command(&mut self, name: &str, _args: &str) -> Result<Option<Value>, String> {
        match name {
            "sec-status" => Ok(Some(
                serde_json::to_value(self.current()).map_err(|err| err.to_string())?,
            )),
            "sec-report" => {
                let report = self
                    .artifact
                    .as_ref()
                    .ok_or_else(|| "no security scan artifact is active".to_string())?
                    .read_report()
                    .map_err(|err| err.to_string())?;
                Ok(Some(json!({"report": report})))
            }
            "sec-abort" => Ok(Some(
                serde_json::to_value(self.cancel().map_err(|err| err.to_string())?)
                    .map_err(|err| err.to_string())?,
            )),
            _ => Ok(None),
        }
    }

    pub fn verify_changed_surface(
        &mut self,
        request: SecurityVerifyRequest<'_>,
    ) -> Result<SecurityVerification, String> {
        self.cwd = request.cwd.to_path_buf();
        let repo_id = repo_id(&self.cwd);
        let now = now_ms();
        let sanitized_run = request
            .graph_run_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(32)
            .collect::<String>();
        let base_id = format_scan_id(&repo_id, now, now_nanos());
        let scan_id = if sanitized_run.is_empty() {
            base_id
        } else {
            format!("{base_id}-{sanitized_run}")
        };

        let mut files_to_scan = Vec::new();
        for file in request.changed_files {
            let rel = Path::new(file);
            let Ok(norm) = normalize_relative_path(rel) else {
                continue;
            };
            if is_ignored(&norm, &self.config) {
                continue;
            }
            let full = request.cwd.join(&norm);
            if full.is_file() {
                files_to_scan.push(norm);
            }
        }
        files_to_scan.sort();
        files_to_scan.dedup();

        let scope_digest = sha256_hex(
            files_to_scan
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("\n")
                .as_bytes(),
        );

        let mut scan = SecurityScan {
            manifest: SecurityScanManifest {
                scan_id: scan_id.clone(),
                repo_id: repo_id.clone(),
                root: self.cwd.to_string_lossy().into_owned(),
                status: ScanStatus::Started,
                started_at: now,
                completed_at: None,
                allow_network: self.config.allow_network,
                scope_digest,
                artifact_digest: None,
                sealed_at: None,
                artifacts: None,
            },
            coverage: SecurityCoverage {
                files_scanned: 0,
                files_skipped: 0,
                bytes_scanned: 0,
                candidate_count: 0,
                finding_count: 0,
                network_used: false,
            },
            candidates: Vec::new(),
            findings: Vec::new(),
        };

        for path in &files_to_scan {
            match scan_file(&self.cwd, path, &self.config, &mut scan) {
                Ok(()) => scan.coverage.files_scanned += 1,
                Err(_) => scan.coverage.files_skipped += 1,
            }
        }
        scan.coverage.candidate_count = scan.candidates.len();
        scan.coverage.finding_count = scan.findings.len();

        let artifact = SecurityArtifactStore::new(&repo_id, &scan_id)
            .map_err(|e| format!("failed to initialize security store: {e}"))?;
        scan.manifest.status = ScanStatus::Completed;
        scan.manifest.completed_at = Some(now_ms());
        scan.manifest.sealed_at = Some(now_ms());

        let digest = artifact
            .write_scan(&scan)
            .map_err(|e| format!("failed to write scan: {e}"))?;
        scan.manifest.artifact_digest = Some(digest);
        scan.manifest.artifacts = Some(
            artifact
                .seal_artifacts()
                .map_err(|e| format!("failed to seal artifacts: {e}"))?,
        );
        artifact
            .write_manifest(&scan)
            .map_err(|e| format!("failed to write manifest: {e}"))?;

        self.artifact = Some(artifact);
        self.current = Some(scan.clone());

        let blockers = scan
            .findings
            .iter()
            .filter(|finding| !finding.false_positive)
            .count();

        if blockers > 0 {
            Ok(SecurityVerification::Failed { scan_id, blockers })
        } else {
            Ok(SecurityVerification::Passed { scan_id })
        }
    }
}

fn ensure_draft(scan: &SecurityScan) -> Result<(), ToolError> {
    match scan.manifest.status {
        ScanStatus::Started | ScanStatus::Draft => Ok(()),
        ScanStatus::Completed => Err(ToolError::Failed(
            "completed security scans are immutable".into(),
        )),
        ScanStatus::Cancelled => Err(ToolError::Failed("security scan is cancelled".into())),
    }
}

fn parse_candidate(args: &Value) -> Result<SecurityCandidate, ToolError> {
    let file = args
        .get("file")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed("candidate file is required".into()))?;
    let line = args
        .get("line")
        .and_then(Value::as_u64)
        .ok_or_else(|| ToolError::Failed("candidate line is required".into()))?
        as usize;
    let rule_id = args
        .get("ruleId")
        .and_then(Value::as_str)
        .unwrap_or("manual");
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("recorded by worker");
    Ok(SecurityCandidate {
        id: format!("{}:{}:{}", rule_id, file, line),
        rule_id: rule_id.into(),
        file: normalize_relative_path(Path::new(file))
            .map_err(ToolError::Failed)?
            .to_string_lossy()
            .into_owned(),
        line,
        reason: reason.into(),
        validated: false,
        disposition: None,
        validation_reason: None,
        attack_path: None,
    })
}

fn scan_file(
    root: &Path,
    relative: &Path,
    config: &SecurityScanConfig,
    scan: &mut SecurityScan,
) -> Result<(), ToolError> {
    let path = safe_join(root, relative)?;
    let metadata = fs::metadata(&path).map_err(|err| ToolError::Failed(err.to_string()))?;
    if metadata.len() > config.max_file_bytes {
        return Err(ToolError::Failed("file exceeds scan limit".into()));
    }
    let bytes = fs::read(&path).map_err(|err| ToolError::Failed(err.to_string()))?;
    if bytes.contains(&0) {
        return Err(ToolError::Failed("binary file".into()));
    }
    let text = String::from_utf8_lossy(&bytes);
    scan.coverage.bytes_scanned = scan
        .coverage
        .bytes_scanned
        .saturating_add(bytes.len() as u64);
    for (rule_id, severity, needle, message) in [
        (
            "secret.private-key",
            FindingSeverity::Critical,
            "BEGIN PRIVATE KEY",
            "Private key material appears in repository content",
        ),
        (
            "secret.api-key",
            FindingSeverity::High,
            "sk-",
            "API-key-shaped secret appears in repository content",
        ),
        (
            "command.eval",
            FindingSeverity::Medium,
            "eval(",
            "Dynamic evaluation can execute untrusted input",
        ),
        (
            "command.shell",
            FindingSeverity::Medium,
            "shell=True",
            "Shell execution with interpolation requires validation",
        ),
    ] {
        for (index, line) in text.lines().enumerate() {
            let line_no = index + 1;
            if !line.contains(needle) {
                continue;
            }
            let file = relative.to_string_lossy().replace('\\', "/");
            let evidence = redact_evidence(line);
            let id = format!(
                "{}-{}",
                rule_id,
                &sha256_hex(format!("{file}:{line_no}:{evidence}").as_bytes())[..12]
            );
            if scan.findings.iter().any(|finding| finding.id == id) {
                continue;
            }
            scan.candidates.push(SecurityCandidate {
                id: id.clone(),
                rule_id: rule_id.into(),
                file: file.clone(),
                line: line_no,
                reason: message.into(),
                validated: false,
                disposition: None,
                validation_reason: None,
                attack_path: None,
            });
            scan.findings.push(SecurityFinding {
                id,
                rule_id: rule_id.into(),
                severity,
                file,
                line: line_no,
                message: message.into(),
                evidence,
                validated: false,
                false_positive: false,
            });
        }
    }
    Ok(())
}

pub fn enumerate_scope(
    root: &Path,
    scope: Option<&str>,
    config: &SecurityScanConfig,
) -> Result<Vec<PathBuf>, ToolError> {
    let base = if let Some(scope) = scope {
        safe_join(root, Path::new(scope))?
    } else {
        root.to_path_buf()
    };
    if !base.exists() {
        return Err(ToolError::Failed("scan scope does not exist".into()));
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(&base).follow_links(false) {
        let entry = entry.map_err(|err| ToolError::Failed(err.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| ToolError::Failed("scope escaped repository".into()))?;
        if is_ignored(relative, config) {
            continue;
        }
        files.push(relative.to_path_buf());
    }
    files.sort();
    Ok(files)
}

fn is_ignored(path: &Path, config: &SecurityScanConfig) -> bool {
    path.components().any(|component| match component {
        Component::Normal(value) => {
            let value = value.to_string_lossy();
            value == ".git"
                || value == "node_modules"
                || value == "target"
                || value == "vector-memory"
                || (!config.include_hidden && value.starts_with('.'))
        }
        _ => false,
    })
}

pub fn normalize_relative_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Err("absolute paths are not allowed in security scope".into());
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err("path escapes security scope".into());
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute paths are not allowed in security scope".into())
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err("empty security scope".into());
    }
    Ok(normalized)
}

fn safe_join(root: &Path, relative: &Path) -> Result<PathBuf, ToolError> {
    let normalized = normalize_relative_path(relative).map_err(ToolError::Failed)?;
    let joined = root.join(normalized);
    let canonical_root = root
        .canonicalize()
        .map_err(|err| ToolError::Failed(err.to_string()))?;
    let canonical_parent = joined
        .parent()
        .unwrap_or(root)
        .canonicalize()
        .map_err(|err| ToolError::Failed(err.to_string()))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(ToolError::Failed("scope escapes repository".into()));
    }
    Ok(joined)
}

fn redact_evidence(line: &str) -> String {
    if line.to_ascii_uppercase().contains("PRIVATE KEY") {
        return "[REDACTED PRIVATE KEY MATERIAL]".into();
    }
    let mut out = line.to_string();
    for prefix in ["sk-", "ghp_", "Bearer "] {
        let mut search_start = 0;
        while let Some(offset) = out[search_start..].find(prefix) {
            let start = search_start + offset;
            let token_start = start + prefix.len();
            let end = out[token_start..]
                .find(char::is_whitespace)
                .map(|offset| token_start + offset)
                .unwrap_or(out.len());
            out.replace_range(start..end, "[REDACTED]");
            search_start = start + "[REDACTED]".len();
        }
    }
    let mut lower = out.to_ascii_lowercase();
    let mut search_start = 0;
    while let Some(offset) = lower[search_start..].find("password=") {
        let start = search_start + offset;
        let token_start = start + "password=".len();
        let end = out[token_start..]
            .find(char::is_whitespace)
            .map(|offset| token_start + offset)
            .unwrap_or(out.len());
        out.replace_range(token_start..end, "[REDACTED]");
        lower = out.to_ascii_lowercase();
        search_start = token_start + "[REDACTED]".len();
    }
    out
}

fn repo_id(root: &Path) -> String {
    let normalized = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    format!("repo-{}", &sha256_hex(normalized.as_bytes())[..16])
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn format_scan_id(repo_id: &str, started_at: u64, nonce: u128) -> String {
    format!(
        "scan-{started_at}-{nonce:x}-{}",
        &sha256_hex(repo_id.as_bytes())[..8]
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn render_report(scan: &SecurityScan) -> String {
    let mut report = format!(
        "# Security scan {}\n\nStatus: {:?}\n\nFiles scanned: {}\nFindings: {}\n\n",
        scan.manifest.scan_id,
        scan.manifest.status,
        scan.coverage.files_scanned,
        scan.findings.len()
    );
    let mut findings = scan.findings.clone();
    findings.sort_by_key(|finding| std::cmp::Reverse(finding.severity.rank()));
    if findings.is_empty() {
        report.push_str("No findings were produced by the deterministic local rules.\n");
    } else {
        for finding in findings {
            report.push_str(&format!(
                "- **{:?}** {} at {}:{} — {}\n",
                finding.severity, finding.rule_id, finding.file, finding.line, finding.message
            ));
        }
    }
    report
}

pub fn render_sarif(scan: &SecurityScan) -> Value {
    let results = scan
        .findings
        .iter()
        .map(|finding| {
            json!({
                "ruleId": finding.rule_id,
                "level": match finding.severity {
                    FindingSeverity::Critical | FindingSeverity::High => "error",
                    FindingSeverity::Medium => "warning",
                    _ => "note",
                },
                "message": {"text": finding.message},
                "locations": [{"physicalLocation": {"artifactLocation": {"uri": finding.file}, "region": {"startLine": finding.line}}}],
            })
        })
        .collect::<Vec<_>>();
    json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{"tool": {"driver": {"name": "pi-security-scan", "informationUri": "https://github.com/earendil-works/pi"}}, "results": results}],
    })
}

pub fn tool_spec(name: &str) -> (&'static str, Value) {
    let description = match name {
        "sec_scan_start" => "Start a scoped, local security scan.",
        "sec_scan_context" => "Read the canonical security scan context.",
        "sec_scan_progress" => "Read security scan progress and coverage.",
        "sec_scan_draft" => "Read the current draft findings.",
        "sec_scan_complete" => "Seal the current security scan artifacts.",
        "sec_scan_cancel" => "Cancel the current security scan.",
        "sec_candidates_record" => "Record an untrusted security candidate.",
        "sec_candidates_list" => "List security candidates.",
        "sec_candidates_validate" => "Validate security candidates.",
        "sec_candidates_attack_path" => "Analyze candidate attack paths.",
        "sec_scope_files" => "Enumerate safe files in a scan scope.",
        "sec_policy_resolve" => "Resolve the local scan policy.",
        "sec_tracking_validate" => "Validate finding tracking metadata.",
        "sec_deep_scan" => "Run deterministic deep-rule scanning.",
        _ => "Security scan operation.",
    };
    (
        description,
        json!({"type":"object","additionalProperties":true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scan_id_includes_a_nonce_for_same_millisecond_starts() {
        let first = format_scan_id("repo-abc", 42, 1);
        let second = format_scan_id("repo-abc", 42, 2);
        assert_ne!(first, second);
        assert!(first.starts_with("scan-42-"));
        assert!(second.starts_with("scan-42-"));
    }

    #[test]
    fn relative_path_validation_rejects_escape_and_absolute_inputs() {
        assert!(normalize_relative_path(Path::new("../secret")).is_err());
        assert!(normalize_relative_path(Path::new("C:\\secret")).is_err());
        assert_eq!(
            normalize_relative_path(Path::new("./src/lib.rs")).unwrap(),
            PathBuf::from("src/lib.rs")
        );
    }

    #[test]
    fn local_scan_finds_redacted_secret_without_network() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("sample.txt"), "token=sk-super-secret\n").unwrap();
        let mut controller = SecurityScanController::new(dir.path().to_path_buf());
        let scan = controller.start(None).unwrap();
        assert!(!scan.manifest.allow_network);
        assert_eq!(scan.findings.len(), 1);
        assert!(!scan.findings[0].evidence.contains("sk-super-secret"));
        assert!(controller
            .complete()
            .unwrap()
            .manifest
            .artifact_digest
            .is_some());
    }

    #[test]
    fn scan_artifacts_are_versioned_and_expose_canonical_sarif() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("sample.txt"), "no secrets here\n").unwrap();
        let mut controller = SecurityScanController::new(dir.path().to_path_buf());
        let scan = controller.start(None).unwrap();
        let root = controller.artifact.as_ref().unwrap().root().to_path_buf();

        let findings: Value =
            serde_json::from_slice(&fs::read(root.join("findings.json")).unwrap()).unwrap();
        assert_eq!(findings["schemaVersion"], 1);
        assert!(findings["findings"].is_array());
        assert!(root.join("scan-manifest.json").is_file());
        assert!(root.join("results.sarif").is_file());

        let manifest: Value =
            serde_json::from_slice(&fs::read(root.join("scan-manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["scanId"], scan.manifest.scan_id);
        assert_eq!(
            manifest["artifactDigest"],
            scan.manifest.artifact_digest.clone().unwrap()
        );
    }

    #[test]
    fn recording_candidate_updates_persisted_artifacts_and_digest() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("sample.txt"), "no secrets here\n").unwrap();
        let mut controller = SecurityScanController::new(dir.path().to_path_buf());
        controller.start(None).unwrap();
        let args = json!({
            "file": "sample.txt",
            "line": 1,
            "ruleId": "manual-auth",
            "reason": "candidate from a worker"
        });
        controller
            .execute_tool("sec_candidates_record", &args)
            .unwrap();

        let root = controller.artifact.as_ref().unwrap().root().to_path_buf();
        let candidates: Value =
            serde_json::from_slice(&fs::read(root.join("candidates.json")).unwrap()).unwrap();
        assert_eq!(candidates["candidates"][0]["ruleId"], "manual-auth");

        let manifest: Value =
            serde_json::from_slice(&fs::read(root.join("scan-manifest.json")).unwrap()).unwrap();
        assert_eq!(
            manifest["artifactDigest"],
            controller
                .current()
                .unwrap()
                .manifest
                .artifact_digest
                .unwrap()
        );
    }

    #[test]
    fn completed_scan_records_per_artifact_seals_and_tracking_validation_rejects_tampering() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("sample.txt"), "no secrets here\n").unwrap();
        let mut controller = SecurityScanController::new(dir.path().to_path_buf());
        controller.start(None).unwrap();
        let completed = controller.complete().unwrap();
        assert!(completed.manifest.sealed_at.is_some());
        let artifacts = completed.manifest.artifacts.as_ref().unwrap();
        for name in [
            "findings.json",
            "coverage.json",
            "report.md",
            "results.sarif",
        ] {
            assert!(artifacts.contains_key(name));
            assert!(artifacts[name].bytes > 0);
        }

        let valid = controller
            .execute_tool("sec_tracking_validate", &json!({}))
            .unwrap();
        assert_eq!(valid.details.unwrap()["securityTracking"]["valid"], true);

        let root = controller.artifact.as_ref().unwrap().root().to_path_buf();
        fs::write(root.join("report.md"), "tampered\n").unwrap();
        let error = controller.execute_tool("sec_tracking_validate", &json!({}));
        assert!(error.is_err());
    }

    #[test]
    fn candidate_validation_persists_disposition_and_attack_path() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("sample.txt"), "no secrets here\n").unwrap();
        let mut controller = SecurityScanController::new(dir.path().to_path_buf());
        controller.start(None).unwrap();
        controller
            .execute_tool(
                "sec_candidates_record",
                &json!({"file":"sample.txt","line":1,"ruleId":"manual","reason":"review"}),
            )
            .unwrap();
        controller
            .execute_tool(
                "sec_candidates_validate",
                &json!({"candidateId":"manual:sample.txt:1","disposition":"reportable","reason":"reproduced"}),
            )
            .unwrap();
        controller
            .execute_tool(
                "sec_candidates_attack_path",
                &json!({"candidateId":"manual:sample.txt:1","inScope":true,"exposure":"local","steps":["read file"]}),
            )
            .unwrap();

        let candidate = &controller.current().unwrap().candidates[0];
        assert_eq!(candidate.disposition.as_deref(), Some("reportable"));
        assert_eq!(candidate.attack_path.as_ref().unwrap()["inScope"], true);
    }

    #[test]
    fn deep_scan_is_an_explicit_stateful_operation() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("sample.txt"), "token=sk-secret\n").unwrap();
        let mut controller = SecurityScanController::new(dir.path().to_path_buf());
        controller.start(Some("sample.txt")).unwrap();
        let before = controller.current().unwrap().findings.len();
        fs::write(dir.path().join("new.txt"), "eval(input)\n").unwrap();
        let result = controller
            .execute_tool("sec_deep_scan", &json!({"scope":"new.txt"}))
            .unwrap();
        assert!(result.content.contains("command.eval"));
        assert!(controller.current().unwrap().findings.len() > before);
    }

    #[test]
    fn evidence_redaction_masks_multiple_tokens_and_private_key_markers() {
        let redacted = redact_evidence(
            "Authorization: Bearer abc sk-first password=hunter2 sk-second ghp_token",
        );
        assert!(!redacted.contains("abc"));
        assert!(!redacted.contains("sk-first"));
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("sk-second"));
        assert!(!redacted.contains("ghp_token"));
        assert_eq!(
            redact_evidence("-----BEGIN PRIVATE KEY-----"),
            "[REDACTED PRIVATE KEY MATERIAL]"
        );
    }

    #[test]
    fn sarif_uses_file_and_line_locations() {
        let scan = SecurityScan {
            manifest: SecurityScanManifest {
                scan_id: "scan".into(),
                repo_id: "repo".into(),
                root: ".".into(),
                status: ScanStatus::Completed,
                started_at: 0,
                completed_at: Some(1),
                allow_network: false,
                scope_digest: "x".into(),
                artifact_digest: None,
                sealed_at: None,
                artifacts: None,
            },
            coverage: SecurityCoverage {
                files_scanned: 1,
                files_skipped: 0,
                bytes_scanned: 1,
                candidate_count: 1,
                finding_count: 1,
                network_used: false,
            },
            candidates: vec![],
            findings: vec![SecurityFinding {
                id: "f".into(),
                rule_id: "r".into(),
                severity: FindingSeverity::High,
                file: "src/lib.rs".into(),
                line: 3,
                message: "bad".into(),
                evidence: "[REDACTED]".into(),
                validated: false,
                false_positive: false,
            }],
        };
        assert_eq!(
            render_sarif(&scan)["runs"][0]["results"][0]["locations"][0]["physicalLocation"]
                ["region"]["startLine"],
            3
        );
    }

    #[test]
    fn verify_changed_surface_detects_blockers_on_sensitive_fixture() {
        let tmp = tempfile::tempdir().unwrap();
        let auth_file = tmp.path().join("src/auth.rs");
        fs::create_dir_all(auth_file.parent().unwrap()).unwrap();
        fs::write(
            &auth_file,
            "pub fn key() -> &'static str { \"sk-secret12345\" }\n",
        )
        .unwrap();

        let mut controller = SecurityScanController::new(tmp.path().to_path_buf());
        let request = SecurityVerifyRequest {
            cwd: tmp.path(),
            changed_files: &["src/auth.rs".to_string()],
            graph_run_id: "run-test-1",
        };

        let result = controller.verify_changed_surface(request).unwrap();
        match result {
            SecurityVerification::Failed { scan_id, blockers } => {
                assert!(scan_id.starts_with("scan-"));
                assert!(blockers > 0);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn verify_changed_surface_passes_clean_fixture() {
        let tmp = tempfile::tempdir().unwrap();
        let clean_file = tmp.path().join("src/ui.rs");
        fs::create_dir_all(clean_file.parent().unwrap()).unwrap();
        fs::write(&clean_file, "pub fn render() { println!(\"Hello\"); }\n").unwrap();

        let mut controller = SecurityScanController::new(tmp.path().to_path_buf());
        let request = SecurityVerifyRequest {
            cwd: tmp.path(),
            changed_files: &["src/ui.rs".to_string()],
            graph_run_id: "run-test-2",
        };

        let result = controller.verify_changed_surface(request).unwrap();
        match result {
            SecurityVerification::Passed { scan_id } => {
                assert!(scan_id.starts_with("scan-"));
            }
            other => panic!("expected Passed, got {other:?}"),
        }
    }
}
