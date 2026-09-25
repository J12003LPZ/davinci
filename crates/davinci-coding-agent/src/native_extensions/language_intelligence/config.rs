//! Validated immutable language-intelligence configuration.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeScriptBackend {
    #[default]
    Auto,
    #[serde(rename = "typescriptNative")]
    TypeScriptNative,
    #[serde(rename = "typescriptLanguageServer")]
    TypeScriptLanguageServer,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RustBackend {
    #[default]
    #[serde(rename = "rustAnalyzer")]
    RustAnalyzer,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PythonBackend {
    #[default]
    Auto,
    Basedpyright,
    Pyright,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RustProfile {
    #[default]
    Navigation,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PythonDiagnosticMode {
    #[default]
    #[serde(rename = "openFilesOnly")]
    OpenFilesOnly,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerOverride {
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeScriptConfig {
    pub enabled: bool,
    pub backend: TypeScriptBackend,
    pub request_timeout_ms: u64,
    pub initialization_timeout_ms: u64,
    pub cold_request_timeout_ms: u64,
    pub max_references: usize,
    pub max_workspace_symbols: usize,
    pub max_diagnostics: usize,
    pub max_sessions: usize,
    pub server: Option<ServerOverride>,
}
impl Default for TypeScriptConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: TypeScriptBackend::Auto,
            request_timeout_ms: 5_000,
            initialization_timeout_ms: 30_000,
            cold_request_timeout_ms: 60_000,
            max_references: 50,
            max_workspace_symbols: 50,
            max_diagnostics: 50,
            max_sessions: 4,
            server: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RustConfig {
    pub enabled: bool,
    pub backend: RustBackend,
    pub profile: RustProfile,
    pub request_timeout_ms: u64,
    pub initialization_timeout_ms: u64,
    pub cold_request_timeout_ms: u64,
    pub max_references: usize,
    pub max_workspace_symbols: usize,
    pub max_diagnostics: usize,
    pub max_sessions: usize,
    pub project_roots: Vec<PathBuf>,
    pub server: Option<ServerOverride>,
    pub toolchain_dir: Option<PathBuf>,
    pub sysroot: Option<PathBuf>,
    pub sysroot_src: Option<PathBuf>,
    pub target: Option<String>,
    pub features: Vec<String>,
}
impl Default for RustConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: RustBackend::RustAnalyzer,
            profile: RustProfile::Navigation,
            request_timeout_ms: 5_000,
            initialization_timeout_ms: 30_000,
            cold_request_timeout_ms: 60_000,
            max_references: 50,
            max_workspace_symbols: 50,
            max_diagnostics: 50,
            max_sessions: 2,
            project_roots: Vec::new(),
            server: None,
            toolchain_dir: None,
            sysroot: None,
            sysroot_src: None,
            target: None,
            features: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonConfig {
    pub enabled: bool,
    pub backend: PythonBackend,
    pub diagnostic_mode: PythonDiagnosticMode,
    pub request_timeout_ms: u64,
    pub initialization_timeout_ms: u64,
    pub cold_request_timeout_ms: u64,
    pub max_references: usize,
    pub max_workspace_symbols: usize,
    pub max_diagnostics: usize,
    pub max_sessions: usize,
    pub project_roots: Vec<PathBuf>,
    pub interpreter: Option<PathBuf>,
    pub server: Option<ServerOverride>,
}
impl Default for PythonConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: PythonBackend::Auto,
            diagnostic_mode: PythonDiagnosticMode::OpenFilesOnly,
            request_timeout_ms: 5_000,
            initialization_timeout_ms: 30_000,
            cold_request_timeout_ms: 60_000,
            max_references: 50,
            max_workspace_symbols: 50,
            max_diagnostics: 50,
            max_sessions: 4,
            project_roots: Vec::new(),
            interpreter: None,
            server: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageIntelligenceConfig {
    pub enabled: bool,
    pub max_sessions: usize,
    pub idle_timeout_ms: u64,
    pub typescript: TypeScriptConfig,
    pub rust: RustConfig,
    pub python: PythonConfig,
    #[serde(skip)]
    pub configuration_error: Option<String>,
    #[serde(skip)]
    pub profile_errors: Vec<String>,
}
impl Default for LanguageIntelligenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_sessions: 8,
            idle_timeout_ms: 300_000,
            typescript: TypeScriptConfig::default(),
            rust: RustConfig::default(),
            python: PythonConfig::default(),
            configuration_error: None,
            profile_errors: Vec::new(),
        }
    }
}

impl<'de> Deserialize<'de> for LanguageIntelligenceConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(Self::from_value(value))
    }
}

impl LanguageIntelligenceConfig {
    pub fn from_value(value: Value) -> Self {
        let mut out = Self::default();
        let Some(object) = value.as_object() else {
            out.enabled = false;
            out.configuration_error = Some("languageIntelligence must be an object".into());
            return out;
        };
        for key in object.keys() {
            if !matches!(
                key.as_str(),
                "enabled" | "maxSessions" | "idleTimeoutMs" | "typescript" | "rust" | "python"
            ) {
                out.enabled = false;
                out.configuration_error =
                    Some(format!("unknown languageIntelligence setting '{key}'"));
                return out;
            }
        }
        if let Some(v) = object.get("enabled") {
            match v.as_bool() {
                Some(value) => out.enabled = value,
                None => {
                    out.enabled = false;
                    out.configuration_error =
                        Some("languageIntelligence.enabled must be boolean".into());
                    return out;
                }
            }
        }
        if let Some(v) = object.get("maxSessions") {
            match v
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| (1..=8).contains(n))
            {
                Some(value) => out.max_sessions = value,
                None => {
                    out.enabled = false;
                    out.configuration_error =
                        Some("languageIntelligence.maxSessions must be 1..8".into());
                    return out;
                }
            }
        }
        if let Some(v) = object.get("idleTimeoutMs") {
            match v.as_u64().filter(|n| (30_000..=900_000).contains(n)) {
                Some(value) => out.idle_timeout_ms = value,
                None => {
                    out.enabled = false;
                    out.configuration_error =
                        Some("languageIntelligence.idleTimeoutMs must be 30000..900000".into());
                    return out;
                }
            }
        }
        if let Some(v) = object.get("typescript") {
            match serde_json::from_value::<TypeScriptConfig>(v.clone())
                .and_then(validate_typescript)
            {
                Ok(profile) => out.typescript = profile,
                Err(error) => {
                    out.typescript.enabled = false;
                    out.profile_errors.push(format!("typescript: {error}"));
                }
            }
        }
        if let Some(v) = object.get("rust") {
            match serde_json::from_value::<RustConfig>(v.clone()).and_then(validate_rust) {
                Ok(profile) => out.rust = profile,
                Err(error) => {
                    out.rust.enabled = false;
                    out.profile_errors.push(format!("rust: {error}"));
                }
            }
        }
        if let Some(v) = object.get("python") {
            match serde_json::from_value::<PythonConfig>(v.clone()).and_then(validate_python) {
                Ok(profile) => out.python = profile,
                Err(error) => {
                    out.python.enabled = false;
                    out.profile_errors.push(format!("python: {error}"));
                }
            }
        }
        out
    }

    pub fn profile_fingerprint<T: Serialize>(&self, profile: &T) -> String {
        let mut value = serde_json::to_value(profile).unwrap_or(Value::Null);
        canonicalize(&mut value);
        let mut hasher = Sha256::new();
        hasher.update(serde_json::to_vec(&value).unwrap_or_default());
        format!("{:x}", hasher.finalize())
    }
}

fn validate_common(
    request: u64,
    init: u64,
    cold: u64,
    refs: usize,
    symbols: usize,
    diagnostics: usize,
    sessions: usize,
) -> Result<(), String> {
    if !(100..=30_000).contains(&request) {
        return Err("requestTimeoutMs must be 100..30000".into());
    }
    if !(1_000..=60_000).contains(&init) {
        return Err("initializationTimeoutMs must be 1000..60000".into());
    }
    if !(1_000..=120_000).contains(&cold) || cold < init {
        return Err(
            "coldRequestTimeoutMs must be 1000..120000 and at least initializationTimeoutMs".into(),
        );
    }
    if [refs, symbols, diagnostics]
        .iter()
        .any(|n| !(1..=200).contains(n))
    {
        return Err("result limits must be 1..200".into());
    }
    if !(1..=8).contains(&sessions) {
        return Err("maxSessions must be 1..8".into());
    }
    Ok(())
}
fn validate_server(server: &Option<ServerOverride>) -> Result<(), String> {
    if let Some(server) = server {
        if !server.program.is_absolute() {
            return Err("server.program must be an absolute trusted path".into());
        }
        if server.args.len() > 64
            || server
                .args
                .iter()
                .any(|arg| arg.len() > 4096 || arg.contains('\0'))
        {
            return Err("server.args exceed bounded argument policy".into());
        }
    }
    Ok(())
}
fn validate_typescript(profile: TypeScriptConfig) -> Result<TypeScriptConfig, serde_json::Error> {
    validate_common(
        profile.request_timeout_ms,
        profile.initialization_timeout_ms,
        profile.cold_request_timeout_ms,
        profile.max_references,
        profile.max_workspace_symbols,
        profile.max_diagnostics,
        profile.max_sessions,
    )
    .and_then(|_| validate_server(&profile.server))
    .map_err(custom_json_error)?;
    Ok(profile)
}
fn validate_rust(profile: RustConfig) -> Result<RustConfig, serde_json::Error> {
    validate_common(
        profile.request_timeout_ms,
        profile.initialization_timeout_ms,
        profile.cold_request_timeout_ms,
        profile.max_references,
        profile.max_workspace_symbols,
        profile.max_diagnostics,
        profile.max_sessions,
    )
    .and_then(|_| validate_server(&profile.server))
    .map_err(custom_json_error)?;
    if profile.features.len() > 128
        || profile.features.iter().any(|f| {
            f.is_empty()
                || f.len() > 128
                || !f
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
    {
        return Err(custom_json_error(
            "rust.features must contain at most 128 bounded Cargo feature names".into(),
        ));
    }
    if profile.target.as_ref().is_some_and(|v| {
        v.is_empty()
            || v.len() > 128
            || !v
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    }) {
        return Err(custom_json_error("rust.target is invalid".into()));
    }
    Ok(profile)
}
fn validate_python(profile: PythonConfig) -> Result<PythonConfig, serde_json::Error> {
    validate_common(
        profile.request_timeout_ms,
        profile.initialization_timeout_ms,
        profile.cold_request_timeout_ms,
        profile.max_references,
        profile.max_workspace_symbols,
        profile.max_diagnostics,
        profile.max_sessions,
    )
    .and_then(|_| validate_server(&profile.server))
    .map_err(custom_json_error)?;
    Ok(profile)
}
fn custom_json_error(message: String) -> serde_json::Error {
    <serde_json::Error as serde::de::Error>::custom(message)
}
fn canonicalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let old = std::mem::take(map);
            let mut keys: Vec<_> = old.keys().cloned().collect();
            keys.sort();
            let mut sorted = Map::new();
            for key in keys {
                let mut value = old.get(&key).cloned().unwrap_or(Value::Null);
                canonicalize(&mut value);
                sorted.insert(key, value);
            }
            *map = sorted;
        }
        Value::Array(values) => values.iter_mut().for_each(canonicalize),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_typescript_settings_round_trip_and_invalid_python_is_isolated() {
        let config = LanguageIntelligenceConfig::from_value(json!({
            "typescript":{"backend":"typescriptLanguageServer"},
            "rust":{"enabled":true},
            "python":{"backend":"misspelled"}
        }));
        assert_eq!(
            config.typescript.backend,
            TypeScriptBackend::TypeScriptLanguageServer
        );
        assert!(config.typescript.enabled);
        assert!(config.rust.enabled);
        assert!(!config.python.enabled);
        assert!(config.configuration_error.is_none());
        assert_eq!(config.profile_errors.len(), 1);
    }

    #[test]
    fn profile_fingerprint_is_order_independent() {
        let config = LanguageIntelligenceConfig::default();
        let a = json!({"b":2,"a":1});
        let b = json!({"a":1,"b":2});
        assert_eq!(
            config.profile_fingerprint(&a),
            config.profile_fingerprint(&b)
        );
    }
}
