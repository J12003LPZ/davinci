//! Read-only checks of existing configuration and runtime services.
//! Native DaVinci diagnostics; no upstream TypeScript counterpart.

use davinci_agent::Agent;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

fn check_file(path: &Path, validate: impl FnOnce(&str) -> bool) -> &'static str {
    match std::fs::read_to_string(path) {
        Ok(raw) if validate(&raw) => "valid",
        Ok(_) => "invalid",
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent",
        Err(_) => "unreadable",
    }
}

fn configuration(agent_dir: &Path, cwd: &Path) -> Vec<Value> {
    let mut rows = Vec::new();
    let settings = std::iter::once(agent_dir.join("settings.json"))
        .chain(crate::project_config::candidates(cwd, "settings.json"));
    for path in settings {
        let status = check_file(&path, crate::settings::valid_settings_json);
        rows.push(json!({"file":path, "status":status}));
    }
    let mut mcp = vec![agent_dir.join("mcp.json")];
    mcp.extend(crate::project_config::candidates(cwd, "mcp.json"));
    if let Some(path) =
        std::env::var_os("DAVINCI_MCP_CONFIG").or_else(|| std::env::var_os("PI_MCP_CONFIG"))
    {
        mcp.push(path.into());
    }
    for path in mcp {
        let status = check_file(&path, |raw| {
            serde_json::from_str::<davinci_mcp::ConfigFile>(raw).is_ok_and(|config| {
                config
                    .mcp_servers
                    .values()
                    .all(|server| server.execution_policy().is_ok())
            })
        });
        rows.push(json!({"file":path, "status":status}));
    }
    let path = agent_dir.join("models.json");
    let status = check_file(&path, |_| {
        davinci_ai::ModelConfig::load(&path).error().is_none()
    });
    rows.push(json!({"file":path,"status":status}));
    let path = agent_dir.join("token-governor.json");
    let status = check_file(&path, |raw| {
        serde_json::from_str::<crate::native_extensions::TokenGovernorConfig>(raw).is_ok()
    });
    rows.push(json!({"file":path,"status":status}));
    let path = agent_dir.join("vector-memory.json");
    let status = check_file(&path, |raw| {
        serde_json::from_str::<crate::native_extensions::VectorMemoryConfig>(raw).is_ok()
    });
    rows.push(json!({"file":path,"status":status}));
    rows
}

fn credential_presence(agent_dir: &Path, provider: &str, explicit: bool) -> Value {
    let auth = davinci_ai::AuthStorage::open(&agent_dir.join("auth.json"));
    let stored = auth
        .as_ref()
        .ok()
        .and_then(|auth| auth.get(provider))
        .is_some_and(|credential| {
            credential
                .key
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty())
                || credential
                    .access
                    .as_ref()
                    .is_some_and(|value| !value.trim().is_empty())
        });
    let environment = davinci_ai::PROVIDER_SPECS
        .iter()
        .find(|spec| spec.id == provider)
        .is_some_and(|spec| {
            spec.env_vars
                .iter()
                .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
        });
    let models = davinci_ai::ModelConfig::load(&agent_dir.join("models.json"));
    let configured = models
        .get_provider(provider)
        .and_then(|config| config.api_key.as_ref())
        .is_some_and(|value| !value.trim().is_empty());
    json!({"provider":provider,"present":stored || environment || explicit || configured,
        "stored":stored,"environment":environment,"explicit":explicit,
        "modelsCredentialConfigured":configured,
        "authFile":if auth.is_ok() {"valid or absent"} else {"invalid or unreadable"},
        "check":"presence only; no helpers executed, refresh, or network authentication"})
}

fn file_sha256(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Some(format!("{:x}", hash.finalize()))
}

pub fn install_identity(executable: &Path) -> Value {
    let mut identity_path = executable.as_os_str().to_os_string();
    identity_path.push(".identity.json");
    let path = std::path::PathBuf::from(identity_path);
    let binary_hash = file_sha256(executable);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => {
            return json!({"executable":executable,"identity":"absent or unreadable",
            "binarySha256":binary_hash,"source":"unknown"})
        }
    };
    let Ok(identity) = serde_json::from_str::<Value>(&raw) else {
        return json!({"executable":executable,"identity":"invalid","binarySha256":binary_hash});
    };
    let schema = identity["schema"]
        .as_u64()
        .or(identity["schema_version"].as_u64())
        .or(identity["schemaVersion"].as_u64());
    let source = identity["source_sha"]
        .as_str()
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let expected = identity["binary_sha256"]
        .as_str()
        .filter(|sha| sha.len() == 64 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let matches = expected
        .zip(binary_hash.as_deref())
        .map(|(expected, actual)| expected.eq_ignore_ascii_case(actual));
    let metadata_supported = schema == Some(3) && source.is_some() && expected.is_some();
    let status = if matches == Some(false) {
        "invalid"
    } else if metadata_supported {
        "unverified"
    } else {
        "incomplete or unsupported"
    };
    json!({"executable":executable,"identity":status,
        "schema":schema,"sourceSha":source,"sourceClean":identity["source_clean"].as_bool(),
        "releaseTag":identity["release_tag"].as_str().map(|tag| tag.chars().filter(|c| !c.is_control()).take(80).collect::<String>()),
        "ciRunRecorded":!identity["ci_run"].is_null(),
        "releaseProvenance":"unverified; recorded tag and CI metadata are not independently validated",
        "binarySha256":binary_hash,"binaryHashMatches":matches})
}

pub fn doctor(agent: &Agent, agent_dir: &Path, explicit_credential: bool) -> Value {
    let native = crate::native_extensions::background_usage::lsp_status(agent);
    let lsp = native.as_ref();
    let mcp = agent
        .tool_context
        .mcp
        .rows()
        .into_iter()
        .map(|row| {
            json!({"name":row.name,"transport":row.transport,"status":row.status,
            "tools":row.tools,"hasError":row.error.is_some()})
        })
        .collect::<Vec<_>>();
    let lsp_sessions = lsp
        .and_then(|lsp| lsp["sessions"].as_array())
        .map(|sessions| {
            sessions
                .iter()
                .map(|session| {
                    json!({"language":session["language"],
            "state":session["session"],"hasError":!session["lastError"].is_null()})
                })
                .collect::<Vec<_>>()
        });
    let lsp_errors = lsp.map(|lsp| {
        !lsp["configurationError"].is_null()
            || lsp["profileErrors"]
                .as_object()
                .is_some_and(|errors| !errors.is_empty())
            || lsp["profileErrors"]
                .as_array()
                .is_some_and(|errors| !errors.is_empty())
    });
    json!({
        "configuration":configuration(agent_dir, &agent.cwd),
        "securityConfiguration":if crate::settings::load_security_scan_config(agent_dir, &agent.cwd, true).is_ok() {"valid"} else {"invalid"},
        "credentials":credential_presence(agent_dir, &agent.provider, explicit_credential),
        "sandbox":crate::sandbox_config::format_sandbox_status(agent.tool_context.sandbox.as_ref()),
        "mcp":{"servers":mcp,"check":"observed runtime state; no connections started"},
        "lsp":{"enabled":lsp.map(|lsp| &lsp["enabled"]),"configurationErrors":lsp_errors,
            "sessions":lsp_sessions,"check":"observed runtime state; no servers or probes started"},
        "install":std::env::current_exe().ok().map(|path| install_identity(&path)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_configuration_and_credentials_never_echo_values_or_run_helpers() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("SHOULD_NOT_EXIST");
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"theme":123,"other":"secret-setting-value"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("auth.json"),
            json!({"openai":{"type":"api_key","key":format!("!touch {} secret-key-value", marker.display())}}).to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("models.json"),
            json!({"providers":{"openai":{
                "apiKey":format!("!touch {} secret-model-value", marker.display())
            }}})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("mcp.json"),
            r#"{"mcpServers":{"bad":{"headers":{"Authorization":"secret-header-value"}}}}"#,
        )
        .unwrap();
        let rows = configuration(dir.path(), dir.path());
        assert_eq!(rows[0]["status"], "invalid");
        assert!(rows.iter().any(|row| row["status"] == "invalid"));
        let credentials = credential_presence(dir.path(), "openai", false);
        assert_eq!(credentials["stored"], true);
        let rendered = json!({"config":rows,"credentials":credentials}).to_string();
        for secret in [
            "secret-setting-value",
            "secret-key-value",
            "secret-header-value",
            "secret-model-value",
        ] {
            assert!(!rendered.contains(secret));
        }
        assert!(!marker.exists());
    }

    #[test]
    fn install_identity_verifies_the_executable_hash_and_distinguishes_missing_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("davinci");
        std::fs::write(&executable, b"fixture binary").unwrap();
        assert_eq!(install_identity(&executable)["source"], "unknown");
        let identity = json!({"schema_version":3,"source_sha":"a".repeat(40),"source_clean":true,
            "release_tag":"v0.0.1","ci_run":"123","binary_sha256":file_sha256(&executable)});
        std::fs::write(
            dir.path().join("davinci.identity.json"),
            identity.to_string(),
        )
        .unwrap();
        assert_eq!(install_identity(&executable)["identity"], "unverified");
        assert_eq!(install_identity(&executable)["binaryHashMatches"], true);
        let mut dirty = identity.clone();
        dirty["source_clean"] = json!(false);
        dirty["release_tag"] = json!("bogus release");
        dirty["ci_run"] = Value::Null;
        std::fs::write(dir.path().join("davinci.identity.json"), dirty.to_string()).unwrap();
        let report = install_identity(&executable);
        assert_eq!(report["identity"], "unverified");
        assert_eq!(report["sourceClean"], false);
        assert_eq!(report["ciRunRecorded"], false);
        assert!(report["releaseProvenance"]
            .as_str()
            .unwrap()
            .contains("unverified"));
        std::fs::write(&executable, b"changed binary").unwrap();
        assert_eq!(install_identity(&executable)["binaryHashMatches"], false);
        assert_eq!(install_identity(&executable)["identity"], "invalid");
    }
}
