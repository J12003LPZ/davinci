use super::model::{
    VerificationPlan, VerificationPlanArgs, VerificationPlannerConfig, VerificationRequirement,
    VerificationStep,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_FILE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManager {
    Cargo,
    Npm,
    Pnpm,
    Yarn,
    Bun,
    Python,
    Generic,
}

impl PackageManager {
    fn command(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
            Self::Python => "python",
            Self::Generic => "git",
        }
    }
}

#[derive(Debug, Default)]
struct Classification {
    labels: BTreeSet<String>,
    frontend: bool,
    source: bool,
    tests: bool,
    docs_only: bool,
    security: bool,
    dependency: bool,
    process: bool,
    public_api: bool,
    ci: bool,
    browser: bool,
}

impl Classification {
    fn new() -> Self {
        Self {
            docs_only: true,
            ..Self::default()
        }
    }

    fn label(&mut self, label: &str) {
        self.labels.insert(label.to_string());
    }
}

pub fn build_plan(
    root: &Path,
    args: &VerificationPlanArgs,
    transaction_files: Option<Vec<String>>,
    transaction_identity: Option<String>,
    config: &VerificationPlannerConfig,
) -> Result<VerificationPlan, String> {
    let config = config.clone().bounded();
    let mut paths = args.files.clone();
    if let Some(path) = &args.path {
        paths.push(path.clone());
    }
    if let Some(transaction_files) = transaction_files {
        if !paths.is_empty() {
            return Err("transactionId cannot be combined with files or path".into());
        }
        paths = transaction_files;
    }
    if paths.is_empty() {
        return Err("verification_plan: files, path, or transactionId required".into());
    }
    if paths.len() > config.max_files {
        return Err(format!(
            "verification_plan: at most {} files are supported",
            config.max_files
        ));
    }

    let mut normalized = paths
        .into_iter()
        .map(|path| normalize_path(&path))
        .collect::<Result<Vec<_>, _>>()?;
    normalized.sort();
    normalized.dedup();

    let source_identity = source_identity(root, &normalized, transaction_identity.as_deref());
    let manager = detect_package_manager(root);
    let mut classification = Classification::new();
    let mut warnings = Vec::new();
    for path in &normalized {
        classify_path(root, path, &mut classification, &mut warnings);
    }

    if args.force_full {
        classification.label("full-requested");
    }
    if args.ci_required == Some(true) {
        classification.ci = true;
        classification.label("ci-required-by-request");
    }
    for requirement in &args.user_requirements {
        let lower = requirement.to_ascii_lowercase();
        if lower.contains("security")
            || lower.contains("auth")
            || lower.contains("permission")
            || lower.contains("credential")
        {
            classification.security = true;
            classification.label("security-required-by-request");
        }
        if lower.contains("browser") || lower.contains("ui") {
            classification.browser = true;
            classification.label("browser-required-by-request");
        }
        if lower.contains("ci") || lower.contains("continuous integration") {
            classification.ci = true;
            classification.label("ci-required-by-request");
        }
    }
    if args.force_full {
        classification.source = true;
        classification.tests = true;
        classification.dependency = true;
        classification.public_api = true;
        classification.security = true;
        classification.ci = true;
    }
    classification.docs_only = !classification.source
        && !classification.frontend
        && !classification.tests
        && !classification.dependency
        && !classification.security
        && !classification.process
        && !classification.public_api
        && !classification.ci
        && !classification.browser;

    let mut requirements = Vec::new();
    let mut steps = Vec::new();
    if classification.docs_only {
        classification.label("documentation-only");
        warnings.push(
            "documentation-only change: no process, package, browser, or CI check was activated"
                .into(),
        );
    } else {
        add_step(
            &mut steps,
            &mut requirements,
            StepSpec::diff_check(root, &source_identity),
        );
        if classification.source || classification.frontend || classification.tests {
            add_requirement(
                &mut requirements,
                "tests",
                "source or test files changed",
                "test_receipt",
                true,
            );
            add_step(
                &mut steps,
                &mut requirements,
                StepSpec::tests(manager, root, &source_identity),
            );
        }
        if classification.frontend || classification.source || classification.public_api {
            if has_script(root, manager, "typecheck") || matches!(manager, PackageManager::Cargo) {
                add_requirement(
                    &mut requirements,
                    "typecheck",
                    "typed or compiled source changed",
                    "process_receipt",
                    true,
                );
                add_step(
                    &mut steps,
                    &mut requirements,
                    StepSpec::typecheck(manager, root, &source_identity),
                );
            }
            if has_script(root, manager, "lint") {
                add_requirement(
                    &mut requirements,
                    "lint",
                    "repository lint script is available for changed source",
                    "process_receipt",
                    true,
                );
                add_step(
                    &mut steps,
                    &mut requirements,
                    StepSpec::lint(manager, root, &source_identity),
                );
            }
        }
        if classification.dependency || classification.public_api || args.force_full {
            add_requirement(
                &mut requirements,
                "build",
                "dependency, public API, or full verification change",
                "process_receipt",
                true,
            );
            add_step(
                &mut steps,
                &mut requirements,
                StepSpec::build(manager, root, &source_identity),
            );
            if matches!(manager, PackageManager::Generic) {
                warnings.push(
                    "no supported package manager was detected; build verification requires an external process receipt".into(),
                );
            }
        }
        if classification.browser {
            add_requirement(
                &mut requirements,
                "browser",
                "frontend or explicit UI/browser requirement",
                "browser_receipt",
                true,
            );
            add_step(
                &mut steps,
                &mut requirements,
                StepSpec::browser(root, &source_identity),
            );
        }
        if classification.security || classification.process {
            add_requirement(
                &mut requirements,
                "security",
                "security-sensitive, process, auth, or permission surface changed",
                "security_receipt",
                true,
            );
            add_step(
                &mut steps,
                &mut requirements,
                StepSpec::security(root, &source_identity),
            );
        }
        if classification.ci {
            add_requirement(
                &mut requirements,
                "ci",
                "CI configuration or explicit CI requirement changed",
                "ci_receipt",
                true,
            );
            add_step(
                &mut steps,
                &mut requirements,
                StepSpec::ci(root, &source_identity),
            );
            warnings.push(
                "CI parity is required; declarative job matching is planned but CI execution remains external".into(),
            );
        }
    }

    if args.force_full {
        warnings.push(
            "forceFull requested: every available verification dimension is required; this plan does not claim that any check passed".into(),
        );
    }
    if !args.changed_symbols.is_empty() {
        requirements.push(VerificationRequirement {
            kind: "symbol-impact".into(),
            reason: "changed symbols require current impact evidence before completion".into(),
            evidence_kind: "impact_receipt".into(),
            required: true,
        });
    }
    if transaction_identity.is_some() {
        warnings.push(
            "transaction source identity is included; any transaction or source change invalidates this plan".into(),
        );
    }

    steps.sort_by_key(|step| (step.tier, step.id.clone()));
    steps.truncate(config.max_steps);
    if steps.len() == config.max_steps {
        warnings.push("verification steps were bounded by configuration".into());
    }
    let mut previous = None;
    for step in &mut steps {
        if let Some(previous) = previous.replace(step.id.clone()) {
            step.depends_on.push(previous);
        }
    }
    let partial = !warnings.is_empty()
        || normalized
            .iter()
            .any(|path| !root.join(path).is_file() && !root.join(path).is_dir());
    Ok(VerificationPlan {
        schema_version: 1,
        enabled: true,
        source_identity,
        changed_files: normalized,
        changed_symbols: args.changed_symbols.clone(),
        classification: classification.labels.into_iter().collect(),
        requirements,
        steps,
        warnings,
        partial,
        complete: false,
        telemetry: Default::default(),
    })
}

fn normalize_path(path: &str) -> Result<String, String> {
    if path.is_empty() || path.len() > 4096 || path.contains('\0') {
        return Err("verification path is empty, oversized, or contains NUL".into());
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Err(format!("verification path is absolute: {path}"));
    }
    if candidate.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!("verification path escapes the workspace: {path}"));
    }
    Ok(path.replace('\\', "/"))
}

fn source_identity(root: &Path, paths: &[String], transaction_identity: Option<&str>) -> String {
    let mut digest = Sha256::new();
    for path in paths {
        digest.update(path.as_bytes());
        digest.update([0]);
        match fs::read(root.join(path)) {
            Ok(bytes) => digest.update(bytes),
            Err(_) => digest.update(b"<missing>"),
        }
        digest.update([0xff]);
    }
    if let Some(identity) = transaction_identity {
        digest.update(b"transaction:");
        digest.update(identity.as_bytes());
    }
    format!("sha256:{:x}", digest.finalize())
}

fn detect_package_manager(root: &Path) -> PackageManager {
    if root.join("Cargo.toml").is_file() {
        return PackageManager::Cargo;
    }
    if root.join("pnpm-lock.yaml").is_file() {
        return PackageManager::Pnpm;
    }
    if root.join("yarn.lock").is_file() {
        return PackageManager::Yarn;
    }
    if root.join("bun.lock").is_file() || root.join("bun.lockb").is_file() {
        return PackageManager::Bun;
    }
    if root.join("package.json").is_file() {
        return PackageManager::Npm;
    }
    if root.join("pyproject.toml").is_file()
        || root.join("pytest.ini").is_file()
        || root.join("requirements.txt").is_file()
    {
        return PackageManager::Python;
    }
    PackageManager::Generic
}

fn classify_path(
    root: &Path,
    path: &str,
    classification: &mut Classification,
    warnings: &mut Vec<String>,
) {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let docs = matches!(extension, "md" | "mdx" | "txt" | "rst" | "adoc" | "pdf");
    let frontend = matches!(
        extension,
        "tsx" | "jsx" | "vue" | "svelte" | "css" | "scss" | "less" | "html"
    ) || lower.contains("/components/")
        || lower.contains("/pages/");
    let source = matches!(
        extension,
        "rs" | "ts"
            | "js"
            | "mjs"
            | "cjs"
            | "go"
            | "py"
            | "java"
            | "kt"
            | "swift"
            | "c"
            | "cc"
            | "cpp"
            | "h"
            | "hpp"
            | "cs"
            | "rb"
            | "php"
    );
    let test = name.contains(".test.")
        || name.contains(".spec.")
        || name.ends_with("_test.rs")
        || name.ends_with("_test.go")
        || lower
            .split('/')
            .any(|part| matches!(part, "test" | "tests" | "__tests__"));
    let dependency = matches!(
        name,
        "package.json"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lock"
            | "bun.lockb"
            | "cargo.toml"
            | "cargo.lock"
            | "pyproject.toml"
            | "requirements.txt"
            | "poetry.lock"
            | "go.mod"
            | "go.sum"
    );
    let ci = lower.starts_with(".github/")
        || lower.starts_with(".gitlab/")
        || lower.contains(".gitlab-ci")
        || name == "jenkinsfile"
        || name == "azure-pipelines.yml"
        || lower.starts_with(".buildkite/");
    let sensitive_name = [
        "auth",
        "oauth",
        "crypto",
        "permission",
        "credential",
        "secret",
        "password",
        "token",
        "security",
        "policy",
        "access",
        "session",
        "identity",
    ]
    .iter()
    .any(|part| lower.contains(part));
    let process = [
        "process",
        "spawn",
        "exec",
        "worker",
        "supervisor",
        "subprocess",
    ]
    .iter()
    .any(|part| lower.contains(part));
    let public_api = ["/api/", "/public/", "/routes/", "/lib/", "index.", "mod."]
        .iter()
        .any(|part| lower.contains(part));

    if docs {
        classification.label("documentation");
    }
    if frontend {
        classification.frontend = true;
        classification.browser = true;
        classification.label("frontend");
    }
    if source {
        classification.source = true;
        classification.label("source");
    }
    if test {
        classification.tests = true;
        classification.label("tests");
    }
    if dependency {
        classification.dependency = true;
        classification.label("dependency-manifest");
    }
    if ci {
        classification.ci = true;
        classification.label("ci-configuration");
    }
    if sensitive_name {
        classification.security = true;
        classification.label("security-sensitive");
    }
    if process {
        classification.process = true;
        classification.label("process-surface");
    }
    if public_api {
        classification.public_api = true;
        classification.label("public-api");
    }
    if !docs
        || frontend
        || source
        || test
        || dependency
        || ci
        || sensitive_name
        || process
        || public_api
    {
        classification.docs_only = false;
    }

    if let Ok(bytes) = fs::read(root.join(path)) {
        let sample =
            String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_FILE_BYTES)]).to_ascii_lowercase();
        if sample.contains("password")
            || sample.contains("client_secret")
            || sample.contains("private_key")
            || sample.contains("authorize")
        {
            classification.security = true;
            classification.label("security-sensitive-content");
        }
    } else {
        warnings.push(format!(
            "changed path is unavailable in the current workspace: {path}"
        ));
    }
}

fn has_script(root: &Path, manager: PackageManager, name: &str) -> bool {
    match manager {
        PackageManager::Cargo => matches!(name, "typecheck" | "lint"),
        PackageManager::Npm | PackageManager::Pnpm | PackageManager::Yarn | PackageManager::Bun => {
            let Ok(raw) = fs::read_to_string(root.join("package.json")) else {
                return false;
            };
            serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|value| value.get("scripts").cloned())
                .and_then(|scripts| scripts.get(name).cloned())
                .is_some_and(|value| value.as_str().is_some_and(|text| !text.trim().is_empty()))
        }
        PackageManager::Python | PackageManager::Generic => false,
    }
}

fn add_requirement(
    requirements: &mut Vec<VerificationRequirement>,
    kind: &str,
    reason: &str,
    evidence_kind: &str,
    required: bool,
) {
    if requirements.iter().any(|item| item.kind == kind) {
        return;
    }
    requirements.push(VerificationRequirement {
        kind: kind.into(),
        reason: reason.into(),
        evidence_kind: evidence_kind.into(),
        required,
    });
}

fn add_step(
    steps: &mut Vec<VerificationStep>,
    requirements: &mut Vec<VerificationRequirement>,
    spec: StepSpec,
) {
    if steps.iter().any(|step| step.operation == spec.operation) {
        return;
    }
    add_requirement(
        requirements,
        spec.operation,
        &spec.reason,
        spec.evidence_kind,
        spec.required,
    );
    steps.push(spec.into_step());
}

struct StepSpec {
    id: &'static str,
    tier: u8,
    required: bool,
    operation: &'static str,
    program: Option<String>,
    argv: Vec<String>,
    cwd: PathBuf,
    reason: String,
    evidence_kind: &'static str,
    source_identity: String,
}

impl StepSpec {
    fn diff_check(root: &Path, identity: &str) -> Self {
        Self {
            id: "diff-check",
            tier: 0,
            required: true,
            operation: "diff-check",
            program: Some("git".into()),
            argv: vec!["diff".into(), "--check".into()],
            cwd: root.to_path_buf(),
            reason: "cheap whitespace and patch integrity check".into(),
            evidence_kind: "process_receipt",
            source_identity: identity.into(),
        }
    }

    fn tests(manager: PackageManager, root: &Path, identity: &str) -> Self {
        let argv = match manager {
            PackageManager::Cargo => vec!["test".into(), "--offline".into(), "--locked".into()],
            PackageManager::Npm => vec!["test".into()],
            PackageManager::Pnpm => vec!["test".into()],
            PackageManager::Yarn => vec!["test".into()],
            PackageManager::Bun => vec!["test".into()],
            PackageManager::Python => vec!["-m".into(), "pytest".into()],
            PackageManager::Generic => vec!["diff".into(), "--check".into()],
        };
        Self {
            id: "targeted-tests",
            tier: 1,
            required: true,
            operation: "tests",
            program: Some(manager.command().into()),
            argv,
            cwd: root.to_path_buf(),
            reason: "current source or test files require matching test evidence".into(),
            evidence_kind: "test_receipt",
            source_identity: identity.into(),
        }
    }

    fn typecheck(manager: PackageManager, root: &Path, identity: &str) -> Self {
        let argv = match manager {
            PackageManager::Cargo => vec!["check".into(), "--offline".into(), "--locked".into()],
            _ => vec!["run".into(), "typecheck".into()],
        };
        Self {
            id: "typecheck",
            tier: 2,
            required: true,
            operation: "typecheck",
            program: Some(manager.command().into()),
            argv,
            cwd: root.to_path_buf(),
            reason: "typed or compiled source changed".into(),
            evidence_kind: "process_receipt",
            source_identity: identity.into(),
        }
    }

    fn lint(manager: PackageManager, root: &Path, identity: &str) -> Self {
        Self {
            id: "lint",
            tier: 3,
            required: true,
            operation: "lint",
            program: Some(manager.command().into()),
            argv: vec!["run".into(), "lint".into()],
            cwd: root.to_path_buf(),
            reason: "repository lint script is available for changed source".into(),
            evidence_kind: "process_receipt",
            source_identity: identity.into(),
        }
    }

    fn build(manager: PackageManager, root: &Path, identity: &str) -> Self {
        let (program, argv) = match manager {
            PackageManager::Cargo => (
                Some("cargo".into()),
                vec!["build".into(), "--offline".into(), "--locked".into()],
            ),
            PackageManager::Generic => (None, Vec::new()),
            _ => (
                Some(manager.command().into()),
                vec!["run".into(), "build".into()],
            ),
        };
        Self {
            id: "build",
            tier: 4,
            required: true,
            operation: "build",
            program,
            argv,
            cwd: root.to_path_buf(),
            reason: "dependency, public API, or full verification change".into(),
            evidence_kind: "process_receipt",
            source_identity: identity.into(),
        }
    }

    fn browser(root: &Path, identity: &str) -> Self {
        Self {
            id: "browser",
            tier: 5,
            required: true,
            operation: "browser",
            program: None,
            argv: vec!["browser_verification".into()],
            cwd: root.to_path_buf(),
            reason: "frontend or explicit UI/browser requirement".into(),
            evidence_kind: "browser_receipt",
            source_identity: identity.into(),
        }
    }

    fn security(root: &Path, identity: &str) -> Self {
        Self {
            id: "security",
            tier: 6,
            required: true,
            operation: "security",
            program: None,
            argv: vec!["sec_scan_start".into()],
            cwd: root.to_path_buf(),
            reason: "security-sensitive, process, auth, or permission surface changed".into(),
            evidence_kind: "security_receipt",
            source_identity: identity.into(),
        }
    }

    fn ci(root: &Path, identity: &str) -> Self {
        Self {
            id: "ci-parity",
            tier: 7,
            required: true,
            operation: "ci",
            program: None,
            argv: vec!["ci_matching_jobs".into()],
            cwd: root.to_path_buf(),
            reason: "CI configuration or explicit CI requirement changed".into(),
            evidence_kind: "ci_receipt",
            source_identity: identity.into(),
        }
    }

    fn into_step(self) -> VerificationStep {
        VerificationStep {
            id: self.id.into(),
            tier: self.tier,
            required: self.required,
            operation: self.operation.into(),
            program: self.program,
            argv: self.argv,
            cwd: self.cwd.to_string_lossy().into_owned(),
            reason: self.reason,
            evidence_kind: self.evidence_kind.into(),
            source_identity: self.source_identity,
            depends_on: Vec::new(),
            status: "planned".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_only_change_does_not_activate_runtime_checks() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "docs").unwrap();
        let plan = build_plan(
            dir.path(),
            &VerificationPlanArgs {
                files: vec!["README.md".into()],
                ..Default::default()
            },
            None,
            None,
            &VerificationPlannerConfig::default(),
        )
        .unwrap();
        assert!(plan.classification.contains(&"documentation-only".into()));
        assert!(plan.steps.is_empty());
    }

    #[test]
    fn frontend_auth_change_orders_cheap_checks_before_browser_and_security() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"typecheck":"tsc","lint":"eslint","test":"vitest","build":"vite build"}}"#,
        )
        .unwrap();
        fs::write(dir.path().join("Login.tsx"), "export function Login() {}").unwrap();
        let plan = build_plan(
            dir.path(),
            &VerificationPlanArgs {
                files: vec!["Login.tsx".into(), "src/auth.ts".into()],
                ..Default::default()
            },
            None,
            None,
            &VerificationPlannerConfig::default(),
        )
        .unwrap();
        let order: Vec<_> = plan
            .steps
            .iter()
            .map(|step| step.operation.as_str())
            .collect();
        assert!(
            order.iter().position(|item| *item == "tests")
                < order.iter().position(|item| *item == "browser")
        );
        assert!(order.contains(&"security"));
        assert_eq!(plan.steps[1].depends_on, vec![plan.steps[0].id.clone()]);
    }

    #[test]
    fn traversal_is_rejected() {
        let error = normalize_path("../secret.txt").unwrap_err();
        assert!(error.contains("escapes"));
    }

    #[test]
    fn generic_force_full_plan_does_not_invent_a_build_command() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("service.txt"), "source").unwrap();
        let plan = build_plan(
            dir.path(),
            &VerificationPlanArgs {
                files: vec!["service.txt".into()],
                force_full: true,
                ..Default::default()
            },
            None,
            None,
            &VerificationPlannerConfig::default(),
        )
        .unwrap();
        let build = plan
            .steps
            .iter()
            .find(|step| step.operation == "build")
            .expect("forceFull always plans a build dimension");
        assert!(build.program.is_none());
        assert!(build.argv.is_empty());
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.contains("external process receipt")));
    }
}
