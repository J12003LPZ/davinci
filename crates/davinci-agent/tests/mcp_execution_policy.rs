//! WOR-168: the public registry routes on `ServerConfig::execution_policy()`,
//! so raw SDK config cannot reach a direct host spawn the policy forbids.
use davinci_agent::McpRegistry;
use davinci_mcp::{ConfigFile, McpExecutionPolicy, ServerConfig};
use std::collections::BTreeMap;
use std::path::Path;

const MARKER_ENV: &str = "DAVINCI_MCP_POLICY_MARKER";

/// Disposable child: writes the marker only when launched by a test below.
#[test]
fn mcp_policy_marker_child() {
    if let Some(marker) = std::env::var_os(MARKER_ENV) {
        std::fs::write(marker, b"host process executed").unwrap();
    }
}

fn marker_server(marker: &Path, execution: Option<McpExecutionPolicy>) -> ServerConfig {
    ServerConfig {
        command: Some(std::env::current_exe().unwrap().to_string_lossy().into()),
        args: vec!["--exact".into(), "mcp_policy_marker_child".into()],
        env: BTreeMap::from([(MARKER_ENV.into(), marker.to_string_lossy().into_owned())]),
        execution,
        ..Default::default()
    }
}

fn single(server: ServerConfig) -> ConfigFile {
    ConfigFile {
        mcp_servers: BTreeMap::from([("local".into(), server)]),
    }
}

#[test]
fn default_sandbox_policy_never_spawns_direct_host_command() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("host-executed.txt");
    let server = marker_server(&marker, None);
    assert_eq!(
        server.execution_policy().unwrap(),
        McpExecutionPolicy::Sandboxed
    );
    let registry = McpRegistry::connect(&single(server), root.path());
    assert!(
        !marker.exists(),
        "default sandbox policy executed on the host"
    );
    let rows = registry.rows();
    assert_eq!(rows[0].transport, "sandboxed-stdio");
    assert_eq!(rows[0].status, "error");
    assert!(rows[0]
        .error
        .as_deref()
        .is_some_and(|error| error.contains("refusing direct host spawn")));
}

#[test]
fn remote_policy_on_a_local_command_is_refused_before_spawn() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("host-executed.txt");
    let server = marker_server(&marker, Some(McpExecutionPolicy::Remote));
    let registry = McpRegistry::connect(&single(server), root.path());
    assert!(!marker.exists(), "invalid policy still spawned the command");
    let rows = registry.rows();
    assert_eq!(rows[0].status, "error");
    assert!(rows[0]
        .error
        .as_deref()
        .is_some_and(|error| error.contains("remote execution policy")));
}

#[test]
fn sandboxed_policy_on_a_url_is_refused_before_connecting() {
    let server = ServerConfig {
        url: Some("http://127.0.0.1:9/mcp".into()),
        execution: Some(McpExecutionPolicy::Sandboxed),
        ..Default::default()
    };
    let registry = McpRegistry::connect(&single(server), Path::new("."));
    let rows = registry.rows();
    assert_eq!(rows[0].transport, "http");
    assert_eq!(rows[0].status, "error");
    assert!(rows[0]
        .error
        .as_deref()
        .is_some_and(|error| error.contains("must be `remote` or `disabled`")));
}

#[test]
fn explicit_host_policy_keeps_the_trusted_legacy_direct_spawn() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("host-executed.txt");
    let server = marker_server(&marker, Some(McpExecutionPolicy::Host));
    let registry = McpRegistry::connect(&single(server), root.path());
    // The fixture is not an MCP server, so the handshake fails; the marker
    // proves the explicit Host route still launched it directly.
    assert!(marker.exists(), "explicit host policy no longer spawns");
    assert_eq!(registry.rows()[0].transport, "stdio");
}
