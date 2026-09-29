//! Load `mcp.json` for the native MCP client.
//!
//! User agent-directory config, then a trusted project's `.davinci/mcp.json`.
//! Legacy `.pi/mcp.json` is used only when no DaVinci project file exists.
//! Enabled plugins' servers (`plugin_<plugin>_<server>`) are the base layer;
//! a user or project entry with the same name wins.

use std::path::Path;

use davinci_mcp::{ConfigFile, McpExecutionPolicy};

/// Where each server may run, from where it was configured.
///
/// With an execution sandbox active, project and plugin local commands always
/// run sandboxed: a project cannot promote its own command to the host.
/// Without one (the default, compatibility mode) nothing is OS-isolated, so
/// they keep running on the host as before; only a server that explicitly
/// asks for `sandboxed` is refused rather than silently run unsandboxed.
fn resolve_origin(config: &mut ConfigFile, project_or_plugin: bool, sandbox_active: bool) {
    for server in config.mcp_servers.values_mut() {
        if server.disabled || server.execution == Some(McpExecutionPolicy::Disabled) {
            server.execution = Some(McpExecutionPolicy::Disabled);
        } else if server.url.is_some() {
            server.execution = Some(McpExecutionPolicy::Remote);
        } else if server.command.is_some() {
            if project_or_plugin && sandbox_active {
                server.execution = Some(McpExecutionPolicy::Sandboxed);
            } else if project_or_plugin {
                if server.execution != Some(McpExecutionPolicy::Sandboxed) {
                    server.execution = Some(McpExecutionPolicy::Host);
                }
            } else if server.execution.is_none() {
                server.execution = Some(McpExecutionPolicy::Host);
            }
        }
    }
}

pub fn load(agent_dir: &Path, cwd: &Path, trusted: bool, sandbox_active: bool) -> ConfigFile {
    if let Ok(path) =
        std::env::var("DAVINCI_MCP_CONFIG").or_else(|_| std::env::var("PI_MCP_CONFIG"))
    {
        let mut config = davinci_mcp::load_path(Path::new(&path)).unwrap_or_default();
        resolve_origin(&mut config, false, sandbox_active);
        return config;
    }
    let mut plugins = ConfigFile {
        mcp_servers: davinci_coding_agent::plugins::active(agent_dir).mcp_servers(),
    };
    resolve_origin(&mut plugins, true, sandbox_active);
    let mut user_file = davinci_mcp::load_path(&agent_dir.join("mcp.json")).unwrap_or_default();
    resolve_origin(&mut user_file, false, sandbox_active);
    let user = davinci_mcp::merge(plugins, user_file);
    if !trusted {
        return user;
    }
    let Some(path) = crate::project_config::resolve(cwd, "mcp.json") else {
        return user;
    };
    let mut project = davinci_mcp::load_path(&path).unwrap_or_default();
    resolve_origin(&mut project, true, sandbox_active);
    davinci_mcp::merge(user, project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_execution_provenance_never_lets_project_or_plugin_local_commands_run_on_host() {
        let dir = tempfile::tempdir().unwrap();
        let agent_dir = dir.path().join("agent");
        let project = dir.path().join("project");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::create_dir_all(project.join(".davinci")).unwrap();
        std::fs::write(
            agent_dir.join("mcp.json"),
            r#"{"mcpServers":{
                "legacy-user":{"command":"user-tool"},
                "explicit-user-sandbox":{"command":"user-safe","execution":"sandboxed"}
            }}"#,
        )
        .unwrap();
        std::fs::write(
            project.join(".davinci/mcp.json"),
            r#"{"mcpServers":{
                "project":{"command":"project-tool","execution":"host"},
                "remote":{"url":"https://example.com/mcp","execution":"remote"}
            }}"#,
        )
        .unwrap();

        let loaded = load(&agent_dir, &project, true, true);
        assert_eq!(
            loaded.mcp_servers["legacy-user"].execution,
            Some(davinci_mcp::McpExecutionPolicy::Host)
        );
        assert_eq!(
            loaded.mcp_servers["explicit-user-sandbox"].execution,
            Some(davinci_mcp::McpExecutionPolicy::Sandboxed)
        );
        assert_eq!(
            loaded.mcp_servers["project"].execution,
            Some(davinci_mcp::McpExecutionPolicy::Sandboxed),
            "a project must not promote its local MCP command to host execution"
        );
        assert_eq!(
            loaded.mcp_servers["remote"].execution,
            Some(davinci_mcp::McpExecutionPolicy::Remote)
        );
    }

    #[test]
    fn without_a_sandbox_project_and_plugin_servers_keep_running_on_the_host() {
        // Compatibility mode claims no OS isolation, so forcing `sandboxed`
        // here would only refuse every project and plugin server.
        let dir = tempfile::tempdir().unwrap();
        let agent_dir = dir.path().join("agent");
        let project = dir.path().join("project");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::create_dir_all(project.join(".davinci")).unwrap();
        std::fs::write(
            project.join(".davinci/mcp.json"),
            r#"{"mcpServers":{
                "project":{"command":"project-tool"},
                "asks-for-sandbox":{"command":"project-tool","execution":"sandboxed"}
            }}"#,
        )
        .unwrap();
        let loaded = load(&agent_dir, &project, true, false);
        assert_eq!(
            loaded.mcp_servers["project"].execution,
            Some(davinci_mcp::McpExecutionPolicy::Host)
        );
        assert_eq!(
            loaded.mcp_servers["asks-for-sandbox"].execution,
            Some(davinci_mcp::McpExecutionPolicy::Sandboxed),
            "an explicit sandbox request is refused, never silently run on the host"
        );
    }

    #[test]
    fn pi_mcp_config_wins_and_an_untrusted_project_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture.json");
        std::fs::write(
            &fixture,
            r#"{"mcpServers":{"from-env":{"command":"echo"}}}"#,
        )
        .unwrap();
        std::env::set_var("PI_MCP_CONFIG", &fixture);
        let loaded = load(Path::new("/nope"), Path::new("/nope"), true, false);
        std::env::remove_var("PI_MCP_CONFIG");
        assert!(loaded.mcp_servers.contains_key("from-env"));

        let agent_dir = dir.path().join("agent");
        let project = dir.path().join("proj");
        std::fs::create_dir_all(agent_dir.join("x")).unwrap();
        std::fs::create_dir_all(project.join(".pi")).unwrap();
        std::fs::write(
            agent_dir.join("mcp.json"),
            r#"{"mcpServers":{"user":{"command":"u"}}}"#,
        )
        .unwrap();
        std::fs::write(
            project.join(".pi").join("mcp.json"),
            r#"{"mcpServers":{"project":{"command":"p"},"user":{"command":"over"}}}"#,
        )
        .unwrap();
        let untrusted = load(&agent_dir, &project, false, false);
        assert!(untrusted.mcp_servers.contains_key("user"));
        assert!(!untrusted.mcp_servers.contains_key("project"));
        assert_eq!(untrusted.mcp_servers["user"].command.as_deref(), Some("u"));
        let trusted = load(&agent_dir, &project, true, false);
        assert_eq!(trusted.mcp_servers["user"].command.as_deref(), Some("over"));
        assert!(trusted.mcp_servers.contains_key("project"));

        // An explicit empty DaVinci config must not resurrect legacy servers.
        std::fs::create_dir(project.join(".davinci")).unwrap();
        std::fs::write(project.join(".davinci/mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
        let current = load(&agent_dir, &project, true, false);
        assert!(!current.mcp_servers.contains_key("project"));
        assert_eq!(current.mcp_servers["user"].command.as_deref(), Some("u"));
    }
}
