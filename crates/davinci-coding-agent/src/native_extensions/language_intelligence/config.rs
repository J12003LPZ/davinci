//! Validated, immutable language-intelligence configuration.
//!
//! Keep serialized TypeScript backend spellings stable. Rust and Python
//! profiles are additive and lazy; merely enabling a profile never launches a
//! server.

use super::servers::Backend;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct LanguageIntelligenceConfig {
    pub enabled: bool,
    pub max_sessions: usize,
    pub idle_timeout_ms: u64,
    pub rust_family_max_sessions: usize,
    pub python_family_max_sessions: usize,
    pub typescript_family_max_sessions: usize,
    pub typescript: TypeScriptConfig,
    pub rust: RustConfig,
    pub python: PythonConfig,
    #[serde(skip)]
    pub configuration_error: Option<String>,
    #[serde(skip)]
    pub profile_errors: std::collections::BTreeMap<String, String>,
}

impl Default for LanguageIntelligenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_sessions: 8,
            idle_timeout_ms: 300_000,
            rust_family_max_sessions: 2,
            python_family_max_sessions: 4,
            typescript_family_max_sessions: 4,
            typescript: TypeScriptConfig::default(),
            rust: RustConfig::default(),
            python: PythonConfig::default(),
            configuration_error: None,
            profile_errors: std::collections::BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeScriptConfig {
    pub enabled: bool,
    pub backend: Backend,
    pub request_timeout_ms: u64,
    pub initialization_timeout_ms: u64,
    pub cold_request_timeout_ms: u64,
    pub max_references: usize,
    pub max_workspace_symbols: usize,
    pub max_diagnostics: usize,
    pub server: Option<ServerOverride>,
}

impl Default for TypeScriptConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: Backend::Auto,
            request_timeout_ms: 5_000,
            initialization_timeout_ms: 30_000,
            cold_request_timeout_ms: 60_000,
            max_references: 50,
            max_workspace_symbols: 50,
            max_diagnostics: 50,
            server: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RustBackend {
    #[default]
    RustAnalyzer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RustConfig {
    pub enabled: bool,
    pub backend: RustBackend,
    pub profile: String,
    pub request_timeout_ms: u64,
    pub initialization_timeout_ms: u64,
    pub cold_request_timeout_ms: u64,
    pub max_references: usize,
    pub max_workspace_symbols: usize,
    pub max_diagnostics: usize,
    pub project_roots: Vec<PathBuf>,
    pub toolchain_dir: Option<PathBuf>,
    pub sysroot: Option<PathBuf>,
    pub sysroot_src: Option<PathBuf>,
    pub target: Option<String>,
    pub features: Vec<String>,
    pub server: Option<ServerOverride>,
}

impl Default for RustConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: RustBackend::RustAnalyzer,
            profile: "navigation".into(),
            request_timeout_ms: 5_000,
            initialization_timeout_ms: 30_000,
            cold_request_timeout_ms: 60_000,
            max_references: 50,
            max_workspace_symbols: 50,
            max_diagnostics: 50,
            project_roots: Vec::new(),
            toolchain_dir: None,
            sysroot: None,
            sysroot_src: None,
            target: None,
            features: Vec::new(),
            server: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PythonBackend {
    #[default]
    Auto,
    Basedpyright,
    Pyright,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonConfig {
    pub enabled: bool,
    pub backend: PythonBackend,
    pub diagnostic_mode: String,
    pub request_timeout_ms: u64,
    pub initialization_timeout_ms: u64,
    pub cold_request_timeout_ms: u64,
    pub max_references: usize,
    pub max_workspace_symbols: usize,
    pub max_diagnostics: usize,
    pub project_roots: Vec<PathBuf>,
    pub interpreter: Option<PathBuf>,
    pub server: Option<ServerOverride>,
}

impl Default for PythonConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: PythonBackend::Auto,
            diagnostic_mode: "openFilesOnly".into(),
            request_timeout_ms: 5_000,
            initialization_timeout_ms: 30_000,
            cold_request_timeout_ms: 60_000,
            max_references: 50,
            max_workspace_symbols: 50,
            max_diagnostics: 50,
            project_roots: Vec::new(),
            interpreter: None,
            server: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerOverride {
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
}

impl LanguageIntelligenceConfig {
    pub fn family_error(
        &self,
        family: crate::native_extensions::language_intelligence::LanguageFamily,
    ) -> Option<&str> {
        let key = match family {
            crate::native_extensions::language_intelligence::LanguageFamily::TypeScript => "typescript",
            crate::native_extensions::language_intelligence::LanguageFamily::Rust => "rust",
            crate::native_extensions::language_intelligence::LanguageFamily::Python => "python",
        };
        self.profile_errors.get(key).map(String::as_str)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=8).contains(&self.max_sessions) {
            return Err("maxSessions must be 1..=8".into());
        }
        if !(30_000..=900_000).contains(&self.idle_timeout_ms) {
            return Err("idleTimeoutMs must be 30000..=900000".into());
        }
        for (name, value) in [
            ("rustFamilyMaxSessions", self.rust_family_max_sessions),
            ("pythonFamilyMaxSessions", self.python_family_max_sessions),
            ("typescriptFamilyMaxSessions", self.typescript_family_max_sessions),
        ] {
            if value == 0 || value > self.max_sessions {
                return Err(format!("{name} must be 1..=maxSessions"));
            }
        }
        validate_profile(
            self.typescript.request_timeout_ms,
            self.typescript.initialization_timeout_ms,
            self.typescript.cold_request_timeout_ms,
            self.typescript.max_references,
            self.typescript.max_workspace_symbols,
            self.typescript.max_diagnostics,
        )?;
        validate_profile(
            self.rust.request_timeout_ms,
            self.rust.initialization_timeout_ms,
            self.rust.cold_request_timeout_ms,
            self.rust.max_references,
            self.rust.max_workspace_symbols,
            self.rust.max_diagnostics,
        )?;
        validate_profile(
            self.python.request_timeout_ms,
            self.python.initialization_timeout_ms,
            self.python.cold_request_timeout_ms,
            self.python.max_references,
            self.python.max_workspace_symbols,
            self.python.max_diagnostics,
        )?;
        if self.rust.profile != "navigation" {
            return Err("rust.profile must be navigation".into());
        }
        if self.python.diagnostic_mode != "openFilesOnly" {
            return Err("python.diagnosticMode must be openFilesOnly".into());
        }
        if self.rust.features.len() > 128
            || self
                .rust
                .features
                .iter()
                .any(|feature| feature.is_empty() || feature.len() > 128)
        {
            return Err("rust.features is limited to 128 non-empty names of at most 128 bytes".into());
        }
        if self
            .rust
            .target
            .as_deref()
            .is_some_and(|target| target.is_empty() || target.len() > 128)
        {
            return Err("rust.target must be at most 128 bytes".into());
        }
        for server in [
            self.typescript.server.as_ref(),
            self.rust.server.as_ref(),
            self.python.server.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !server.program.is_absolute() {
                return Err("language server override program must be an absolute path".into());
            }
            if server.args.len() > 64 || server.args.iter().any(|arg| arg.len() > 4096 || arg.contains('\0')) {
                return Err("language server override arguments exceed bounds".into());
            }
        }
        Ok(())
    }

    /// Stable fingerprint for session identity. Serde field order is fixed by
    /// these structs; no map insertion order participates.
    pub fn profile_fingerprint<T: Serialize>(profile: &T) -> String {
        let bytes = serde_json::to_vec(profile).expect("serializable language profile");
        let digest = Sha256::digest(bytes);
        format!("{digest:x}")
    }
}

fn validate_profile(
    request_timeout_ms: u64,
    initialization_timeout_ms: u64,
    cold_request_timeout_ms: u64,
    max_references: usize,
    max_workspace_symbols: usize,
    max_diagnostics: usize,
) -> Result<(), String> {
    if !(100..=30_000).contains(&request_timeout_ms) {
        return Err("requestTimeoutMs must be 100..=30000".into());
    }
    if !(1_000..=60_000).contains(&initialization_timeout_ms) {
        return Err("initializationTimeoutMs must be 1000..=60000".into());
    }
    if !(1_000..=120_000).contains(&cold_request_timeout_ms)
        || cold_request_timeout_ms < initialization_timeout_ms
    {
        return Err("coldRequestTimeoutMs must be 1000..=120000 and >= initializationTimeoutMs".into());
    }
    if [max_references, max_workspace_symbols, max_diagnostics]
        .into_iter()
        .any(|limit| !(1..=200).contains(&limit))
    {
        return Err("result caps must be 1..=200".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_typescript_settings_round_trip() {
        let config: LanguageIntelligenceConfig = serde_json::from_value(serde_json::json!({
            "typescript": {"backend":"typescriptLanguageServer"}
        }))
        .unwrap();
        assert_eq!(config.typescript.backend, Backend::TypeScriptLanguageServer);
        assert!(config.rust.enabled);
        assert!(config.python.enabled);
        config.validate().unwrap();
    }

    #[test]
    fn profile_fingerprint_is_order_independent_for_typed_profiles() {
        let a: RustConfig = serde_json::from_value(serde_json::json!({
            "features":["one","two"], "target":"x86_64-unknown-linux-gnu"
        }))
        .unwrap();
        let b: RustConfig = serde_json::from_value(serde_json::json!({
            "target":"x86_64-unknown-linux-gnu", "features":["one","two"]
        }))
        .unwrap();
        assert_eq!(
            LanguageIntelligenceConfig::profile_fingerprint(&a),
            LanguageIntelligenceConfig::profile_fingerprint(&b)
        );
    }

    #[test]
    fn invalid_bounds_are_rejected() {
        let mut config = LanguageIntelligenceConfig::default();
        config.python.request_timeout_ms = 99;
        assert!(config.validate().is_err());
    }
}
