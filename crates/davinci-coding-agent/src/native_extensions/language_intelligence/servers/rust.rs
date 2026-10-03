//! rust-analyzer adapter and restricted navigation profile.
use super::super::config::{LanguageIntelligenceConfig, RustProfile};
use super::super::identity::{LanguageFamily, ResolvedProject};
use super::super::metadata::ResolutionContext;
use super::super::protocol::{IntelligenceError, Result};
use super::{discovery, ServerAdapter, ServerBackend, ServerCommand};
use serde_json::json;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(in crate::native_extensions::language_intelligence) struct RustAdapter;

impl ServerAdapter for RustAdapter {
    fn family(&self) -> LanguageFamily {
        LanguageFamily::Rust
    }
    fn language_id(&self, path: &Path) -> Option<&'static str> {
        (path.extension()?.to_str()? == "rs").then_some("rust")
    }
    fn explicit_roots<'a>(&self, settings: &'a LanguageIntelligenceConfig) -> &'a [PathBuf] {
        &settings.rust.project_roots
    }
}

pub(super) fn discover(
    _context: &ResolutionContext,
    project: &ResolvedProject,
    settings: &LanguageIntelligenceConfig,
    search_path: &OsStr,
) -> Result<Vec<ServerCommand>> {
    let profile = &settings.rust;
    if !profile.enabled {
        return Err(IntelligenceError::new(
            "disabled",
            "Rust language intelligence is disabled",
        ));
    }
    let invocation = if let Some(server) = &profile.server {
        discovery::explicit(server, search_path)?
    } else if let Some(invocation) =
        discovery::env_absolute("DAVINCI_RUST_ANALYZER", Vec::new(), search_path)
    {
        invocation
    } else {
        discovery::named(search_path, "rust-analyzer", Vec::new()).ok_or_else(|| {
            IntelligenceError::new(
                "server_not_installed",
                "Install rust-analyzer or configure languageIntelligence.rust.server",
            )
        })?
    };

    for value in [
        &profile.toolchain_dir,
        &profile.sysroot,
        &profile.sysroot_src,
    ]
    .into_iter()
    .flatten()
    {
        if !value.is_absolute() || !value.is_dir() {
            return Err(IntelligenceError::new(
                "invalid_settings",
                "Configured Rust toolchain/sysroot paths must be existing absolute directories",
            ));
        }
    }
    if let Some(src) = &profile.sysroot_src {
        if !src.join("core/src/lib.rs").is_file() {
            return Err(IntelligenceError::new(
                "invalid_settings",
                "rust.sysrootSrc must contain core/src/lib.rs",
            ));
        }
    }

    let fingerprint = settings.profile_fingerprint(profile);
    let target_dir = std::env::temp_dir()
        .join("davinci-language-intelligence")
        .join(&fingerprint[..16.min(fingerprint.len())]);
    let mut cargo = json!({
        "buildScripts":{"enable":false},
        "sysroot": profile.sysroot.as_ref().map(|p| p.to_string_lossy().into_owned()),
        "sysrootSrc": profile.sysroot_src.as_ref().map(|p| p.to_string_lossy().into_owned()),
        "extraEnv":{"CARGO_NET_OFFLINE":"true","RUSTUP_AUTO_INSTALL":"0"},
        "extraArgs":["--locked","--offline"],
        "targetDir": target_dir
    });
    if let Some(target) = &profile.target {
        cargo["target"] = json!(target);
    }
    if !profile.features.is_empty() {
        cargo["features"] = json!(profile.features);
    }
    let configuration = json!({
        "cargo": cargo,
        "procMacro":{"enable":false},
        "checkOnSave": false
    });
    let mut env = BTreeMap::new();
    env.insert("CARGO_NET_OFFLINE".into(), "true".into());
    env.insert("RUSTUP_AUTO_INSTALL".into(), "0".into());
    env.insert(
        "CARGO_TARGET_DIR".into(),
        target_dir.to_string_lossy().into_owned(),
    );

    let mut limitations = vec![
        "proc_macros_disabled".into(),
        "build_scripts_disabled".into(),
        "cargo_checks_disabled".into(),
    ];
    if profile.sysroot_src.is_none() {
        limitations.push("rust_src_not_installed".into());
    }
    if matches!(profile.profile, RustProfile::Navigation) {
        limitations.push("navigation_profile_requires_project_trust".into());
    }

    Ok(vec![ServerCommand {
        kind: ServerBackend::RustAnalyzer,
        backend: ServerBackend::RustAnalyzer,
        program: invocation.program.clone(),
        args: invocation.args.clone(),
        invocation,
        workspace: project.root.clone(),
        version: None,
        typescript_version: None,
        initialization_options: configuration.clone(),
        client_configuration: json!({"rust-analyzer":configuration}),
        family: LanguageFamily::Rust,
        profile_fingerprint: fingerprint,
        analysis_environment: profile.toolchain_dir.clone(),
        limitations,
        env,
    }])
}
