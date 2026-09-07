//! Private, create-new review checkpoints and immutable report publication.
//! Hashes detect inconsistent artifacts; they do not attest against their owner.
use super::{command::ScanCommand, snapshot::Snapshot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Store {
    directory: PathBuf,
    scan_id: String,
    _lease: std::sync::Arc<fs::File>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Seal {
    schema_version: u32,
    generation: u64,
    scan_id: String,
    artifacts: Vec<SealedArtifact>,
    evidence: Vec<super::artifact_index::Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedArtifact {
    path: String,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Checkpoint {
    pub schema_version: u32,
    pub request: ScanCommand,
    pub snapshot: Snapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<CheckpointBinding>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckpointBinding {
    policy_sha256: String,
    methodology_sha256: String,
    #[serde(default)]
    budget_schema_version: u32,
    #[serde(default)]
    record_identity_version: u32,
}

impl CheckpointBinding {
    fn new(config: &super::config::ScanConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            budget_schema_version: 3,
            record_identity_version: super::identity::VERSION,
            policy_sha256: super::sha256_hex(
                &serde_json::to_vec(config).map_err(|_| "cannot bind scan policy")?,
            ),
            methodology_sha256: super::sha256_hex(
                &serde_json::to_vec(&super::skills::manifest())
                    .map_err(|_| "cannot bind scan methodology")?,
            ),
        })
    }
}

impl Checkpoint {
    pub fn validate_resume(&self, config: &super::config::ScanConfig) -> Result<(), String> {
        let binding = self
            .binding
            .as_ref()
            .ok_or("checkpoint has no policy or methodology binding; start a new scan")?;
        let current = CheckpointBinding::new(config)?;
        if binding.record_identity_version != current.record_identity_version {
            return Err("checkpoint record identity changed; start a new scan".into());
        }
        if binding.budget_schema_version != current.budget_schema_version {
            return Err("checkpoint lacks durable request accounting; start a new scan".into());
        }
        if binding.policy_sha256 != current.policy_sha256 {
            return Err("checkpoint policy changed; start a new scan".into());
        }
        if binding.methodology_sha256 != current.methodology_sha256 {
            return Err("checkpoint methodology changed; start a new scan".into());
        }
        config.validate_snapshot(&self.snapshot)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CheckpointBundle {
    schema_version: u32,
    scan_id: String,
    sha256: String,
    checkpoint: Checkpoint,
}

impl Store {
    pub fn open(agent_dir: &Path, root: &Path, id: &str, create: bool) -> Result<Self, String> {
        let parsed = uuid::Uuid::parse_str(id).map_err(|_| "invalid scan identity")?;
        if parsed.to_string() != id {
            return Err("noncanonical scan identity".into());
        }
        let root = root
            .canonicalize()
            .map_err(|_| "cannot bind scan repository")?;
        let agent_dir = agent_dir
            .canonicalize()
            .map_err(|_| "cannot resolve agent storage directory")?;
        if agent_dir.starts_with(&root) {
            return Err("security storage must be outside the reviewed repository".into());
        }
        let repo = super::sha256_hex(root.to_string_lossy().as_bytes());
        let mut directory = agent_dir;
        for component in ["security-scans", &repo, id] {
            directory.push(component);
            if create {
                private_directory(&directory)?;
            }
            let metadata = fs::symlink_metadata(&directory)
                .map_err(|_| "security checkpoint directory unavailable")?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("linked security storage denied".into());
                }
            }
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("invalid security storage directory".into());
            }
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0).custom_flags(0x00200000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let lease = options
            .open(directory.join("active.lock"))
            .map_err(|_| "security run is active or its lock is unavailable")?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err("security run is already active".into());
            }
        }
        Ok(Self {
            directory,
            scan_id: id.into(),
            _lease: std::sync::Arc::new(lease),
        })
    }

    pub fn load_feedback(&self) -> Result<Vec<super::feedback::ControlFeedback>, String> {
        let Some(parent) = self.directory.parent() else {
            return Ok(Vec::new());
        };
        let path = parent.join("feedback.json");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(_) => Err("cannot inspect security feedback".into()),
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err("linked security feedback denied".into())
            }
            Ok(_) => {
                let bytes = fs::read(&path).map_err(|_| "cannot read security feedback")?;
                if bytes.len() as u64 > 1024 * 1024 {
                    return Err("security feedback exceeds byte limit".into());
                }
                serde_json::from_slice(&bytes)
                    .map_err(|_| "invalid security feedback schema".into())
            }
        }
    }

    pub(super) fn publish(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let target = self.directory.join(name);
        if target.exists() {
            return Err("immutable security artifact already exists".into());
        }
        let temp = self
            .directory
            .join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|_| "cannot create private security artifact")?;
        let result = (|| {
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|_| "cannot persist security artifact")?;
            drop(file);
            // Hard-link publication is atomic and fails if the destination exists.
            fs::hard_link(&temp, &target)
                .map_err(|_| "cannot publish immutable security artifact")?;
            self.sync_directory()
        })();
        // Remove only the temporary file created by this invocation, including
        // on failed publication. An abrupt process exit can leave it orphaned;
        // readers never interpret temporary files as committed artifacts.
        let cleanup = fs::remove_file(&temp)
            .map_err(|_| "cannot remove security temporary artifact".to_string());
        result?;
        cleanup?;
        self.sync_directory()
    }

    fn sync_directory(&self) -> Result<(), String> {
        #[cfg(unix)]
        fs::File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "cannot persist security artifact directory")?;
        Ok(())
    }

    #[cfg(test)]
    pub fn checkpoint(&self, request: &ScanCommand, snapshot: &Snapshot) -> Result<(), String> {
        self.checkpoint_bound(request, snapshot, &super::config::ScanConfig::default())
    }

    pub fn checkpoint_bound(
        &self,
        request: &ScanCommand,
        snapshot: &Snapshot,
        config: &super::config::ScanConfig,
    ) -> Result<(), String> {
        if fs::symlink_metadata(self.directory.join("checkpoint.json")).is_ok() {
            return Err("immutable security checkpoint already exists".into());
        }
        let checkpoint = Checkpoint {
            schema_version: 2,
            binding: Some(CheckpointBinding::new(config)?),
            request: request.clone(),
            snapshot: snapshot.clone(),
        };
        let limit = config.checkpoint_byte_limit();
        let sha256 = super::sha256_hex(&super::checkpoint_encoding::encode(&checkpoint, limit)?);
        let bundle = CheckpointBundle {
            schema_version: 1,
            scan_id: self.scan_id.clone(),
            sha256,
            checkpoint,
        };
        self.publish(
            "checkpoint.bundle.json",
            &super::checkpoint_encoding::encode(&bundle, limit)?,
        )
    }

    pub fn load(&self, max_bytes: u64) -> Result<Checkpoint, String> {
        let checkpoint = match fs::symlink_metadata(self.directory.join("checkpoint.bundle.json")) {
            Ok(_) => {
                let bundle: CheckpointBundle =
                    serde_json::from_slice(&self.read("checkpoint.bundle.json", max_bytes)?)
                        .map_err(|_| "invalid security checkpoint bundle")?;
                if bundle.schema_version != 1 || bundle.scan_id != self.scan_id {
                    return Err("security checkpoint bundle identity mismatch".into());
                }
                let bytes = serde_json::to_vec(&bundle.checkpoint)
                    .map_err(|_| "cannot encode security checkpoint")?;
                if bundle.sha256 != super::sha256_hex(&bytes) {
                    return Err("security checkpoint hash mismatch".into());
                }
                bundle.checkpoint
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.load_legacy_checkpoint(max_bytes)?
            }
            Err(_) => return Err("cannot inspect security checkpoint bundle".into()),
        };
        Self::validate_checkpoint(checkpoint)
    }

    fn load_legacy_checkpoint(&self, max_bytes: u64) -> Result<Checkpoint, String> {
        let bytes = self.read("checkpoint.json", max_bytes)?;
        let hash = self.read("checkpoint.sha256", 64)?;
        if hash != super::sha256_hex(&bytes).as_bytes() {
            return Err("security checkpoint hash mismatch".into());
        }
        serde_json::from_slice(&bytes).map_err(|_| "invalid security checkpoint schema".into())
    }

    fn validate_checkpoint(checkpoint: Checkpoint) -> Result<Checkpoint, String> {
        if checkpoint.schema_version != 2 {
            return Err("unsupported security checkpoint version".into());
        }
        let mut conflicts = std::collections::BTreeMap::new();
        let deferred_paths: std::collections::BTreeSet<_> = checkpoint
            .snapshot
            .skipped
            .iter()
            .chain(&checkpoint.snapshot.supporting_skipped)
            .filter(|skip| !skip.reason.trim().is_empty())
            .map(|skip| &skip.path)
            .collect();
        for conflict in &checkpoint.snapshot.conflicts {
            conflict.validate()?;
            if conflicts
                .insert((&conflict.path, conflict.stage), conflict)
                .is_some()
            {
                return Err("duplicate checkpoint conflict stage".into());
            }
            if !deferred_paths.contains(&conflict.path) {
                return Err("checkpoint conflict is missing its deferred coverage".into());
            }
        }
        let mut source_bytes = 0u64;
        if checkpoint.snapshot.conflict_sources.windows(2).any(|pair| {
            (&pair[0].identity.path, pair[0].identity.stage)
                >= (&pair[1].identity.path, pair[1].identity.stage)
        }) {
            return Err("checkpoint conflict sources are not uniquely sorted".into());
        }
        for source in &checkpoint.snapshot.conflict_sources {
            source.identity.validate()?;
            if !["100644", "100755"].contains(&source.identity.mode.as_str())
                || conflicts
                    .get(&(&source.identity.path, source.identity.stage))
                    .copied()
                    != Some(&source.identity)
            {
                return Err(
                    "checkpoint conflict source has no matching regular index identity".into(),
                );
            }
        }
        let mut source_identities = std::collections::BTreeSet::new();
        for (side, path, file) in checkpoint.snapshot.sources() {
            if !source_identities.insert((side, path)) {
                return Err("duplicate target/supporting source identity".into());
            }
            let relative = super::snapshot::relative_scope(path)?;
            if relative.to_string_lossy().replace('\\', "/") != path
                || super::snapshot::denied(&relative)
            {
                return Err("invalid checkpoint source path".into());
            }
            if file.hash != super::sha256_hex(file.text.as_bytes()) {
                return Err("checkpoint source hash mismatch".into());
            }
            source_bytes = source_bytes
                .checked_add(file.text.len() as u64)
                .ok_or("checkpoint byte count overflow")?;
        }
        if source_bytes != checkpoint.snapshot.bytes {
            return Err("checkpoint source byte count mismatch".into());
        }
        Ok(checkpoint)
    }

    pub(super) fn read_optional(&self, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
        match fs::symlink_metadata(self.directory.join(name)) {
            Ok(_) => self.read(name, limit).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("cannot inspect security artifact".into()),
        }
    }

    pub(super) fn read(&self, name: &str, limit: u64) -> Result<Vec<u8>, String> {
        let root = self
            .directory
            .canonicalize()
            .map_err(|_| "cannot resolve checkpoint storage")?;
        let file = super::snapshot::open_confined(&root, Path::new(name))
            .map_err(|_| "cannot open private security artifact")?;
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| "cannot read security artifact")?;
        if bytes.len() as u64 > limit {
            return Err("security artifact exceeds limit".into());
        }
        Ok(bytes)
    }

    pub fn complete(&self, value: &Value, generation: u64) -> Result<(), String> {
        self.validate_report_identity(value, generation)?;
        super::report_contract::validate(value)?;
        self.read(&format!("generation-{generation}"), 0)?;
        self.validate_report_evidence(value)?;
        let report =
            serde_json::to_vec_pretty(value).map_err(|_| "cannot encode security report")?;
        let name = format!("report-{generation}.json");
        self.publish(&name, &report)?;
        let mut artifacts = vec![json!({"path":name,"sha256":super::sha256_hex(&report)})];
        for (name, format) in [
            (
                format!("report-{generation}.md"),
                super::command::ReportFormat::Terminal,
            ),
            (
                format!("results-{generation}.sarif"),
                super::command::ReportFormat::Sarif,
            ),
        ] {
            let projection = super::report::render(value, format)?;
            self.publish(&name, projection.as_bytes())?;
            artifacts.push(json!({"path":name,"sha256":super::sha256_hex(projection.as_bytes())}));
        }
        let evidence = super::artifact_index::capture(&self.directory, generation)?;
        let index = json!({"schemaVersion":3,"generation":generation,"scanId":self.scan_id,"artifacts":artifacts,"evidence":evidence});
        let index = serde_json::to_vec(&index).map_err(|_| "cannot encode security seal")?;
        if index.len() as u64 > super::artifact_index::MAX_SEAL_BYTES {
            return Err("security seal exceeds byte limit".into());
        }
        self.publish(&format!("seal-{generation}.json"), &index)
    }

    fn validate_report_identity(&self, value: &Value, generation: u64) -> Result<(), String> {
        if !(1..=1024).contains(&generation)
            || value["schemaVersion"] != 2
            || value["generation"].as_u64() != Some(generation)
            || value["scanId"].as_str() != Some(self.scan_id.as_str())
        {
            return Err("security report does not match its run and generation".into());
        }
        Ok(())
    }

    pub fn latest_report(&self) -> Result<Value, String> {
        let mut latest = None;
        for generation in 1..=1024 {
            if self
                .directory
                .join(format!("seal-{generation}.json"))
                .exists()
            {
                latest = Some(generation);
            }
        }
        let generation = latest.ok_or("no sealed security report is available")?;
        let seal: Seal = serde_json::from_slice(&self.read(
            &format!("seal-{generation}.json"),
            super::artifact_index::MAX_SEAL_BYTES,
        )?)
        .map_err(|_| "invalid security seal")?;
        if seal.schema_version != 3 || seal.generation != generation || seal.scan_id != self.scan_id
        {
            return Err("security seal identity mismatch".into());
        }
        if seal.evidence != super::artifact_index::capture(&self.directory, generation)? {
            return Err("security evidence artifact index mismatch".into());
        }
        let artifacts = &seal.artifacts;
        if artifacts.len() != 3 {
            return Err("incomplete security artifact index".into());
        }
        let mut report = None;
        for (artifact, expected) in artifacts.iter().zip([
            format!("report-{generation}.json"),
            format!("report-{generation}.md"),
            format!("results-{generation}.sarif"),
        ]) {
            if artifact.path != expected {
                return Err("invalid sealed artifact path".into());
            }
            let bytes = self.read(&expected, 32 * 1024 * 1024)?;
            if artifact.sha256 != super::sha256_hex(&bytes) {
                return Err("security report seal mismatch".into());
            }
            if expected.ends_with(".json") {
                report = Some(
                    serde_json::from_slice(&bytes).map_err(|_| "invalid security report JSON")?,
                );
            }
        }
        let report = report.ok_or("security report missing from seal")?;
        self.validate_report_identity(&report, generation)?;
        super::report_contract::validate(&report)?;
        self.validate_report_evidence(&report)?;
        Ok(report)
    }

    fn validate_report_evidence(&self, report: &Value) -> Result<(), String> {
        let candidates = report["candidates"]
            .as_array()
            .ok_or("report lacks candidate history")?;
        let limit = super::config::ScanConfig::default().checkpoint_byte_limit();
        let checkpoint = self.load(limit)?;
        super::report_contract::validate_snapshot(report, &checkpoint.snapshot)?;
        super::report_contract::validate_mode(report, checkpoint.request.mode)?;
        for candidate in candidates {
            let claim =
                serde_json::from_value::<super::validation::Claim>(candidate["claim"].clone())
                    .map_err(|_| "saved claim schema is invalid")?;
            super::validation::validate_claim(&claim, &checkpoint.snapshot)?;
            for key in ["assessment", "priorAssessment"] {
                if candidate[key].get("gates").is_some() {
                    let assessment = serde_json::from_value::<super::validation::Assessment>(
                        candidate[key].clone(),
                    )
                    .map_err(|_| "saved assessment schema is invalid")?;
                    super::validation::validate_assessment(
                        &claim,
                        &assessment,
                        &checkpoint.snapshot,
                    )?;
                }
            }
        }
        Ok(())
    }

    pub fn partial_report(&self, report: &Value) -> Result<(), String> {
        let generation = report["generation"]
            .as_u64()
            .ok_or("missing partial generation")?;
        self.validate_report_identity(report, generation)?;
        let sequence = report["checkpointSequence"]
            .as_u64()
            .ok_or("missing partial sequence")?;
        if !(1..=256).contains(&sequence)
            || report["coverageComplete"] != false
            || report["partial"] != true
            || report["status"] != "interrupted"
        {
            return Err("invalid partial report status".into());
        }
        self.read(&format!("generation-{generation}"), 0)?;
        let artifact = super::partial::Artifact {
            schema_version: 1,
            digest: super::sha256_hex(
                &serde_json::to_vec(report).map_err(|_| "cannot encode partial report")?,
            ),
            report: report.clone(),
        };
        let bytes = serde_json::to_vec(&artifact).map_err(|_| "cannot encode partial artifact")?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err("partial report exceeds limit".into());
        }
        self.publish(&format!("partial-{generation}-{sequence}.json"), &bytes)
    }

    pub fn available_report(&self) -> Result<Value, String> {
        if self.has_sealed_report() {
            return self.latest_report();
        }
        if let Some(legacy) = self.legacy_v1_report()? {
            return Ok(legacy);
        }
        let mut latest = None;
        for (count, entry) in fs::read_dir(&self.directory)
            .map_err(|_| "cannot enumerate partial reports")?
            .enumerate()
        {
            if count >= 32768 {
                return Err("security artifact inventory exceeds limit".into());
            }
            let entry = entry.map_err(|_| "cannot inspect partial report entry")?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(parts) = name
                .strip_prefix("partial-")
                .and_then(|s| s.strip_suffix(".json"))
            else {
                continue;
            };
            let (generation, sequence) = parts
                .split_once('-')
                .ok_or("invalid partial artifact name")?;
            let generation: u64 = generation
                .parse()
                .map_err(|_| "invalid partial generation")?;
            let sequence: u64 = sequence.parse().map_err(|_| "invalid partial sequence")?;
            if !(1..=1024).contains(&generation)
                || !(1..=256).contains(&sequence)
                || name != format!("partial-{generation}-{sequence}.json")
            {
                return Err("invalid partial artifact identity".into());
            }
            latest = Some(latest.map_or((generation, sequence), |old: (u64, u64)| {
                old.max((generation, sequence))
            }));
        }
        let (generation, sequence) = latest.ok_or("no security report checkpoint is available")?;
        self.read(&format!("generation-{generation}"), 0)?;
        let artifact: super::partial::Artifact = serde_json::from_slice(&self.read(
            &format!("partial-{generation}-{sequence}.json"),
            32 * 1024 * 1024,
        )?)
        .map_err(|_| "invalid partial report artifact")?;
        self.validate_report_identity(&artifact.report, generation)?;
        if artifact.schema_version != 1
            || artifact.report["checkpointSequence"] != sequence
            || artifact.report["partial"] != true
            || artifact.report["coverageComplete"] != false
            || artifact.report["status"] != "interrupted"
            || artifact.digest
                != super::sha256_hex(
                    &serde_json::to_vec(&artifact.report)
                        .map_err(|_| "cannot verify partial report")?,
                )
        {
            return Err("partial report integrity mismatch".into());
        }
        self.validate_report_evidence(&artifact.report)?;
        Ok(artifact.report)
    }

    pub fn has_sealed_report(&self) -> bool {
        (1..=1024).any(|generation| {
            self.directory
                .join(format!("seal-{generation}.json"))
                .exists()
        })
    }

    fn legacy_v1_report(&self) -> Result<Option<Value>, String> {
        match fs::symlink_metadata(self.directory.join("findings.json")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("cannot inspect legacy v1 report".into()),
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err("legacy v1 report must be a regular file".into())
            }
            Ok(_) => {
                let bytes = self.read("findings.json", 32 * 1024 * 1024)?;
                Ok(Some(super::report::project_legacy_v1(&bytes)?))
            }
        }
    }

    pub fn next_generation(&self) -> Result<u64, String> {
        for generation in 1..=1024 {
            let path = self.directory.join(format!("generation-{generation}"));
            match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(file) => {
                    file.sync_all()
                        .map_err(|_| "cannot persist generation identity")?;
                    self.sync_directory()?;
                    return Ok(generation);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(_) => return Err("cannot reserve scan generation".into()),
            }
        }
        Err("scan generation limit reached".into())
    }
}

#[cfg(unix)]
fn private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata =
                fs::symlink_metadata(path).map_err(|_| "cannot inspect security directory")?;
            if metadata.permissions().mode() & 0o077 != 0 {
                Err("security directory permissions are not private".into())
            } else {
                Ok(())
            }
        }
        Err(_) => Err("cannot create private security directory".into()),
    }
}

#[cfg(windows)]
fn private_directory(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut std::ffi::c_void,
        inherit: i32,
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut *mut std::ffi::c_void,
            size: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateDirectoryW(path: *const u16, attributes: *const SecurityAttributes) -> i32;
        fn LocalFree(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }
    if path.exists() {
        return Ok(());
    }
    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err("cannot construct private security ACL".into());
    }
    let attributes = SecurityAttributes {
        length: std::mem::size_of::<SecurityAttributes>() as u32,
        descriptor,
        inherit: 0,
    };
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe { CreateDirectoryW(path.as_ptr(), &attributes) };
    let error = std::io::Error::last_os_error();
    unsafe {
        LocalFree(descriptor);
    }
    if result != 0 || error.kind() == std::io::ErrorKind::AlreadyExists {
        Ok(())
    } else {
        Err("cannot create private security directory".into())
    }
}

#[cfg(not(any(unix, windows)))]
fn private_directory(_: &Path) -> Result<(), String> {
    Err("private security storage unsupported on this platform".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn security_seal_covers_worker_and_accounting_evidence() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        store
            .checkpoint(&ScanCommand::parse("").unwrap(), &Snapshot::default())
            .unwrap();
        store.next_generation().unwrap();
        let worker = format!("worker-{}.json", "a".repeat(64));
        store.publish(&worker, b"original worker evidence").unwrap();
        store
            .publish("request-budget-1.json", b"original accounting")
            .unwrap();
        store
            .complete(&super::super::report_contract::fixture(&id, 1, false), 1)
            .unwrap();
        assert!(store.latest_report().is_ok());
        for name in [
            &worker,
            "request-budget-1.json",
            "checkpoint.bundle.json",
            "generation-1",
        ] {
            let path = store.directory.join(name);
            let original = fs::read(&path).unwrap();
            fs::write(&path, b"changed").unwrap();
            assert!(store.latest_report().is_err(), "accepted changed {name}");
            fs::write(&path, original).unwrap();
        }
        store
            .publish("request-usage-1.json", b"late accounting")
            .unwrap();
        assert!(
            store.latest_report().is_err(),
            "accepted added evidence after seal"
        );
        fs::remove_file(store.directory.join("request-usage-1.json")).unwrap();
        fs::remove_file(store.directory.join(&worker)).unwrap();
        assert!(
            store.latest_report().is_err(),
            "accepted missing worker evidence"
        );
    }

    #[test]
    fn security_archive_rechecks_snapshot_evidence_even_with_updated_report_seal() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        fs::write(root.path().join("fixture.rs"), "fn fixture() {}\n").unwrap();
        let snapshot = Snapshot::capture(
            root.path(),
            &ScanCommand::parse("").unwrap(),
            &Default::default(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        store
            .checkpoint(&ScanCommand::parse("").unwrap(), &snapshot)
            .unwrap();
        store.next_generation().unwrap();
        let mut report = super::super::report_contract::fixture(&id, 1, false);
        report["snapshotId"] = json!(snapshot.id);
        report["coverage"] = super::super::report_contract::coverage_fixture(&snapshot, false);
        report["candidates"] = json!([super::super::report_contract::finding_fixture(
            &uuid::Uuid::new_v4().to_string()
        )]);
        report["findings"] = super::super::report_contract::project_fixture(&report);
        store.complete(&report, 1).unwrap();
        assert!(store.latest_report().is_ok());
        report["candidates"][0]["claim"]["locations"][0]["contentHash"] = json!("f".repeat(64));
        report["findings"] = super::super::report_contract::project_fixture(&report);
        let bytes = serde_json::to_vec(&report).unwrap();
        let seal_path = store.directory.join("seal-1.json");
        let mut seal: Value = serde_json::from_slice(&fs::read(&seal_path).unwrap()).unwrap();
        seal["artifacts"][0]["sha256"] = json!(super::super::sha256_hex(&bytes));
        fs::write(store.directory.join("report-1.json"), bytes).unwrap();
        fs::write(seal_path, serde_json::to_vec(&seal).unwrap()).unwrap();
        assert!(store
            .latest_report()
            .unwrap_err()
            .contains("source hash mismatch"));
    }

    #[test]
    fn security_store_rejects_incomplete_canonical_report_before_publication() {
        let agent = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        store.next_generation().unwrap();
        let value = serde_json::json!({"schemaVersion":2,"scanId":id,"generation":1,"coverageComplete":true,"findings":[]});
        assert!(store.complete(&value, 1).is_err());
        assert!(!store.directory.join("report-1.json").exists());
    }

    #[test]
    fn security_partial_reports_reopen_and_reject_corrupt_latest() {
        let agent = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        store
            .checkpoint(&ScanCommand::parse("").unwrap(), &Snapshot::default())
            .unwrap();
        store.next_generation().unwrap();
        let mut report = super::super::report_contract::fixture(&id, 1, false);
        report["checkpointSequence"] = json!(1);
        report["status"] = json!("interrupted");
        report["partial"] = json!(true);
        store.partial_report(&report).unwrap();
        assert!(store.partial_report(&report).is_err());
        drop(store);
        let store = Store::open(agent.path(), root.path(), &id, false).unwrap();
        assert_eq!(store.available_report().unwrap(), report);
        store.publish("partial-1-2.json", b"{}").unwrap();
        assert!(store.available_report().is_err());
        let final_report = super::super::report_contract::fixture(&id, 1, true);
        store.complete(&final_report, 1).unwrap();
        assert_eq!(store.available_report().unwrap(), final_report);
    }

    #[test]
    fn security_resume_requires_matching_policy_and_methodology_binding() {
        let config = super::super::config::ScanConfig::default();
        let mut checkpoint = super::Checkpoint {
            schema_version: 2,
            request: super::ScanCommand::parse("").unwrap(),
            snapshot: super::Snapshot::default(),
            binding: Some(super::CheckpointBinding::new(&config).unwrap()),
        };
        assert!(checkpoint.validate_resume(&config).is_ok());
        let changed = super::super::config::ScanConfig {
            supporting_reads: false,
            ..config.clone()
        };
        assert!(checkpoint.validate_resume(&changed).is_err());
        let mut old = serde_json::to_value(&checkpoint).unwrap();
        old["binding"]
            .as_object_mut()
            .unwrap()
            .remove("recordIdentityVersion");
        let old: super::Checkpoint = serde_json::from_value(old).unwrap();
        assert!(old
            .validate_resume(&config)
            .unwrap_err()
            .contains("record identity"));
        checkpoint.binding.as_mut().unwrap().methodology_sha256 = "0".repeat(64);
        assert!(checkpoint.validate_resume(&config).is_err());
        checkpoint.binding = None;
        assert!(checkpoint.validate_resume(&config).is_err());
    }
    use super::*;

    #[test]
    fn security_checkpoint_is_one_atomic_identity_bound_publication() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        store
            .checkpoint(&ScanCommand::parse("").unwrap(), &Snapshot::default())
            .unwrap();
        assert!(store.directory.join("checkpoint.bundle.json").is_file());
        assert!(!store.directory.join("checkpoint.sha256").exists());
        assert_eq!(store.load(1024 * 1024).unwrap().schema_version, 2);
        let path = store.directory.join("checkpoint.bundle.json");
        let mut bundle: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        bundle["scanId"] = json!(uuid::Uuid::new_v4().to_string());
        fs::write(&path, serde_json::to_vec(&bundle).unwrap()).unwrap();
        assert!(store.load(1024 * 1024).is_err());
    }

    #[test]
    fn security_checkpoint_rejects_invalid_or_unaccounted_conflict_stages() {
        use super::super::snapshot::{ConflictStage, SkippedFile};
        let valid = ConflictStage {
            path: "source.rs".into(),
            stage: 1,
            object_id: "a".repeat(40),
            mode: "100644".into(),
        };
        let make = |conflicts| Checkpoint {
            schema_version: 2,
            binding: None,
            request: ScanCommand::parse("").unwrap(),
            snapshot: Snapshot {
                conflicts,
                skipped: vec![SkippedFile {
                    path: "source.rs".into(),
                    reason: "unresolved merge; index stages require separate review".into(),
                }],
                ..Snapshot::default()
            },
        };
        assert!(Store::validate_checkpoint(make(vec![valid.clone()])).is_ok());
        for bad in [
            ConflictStage {
                stage: 0,
                ..valid.clone()
            },
            ConflictStage {
                path: "../source.rs".into(),
                ..valid.clone()
            },
            ConflictStage {
                object_id: "not-an-object".into(),
                ..valid.clone()
            },
            ConflictStage {
                mode: "777777".into(),
                ..valid.clone()
            },
        ] {
            assert!(Store::validate_checkpoint(make(vec![bad])).is_err());
        }
        assert!(Store::validate_checkpoint(make(vec![valid.clone(), valid.clone()])).is_err());
        let mut uncovered = make(vec![valid]);
        uncovered.snapshot.skipped.clear();
        assert!(Store::validate_checkpoint(uncovered).is_err());
    }

    #[test]
    fn security_checkpoint_binds_conflict_source_to_stage_and_byte_ledger() {
        use super::super::snapshot::{ConflictSource, ConflictStage, SkippedFile, SourceFile};
        let identity = ConflictStage {
            path: "source.rs".into(),
            stage: 2,
            object_id: "a".repeat(40),
            mode: "100644".into(),
        };
        let text = "ours\n";
        let source = ConflictSource {
            identity: identity.clone(),
            source: SourceFile {
                text: text.into(),
                hash: super::super::sha256_hex(text.as_bytes()),
            },
            supporting: false,
        };
        let snapshot = Snapshot {
            conflicts: vec![identity],
            conflict_sources: vec![source],
            bytes: text.len() as u64,
            skipped: vec![SkippedFile {
                path: "source.rs".into(),
                reason: "unresolved merge".into(),
            }],
            ..Snapshot::default()
        };
        let make = |snapshot| Checkpoint {
            schema_version: 2,
            binding: None,
            request: ScanCommand::parse("").unwrap(),
            snapshot,
        };
        assert!(Store::validate_checkpoint(make(snapshot.clone())).is_ok());
        for mutation in 0..7 {
            let mut bad = snapshot.clone();
            match mutation {
                0 => bad.conflicts.clear(),
                1 => bad.conflict_sources[0].identity.stage = 3,
                2 => bad.conflict_sources[0].identity.object_id = "b".repeat(40),
                3 => bad.conflict_sources[0].source.text = "changed".into(),
                4 => bad.conflict_sources.push(bad.conflict_sources[0].clone()),
                5 => bad.bytes = 0,
                _ => {
                    bad.conflicts[0].mode = "120000".into();
                    bad.conflict_sources[0].identity.mode = "120000".into();
                }
            }
            assert!(
                Store::validate_checkpoint(make(bad)).is_err(),
                "mutation {mutation}"
            );
        }
        let mut unordered = snapshot;
        let mut second = unordered.conflict_sources[0].clone();
        second.identity.stage = 1;
        unordered.conflicts.push(second.identity.clone());
        unordered.conflict_sources.push(second);
        unordered.bytes *= 2;
        assert!(Store::validate_checkpoint(make(unordered.clone())).is_err());
        unordered.conflict_sources.reverse();
        assert!(Store::validate_checkpoint(make(unordered)).is_ok());
    }

    #[test]
    fn security_checkpoint_rejects_oversized_metadata_before_publication() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let store = Store::open(
            agent.path(),
            root.path(),
            &uuid::Uuid::new_v4().to_string(),
            true,
        )
        .unwrap();
        let config = super::super::config::ScanConfig {
            max_file_bytes: 1024,
            max_policy_bytes: 1024,
            max_snapshot_bytes: 1024,
            ..Default::default()
        };
        let snapshot = Snapshot {
            skipped: (0..6000)
                .map(|index| super::super::snapshot::SkippedFile {
                    path: format!("build/{index:05}-{}.rs", "x".repeat(180)),
                    reason: "excluded directory".into(),
                })
                .collect(),
            ..Snapshot::default()
        };
        config.validate_snapshot(&snapshot).unwrap();
        assert!(store
            .checkpoint_bound(&ScanCommand::parse("").unwrap(), &snapshot, &config)
            .is_err());
        assert!(!store.directory.join("checkpoint.bundle.json").exists());
        // Failed serialization has not consumed the immutable publication slot.
        store
            .checkpoint_bound(
                &ScanCommand::parse("").unwrap(),
                &Snapshot::default(),
                &config,
            )
            .unwrap();
        assert!(store.load(1024 * 1024 + 6 * 1024).is_ok());
    }

    #[test]
    fn security_checkpoint_limit_includes_bundle_identity_and_checksum() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let store = Store::open(
            agent.path(),
            root.path(),
            &uuid::Uuid::new_v4().to_string(),
            true,
        )
        .unwrap();
        let config = super::super::config::ScanConfig {
            max_file_bytes: 1024,
            max_policy_bytes: 1024,
            max_snapshot_bytes: 1024,
            ..Default::default()
        };
        let mut checkpoint = Checkpoint {
            schema_version: 2,
            binding: Some(CheckpointBinding::new(&config).unwrap()),
            request: ScanCommand::parse("").unwrap(),
            snapshot: Snapshot {
                skipped: vec![super::super::snapshot::SkippedFile {
                    path: "build".into(),
                    reason: String::new(),
                }],
                ..Snapshot::default()
            },
        };
        let limit = config.checkpoint_byte_limit();
        let overhead = serde_json::to_vec(&checkpoint).unwrap().len();
        checkpoint.snapshot.skipped[0].reason = "x".repeat(limit as usize - overhead);
        assert_eq!(
            super::super::checkpoint_encoding::encode(&checkpoint, limit)
                .unwrap()
                .len() as u64,
            limit
        );
        assert_eq!(
            store
                .checkpoint_bound(&checkpoint.request, &checkpoint.snapshot, &config)
                .unwrap_err(),
            "security checkpoint exceeds encoded byte limit"
        );
        assert!(!store.directory.join("checkpoint.bundle.json").exists());
        checkpoint.snapshot.skipped[0]
            .reason
            .truncate(limit as usize - overhead - 512);
        store
            .checkpoint_bound(&checkpoint.request, &checkpoint.snapshot, &config)
            .unwrap();
        let reopened = store.load(limit).unwrap();
        reopened.validate_resume(&config).unwrap();
        assert_eq!(
            reopened.snapshot.skipped[0].reason,
            checkpoint.snapshot.skipped[0].reason
        );
    }

    #[test]
    fn security_checkpoint_legacy_read_and_corrupt_bundle_downgrade_guard() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let store = Store::open(
            agent.path(),
            root.path(),
            &uuid::Uuid::new_v4().to_string(),
            true,
        )
        .unwrap();
        let checkpoint = Checkpoint {
            schema_version: 2,
            binding: None,
            request: ScanCommand::parse("").unwrap(),
            snapshot: Snapshot::default(),
        };
        let bytes = serde_json::to_vec(&checkpoint).unwrap();
        store.publish("checkpoint.json", &bytes).unwrap();
        assert!(
            store.load(1024 * 1024).is_err(),
            "torn legacy pair must fail closed"
        );
        store
            .publish(
                "checkpoint.sha256",
                super::super::sha256_hex(&bytes).as_bytes(),
            )
            .unwrap();
        assert_eq!(store.load(1024 * 1024).unwrap().schema_version, 2);
        assert!(store
            .checkpoint(&checkpoint.request, &checkpoint.snapshot)
            .is_err());
        // An interrupted temporary write has no authority, but a published
        // corrupt bundle must never silently fall back to the older format.
        fs::write(store.directory.join(".interrupted.tmp"), b"partial").unwrap();
        assert!(store.load(1024 * 1024).is_ok());
        fs::write(store.directory.join("checkpoint.bundle.json"), b"{}").unwrap();
        assert!(store.load(1024 * 1024).is_err());
    }

    #[test]
    fn security_publication_failure_cleans_only_its_own_temporary_file() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let store = Store::open(
            agent.path(),
            root.path(),
            &uuid::Uuid::new_v4().to_string(),
            true,
        )
        .unwrap();
        let existing = store.directory.join(".older.tmp");
        fs::write(&existing, b"keep").unwrap();
        assert!(store
            .publish("missing-parent/artifact", b"fixture")
            .is_err());
        let temps: Vec<_> = fs::read_dir(&store.directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "tmp"))
            .collect();
        assert_eq!(temps, vec![existing]);
    }
    #[test]
    fn security_checkpoint_recounts_source_bytes_before_resume() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source.rs"), "fn main() {}\n").unwrap();
        let request = ScanCommand::parse("").unwrap();
        let mut snapshot =
            Snapshot::capture(root.path(), &request, &super::super::ScanConfig::default()).unwrap();
        snapshot.bytes = 0;
        let store = Store::open(
            agent.path(),
            root.path(),
            &uuid::Uuid::new_v4().to_string(),
            true,
        )
        .unwrap();
        store.checkpoint(&request, &snapshot).unwrap();
        assert!(store
            .load(1024 * 1024)
            .unwrap_err()
            .contains("byte count mismatch"));
    }
    #[test]
    fn security_store_rejects_mismatched_seal_generation() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        store
            .checkpoint(&ScanCommand::parse("").unwrap(), &Snapshot::default())
            .unwrap();
        store.next_generation().unwrap();
        let report = super::super::report_contract::fixture(&id, 1, false);
        store.complete(&report, 1).unwrap();
        assert!(store.latest_report().is_ok());
        let path = store.directory.join("seal-1.json");
        let mut seal: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        seal["generation"] = json!(2);
        fs::write(&path, serde_json::to_vec(&seal).unwrap()).unwrap();
        assert!(store.latest_report().is_err());
    }
    #[test]
    fn security_store_rejects_cross_run_reports_even_with_matching_hashes() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        store
            .checkpoint(&ScanCommand::parse("").unwrap(), &Snapshot::default())
            .unwrap();
        let report = super::super::report_contract::fixture(&id, 1, false);
        assert!(
            store.complete(&report, 1).is_err(),
            "generation must be reserved"
        );
        assert!(!store.directory.join("report-1.json").exists());
        store.next_generation().unwrap();
        let mut wrong = report.clone();
        wrong["scanId"] = json!(uuid::Uuid::new_v4().to_string());
        assert!(store.complete(&wrong, 1).is_err());
        assert!(!store.directory.join("report-1.json").exists());
        store.complete(&report, 1).unwrap();
        let seal_path = store.directory.join("seal-1.json");
        let original: Value = serde_json::from_slice(&fs::read(&seal_path).unwrap()).unwrap();
        for (key, replacement) in [
            ("scanId", json!(uuid::Uuid::new_v4().to_string())),
            ("generation", json!(2)),
            ("schemaVersion", json!(3)),
        ] {
            let mut changed = report.clone();
            changed[key] = replacement;
            let bytes = serde_json::to_vec(&changed).unwrap();
            let mut seal = original.clone();
            seal["artifacts"][0]["sha256"] = json!(super::super::sha256_hex(&bytes));
            fs::write(store.directory.join("report-1.json"), bytes).unwrap();
            fs::write(&seal_path, serde_json::to_vec(&seal).unwrap()).unwrap();
            assert!(store.latest_report().is_err(), "accepted wrong {key}");
        }
    }
    #[test]
    fn security_store_excludes_target_locks_run_and_detects_tampering() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        assert!(Store::open(root.path(), root.path(), &id, true).is_err());
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        assert!(Store::open(agent.path(), root.path(), &id, false).is_err());
        let request = ScanCommand::parse("").unwrap();
        store.checkpoint(&request, &Snapshot::default()).unwrap();
        assert_eq!(store.load(1024 * 1024).unwrap().schema_version, 2);
        assert_eq!(store.next_generation().unwrap(), 1);
        assert_eq!(store.next_generation().unwrap(), 2);
        let report = super::super::report_contract::fixture(&id, 1, false);
        store.complete(&report, 1).unwrap();
        assert!(store.complete(&report, 1).is_err());
        let path = store.directory.join("checkpoint.bundle.json");
        let mut bundle: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        bundle["sha256"] = json!("0".repeat(64));
        fs::write(path, serde_json::to_vec(&bundle).unwrap()).unwrap();
        assert!(store
            .load(1024 * 1024)
            .unwrap_err()
            .contains("hash mismatch"));
    }

    #[test]
    fn security_legacy_v1_report_is_read_only_and_not_resume_or_v2() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        store
            .publish(
                "findings.json",
                br#"{"schemaVersion":1,"validated":true,"findings":[{"severity":"critical","title":"old"}]}"#,
            )
            .unwrap();
        let view = store.available_report().unwrap();
        assert_eq!(view["schemaVersion"], 1);
        assert_eq!(view["readOnly"], true);
        assert_eq!(view["coverageComplete"], false);
        assert_eq!(view["legacyValidatedFlag"], true);
        assert_eq!(super::super::report::exit_code(&view), 2);
        assert!(super::super::report::select_finding(view, Some("old")).is_err());
        assert!(store.load(1024 * 1024).is_err());
        assert!(!store.directory.join("report-1.json").exists());
        assert!(!store.has_sealed_report());
    }

    #[test]
    fn security_legacy_v1_feedback_and_report_stay_outside_resume() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        assert!(store.load_feedback().unwrap().is_empty());
        let parent = store.directory.parent().unwrap();
        std::fs::write(
            parent.join("feedback.json"),
            br#"[{"fingerprint":"root-1","controlPath":"a.rs","controlHash":"abcd","snapshotSide":"worktree"}]"#,
        )
        .unwrap();
        let records = store.load_feedback().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].fingerprint, "root-1");
        assert!(store.load(1024 * 1024).is_err());
        assert!(!store.has_sealed_report());
    }

    #[test]
    fn security_store_cannot_escape_through_existing_link() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let link = agent.path().join("security-scans");
        #[cfg(windows)]
        let planted = std::os::windows::fs::symlink_dir(target.path(), &link).is_ok()
            || std::process::Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    &link.to_string_lossy(),
                    &target.path().to_string_lossy(),
                ])
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
        #[cfg(unix)]
        let planted = std::os::unix::fs::symlink(target.path(), &link).is_ok();
        #[cfg(not(any(windows, unix)))]
        let planted = false;
        assert!(planted, "could not plant a directory link for confinement");
        let id = uuid::Uuid::new_v4().to_string();
        assert!(Store::open(agent.path(), root.path(), &id, true).is_err());
        assert!(target.path().read_dir().unwrap().next().is_none());
    }

    #[test]
    fn security_store_detects_tampering_and_torn_checkpoint() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        store
            .checkpoint(&ScanCommand::parse("").unwrap(), &Snapshot::default())
            .unwrap();
        let path = store.directory.join("checkpoint.bundle.json");
        let original = fs::read(&path).unwrap();
        fs::write(&path, &original[..original.len() / 2]).unwrap();
        assert!(store.load(1024 * 1024).is_err());
        fs::write(&path, original).unwrap();
        let mut bundle: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        bundle["sha256"] = json!("0".repeat(64));
        fs::write(&path, serde_json::to_vec(&bundle).unwrap()).unwrap();
        assert!(store
            .load(1024 * 1024)
            .unwrap_err()
            .contains("hash mismatch"));
    }

    #[test]
    fn security_scan_shutdown_leaves_recoverable_checkpoint() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        store
            .checkpoint(&ScanCommand::parse("").unwrap(), &Snapshot::default())
            .unwrap();
        drop(store);
        let reopened = Store::open(agent.path(), root.path(), &id, false).unwrap();
        let checkpoint = reopened.load(1024 * 1024).unwrap();
        assert_eq!(checkpoint.schema_version, 2);
        checkpoint
            .validate_resume(&super::super::config::ScanConfig::default())
            .unwrap();
        assert!(!reopened.has_sealed_report());
    }
}
