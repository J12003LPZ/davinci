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

mod analyzers;
mod artifact_index;
mod budget;
mod checkpoint_encoding;
pub mod command;
pub mod config;
mod conflict_capture;
pub mod controller;
mod deadline;
mod feedback;
mod gate;
mod git;
mod grouping;
mod identity;
pub mod incremental;
pub mod interrupt;
mod partial;
mod policy;
pub mod recon;
mod reconciliation;
mod redaction;
pub mod report;
mod report_contract;
mod report_envelope;
mod retention;
pub mod review;
mod root_cause;
pub mod skills;
pub mod snapshot;
mod store;
mod supporting;
pub mod tools;
pub mod types;
pub mod usage;
pub mod validation;
mod validation_budget;
mod watch;
pub mod worker;
mod worker_cache;

#[allow(unused_imports)]
pub use incremental::{
    CachedFileScan, IncrementalSecurityCache, IncrementalSecurityTelemetry, SecurityFileCacheKey,
    SECURITY_RULESET_VERSION,
};

pub use config::ScanConfig;
pub use watch::status_line as watch_status_line;

#[derive(Debug, Clone)]
pub struct SecurityVerifyRequest<'a> {
    pub cwd: &'a Path,
    pub changed_files: &'a [String],
    pub graph_run_id: &'a str,
    /// Unified diff of the change (the graph's mutation delta or `git diff`).
    /// When present only the lines it adds are judged, so text that predates
    /// the change never blocks it.
    pub diff: Option<&'a str>,
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
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Informational => "informational",
        }
    }

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
    #[serde(default)]
    pub files_scanned_cold: usize,
    #[serde(default)]
    pub files_reused: usize,
    #[serde(default)]
    pub files_rescanned: usize,
    #[serde(default)]
    pub cache_read_errors: usize,
    #[serde(default)]
    pub cache_write_errors: usize,
    /// Changed paths that no longer exist; nothing remains to scan in them.
    #[serde(default)]
    pub files_deleted: usize,
    /// `path: reason` for every requested path that was not scanned.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
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

fn ensure_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn atomic_report_write(path: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    davinci_sys::fs::atomic_write(path, bytes).map_err(|err| ToolError::Failed(err.to_string()))
}

fn prune_scan_reports(root: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut entries = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| {
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            (modified, entry.path())
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|(modified, _)| *modified);
    let remove = entries.len().saturating_sub(keep);
    for (_, path) in entries.into_iter().take(remove) {
        let _ = fs::remove_dir_all(path);
    }
}

impl SecurityArtifactStore {
    pub fn new(agent_dir: &Path, repo_id: &str, scan_id: &str) -> Result<Self, ToolError> {
        if !safe_component(repo_id) || !safe_component(scan_id) {
            return Err(ToolError::Failed(
                "invalid security artifact identity".into(),
            ));
        }
        let reports = agent_dir.join("security-scans");
        let repo_root = reports.join(repo_id);
        let root = repo_root.join(scan_id);
        fs::create_dir_all(&root).map_err(|err| ToolError::Failed(err.to_string()))?;
        ensure_private_dir(&reports).map_err(|err| ToolError::Failed(err.to_string()))?;
        ensure_private_dir(&repo_root).map_err(|err| ToolError::Failed(err.to_string()))?;
        ensure_private_dir(&root).map_err(|err| ToolError::Failed(err.to_string()))?;
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
        atomic_report_write(&self.root.join("findings.json"), &findings)?;
        atomic_report_write(&self.root.join("candidates.json"), &candidates)?;
        atomic_report_write(&self.root.join("coverage.json"), &coverage)?;
        let report = render_report(scan);
        atomic_report_write(&self.root.join("report.md"), report.as_bytes())?;
        let sarif = render_sarif(scan);
        let sarif_bytes =
            serde_json::to_vec_pretty(&sarif).map_err(|err| ToolError::Failed(err.to_string()))?;
        atomic_report_write(&self.root.join("report.sarif"), &sarif_bytes)?;
        atomic_report_write(&self.root.join("results.sarif"), &sarif_bytes)?;
        let digest = self.combined_digest()?;
        atomic_report_write(&self.root.join("artifact.sha256"), digest.as_bytes())?;
        self.write_manifest(scan)?;
        if let Some(repo_root) = self.root.parent() {
            prune_scan_reports(repo_root, 10);
        }
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
        atomic_report_write(&self.root.join("scan-manifest.json"), &content)
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
    review: controller::ScanCoordinator,
    runner: Option<worker::SecurityWorkerRunner>,
    review_config: ScanConfig,
    report: std::sync::Arc<std::sync::Mutex<Option<Value>>>,
    review_agent_dir: Option<PathBuf>,
    incremental_cache: IncrementalSecurityCache,
    incremental_cache_repo: Option<String>,
    /// Invalid `securityScan` settings: the graph gate fails closed on them.
    config_error: Option<String>,
    watch: watch::SecurityWatch,
    /// `scanId/generation` whose completion notice was already shown.
    announced: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

enum Interrupted {
    Resumable {
        id: String,
        request: command::ScanCommand,
    },
    Stale {
        id: String,
        reason: String,
    },
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
            review: controller::ScanCoordinator::default(),
            runner: None,
            review_config: ScanConfig::default(),
            report: Default::default(),
            review_agent_dir: None,
            incremental_cache: IncrementalSecurityCache::new(),
            incremental_cache_repo: None,
            config_error: None,
            watch: watch::SecurityWatch::default(),
            announced: Default::default(),
        }
    }

    /// A controller whose file policy, failure policy, watch and storage come
    /// from `securityScan` settings (trusted projects may only narrow them).
    pub fn with_settings(cwd: PathBuf, agent_dir: PathBuf, trusted: bool) -> Self {
        let mut controller = Self::new(cwd.clone());
        controller.review_agent_dir = Some(agent_dir.clone());
        match crate::settings::load_security_scan_config(&agent_dir, &cwd, trusted) {
            Ok(config) => controller.apply_config(config),
            Err(error) => controller.config_error = Some(error),
        }
        controller
    }

    /// The controller the graph security gate uses: the session's agent
    /// directory, never the reviewed repository, holds its artifacts.
    pub fn for_workspace(cwd: PathBuf) -> Self {
        // Unit tests never touch the real agent directory.
        let agent_dir = if cfg!(test) {
            std::env::temp_dir().join("davinci-security-scan-tests")
        } else {
            davinci_session::default_agent_dir()
        };
        let settings = crate::settings::load_merged_settings(&agent_dir, &cwd);
        let trusted = crate::settings::is_trusted(&settings, &cwd, None);
        Self::with_settings(cwd, agent_dir, trusted)
    }

    fn apply_config(&mut self, config: ScanConfig) {
        self.config = config.file_policy();
        self.watch.configure(&config.watch);
        self.review_config = config;
        self.config_error = None;
    }

    /// Scan artifacts always live under an agent directory, outside the
    /// reviewed repository.
    fn artifact_agent_dir(&self) -> PathBuf {
        if let Some(dir) = &self.review_agent_dir {
            return dir.clone();
        }
        if cfg!(test) {
            return std::env::temp_dir().join("davinci-security-scan-tests");
        }
        davinci_session::default_agent_dir()
    }

    fn prepare_incremental_cache(&mut self, repo_id: &str) {
        if self.incremental_cache_repo.as_deref() == Some(repo_id) {
            self.incremental_cache.reset_telemetry();
            return;
        }
        // Security findings are trusted only when they were produced in this
        // controller process. A cache loaded from a shared temporary directory
        // could be forged by another local process and suppress a cold scan.
        self.incremental_cache = IncrementalSecurityCache::new();
        self.incremental_cache_repo = Some(repo_id.to_string());
    }

    fn apply_incremental_telemetry(&self, scan: &mut SecurityScan) {
        scan.coverage.files_scanned_cold = self.incremental_cache.telemetry.files_scanned_cold;
        scan.coverage.files_reused = self.incremental_cache.telemetry.files_reused;
        scan.coverage.files_rescanned = self.incremental_cache.telemetry.files_rescanned;
        scan.coverage.cache_read_errors = self.incremental_cache.telemetry.cache_read_errors;
        scan.coverage.cache_write_errors = self.incremental_cache.telemetry.cache_write_errors;
    }

    fn scan_files_incrementally(&mut self, files: &[PathBuf], scan: &mut SecurityScan) {
        for path in files {
            let relative = path.to_string_lossy().replace('\\', "/");
            let bytes = match read_scan_file(&self.cwd, path, &self.config) {
                Ok(bytes) => bytes,
                Err(_) => {
                    scan.coverage.files_skipped += 1;
                    continue;
                }
            };
            let key = SecurityFileCacheKey::for_file(&bytes, &self.config);

            if let Some(cached) = self.incremental_cache.lookup(&relative, &key) {
                scan.coverage.files_scanned += 1;
                self.incremental_cache.telemetry.files_reused = self
                    .incremental_cache
                    .telemetry
                    .files_reused
                    .saturating_add(1);
                scan.coverage.bytes_scanned = scan
                    .coverage
                    .bytes_scanned
                    .saturating_add(cached.coverage.bytes_scanned);
                scan.candidates.extend(cached.candidates);
                scan.findings.extend(cached.findings);
                continue;
            }

            let had_entry = self.incremental_cache.contains_path(&relative);
            let finding_start = scan.findings.len();
            let candidate_start = scan.candidates.len();
            if scan_file_bytes(path, &bytes, scan).is_err() {
                scan.coverage.files_skipped += 1;
                continue;
            }

            let findings = scan.findings[finding_start..].to_vec();
            let candidates = scan.candidates[candidate_start..].to_vec();
            self.incremental_cache.insert(
                relative,
                CachedFileScan {
                    key,
                    findings,
                    candidates,
                    coverage: SecurityCoverage {
                        files_scanned: 1,
                        files_skipped: 0,
                        bytes_scanned: bytes.len() as u64,
                        candidate_count: scan.candidates.len() - candidate_start,
                        finding_count: scan.findings.len() - finding_start,
                        network_used: false,
                        files_scanned_cold: 0,
                        files_reused: 0,
                        files_rescanned: 0,
                        cache_read_errors: 0,
                        cache_write_errors: 0,
                        files_deleted: 0,
                        skipped: Vec::new(),
                    },
                },
            );
            scan.coverage.files_scanned += 1;
            if had_entry {
                self.incremental_cache.telemetry.files_rescanned = self
                    .incremental_cache
                    .telemetry
                    .files_rescanned
                    .saturating_add(1);
            } else {
                self.incremental_cache.telemetry.files_scanned_cold = self
                    .incremental_cache
                    .telemetry
                    .files_scanned_cold
                    .saturating_add(1);
            }
        }
    }

    pub fn start(&mut self, scope: Option<&str>) -> Result<SecurityScan, ToolError> {
        let repo_id = repo_id(&self.cwd);
        self.prepare_incremental_cache(&repo_id);
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
                files_scanned_cold: 0,
                files_reused: 0,
                files_rescanned: 0,
                cache_read_errors: 0,
                cache_write_errors: 0,
                files_deleted: 0,
                skipped: Vec::new(),
            },
            candidates: Vec::new(),
            findings: Vec::new(),
        };
        self.scan_files_incrementally(&files, &mut scan);
        self.apply_incremental_telemetry(&mut scan);
        scan.coverage.candidate_count = scan.candidates.len();
        scan.coverage.finding_count = scan.findings.len();
        scan.manifest.status = ScanStatus::Draft;
        let artifact = SecurityArtifactStore::new(&self.artifact_agent_dir(), &repo_id, &scan_id)?;
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

    pub fn configure_review(&mut self, runner: worker::SecurityWorkerRunner, config: ScanConfig) {
        self.runner = Some(runner);
        self.apply_config(config);
    }

    pub fn set_review_storage(&mut self, agent_dir: PathBuf) {
        self.review_agent_dir = Some(agent_dir);
    }

    pub fn has_review(&self) -> bool {
        self.review.status().is_some()
    }

    pub fn wait_for_review(&self) -> bool {
        self.review.wait_timeout(std::time::Duration::from_secs(30))
    }

    pub fn abort_review(&self) {
        let _ = self.review.abort(None);
    }

    pub fn wait_for_review_interruptible(&self, interrupt: &interrupt::Interrupt) {
        while self
            .review
            .status()
            .is_some_and(|state| !state.status.terminal())
        {
            if interrupt.requested() {
                let _ = self.review.abort(None);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let _ = self.wait_for_review();
    }

    /// Start a new model review of `request` in this session.
    fn start_review(&mut self, request: command::ScanCommand) -> Result<Value, String> {
        self.review_config.admit(self.runner.is_some())?;
        let runner = self.runner.clone().ok_or("security provider unavailable")?;
        let root = self.cwd.clone();
        let config = self.review_config.clone();
        let report = self.report.clone();
        let agent_dir = self.review_agent_dir.clone();
        let run = self.review.start(move |run| {
            let result = (|| {
                let store = agent_dir
                    .as_ref()
                    .map(|dir| {
                        store::Store::open(dir, &git::root(&root), &run.status().scan_id, true)
                    })
                    .transpose()?;
                if let Some(store) = &store {
                    run.set_generation(store.next_generation()?)?;
                }
                let mut value = review::execute(
                    &root,
                    &request,
                    &config,
                    &runner,
                    &run,
                    store.as_ref(),
                    None,
                )?;
                report::sanitize(&mut value);
                run.begin_publication()?;
                if let Some(store) = &store {
                    store.complete(&value, run.status().generation)?;
                }
                Ok::<_, String>(value)
            })();
            match result {
                Ok(mut value) => {
                    report::sanitize(&mut value);
                    let complete = value["coverageComplete"].as_bool().unwrap_or(false);
                    *report.lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
                    run.finish(Ok(complete));
                }
                Err(error) => run.finish(Err(error)),
            }
        })?;
        serde_json::to_value(run.status()).map_err(|e| e.to_string())
    }

    /// Continue an interrupted scan from its immutable checkpoint.
    fn resume_review(&mut self, id: String) -> Result<Value, String> {
        if uuid::Uuid::parse_str(&id)
            .map_err(|_| "invalid scan identity")?
            .to_string()
            != id
        {
            return Err("invalid scan identity".into());
        }
        self.review_config.admit(self.runner.is_some())?;
        let runner = self.runner.clone().ok_or("security provider unavailable")?;
        let config = self.review_config.clone();
        let agent_dir = self
            .review_agent_dir
            .clone()
            .ok_or("security checkpoint storage unavailable")?;
        let root = self.cwd.clone();
        let report = self.report.clone();
        let run = self.review.start_generation(id, 0, move |run| {
            let result = (|| {
                let store = store::Store::open(
                    &agent_dir,
                    &git::root(&root),
                    &run.status().scan_id,
                    false,
                )?;
                if store.has_sealed_report() {
                    store.latest_report()?;
                    return Err("sealed security reviews are immutable; start a new scan".into());
                }
                let checkpoint = store.load(config.checkpoint_byte_limit())?;
                checkpoint.validate_resume(&config)?;
                run.set_generation(store.next_generation()?)?;
                let mut value = review::execute(
                    &root,
                    &checkpoint.request,
                    &config,
                    &runner,
                    &run,
                    Some(&store),
                    Some(checkpoint.snapshot),
                )?;
                report::sanitize(&mut value);
                run.begin_publication()?;
                store.complete(&value, run.status().generation)?;
                Ok::<_, String>(value)
            })();
            match result {
                Ok(value) => {
                    let complete = value["coverageComplete"].as_bool().unwrap_or(false);
                    *report.lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
                    run.finish(Ok(complete));
                }
                Err(error) => run.finish(Err(error)),
            }
        })?;
        serde_json::to_value(run.status()).map_err(|e| e.to_string())
    }

    /// The newest interrupted scan of this repository, and whether it can
    /// continue under the current policy, methodology and wall-clock deadline.
    fn probe_interrupted(&self) -> Option<Interrupted> {
        let agent_dir = self.review_agent_dir.as_ref()?;
        let root = git::root(&self.cwd);
        for id in store::interrupted_scans(agent_dir, &root) {
            // A scan another live session holds is not ours to resume.
            let Ok(store) = store::Store::open(agent_dir, &root, &id, false) else {
                continue;
            };
            let verdict = store
                .load(self.review_config.checkpoint_byte_limit())
                .and_then(|checkpoint| {
                    checkpoint.validate_resume(&self.review_config)?;
                    deadline::remaining(
                        Some(&store),
                        &id,
                        review::wall_clock_limit(checkpoint.request.mode),
                        true,
                    )?;
                    Ok(checkpoint.request)
                });
            return Some(match verdict {
                Ok(request) => Interrupted::Resumable { id, request },
                Err(reason) => Interrupted::Stale { id, reason },
            });
        }
        None
    }

    fn discard_interrupted(&self, id: &str, reason: &str) {
        if let Some(agent_dir) = &self.review_agent_dir {
            if let Ok(store) = store::Store::open(agent_dir, &git::root(&self.cwd), id, false) {
                let _ = store.discard(reason);
            }
        }
    }

    fn sweep_retention(&self) {
        if let Some(agent_dir) = &self.review_agent_dir {
            retention::sweep(
                agent_dir,
                self.review_config.retention_days,
                SystemTime::now(),
            );
        }
    }

    /// Session start: continue this repository's interrupted scan, or say once
    /// that it cannot continue. Returns the notice for the transcript.
    pub fn resume_interrupted(&mut self) -> Option<String> {
        if self
            .review
            .status()
            .is_some_and(|run| !run.status.terminal())
        {
            return None;
        }
        match self.probe_interrupted()? {
            Interrupted::Resumable { id, .. } => Some(match self.resume_review(id.clone()) {
                Ok(_) => format!(
                    "Resuming the interrupted security scan {id} in the background; /security-scan shows its progress."
                ),
                Err(error) => format!("The interrupted security scan {id} could not resume: {error}"),
            }),
            Interrupted::Stale { id, reason } => {
                self.discard_interrupted(&id, &reason);
                Some(format!(
                    "The interrupted security scan {id} cannot be resumed ({reason}); the next /security-scan starts a new one."
                ))
            }
        }
    }

    /// Whether this repository has an interrupted scan left to consider.
    pub fn has_interrupted_scan(&self) -> bool {
        self.review_agent_dir.as_ref().is_some_and(|agent_dir| {
            !store::interrupted_scans(agent_dir, &git::root(&self.cwd)).is_empty()
        })
    }

    fn report_view(&self, finding: Option<&str>) -> Result<Value, String> {
        if let Some(progress) = self.review.status() {
            let report = self.report.lock().unwrap_or_else(|e| e.into_inner());
            return report::select_finding(
                report
                    .as_ref()
                    .filter(|value| {
                        value["scanId"] == progress.scan_id
                            && value["generation"] == progress.generation
                    })
                    .cloned()
                    .or_else(|| self.review.partial_report())
                    .unwrap_or(serde_json::to_value(progress).map_err(|e| e.to_string())?),
                finding,
            );
        }
        // Otherwise the newest stored report of this repository that holds it.
        if let Some(agent_dir) = &self.review_agent_dir {
            let root = git::root(&self.cwd);
            for id in store::scan_directories(agent_dir, &root, false)
                .into_iter()
                .take(10)
            {
                let Ok(store) = store::Store::open(agent_dir, &root, &id, false) else {
                    continue;
                };
                let Ok(report) = store.available_report() else {
                    continue;
                };
                if let Ok(view) = report::select_finding(report, finding) {
                    return Ok(view);
                }
            }
        }
        Err(match finding {
            Some(_) => "that finding is not in a recent security report of this repository".into(),
            None => "no security report is available yet; run /security-scan".into(),
        })
    }

    fn scan_command(&mut self, args: &str) -> Result<Value, String> {
        let invocation = command::Invocation::parse(args, self.review_config.default_mode)?;
        if invocation.report {
            return self.report_view(invocation.finding.as_deref());
        }
        if let Some(progress) = self
            .review
            .status()
            .filter(|progress| !progress.status.terminal())
        {
            if invocation.selects || invocation.new {
                return Err(format!(
                    "security scan {} is already running; /security-scan shows its progress",
                    progress.scan_id
                ));
            }
            let mut value = serde_json::to_value(progress).map_err(|e| e.to_string())?;
            value["notice"] = json!("A security scan is already running; showing its progress.");
            return Ok(value);
        }
        self.sweep_retention();
        let mut notice = None;
        match self.probe_interrupted() {
            Some(Interrupted::Resumable { id, request }) if !invocation.new => {
                let mut matching = request.clone();
                matching.format = invocation.scan.format;
                if !invocation.selects || matching == invocation.scan {
                    self.review_config.admit(self.runner.is_some())?;
                    let mut value = self.resume_review(id.clone())?;
                    value["notice"] = json!(format!(
                        "Resumed the interrupted security scan {id}; /security-scan --new starts over instead."
                    ));
                    return Ok(value);
                }
                self.discard_interrupted(&id, "a different scan was requested");
                notice = Some(format!(
                    "Discarded the interrupted security scan {id}: a different scan was requested."
                ));
            }
            Some(Interrupted::Resumable { id, .. }) => {
                self.discard_interrupted(&id, "--new");
                notice = Some(format!(
                    "Discarded the interrupted security scan {id} (--new)."
                ));
            }
            Some(Interrupted::Stale { id, reason }) => {
                self.discard_interrupted(&id, &reason);
                notice = Some(format!(
                    "The interrupted security scan {id} cannot be resumed ({reason}); starting a new scan."
                ));
            }
            None => {}
        }
        let mut value = self.start_review(invocation.scan)?;
        if let Some(notice) = notice {
            value["notice"] = json!(notice);
        }
        Ok(value)
    }

    /// Transcript notices: a finished explicit scan (once) and watch findings.
    pub fn drain_notices(&self) -> Vec<String> {
        let mut notices = Vec::new();
        if let Some(progress) = self
            .review
            .status()
            .filter(|progress| progress.status.terminal())
        {
            let key = format!("{}/{}", progress.scan_id, progress.generation);
            let mut announced = self.announced.lock().unwrap_or_else(|e| e.into_inner());
            if announced.as_deref() != Some(key.as_str()) {
                *announced = Some(key);
                notices.push(self.completion_notice(&progress));
            }
        }
        self.watch.poll(&self.review_config);
        notices.extend(self.watch.take_notices());
        notices
    }

    fn completion_notice(&self, progress: &types::RunProgress) -> String {
        let report = self.report.lock().unwrap_or_else(|e| e.into_inner());
        let report = report
            .as_ref()
            .filter(|value| value["scanId"] == progress.scan_id);
        let status = serde_json::to_value(progress.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default();
        let Some(report) = report else {
            let reason = progress
                .limitations
                .last()
                .map(|reason| format!(": {reason}"))
                .unwrap_or_default();
            return format!(
                "Security scan {} {status}{reason}. It stays resumable while its deadline allows; /security-scan continues it.",
                progress.scan_id
            );
        };
        let findings = report["findings"].as_array().map_or(0, Vec::len);
        let blocking = watch::policy_findings(report, &self.review_config).len();
        format!(
            "Security scan {} {status}: {findings} finding(s), {blocking} at or above the failure policy; coverage {}. /security-scan --report shows the report.",
            progress.scan_id,
            if report["coverageComplete"] == true { "complete" } else { "partial" }
        )
    }

    /// Context for the next turn: watch findings, bounded and untrusted.
    pub fn take_watch_injection(&self) -> Option<String> {
        self.watch.poll(&self.review_config);
        self.watch.take_injection()
    }

    /// After a settled interactive/RPC turn: start a watch review if due.
    pub fn watch_settled_turn(&self) -> Result<bool, String> {
        let runner = self.runner.clone().ok_or("security provider unavailable")?;
        self.review_config.admit(true)?;
        let explicit = self
            .review
            .status()
            .is_some_and(|run| !run.status.terminal());
        self.watch
            .dispatch(&self.cwd, &runner, &self.review_config, explicit)
    }

    pub fn watch_due(&self) -> bool {
        self.watch.due(now_ms())
    }

    pub fn watch_status(&self) -> Value {
        self.watch.status()
    }

    /// Session end: the explicit scan stops (its checkpoint stays resumable)
    /// and the watch stops.
    pub fn shutdown(&self) {
        self.abort_review();
        self.watch.stop();
    }

    pub fn command(&mut self, name: &str, args: &str) -> Result<Option<Value>, String> {
        match name {
            "security-scan" => self.scan_command(args).map(Some),
            // Internal: the checkpoint resume the session-level flow uses.
            "sec-resume" => {
                let id = if args.trim().is_empty() {
                    self.review
                        .status()
                        .map(|s| s.scan_id)
                        .ok_or("provide a scan ID to resume")?
                } else {
                    args.trim().to_string()
                };
                self.resume_review(id).map(Some)
            }
            "sec-status" if self.review.status().is_some() => {
                let progress = self.review.status().unwrap();
                if !args.trim().is_empty() && args.trim() != progress.scan_id {
                    return Err("scan identity does not match this session".into());
                }
                let mut value = serde_json::to_value(progress).map_err(|err| err.to_string())?;
                value["watch"] = self.watch.status();
                Ok(Some(value))
            }
            "sec-abort" if self.review.status().is_some() => {
                let id = (!args.trim().is_empty()).then_some(args.trim());
                Ok(Some(
                    serde_json::to_value(self.review.abort(id)?).map_err(|err| err.to_string())?,
                ))
            }
            "sec-status" => Ok(Some(
                serde_json::to_value(self.current()).map_err(|err| err.to_string())?,
            )),
            "sec-report" if self.review.status().is_some() => {
                let request = command::ReportCommand::parse(args)?;
                let progress = self.review.status().unwrap();
                if request
                    .scan_id
                    .as_ref()
                    .is_some_and(|id| id != &progress.scan_id)
                {
                    return Err("scan identity does not match this session".into());
                }
                self.report_view(request.finding_id.as_deref()).map(Some)
            }
            "sec-report" if !args.trim().is_empty() => {
                let request = command::ReportCommand::parse(args)?;
                let id = request
                    .scan_id
                    .as_deref()
                    .ok_or("scan identity required without an active review")?;
                let agent_dir = self
                    .review_agent_dir
                    .as_ref()
                    .ok_or("security report storage unavailable")?;
                let store = store::Store::open(agent_dir, &git::root(&self.cwd), id, false)?;
                Ok(Some(report::select_finding(
                    store.available_report()?,
                    request.finding_id.as_deref(),
                )?))
            }
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
        if let Some(error) = &self.config_error {
            return Err(format!("security gate settings are invalid: {error}"));
        }
        self.cwd = request.cwd.to_path_buf();
        let repo_id = repo_id(&self.cwd);
        self.prepare_incremental_cache(&repo_id);
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
        let changes = request.diff.map(gate::parse_diff);

        let mut requested = request
            .changed_files
            .iter()
            .map(|file| file.replace('\\', "/"))
            .collect::<Vec<_>>();
        requested.sort();
        requested.dedup();
        let mut files_to_scan = Vec::new();
        let mut deleted = 0usize;
        // Text the gate could not read: coverage is incomplete, never a pass.
        let mut unscanned = Vec::new();
        // Inspected by name only (binary or excluded by policy); visible in the verdict.
        let mut uninspected = Vec::new();
        for file in &requested {
            let norm = match normalize_relative_path(Path::new(file)) {
                Ok(norm) => norm,
                Err(reason) => {
                    unscanned.push(format!("{file}: {reason}"));
                    continue;
                }
            };
            let key = norm.to_string_lossy().replace('\\', "/");
            let full = request.cwd.join(&norm);
            let marked_deleted = changes
                .as_ref()
                .and_then(|changes| changes.get(&key))
                .is_some_and(|change| change.deleted);
            if marked_deleted || fs::symlink_metadata(&full).is_err() {
                deleted += 1;
                continue;
            }
            if is_ignored(&norm, &self.config) {
                uninspected.push(format!("{key}: excluded by scan policy"));
                continue;
            }
            match fs::symlink_metadata(&full) {
                Ok(meta) if meta.is_file() => {}
                _ => {
                    unscanned.push(format!("{key}: not a regular file"));
                    continue;
                }
            }
            match read_scan_file(&self.cwd, &norm, &self.config) {
                Ok(_) => files_to_scan.push(norm),
                Err(ToolError::Failed(reason)) if reason == "binary file" => {
                    uninspected.push(format!("{key}: binary"));
                }
                Err(ToolError::Failed(reason)) if reason == "file exceeds scan limit" => {
                    unscanned.push(format!(
                        "{key}: exceeds the {} byte scan limit (securityScan.maxFileBytes)",
                        self.config.max_file_bytes
                    ));
                }
                Err(error) => unscanned.push(format!("{key}: unreadable ({error})")),
            }
        }

        let scope_digest = sha256_hex(requested.join("\n").as_bytes());
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
                files_scanned_cold: 0,
                files_reused: 0,
                files_rescanned: 0,
                cache_read_errors: 0,
                cache_write_errors: 0,
                files_deleted: 0,
                skipped: Vec::new(),
            },
            candidates: Vec::new(),
            findings: Vec::new(),
        };

        self.scan_files_incrementally(&files_to_scan, &mut scan);
        self.apply_incremental_telemetry(&mut scan);
        // Only lines the change added are judged. A file the diff does not
        // describe line by line (binary, oversized, absent) is judged whole.
        if let Some(changes) = &changes {
            let judged = |file: &str, line: usize| {
                changes
                    .get(file)
                    .and_then(|change| change.added.as_ref())
                    .is_none_or(|added| added.contains(&line))
            };
            scan.findings
                .retain(|finding| judged(&finding.file, finding.line));
            scan.candidates
                .retain(|candidate| judged(&candidate.file, candidate.line));
        }
        scan.coverage.files_deleted = deleted;
        scan.coverage.files_skipped += unscanned.len() + uninspected.len();
        scan.coverage.skipped = unscanned.iter().chain(&uninspected).cloned().collect();
        scan.coverage.candidate_count = scan.candidates.len();
        scan.coverage.finding_count = scan.findings.len();

        let artifact = SecurityArtifactStore::new(&self.artifact_agent_dir(), &repo_id, &scan_id)
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

        let threshold = config::severity_rank(&self.review_config.fail_on.minimum_severity);
        let (blocking, advisory): (Vec<_>, Vec<_>) = scan
            .findings
            .iter()
            .filter(|finding| !finding.false_positive)
            .partition(|finding| config::severity_rank(finding.severity.as_str()) >= threshold);
        let row = |finding: &&SecurityFinding| {
            format!(
                "{}:{} {} ({}) - {}",
                finding.file,
                finding.line,
                finding.rule_id,
                finding.severity.as_str(),
                finding.message
            )
        };
        if !blocking.is_empty() {
            let mut details: Vec<String> = blocking.iter().map(row).collect();
            details.extend(
                unscanned
                    .iter()
                    .map(|reason| format!("not scanned: {reason}")),
            );
            return Ok(SecurityVerification::Failed {
                scan_id,
                blockers: blocking.len(),
                details,
            });
        }
        if !unscanned.is_empty() {
            return Ok(SecurityVerification::Unavailable {
                reason: format!(
                    "incomplete security coverage (scan {scan_id}): {} changed file(s) could not be scanned: {}",
                    unscanned.len(),
                    unscanned.join("; ")
                ),
            });
        }
        if requested.is_empty() {
            return Ok(SecurityVerification::Unavailable {
                reason: "no changed files were supplied to the security gate".into(),
            });
        }
        let mut note = Vec::new();
        if deleted > 0 {
            note.push(format!("{deleted} deleted file(s) had nothing to scan"));
        }
        if !uninspected.is_empty() {
            note.push(format!("not inspected: {}", uninspected.join("; ")));
        }
        if !advisory.is_empty() {
            note.push(format!(
                "below the {} failure threshold: {}",
                self.review_config.fail_on.minimum_severity,
                advisory
                    .iter()
                    .take(10)
                    .map(row)
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        Ok(SecurityVerification::Passed {
            scan_id,
            note: (!note.is_empty()).then(|| note.join("; ")),
        })
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
    let bytes = read_scan_file_at_path(&path, config)?;
    scan_file_bytes(relative, &bytes, scan)
}

fn read_scan_file(
    root: &Path,
    relative: &Path,
    config: &SecurityScanConfig,
) -> Result<Vec<u8>, ToolError> {
    let path = safe_join(root, relative)?;
    read_scan_file_at_path(&path, config)
}

fn read_scan_file_at_path(path: &Path, config: &SecurityScanConfig) -> Result<Vec<u8>, ToolError> {
    let metadata = fs::metadata(path).map_err(|err| ToolError::Failed(err.to_string()))?;
    if metadata.len() > config.max_file_bytes {
        return Err(ToolError::Failed("file exceeds scan limit".into()));
    }
    let bytes = fs::read(path).map_err(|err| ToolError::Failed(err.to_string()))?;
    if bytes.contains(&0) {
        return Err(ToolError::Failed("binary file".into()));
    }
    Ok(bytes)
}

fn scan_file_bytes(
    relative: &Path,
    bytes: &[u8],
    scan: &mut SecurityScan,
) -> Result<(), ToolError> {
    let text = String::from_utf8_lossy(bytes);
    scan.coverage.bytes_scanned = scan
        .coverage
        .bytes_scanned
        .saturating_add(bytes.len() as u64);
    let file = relative.to_string_lossy().replace('\\', "/");
    for hit in gate::detect(&text) {
        let id = format!(
            "{}-{}",
            hit.rule_id,
            &sha256_hex(format!("{file}:{}:{}", hit.line, hit.evidence).as_bytes())[..12]
        );
        if scan.findings.iter().any(|finding| finding.id == id) {
            continue;
        }
        scan.candidates.push(SecurityCandidate {
            id: id.clone(),
            rule_id: hit.rule_id.into(),
            file: file.clone(),
            line: hit.line,
            reason: hit.message.into(),
            validated: false,
            disposition: None,
            validation_reason: None,
            attack_path: None,
        });
        scan.findings.push(SecurityFinding {
            id,
            rule_id: hit.rule_id.into(),
            severity: hit.severity,
            file: file.clone(),
            line: hit.line,
            message: hit.message.into(),
            evidence: hit.evidence,
            validated: false,
            false_positive: false,
        });
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
    let path_str = path.to_string_lossy();
    if path_str.starts_with('/')
        || path_str.starts_with('\\')
        || (path_str.len() >= 2
            && path_str.as_bytes()[0].is_ascii_alphabetic()
            && path_str.as_bytes()[1] == b':')
    {
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

    fn test_agent(controller: &mut SecurityScanController) -> tempfile::TempDir {
        let agent = tempdir().unwrap();
        controller.set_review_storage(agent.path().to_path_buf());
        agent
    }

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
        fs::write(
            dir.path().join("sample.txt"),
            "token=sk-proj-4f9Qa2Lk8Zt3Vb7Nc1Xd6Rm0Hs\n",
        )
        .unwrap();
        let mut controller = SecurityScanController::new(dir.path().to_path_buf());
        let _agent = test_agent(&mut controller);
        let scan = controller.start(None).unwrap();
        assert!(!scan.manifest.allow_network);
        assert_eq!(scan.findings.len(), 1);
        assert!(!scan.findings[0].evidence.contains("4f9Qa2Lk8Zt3"));
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
        let _agent = test_agent(&mut controller);
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
        let _agent = test_agent(&mut controller);
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
        let _agent = test_agent(&mut controller);
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
        let _agent = test_agent(&mut controller);
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
        fs::write(
            dir.path().join("sample.txt"),
            "token=sk-proj-4f9Qa2Lk8Zt3Vb7Nc1Xd6Rm0Hs\n",
        )
        .unwrap();
        let mut controller = SecurityScanController::new(dir.path().to_path_buf());
        let _agent = test_agent(&mut controller);
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
                files_scanned_cold: 0,
                files_reused: 0,
                files_rescanned: 0,
                cache_read_errors: 0,
                cache_write_errors: 0,
                files_deleted: 0,
                skipped: Vec::new(),
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
            "pub fn key() -> &'static str { \"sk-proj-4f9Qa2Lk8Zt3Vb7Nc1Xd6Rm0Hs\" }\n",
        )
        .unwrap();

        let mut controller = SecurityScanController::new(tmp.path().to_path_buf());
        let _agent = test_agent(&mut controller);
        let request = SecurityVerifyRequest {
            cwd: tmp.path(),
            changed_files: &["src/auth.rs".to_string()],
            graph_run_id: "run-test-1",
            diff: None,
        };

        let result = controller.verify_changed_surface(request).unwrap();
        match result {
            SecurityVerification::Failed {
                scan_id, blockers, ..
            } => {
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
        let _agent = test_agent(&mut controller);
        let request = SecurityVerifyRequest {
            cwd: tmp.path(),
            changed_files: &["src/ui.rs".to_string()],
            graph_run_id: "run-test-2",
            diff: None,
        };

        let result = controller.verify_changed_surface(request).unwrap();
        match result {
            SecurityVerification::Passed { scan_id, .. } => {
                assert!(scan_id.starts_with("scan-"));
            }
            other => panic!("expected Passed, got {other:?}"),
        }
    }

    #[test]
    fn security_scan_reuses_unchanged_files_but_reseals_run() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("one.rs"), "pub fn one() {}\n").unwrap();
        fs::write(tmp.path().join("two.rs"), "pub fn two() {}\n").unwrap();
        let files = vec!["one.rs".to_string(), "two.rs".to_string()];
        let mut controller = SecurityScanController::new(tmp.path().to_path_buf());
        let _agent = test_agent(&mut controller);

        controller
            .verify_changed_surface(SecurityVerifyRequest {
                cwd: tmp.path(),
                changed_files: &files,
                graph_run_id: "cold",
                diff: None,
            })
            .unwrap();
        let first = controller.current().unwrap();
        let first_artifact = controller.artifact.as_ref().unwrap().root().to_path_buf();

        controller
            .verify_changed_surface(SecurityVerifyRequest {
                cwd: tmp.path(),
                changed_files: &files,
                graph_run_id: "warm",
                diff: None,
            })
            .unwrap();
        let second = controller.current().unwrap();
        let second_artifact = controller.artifact.as_ref().unwrap().root().to_path_buf();

        assert_eq!(second.coverage.files_reused, 2);
        assert_eq!(second.coverage.files_rescanned, 0);
        assert_eq!(second.findings, first.findings);
        assert_eq!(second.candidates, first.candidates);
        assert_eq!(second.coverage.bytes_scanned, first.coverage.bytes_scanned);
        assert_ne!(second.manifest.scan_id, first.manifest.scan_id);
        assert_ne!(second_artifact, first_artifact);
        assert!(second.manifest.sealed_at.is_some());
        assert!(second_artifact.join("scan-manifest.json").is_file());
    }

    #[test]
    fn security_scan_new_controller_never_inherits_incremental_evidence() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("one.rs"), "pub fn one() {}\n").unwrap();
        let files = vec!["one.rs".to_string()];

        let mut first_controller = SecurityScanController::new(tmp.path().to_path_buf());
        let _first_agent = test_agent(&mut first_controller);
        first_controller
            .verify_changed_surface(SecurityVerifyRequest {
                cwd: tmp.path(),
                changed_files: &files,
                graph_run_id: "first-process",
                diff: None,
            })
            .unwrap();

        let mut second_controller = SecurityScanController::new(tmp.path().to_path_buf());
        let _second_agent = test_agent(&mut second_controller);
        second_controller
            .verify_changed_surface(SecurityVerifyRequest {
                cwd: tmp.path(),
                changed_files: &files,
                graph_run_id: "second-process",
                diff: None,
            })
            .unwrap();
        let second = second_controller.current().unwrap();

        assert_eq!(second.coverage.files_scanned_cold, 1);
        assert_eq!(second.coverage.files_reused, 0);
        assert_eq!(second.coverage.files_rescanned, 0);
    }

    #[test]
    fn security_scan_invalidates_changed_file_cache_entry() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("one.rs"), "pub fn one() {}\n").unwrap();
        fs::write(tmp.path().join("two.rs"), "pub fn two() {}\n").unwrap();
        let files = vec!["one.rs".to_string(), "two.rs".to_string()];
        let mut controller = SecurityScanController::new(tmp.path().to_path_buf());
        let _agent = test_agent(&mut controller);
        controller
            .verify_changed_surface(SecurityVerifyRequest {
                cwd: tmp.path(),
                changed_files: &files,
                graph_run_id: "before-change",
                diff: None,
            })
            .unwrap();

        fs::write(tmp.path().join("one.rs"), "pub fn one() { eval(input); }\n").unwrap();
        controller
            .verify_changed_surface(SecurityVerifyRequest {
                cwd: tmp.path(),
                changed_files: &files,
                graph_run_id: "after-change",
                diff: None,
            })
            .unwrap();
        let scan = controller.current().unwrap();
        assert_eq!(scan.coverage.files_reused, 1);
        assert_eq!(scan.coverage.files_rescanned, 1);
        assert_eq!(scan.coverage.files_scanned_cold, 0);
        assert!(scan
            .findings
            .iter()
            .any(|finding| finding.rule_id == "command.eval"));
    }

    #[test]
    fn security_graph_deterministic_gate_preserves_blockers() {
        use crate::native_extensions::ecosystem::verification::{
            SecurityPolicyMode, VerificationBundle,
        };
        let tmp = tempfile::tempdir().unwrap();
        let auth_file = tmp.path().join("src/auth.rs");
        fs::create_dir_all(auth_file.parent().unwrap()).unwrap();
        fs::write(
            &auth_file,
            "pub fn key() -> &'static str { \"sk-proj-4f9Qa2Lk8Zt3Vb7Nc1Xd6Rm0Hs\" }\n",
        )
        .unwrap();
        let mut controller = SecurityScanController::new(tmp.path().to_path_buf());
        let _agent = test_agent(&mut controller);
        let result = controller
            .verify_changed_surface(SecurityVerifyRequest {
                cwd: tmp.path(),
                changed_files: &["src/auth.rs".to_string()],
                graph_run_id: "run-gate",
                diff: None,
            })
            .unwrap();
        match &result {
            SecurityVerification::Failed { blockers, .. } => assert!(*blockers > 0),
            other => panic!("expected Failed, got {other:?}"),
        }
        let bundle = VerificationBundle {
            commands_ran: 1,
            commands_failed: 0,
            deterministic_passed: true,
            security: result.clone(),
            changed_files: vec!["src/auth.rs".into()],
            graph_run_id: Some("run-gate".into()),
            source_manifest_digest: None,
        };
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Risk));
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Always));
        let unavailable = VerificationBundle {
            security: SecurityVerification::Unavailable {
                reason: "ai scan incomplete".into(),
            },
            ..bundle.clone()
        };
        assert!(
            !unavailable.approval_eligible(SecurityPolicyMode::Always),
            "incomplete AI scan must not clear a mandatory security gate"
        );
        assert!(!bundle.approval_eligible(SecurityPolicyMode::Risk));
    }

    fn gate(
        controller: &mut SecurityScanController,
        root: &Path,
        files: &[&str],
        diff: Option<&str>,
    ) -> SecurityVerification {
        let files: Vec<String> = files.iter().map(|file| file.to_string()).collect();
        controller
            .verify_changed_surface(SecurityVerifyRequest {
                cwd: root,
                changed_files: &files,
                graph_run_id: "gate",
                diff,
            })
            .unwrap()
    }

    const KEY: &str = "sk-proj-4f9Qa2Lk8Zt3Vb7Nc1Xd6Rm0Hs";

    #[test]
    fn security_gate_judges_only_added_lines_and_names_each_blocker() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        // A pre-existing credential on line 1; the change adds line 2 only.
        fs::write(
            tmp.path().join("src/config.rs"),
            format!("const OLD: &str = \"{KEY}\";\nfn task_retrieval() {{}}\n"),
        )
        .unwrap();
        let mut controller = SecurityScanController::new(tmp.path().to_path_buf());
        let _agent = test_agent(&mut controller);
        let diff = "diff --git a/src/config.rs b/src/config.rs\n--- a/src/config.rs\n+++ b/src/config.rs\n@@ -1,1 +1,2 @@\n const OLD\n+fn task_retrieval() {}\n";
        assert!(matches!(
            gate(&mut controller, tmp.path(), &["src/config.rs"], Some(diff)),
            SecurityVerification::Passed { note: None, .. }
        ));
        // Now the change adds the credential itself.
        let diff = "diff --git a/src/config.rs b/src/config.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/config.rs\n@@ -0,0 +1,2 @@\n+const OLD\n+fn task_retrieval() {}\n";
        match gate(&mut controller, tmp.path(), &["src/config.rs"], Some(diff)) {
            SecurityVerification::Failed {
                blockers, details, ..
            } => {
                assert_eq!(blockers, 1);
                assert!(
                    details[0].starts_with("src/config.rs:1 secret.api-key (high)"),
                    "{details:?}"
                );
                assert!(!details[0].contains("4f9Qa2Lk8Zt3"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn security_gate_respects_the_failure_severity_threshold() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("run.py"), "value = eval(user_input)\n").unwrap();
        let mut controller = SecurityScanController::new(tmp.path().to_path_buf());
        let _agent = test_agent(&mut controller);
        match gate(&mut controller, tmp.path(), &["run.py"], None) {
            SecurityVerification::Passed {
                note: Some(note), ..
            } => assert!(note.contains("command.eval"), "{note}"),
            other => panic!("medium must not block at the high default: {other:?}"),
        }
        controller.review_config.fail_on.minimum_severity = "medium".into();
        assert!(matches!(
            gate(&mut controller, tmp.path(), &["run.py"], None),
            SecurityVerification::Failed { blockers: 1, .. }
        ));
    }

    #[test]
    fn security_gate_makes_incomplete_coverage_visible() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("big.rs"), "x".repeat(64)).unwrap();
        fs::write(tmp.path().join("logo.png"), [0u8, 1, 2, 3]).unwrap();
        let mut controller = SecurityScanController::new(tmp.path().to_path_buf());
        let _agent = test_agent(&mut controller);
        controller.config.max_file_bytes = 16;
        match gate(&mut controller, tmp.path(), &["big.rs"], None) {
            SecurityVerification::Unavailable { reason } => {
                assert!(reason.contains("big.rs"), "{reason}");
                assert!(reason.contains("securityScan.maxFileBytes"), "{reason}");
            }
            other => panic!("an unscanned text file must not pass: {other:?}"),
        }
        match gate(&mut controller, tmp.path(), &["gone.rs", "logo.png"], None) {
            SecurityVerification::Passed {
                note: Some(note), ..
            } => {
                assert!(note.contains("1 deleted"), "{note}");
                assert!(note.contains("logo.png: binary"), "{note}");
            }
            other => panic!("deletions and binaries are counted, not hidden: {other:?}"),
        }
        assert_eq!(controller.current().unwrap().coverage.files_deleted, 1);
        assert!(matches!(
            gate(&mut controller, tmp.path(), &[], None),
            SecurityVerification::Unavailable { .. }
        ));
    }

    #[test]
    fn security_gate_artifacts_never_land_in_the_repository() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("a.rs"), "fn a() {}\n").unwrap();
        let mut controller = SecurityScanController::new(tmp.path().to_path_buf());
        let agent = test_agent(&mut controller);
        gate(&mut controller, tmp.path(), &["a.rs"], None);
        assert!(!tmp.path().join(".davinci").exists());
        assert!(agent.path().join("security-scans").is_dir());
    }

    #[test]
    fn security_scan_settings_load_file_policy_and_fail_closed_when_invalid() {
        let tmp = tempdir().unwrap();
        let agent = tempdir().unwrap();
        fs::write(
            agent.path().join("settings.json"),
            r#"{"securityScan":{"maxFileBytes":4096,"includeHidden":false,"failOn":{"classifications":["confirmed"],"minimumSeverity":"medium"},"watch":{"enabled":false}}}"#,
        )
        .unwrap();
        let controller = SecurityScanController::with_settings(
            tmp.path().to_path_buf(),
            agent.path().to_path_buf(),
            false,
        );
        assert_eq!(controller.config.max_file_bytes, 4096);
        assert!(!controller.config.include_hidden);
        assert!(!controller.config.allow_network);
        assert_eq!(controller.review_config.fail_on.minimum_severity, "medium");
        assert_eq!(controller.watch_status()["enabled"], false);

        fs::write(
            agent.path().join("settings.json"),
            r#"{"securityScan":{"toolNetwork":"allow"}}"#,
        )
        .unwrap();
        let mut invalid = SecurityScanController::with_settings(
            tmp.path().to_path_buf(),
            agent.path().to_path_buf(),
            false,
        );
        let files = vec!["a.rs".to_string()];
        assert!(invalid
            .verify_changed_surface(SecurityVerifyRequest {
                cwd: tmp.path(),
                changed_files: &files,
                graph_run_id: "invalid",
                diff: None,
            })
            .is_err());
    }
}
