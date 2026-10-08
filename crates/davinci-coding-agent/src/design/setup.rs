//! What `/design` still needs on this machine, and the explicit installer that
//! provides it. Nothing here installs anything on its own: the interactive
//! shell asks first, then hands the terminal to [`Installer`].
use super::commands::{DesignCommand, ExportFormat};
use std::path::{Path, PathBuf};

/// Why `/design` cannot run yet, and what would fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupNeeded {
    /// A runtime is configured and present; only the feature switch is off.
    Disabled,
    /// No usable runtime: the reason names what is missing.
    Runtime(String),
}

impl SetupNeeded {
    pub fn reason(&self) -> &str {
        match self {
            SetupNeeded::Disabled => "design artifacts are turned off",
            SetupNeeded::Runtime(reason) => reason,
        }
    }
}

/// What `command` still needs, `None` when it can run as things are. A cheap
/// presence check, not the fingerprint verification every runtime operation
/// still performs.
pub fn setup_needed(command: &DesignCommand) -> Option<SetupNeeded> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    check(
        command,
        super::enabled(),
        var("DAVINCI_DESIGN_RUNTIME").as_deref().map(Path::new),
        var("DAVINCI_DESIGN_NODE").as_deref().map(Path::new),
    )
}

/// Compiling, rendering, the companion and image exports run Node and
/// Chromium; reading and copying retained source does not.
pub fn needs_runtime(command: &DesignCommand) -> bool {
    match command {
        DesignCommand::Create { .. }
        | DesignCommand::Revise { .. }
        | DesignCommand::Verify { .. }
        | DesignCommand::Open { .. } => true,
        DesignCommand::Export { format, .. } => *format != ExportFormat::Source,
        _ => false,
    }
}

/// What the controller still serves with the feature switched off.
pub fn allowed_when_disabled(command: &DesignCommand) -> bool {
    matches!(
        command,
        DesignCommand::List {}
            | DesignCommand::Status { .. }
            | DesignCommand::Export {
                format: ExportFormat::Source,
                ..
            }
    )
}

fn check(
    command: &DesignCommand,
    enabled: bool,
    runtime: Option<&Path>,
    node: Option<&Path>,
) -> Option<SetupNeeded> {
    let missing = match (runtime, node) {
        (None, _) => Some("no design runtime is installed"),
        (_, None) => Some("no Node executable is configured for the design runtime"),
        (Some(runtime), _) if !runtime.join("runtime-manifest.json").is_file() => {
            Some("the configured design runtime is missing")
        }
        (_, Some(node)) if !node.is_file() => Some("the configured Node executable is missing"),
        _ => None,
    };
    match missing {
        Some(reason) if needs_runtime(command) => Some(SetupNeeded::Runtime(reason.into())),
        _ if !enabled && !allowed_when_disabled(command) => Some(SetupNeeded::Disabled),
        _ => None,
    }
}

/// Why runtime operations refuse a workspace that holds home.
pub const HOME_WORKSPACE: &str = "DaVinci was started in your home folder (or above it), where the session can rewrite the design runtime it would trust; start DaVinci in a project folder to use /design";

/// A workspace that contains the user's home directory (DaVinci started in
/// home, or above it) contains the design runtime, the browser cache and the
/// settings that point at them. The runtime's integrity check is its own
/// manifest, so a session that can write there could have its own code run
/// as trusted tooling: runtime operations refuse such a workspace.
pub fn workspace_holds_home(workspace: &Path) -> bool {
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    davinci_session::home_dir().is_some_and(|home| holds(&canonical(workspace), &canonical(&home)))
}

fn holds(workspace: &Path, home: &Path) -> bool {
    home.starts_with(workspace)
}

/// The setup script and the PowerShell that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installer {
    pub program: PathBuf,
    pub script: PathBuf,
}

impl Installer {
    pub fn args(&self) -> Vec<String> {
        vec![
            "-NoProfile".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-File".into(),
            self.script.to_string_lossy().into_owned(),
        ]
    }

    /// The command as a person would type it, for messages.
    pub fn display(&self) -> String {
        format!("pwsh -NoProfile -File \"{}\"", self.script.display())
    }
}

/// The checkout this binary was built from ships the setup script; a build
/// from elsewhere (or a deleted checkout) has none to offer.
pub fn setup_script() -> Option<PathBuf> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("scripts")
        .join("setup-design-runtime.ps1");
    script.is_file().then_some(script)
}

/// `Err` says why there is no automatic install here and what to do instead.
///
/// The script builds and pins what every later runtime check trusts, so it
/// must not come from where this session can write: a checkout inside
/// `workspace` (DaVinci started in its own repository) could have had the
/// script, the companion or its lockfile edited first. That case is left to
/// the person, from a terminal of their own.
pub fn installer(workspace: &Path) -> Result<Installer, String> {
    if !cfg!(windows) {
        return Err(
            "automatic setup is Windows-only for now; follow the Setup steps in docs/design-artifacts.md"
                .into(),
        );
    }
    let script = setup_script().ok_or_else(|| {
        "this DaVinci build has no checkout with scripts/setup-design-runtime.ps1; run it from a DaVinci checkout (see docs/design-artifacts.md)".to_string()
    })?;
    if checkout_in_workspace(&script, workspace) {
        return Err(format!(
            "the setup script is inside this workspace, where the session can edit it, so DaVinci will not run it for you; review the checkout, then run it yourself in a separate terminal: {}",
            Installer {
                program: PathBuf::new(),
                script: script.clone(),
            }
            .display()
        ));
    }
    // The script needs PowerShell 7 (`ConvertFrom-Json -AsHashtable`).
    let program = find_on_path("pwsh").ok_or_else(|| {
        format!(
            "the design setup needs PowerShell 7: winget install Microsoft.PowerShell, then run {}",
            Installer {
                program: PathBuf::new(),
                script: script.clone(),
            }
            .display()
        )
    })?;
    Ok(Installer { program, script })
}

/// Absolute PATH entries only: a relative one (`.`) resolves against the
/// workspace, which must not choose the program that builds trusted tooling.
/// `symlink_metadata` rather than `is_file`: a Microsoft Store `pwsh.exe` is
/// an app execution alias whose target `metadata` cannot open.
/// The whole checkout counts, not only the script: it builds `design-ui`
/// from the same tree.
fn checkout_in_workspace(script: &Path, workspace: &Path) -> bool {
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let checkout = script
        .parent()
        .and_then(Path::parent)
        .map(canonical)
        .unwrap_or_default();
    let workspace = canonical(workspace);
    checkout.starts_with(&workspace) || workspace.starts_with(&checkout)
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|directory| directory.is_absolute())
        .find_map(|directory| {
            let plain = directory.join(program);
            let mut candidates = vec![plain.clone()];
            if cfg!(windows) {
                candidates.push(plain.with_extension("exe"));
            }
            candidates.into_iter().find(|candidate| {
                std::fs::symlink_metadata(candidate).is_ok_and(|meta| !meta.is_dir())
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("runtime-manifest.json"), "{}").unwrap();
        dir
    }

    fn command(line: &str) -> DesignCommand {
        super::super::commands::parse_design_command(line).unwrap()
    }

    const ID: &str = "00000000-0000-4000-8000-000000000000";

    #[test]
    fn nothing_configured_asks_only_for_what_runs_node() {
        let create = command("/design new \"a pricing page\" --kind landing");
        assert_eq!(
            check(&create, true, None, None),
            Some(SetupNeeded::Runtime(
                "no design runtime is installed".into()
            ))
        );
        // Turning the switch on does not conjure a runtime.
        assert!(matches!(
            check(&create, false, None, None),
            Some(SetupNeeded::Runtime(_))
        ));
        // Listing and reading retained source never needed one.
        assert_eq!(check(&command("/design"), true, None, None), None);
        assert_eq!(check(&command("/design list"), false, None, None), None);
        assert_eq!(
            check(&command(&format!("/design status {ID}")), false, None, None),
            None
        );
        assert_eq!(check(&command("/design-sync src"), true, None, None), None);
    }

    #[test]
    fn every_runtime_operation_is_gated() {
        for line in [
            format!("/design open {ID}"),
            format!("/design verify {ID}"),
            format!("/design revise {ID} \"darker\""),
            format!("/design export {ID} --revision 1 --format html --destination C:/out"),
        ] {
            assert!(needs_runtime(&command(&line)), "{line}");
        }
        let source = command(&format!(
            "/design export {ID} --revision 1 --format source --destination C:/out"
        ));
        assert!(!needs_runtime(&source));
        assert!(allowed_when_disabled(&source));
    }

    #[test]
    fn configured_paths_must_exist() {
        let create = command("/design new \"x\"");
        let runtime = runtime_dir();
        let node = runtime.path().join("runtime-manifest.json");
        let gone = runtime.path().join("gone");
        assert_eq!(
            check(&create, true, Some(&gone), Some(&node)),
            Some(SetupNeeded::Runtime(
                "the configured design runtime is missing".into()
            ))
        );
        assert_eq!(
            check(&create, true, Some(runtime.path()), Some(&gone)),
            Some(SetupNeeded::Runtime(
                "the configured Node executable is missing".into()
            ))
        );
        assert!(matches!(
            check(&create, true, Some(runtime.path()), None),
            Some(SetupNeeded::Runtime(_))
        ));
    }

    #[test]
    fn switched_off_asks_for_the_switch_only_where_the_controller_refuses() {
        let runtime = runtime_dir();
        let node = runtime.path().join("runtime-manifest.json");
        let ready =
            |line: &str, enabled| check(&command(line), enabled, Some(runtime.path()), Some(&node));
        assert_eq!(
            ready("/design new \"x\"", false),
            Some(SetupNeeded::Disabled)
        );
        assert_eq!(
            ready("/design-sync src", false),
            Some(SetupNeeded::Disabled)
        );
        assert_eq!(ready("/design list", false), None);
        assert_eq!(ready("/design new \"x\"", true), None);
        // Off and without a runtime: a source-only command still only needs the switch.
        assert_eq!(
            check(
                &command(&format!("/design fork {ID} --revision 1")),
                false,
                None,
                None
            ),
            Some(SetupNeeded::Disabled)
        );
    }

    #[test]
    fn home_or_above_holds_home_and_a_project_does_not() {
        let home = Path::new("/home/ada");
        assert!(holds(home, home));
        assert!(holds(Path::new("/home"), home));
        assert!(holds(Path::new("/"), home));
        assert!(!holds(Path::new("/home/ada/code/app"), home));
        assert!(!holds(Path::new("/home/bob"), home));
    }

    #[test]
    fn a_checkout_the_session_can_edit_is_never_run_for_it() {
        let script = setup_script().unwrap();
        let checkout = script.parent().unwrap().parent().unwrap();
        // DaVinci started in its own repository, a subfolder, or above it.
        assert!(checkout_in_workspace(&script, checkout));
        assert!(checkout_in_workspace(
            &script,
            &checkout.join("crates").join("davinci-coding-agent")
        ));
        assert!(checkout_in_workspace(&script, checkout.parent().unwrap()));
        let elsewhere = tempfile::tempdir().unwrap();
        assert!(!checkout_in_workspace(&script, elsewhere.path()));
        if cfg!(windows) {
            let refused = installer(checkout).unwrap_err();
            assert!(refused.contains("separate terminal"), "{refused}");
        }
    }

    #[test]
    fn this_checkout_ships_the_setup_script() {
        let script = setup_script().expect("scripts/setup-design-runtime.ps1");
        assert!(script.ends_with("setup-design-runtime.ps1"));
        let installer = Installer {
            program: PathBuf::from("pwsh"),
            script,
        };
        let args = installer.args();
        assert_eq!(
            args[..4],
            ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]
        );
        assert!(args[4].ends_with("setup-design-runtime.ps1"));
    }
}
