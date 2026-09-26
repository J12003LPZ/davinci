//! Load `mcp.json` for the native MCP client.
//!
//! User agent-directory config, then a trusted project's `.davinci/mcp.json`.
//! Legacy `.pi/mcp.json` is used only when no DaVinci project file exists.
//! Enabled plugins' servers (`plugin_<plugin>_<server>`) are the base layer;
//! a user or project entry with the same name wins.

use std::path::Path;

use davinci_mcp::ConfigFile;

pub fn load(agent_dir: &Path, cwd: &Path, trusted: bool) -> ConfigFile {
    if let Ok(path) =
        std::env::var("DAVINCI_MCP_CONFIG").or_else(|_| std::env::var("PI_MCP_CONFIG"))
    {
        return davinci_mcp::load_path(Path::new(&path)).unwrap_or_default();
    }
    let plugins = ConfigFile {
        mcp_servers: davinci_coding_agent::plugins::active(agent_dir).mcp_servers(),
    };
    let user = davinci_mcp::merge(
        plugins,
        davinci_mcp::load_path(&agent_dir.join("mcp.json")).unwrap_or_default(),
    );
    if !trusted {
        return user;
    }
    let Some(path) = crate::project_config::resolve(cwd, "mcp.json") else {
        return user;
    };
    let project = davinci_mcp::load_path(&path).unwrap_or_default();
    davinci_mcp::merge(user, project)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let loaded = load(Path::new("/nope"), Path::new("/nope"), true);
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
        let untrusted = load(&agent_dir, &project, false);
        assert!(untrusted.mcp_servers.contains_key("user"));
        assert!(!untrusted.mcp_servers.contains_key("project"));
        assert_eq!(untrusted.mcp_servers["user"].command.as_deref(), Some("u"));
        let trusted = load(&agent_dir, &project, true);
        assert_eq!(trusted.mcp_servers["user"].command.as_deref(), Some("over"));
        assert!(trusted.mcp_servers.contains_key("project"));

        // An explicit empty DaVinci config must not resurrect legacy servers.
        std::fs::create_dir(project.join(".davinci")).unwrap();
        std::fs::write(project.join(".davinci/mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
        let current = load(&agent_dir, &project, true);
        assert!(!current.mcp_servers.contains_key("project"));
        assert_eq!(current.mcp_servers["user"].command.as_deref(), Some("u"));
    }
}
