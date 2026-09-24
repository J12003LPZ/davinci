//! Optional host-owned cargo-audit adapter. Advisory output is a signal, not exploitability.

use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

#[cfg_attr(not(test), allow(dead_code))]
pub const SUPPORTED_CARGO_AUDIT: &str = "0.22";
#[cfg_attr(not(test), allow(dead_code))]
pub const CARGO_AUDIT_LICENSE: &str = "MIT OR Apache-2.0";
const MAX_OUTPUT: u64 = 2 * 1024 * 1024;
const MAX_DB_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Debug, Clone)]
pub struct CargoAuditPlan {
    pub executable: PathBuf,
    pub database: PathBuf,
    pub timeout: Duration,
    pub max_output: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisorySignal {
    pub id: String,
    pub package: String,
    pub version: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalyzerOutcome {
    Signals(Vec<AdvisorySignal>),
    Limitation(String),
}

impl AnalyzerOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Signals(_))
    }
}

pub fn host_plan_from_env() -> Option<Result<CargoAuditPlan, String>> {
    let executable = std::env::var_os("DAVINCI_SECURITY_CARGO_AUDIT")?;
    let database = std::env::var_os("DAVINCI_SECURITY_ADVISORY_DB")?;
    Some(CargoAuditPlan::new(
        PathBuf::from(executable),
        PathBuf::from(database),
    ))
}

impl CargoAuditPlan {
    pub fn new(executable: PathBuf, database: PathBuf) -> Result<Self, String> {
        Ok(Self {
            executable,
            database,
            timeout: Duration::from_secs(20),
            max_output: MAX_OUTPUT,
        })
    }
}

pub fn analyzer_checks(
    config: &super::config::ScanConfig,
    snapshot_root: &Path,
    snapshot: &super::snapshot::Snapshot,
    cancelled: &dyn Fn() -> bool,
) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"name":"runtime-reproduction","status":"not_attempted"}),
        cargo_audit_check(config, snapshot_root, snapshot, cancelled),
    ]
}

fn cargo_audit_check(
    config: &super::config::ScanConfig,
    snapshot_root: &Path,
    snapshot: &super::snapshot::Snapshot,
    cancelled: &dyn Fn() -> bool,
) -> serde_json::Value {
    if !config.analyzers.iter().any(|name| name == "cargo-audit") {
        return serde_json::json!({"name":"external-analyzers","status":"disabled"});
    }
    let Some(lockfile) = snapshot.files.get("Cargo.lock") else {
        return limitation("snapshot has no Cargo.lock");
    };
    let plan = match host_plan_from_env() {
        None => return limitation("cargo-audit executable is not provisioned"),
        Some(Err(reason)) => return limitation(&reason),
        Some(Ok(plan)) => plan,
    };
    let isolated = match IsolatedDir::create() {
        Ok(dir) => dir,
        Err(reason) => return limitation(&reason),
    };
    let staged = isolated.0.join("Cargo.lock");
    if std::fs::write(&staged, &lockfile.text).is_err() {
        return limitation("cannot stage snapshot lockfile");
    }
    let outcome = audit_lockfile(&plan, snapshot_root, &staged, cancelled);
    if !outcome.is_success() {
        return match outcome {
            AnalyzerOutcome::Limitation(reason) => limitation(&reason),
            AnalyzerOutcome::Signals(_) => limitation("cargo-audit failed"),
        };
    }
    match outcome {
        AnalyzerOutcome::Signals(signals) => serde_json::json!({
            "name": "external-analyzers",
            "status": "completed",
            "signals": signals.iter().map(|signal| serde_json::json!({
                "id": signal.id,
                "package": signal.package,
                "version": signal.version,
                "title": signal.title,
            })).collect::<Vec<_>>(),
        }),
        AnalyzerOutcome::Limitation(reason) => limitation(&reason),
    }
}

fn limitation(reason: &str) -> serde_json::Value {
    serde_json::json!({
        "name": "external-analyzers",
        "status": "limitation",
        "reason": reason,
    })
}

pub fn audit_lockfile(
    plan: &CargoAuditPlan,
    snapshot_root: &Path,
    lockfile: &Path,
    cancelled: &dyn Fn() -> bool,
) -> AnalyzerOutcome {
    match run_audit(plan, snapshot_root, lockfile, cancelled) {
        Ok(signals) => AnalyzerOutcome::Signals(signals),
        Err(message) => AnalyzerOutcome::Limitation(message),
    }
}

fn run_audit(
    plan: &CargoAuditPlan,
    snapshot_root: &Path,
    lockfile: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<AdvisorySignal>, String> {
    if cancelled() {
        return Err("cargo-audit cancelled".into());
    }
    let executable = provisioned_executable(&plan.executable, snapshot_root)?;
    let database = provisioned_database(&plan.database)?;
    if !lockfile.is_file() {
        return Err("snapshot lockfile is missing".into());
    }
    let isolated = IsolatedDir::create()?;
    let staged_lock = isolated.0.join("Cargo.lock");
    std::fs::copy(lockfile, &staged_lock).map_err(|_| "cannot stage snapshot lockfile")?;
    let args = fixed_argv(&database, &staged_lock);
    let bytes = run_bounded(
        &executable,
        &args,
        &isolated.0,
        plan.timeout,
        plan.max_output,
        cancelled,
    )?;
    parse_signals(&bytes)
}

struct IsolatedDir(PathBuf);

impl IsolatedDir {
    fn create() -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!("davinci-audit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).map_err(|_| "cannot isolate cargo-audit")?;
        Ok(Self(dir))
    }
}

impl Drop for IsolatedDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn provisioned_executable(path: &Path, snapshot_root: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || !path.is_file() {
        return Err("cargo-audit executable is not provisioned".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "bat" | "cmd" | "ps1" | "sh") {
        return Err("cargo-audit executable must not be a shell script".into());
    }
    let executable = path
        .canonicalize()
        .map_err(|_| "cannot resolve cargo-audit executable")?;
    let snapshot = snapshot_root
        .canonicalize()
        .unwrap_or_else(|_| snapshot_root.to_path_buf());
    if executable.starts_with(&snapshot) {
        return Err("repository-owned cargo-audit executable denied".into());
    }
    Ok(executable)
}

fn provisioned_database(path: &Path) -> Result<PathBuf, String> {
    let git_head = path.join(".git").join("HEAD");
    if !path.is_absolute() || !path.is_dir() || !path.join("crates").is_dir() || !git_head.is_file()
    {
        return Err("advisory database is missing".into());
    }
    let modified = std::fs::metadata(&git_head)
        .and_then(|metadata| metadata.modified())
        .map_err(|_| "advisory database is missing")?;
    if SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO)
        > MAX_DB_AGE
    {
        return Err("advisory database is stale".into());
    }
    path.canonicalize()
        .map_err(|_| "cannot resolve advisory database".into())
}

fn fixed_argv(database: &Path, lockfile: &Path) -> Vec<OsString> {
    vec![
        "audit".into(),
        "--json".into(),
        "--no-fetch".into(),
        "--no-yanked".into(),
        "--color".into(),
        "never".into(),
        "--quiet".into(),
        "--db".into(),
        database.as_os_str().to_os_string(),
        "--file".into(),
        lockfile.as_os_str().to_os_string(),
    ]
}

fn run_bounded(
    executable: &Path,
    args: &[OsString],
    cwd: &Path,
    timeout: Duration,
    max_output: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, String> {
    if cancelled() {
        return Err("cargo-audit cancelled".into());
    }
    let mut command = Command::new(executable);
    command
        .current_dir(cwd)
        .env_clear()
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TERM_COLOR", "never")
        .env("PATH", "")
        .args(args);
    if let Some(value) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", value);
    }
    if let Some(value) = std::env::var_os("WINDIR") {
        command.env("WINDIR", value);
    }
    let output = davinci_sys::process::run_bounded(
        command,
        None,
        davinci_sys::process::RunLimits {
            timeout,
            output_cap: max_output as usize,
        },
        cancelled,
    )
    .map_err(|_| "cannot start provisioned cargo-audit".to_string())?;
    if output.cancelled {
        return Err("cargo-audit cancelled".into());
    }
    if output.timed_out {
        return Err("cargo-audit timed out".into());
    }
    if output.stdout_truncated {
        return Err("cargo-audit output exceeded bound".into());
    }
    let status = output
        .status
        .ok_or_else(|| "cargo-audit failed".to_string())?;
    if status.code() == Some(2) || (!status.success() && status.code() != Some(1)) {
        return Err("cargo-audit failed".into());
    }
    Ok(output.stdout)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::SystemTime;

    fn stub_source() -> &'static str {
        r#"
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::env::current_exe().ok().and_then(|path| path.parent().map(|path| path.to_path_buf()));
    if let Some(dir) = &dir {
        let _ = std::fs::write(dir.join("argv.txt"), args.join("\n"));
    }
    if args.iter().any(|arg| arg == "--version") {
        print!("cargo-audit 0.22.2");
        return;
    }
    if let Some(dir) = &dir {
        if let Ok(ms) = std::fs::read_to_string(dir.join("sleep_ms.txt")) {
            let ms: u64 = ms.trim().parse().unwrap_or(0);
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
        if let Ok(payload) = std::fs::read_to_string(dir.join("payload.json")) {
            print!("{payload}");
            return;
        }
    }
    print!("{{\"vulnerabilities\":{{\"list\":[]}}}}");
}
"#
    }

    fn compile_stub(dir: &Path) -> PathBuf {
        let src = dir.join("stub.rs");
        fs::write(&src, stub_source()).unwrap();
        let exe = dir.join(if cfg!(windows) { "stub.exe" } else { "stub" });
        let status = Command::new("rustc")
            .args(["--edition", "2021", "-o"])
            .arg(&exe)
            .arg(&src)
            .status()
            .expect("rustc available for analyzer stub");
        assert!(status.success(), "rustc failed to build analyzer stub");
        exe
    }

    fn database(dir: &Path) -> PathBuf {
        let db = dir.join("advisory-db");
        fs::create_dir_all(db.join("crates")).unwrap();
        fs::create_dir_all(db.join(".git")).unwrap();
        fs::write(db.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        db
    }

    fn lockfile(dir: &Path) -> PathBuf {
        let path = dir.join("Cargo.lock");
        fs::write(&path, "version = 3\n").unwrap();
        path
    }

    fn plan(dir: &Path) -> (CargoAuditPlan, PathBuf, PathBuf) {
        let host = dir.join("host");
        fs::create_dir_all(&host).unwrap();
        let exe = compile_stub(&host);
        let db = database(dir);
        let snapshot = dir.join("snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        let lock = lockfile(&snapshot);
        (
            CargoAuditPlan {
                executable: exe,
                database: db,
                timeout: Duration::from_secs(5),
                max_output: MAX_OUTPUT,
            },
            snapshot,
            lock,
        )
    }

    #[test]
    fn security_analyzer_checks_stay_disabled_without_host_plan() {
        let checks = analyzer_checks(
            &super::super::config::ScanConfig::default(),
            Path::new("."),
            &super::super::snapshot::Snapshot::default(),
            &|| false,
        );
        assert_eq!(
            serde_json::Value::Array(checks),
            serde_json::json!([
                {"name":"runtime-reproduction","status":"not_attempted"},
                {"name":"external-analyzers","status":"disabled"}
            ])
        );
        let enabled = super::super::config::ScanConfig {
            analyzers: vec!["cargo-audit".into()],
            ..Default::default()
        };
        let limited = analyzer_checks(
            &enabled,
            Path::new("."),
            &super::super::snapshot::Snapshot::default(),
            &|| false,
        );
        assert_eq!(limited[1]["status"], "limitation");
        assert_ne!(limited[1]["status"], "completed");
    }

    #[test]
    fn security_analyzer_never_auto_installs_or_updates() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = dir.path().join("snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        let lock = lockfile(&snapshot);
        let plan = CargoAuditPlan {
            executable: dir.path().join("missing-cargo-audit"),
            database: database(dir.path()),
            timeout: Duration::from_secs(1),
            max_output: MAX_OUTPUT,
        };
        let outcome = audit_lockfile(&plan, &snapshot, &lock, &|| false);
        assert_eq!(
            outcome,
            AnalyzerOutcome::Limitation("cargo-audit executable is not provisioned".into())
        );
        assert!(!dir.path().join("cargo-audit").exists());
        assert!(!dir.path().join("cargo-audit.exe").exists());
    }

    #[test]
    fn security_analyzer_does_not_use_repository_path_executable() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = dir.path().join("snapshot");
        fs::create_dir_all(&snapshot).unwrap();
        let exe = compile_stub(&snapshot);
        let lock = lockfile(&snapshot);
        let plan = CargoAuditPlan {
            executable: exe,
            database: database(dir.path()),
            timeout: Duration::from_secs(5),
            max_output: MAX_OUTPUT,
        };
        let outcome = audit_lockfile(&plan, &snapshot, &lock, &|| false);
        assert_eq!(
            outcome,
            AnalyzerOutcome::Limitation("repository-owned cargo-audit executable denied".into())
        );
    }

    #[test]
    fn security_analyzer_argv_is_not_shell_interpreted() {
        let dir = tempfile::tempdir().unwrap();
        let (plan, snapshot, _lock) = plan(dir.path());
        let hostile = snapshot.join("Cargo.lock; echo pwned");
        fs::write(&hostile, "version = 3\n").unwrap();
        let outcome = audit_lockfile(&plan, &snapshot, &hostile, &|| false);
        assert!(outcome.is_success(), "{outcome:?}");
        let recorded =
            fs::read_to_string(plan.executable.parent().unwrap().join("argv.txt")).unwrap();
        assert!(recorded.contains("audit"));
        assert!(recorded.contains("--no-fetch"));
        assert!(!recorded.contains("--url"));
        assert!(!recorded.contains("--stale"));
        assert!(!recorded.contains("fix"));
        assert!(!dir.path().join("pwned").exists());
    }

    #[test]
    fn security_analyzer_denied_network_stays_denied() {
        let dir = tempfile::tempdir().unwrap();
        let (plan, snapshot, lock) = plan(dir.path());
        let outcome = audit_lockfile(&plan, &snapshot, &lock, &|| false);
        assert!(outcome.is_success(), "{outcome:?}");
        let recorded =
            fs::read_to_string(plan.executable.parent().unwrap().join("argv.txt")).unwrap();
        assert!(recorded.contains("--no-fetch"));
        assert!(!recorded.contains("--url"));
        assert!(!recorded.contains("127.0.0.1"));
    }

    #[test]
    fn security_analyzer_missing_or_stale_database_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (mut plan, snapshot, lock) = plan(dir.path());
        plan.database = dir.path().join("missing-db");
        assert_eq!(
            audit_lockfile(&plan, &snapshot, &lock, &|| false),
            AnalyzerOutcome::Limitation("advisory database is missing".into())
        );
        let stale = dir.path().join("stale-db");
        fs::create_dir_all(stale.join("crates")).unwrap();
        fs::create_dir_all(stale.join(".git")).unwrap();
        let head = stale.join(".git").join("HEAD");
        fs::write(&head, "ref: refs/heads/main\n").unwrap();
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        fs::File::options()
            .write(true)
            .open(&head)
            .unwrap()
            .set_modified(old)
            .unwrap();
        plan.database = stale;
        assert_eq!(
            audit_lockfile(&plan, &snapshot, &lock, &|| false),
            AnalyzerOutcome::Limitation("advisory database is stale".into())
        );
    }

    #[test]
    fn security_analyzer_timeout_or_invalid_json_is_not_pass() {
        let dir = tempfile::tempdir().unwrap();
        let (mut plan, snapshot, lock) = plan(dir.path());
        fs::write(
            plan.executable.parent().unwrap().join("sleep_ms.txt"),
            "2000",
        )
        .unwrap();
        plan.timeout = Duration::from_millis(50);
        let timeout = audit_lockfile(&plan, &snapshot, &lock, &|| false);
        assert_eq!(
            timeout,
            AnalyzerOutcome::Limitation("cargo-audit timed out".into())
        );

        fs::remove_file(plan.executable.parent().unwrap().join("sleep_ms.txt")).unwrap();
        fs::write(
            plan.executable.parent().unwrap().join("payload.json"),
            "not-json",
        )
        .unwrap();
        plan.timeout = Duration::from_secs(5);
        let invalid = audit_lockfile(&plan, &snapshot, &lock, &|| false);
        assert_eq!(
            invalid,
            AnalyzerOutcome::Limitation("cargo-audit returned invalid JSON".into())
        );
    }

    #[test]
    fn security_advisory_signal_does_not_invent_exploitability() {
        let dir = tempfile::tempdir().unwrap();
        let (plan, snapshot, lock) = plan(dir.path());
        fs::write(
            plan.executable.parent().unwrap().join("payload.json"),
            r#"{"vulnerabilities":{"list":[{"advisory":{"id":"RUSTSEC-2020-0071","title":"soundness","exploitable":true},"package":{"name":"time","version":"0.1.0"},"cvss":{"exploitability":9.8}}]}}"#,
        )
        .unwrap();
        let outcome = audit_lockfile(&plan, &snapshot, &lock, &|| false);
        match outcome {
            AnalyzerOutcome::Signals(signals) => {
                assert_eq!(signals[0].id, "RUSTSEC-2020-0071");
                assert_eq!(signals[0].package, "time");
                let encoded = format!("{signals:?}");
                assert!(!encoded.contains("exploit"));
                assert!(!encoded.contains("confirmed"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(CARGO_AUDIT_LICENSE, "MIT OR Apache-2.0");
        assert_eq!(SUPPORTED_CARGO_AUDIT, "0.22");
    }
}
