//! Whether `/design` can run here, and the one thing to do if it cannot.
//!
//! This reads the checks the runtime already makes (presence, pinned Node,
//! pinned browser, a runtime kept outside the workspace) and names the first
//! thing that is wrong. It installs nothing and runs no project script: the
//! only fix it ever offers is a command for the person to run, or a switch.
//! The cheap pass looks at files and the manifest; the deep pass also hashes
//! every pinned file, the way a real run does, and is the only one that can
//! tell a tampered runtime from an intact one.

use super::runtime::{RuntimeManifest, TrustedDesignRuntime};
use super::{error::DesignError, setup};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The Node release the runtime is pinned to.
const PINNED_NODE: &str = "24.19.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    /// Everything this check can see is in order.
    Ready,
    /// Installed and sound; only the feature switch is off.
    Disabled,
    /// Something `/design` needs is not installed.
    Missing,
    /// Installed but refused: changed, unsupported, in the wrong place, or
    /// used from the wrong workspace or model.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthDiagnostic {
    pub code: &'static str,
    pub message: String,
}

/// What to do about the first problem found. Always something the person
/// runs or switches; never something DaVinci does behind their back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum FixAction {
    /// Turn design artifacts on (`/config`).
    TurnOn,
    /// Install the pinned runtime. `instruction` is the exact command, or why
    /// it cannot be run from here and what to do instead.
    RunSetup { instruction: String },
    /// Start DaVinci in a project folder, not in or above the home folder.
    StartInProject,
    /// An environment variable overrides the saved setting.
    UnsetEnvironment { name: String },
    /// Reinstall the pinned runtime explicitly.
    Reinstall { instruction: String },
    /// Generation needs the signed-in subscription model.
    SelectSubscriptionModel,
    /// The runtime must live outside the workspace the session can write.
    MoveRuntimeOutsideWorkspace,
}

impl FixAction {
    pub fn describe(&self) -> String {
        match self {
            FixAction::TurnOn => "turn on design artifacts in /config".into(),
            FixAction::RunSetup { instruction } => instruction.clone(),
            FixAction::StartInProject => setup::HOME_WORKSPACE.into(),
            FixAction::UnsetEnvironment { name } => {
                format!("unset {name}; the environment overrides your saved settings")
            }
            FixAction::Reinstall { instruction } => instruction.clone(),
            FixAction::SelectSubscriptionModel => {
                "select a subscription model; design generation does not use API keys or offline mode".into()
            }
            FixAction::MoveRuntimeOutsideWorkspace => {
                "install the design runtime outside this workspace".into()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesignRuntimeHealth {
    pub state: HealthState,
    pub diagnostics: Vec<HealthDiagnostic>,
    pub fix_action: Option<FixAction>,
}

/// Everything the check reads, so it is the same input every time.
#[derive(Debug, Clone)]
pub struct HealthProbe {
    pub enabled: bool,
    pub runtime: Option<PathBuf>,
    pub node: Option<PathBuf>,
    pub workspace: PathBuf,
    /// Hash every pinned file, as a real run does.
    pub deep: bool,
    /// Names of environment variables that override the saved setting.
    pub overrides: Vec<String>,
    /// Why generation would refuse the current model route, if it would.
    pub provider_refusal: Option<String>,
}

impl HealthProbe {
    /// The probe for this process: its environment and its workspace.
    pub fn from_environment(workspace: &Path, deep: bool) -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        Self {
            enabled: super::enabled(),
            runtime: var("DAVINCI_DESIGN_RUNTIME").map(PathBuf::from),
            node: var("DAVINCI_DESIGN_NODE").map(PathBuf::from),
            workspace: workspace.to_path_buf(),
            deep,
            overrides: vec![],
            provider_refusal: None,
        }
    }
}

struct Findings {
    state: HealthState,
    fix: Option<FixAction>,
    diagnostics: Vec<HealthDiagnostic>,
}

impl Findings {
    /// Record a problem. The worst state wins, and the first fix at that
    /// level is the one offered.
    fn add(
        &mut self,
        state: HealthState,
        code: &'static str,
        message: impl Into<String>,
        fix: FixAction,
    ) {
        if state > self.state {
            self.state = state;
            self.fix = Some(fix);
        }
        self.diagnostics.push(HealthDiagnostic {
            code,
            message: message.into(),
        });
    }
}

fn setup_instruction(workspace: &Path) -> String {
    match setup::installer(workspace) {
        Ok(installer) => format!("run {}", installer.display()),
        Err(why) => why,
    }
}

/// Look at the design runtime and report what is wrong, if anything.
pub fn assess(probe: &HealthProbe) -> DesignRuntimeHealth {
    let mut found = Findings {
        state: HealthState::Ready,
        fix: None,
        diagnostics: vec![],
    };
    let install = || FixAction::RunSetup {
        instruction: setup_instruction(&probe.workspace),
    };
    let reinstall = || FixAction::Reinstall {
        instruction: setup_instruction(&probe.workspace),
    };

    if setup::workspace_holds_home(&probe.workspace) {
        found.add(
            HealthState::Invalid,
            "workspace_holds_home",
            setup::HOME_WORKSPACE,
            FixAction::StartInProject,
        );
    }

    let mut manifest_root = None;
    match &probe.runtime {
        None => found.add(
            HealthState::Missing,
            "runtime_not_installed",
            "no design runtime is installed",
            install(),
        ),
        Some(root) if !root.join("runtime-manifest.json").is_file() => found.add(
            HealthState::Missing,
            "runtime_manifest_missing",
            format!("{} has no runtime-manifest.json", root.display()),
            install(),
        ),
        Some(root) => manifest_root = Some(root.clone()),
    }
    match &probe.node {
        None => found.add(
            HealthState::Missing,
            "node_not_configured",
            "no Node executable is configured for the design runtime",
            install(),
        ),
        Some(node) if !node.is_file() => found.add(
            HealthState::Missing,
            "node_not_found",
            format!(
                "the configured Node executable {} is missing",
                node.display()
            ),
            install(),
        ),
        Some(_) => {}
    }

    if let Some(root) = &manifest_root {
        inspect_manifest(probe, root, &mut found, &install, &reinstall);
    }

    if found.state == HealthState::Ready && probe.deep {
        if let (Some(root), Some(node)) = (&probe.runtime, &probe.node) {
            match TrustedDesignRuntime::load(node, root, &probe.workspace) {
                Ok(_) => {}
                Err(DesignError::CorruptArtifact(why)) => found.add(
                    HealthState::Invalid,
                    "runtime_changed",
                    format!("the installed runtime no longer matches its manifest: {why}"),
                    reinstall(),
                ),
                Err(DesignError::MissingCapability(why)) => {
                    found.add(HealthState::Missing, "runtime_incomplete", why, install())
                }
                Err(other) => found.add(
                    HealthState::Invalid,
                    "runtime_refused",
                    format!("the runtime was refused: {other}"),
                    reinstall(),
                ),
            }
        }
    }

    if let Some(refusal) = &probe.provider_refusal {
        found.add(
            HealthState::Invalid,
            "wrong_provider",
            refusal.clone(),
            FixAction::SelectSubscriptionModel,
        );
    }

    if !probe.enabled && found.state == HealthState::Ready {
        found.add(
            HealthState::Disabled,
            "feature_off",
            "design artifacts are turned off",
            FixAction::TurnOn,
        );
    }

    // An override explains a problem that saved settings cannot fix; it is
    // not a problem of its own.
    if found.state != HealthState::Ready {
        for name in &probe.overrides {
            found.diagnostics.push(HealthDiagnostic {
                code: "env_override",
                message: format!("{name} is set in the environment and overrides settings.json"),
            });
            if found.state <= HealthState::Disabled || found.fix.is_none() {
                found.fix = Some(FixAction::UnsetEnvironment { name: name.clone() });
            }
        }
    }

    DesignRuntimeHealth {
        state: found.state,
        diagnostics: found.diagnostics,
        fix_action: found.fix,
    }
}

/// The checks that need only the manifest: Node pin, browser, locations.
fn inspect_manifest(
    probe: &HealthProbe,
    root: &Path,
    found: &mut Findings,
    install: &dyn Fn() -> FixAction,
    reinstall: &dyn Fn() -> FixAction,
) {
    let workspace = probe
        .workspace
        .canonicalize()
        .unwrap_or_else(|_| probe.workspace.clone());
    let inside = |path: &Path| {
        path.canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .starts_with(&workspace)
    };
    if inside(root) || probe.node.as_deref().is_some_and(inside) {
        found.add(
            HealthState::Invalid,
            "runtime_in_workspace",
            "the design runtime or Node is inside this workspace, where the session can rewrite what it would trust",
            FixAction::MoveRuntimeOutsideWorkspace,
        );
    }
    let manifest: RuntimeManifest = match std::fs::read(root.join("runtime-manifest.json"))
        .map_err(|error| error.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
    {
        Ok(manifest) => manifest,
        Err(why) => {
            found.add(
                HealthState::Invalid,
                "manifest_unreadable",
                format!("runtime-manifest.json cannot be read: {why}"),
                reinstall(),
            );
            return;
        }
    };
    if manifest.node_version != PINNED_NODE {
        found.add(
            HealthState::Invalid,
            "node_version_unsupported",
            format!(
                "the runtime is pinned to Node {}, but {PINNED_NODE} is the supported version",
                manifest.node_version
            ),
            reinstall(),
        );
    }
    if !manifest.browser.executable.is_file() {
        found.add(
            HealthState::Missing,
            "browser_missing",
            format!(
                "the pinned Chromium is not installed at {}",
                manifest.browser.executable.display()
            ),
            install(),
        );
    }
    if inside(&manifest.browser.cache) || manifest.fonts.iter().any(|f| inside(&f.directory)) {
        found.add(
            HealthState::Invalid,
            "browser_in_workspace",
            "the pinned browser cache or fonts are inside this workspace",
            FixAction::MoveRuntimeOutsideWorkspace,
        );
    }
}
