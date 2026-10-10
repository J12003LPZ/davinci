use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

use crate::{Error, Result, TransportConfig};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, ServerConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpExecutionPolicy {
    Host,
    Sandboxed,
    Remote,
    Disabled,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub disabled: bool,
    /// Where this MCP server is allowed to execute. Local commands default
    /// to sandboxed; HTTP endpoints default to remote. The coding-agent host
    /// may explicitly preserve legacy trusted user configuration as host.
    #[serde(default)]
    pub execution: Option<McpExecutionPolicy>,
    /// Explicit local attestation that this server's read-only annotations may
    /// authorize tools. Discovery and project trust do not imply this opt-in.
    #[serde(default, rename = "trustReadOnlyHints")]
    pub trust_read_only_hints: bool,
}

impl ServerConfig {
    pub fn execution_policy(&self) -> Result<McpExecutionPolicy> {
        if self.disabled || self.execution == Some(McpExecutionPolicy::Disabled) {
            return Ok(McpExecutionPolicy::Disabled);
        }
        match (&self.url, &self.command, self.execution) {
            (Some(_), _, None | Some(McpExecutionPolicy::Remote)) => Ok(McpExecutionPolicy::Remote),
            (Some(_), _, Some(_)) => Err(Error::Protocol(
                "remote MCP server execution must be `remote` or `disabled`".into(),
            )),
            (None, Some(_), None | Some(McpExecutionPolicy::Sandboxed)) => {
                Ok(McpExecutionPolicy::Sandboxed)
            }
            (None, Some(_), Some(McpExecutionPolicy::Host)) => Ok(McpExecutionPolicy::Host),
            (None, Some(_), Some(McpExecutionPolicy::Remote)) => Err(Error::Protocol(
                "local MCP command cannot use remote execution policy".into(),
            )),
            (None, Some(_), Some(McpExecutionPolicy::Disabled)) => Ok(McpExecutionPolicy::Disabled),
            (None, None, _) => Err(Error::Protocol("server needs `command` or `url`".into())),
        }
    }

    pub fn transport(&self) -> Result<TransportConfig> {
        let expand = |map: &BTreeMap<String, String>| {
            map.iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        expand_env(value, |name| std::env::var(name).ok()),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        };
        if let Some(url) = &self.url {
            return Ok(TransportConfig::Http {
                url: url.clone(),
                headers: expand(&self.headers),
            });
        }
        let command = self
            .command
            .clone()
            .ok_or_else(|| Error::Protocol("server needs `command` or `url`".into()))?;
        Ok(TransportConfig::Stdio {
            command,
            args: self.args.clone(),
            env: expand(&self.env),
        })
    }
}

/// `${NAME}` becomes the parent's value of NAME (empty when unset). A bare
/// `$NAME` is left alone, so values that legitimately contain `$` survive.
pub fn expand_env(value: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start + 2..].find('}') else {
            break;
        };
        out.push_str(&rest[..start]);
        let name = &rest[start + 2..start + 2 + len];
        out.push_str(&lookup(name).unwrap_or_default());
        rest = &rest[start + 2 + len + 1..];
    }
    out.push_str(rest);
    out
}

/// A missing file is an empty configuration. Any other read failure
/// (permissions, a directory in its place) is an error: `Path::exists`
/// reports those as "missing" too, which silently disabled every server.
pub fn load_path(path: &Path) -> Result<File> {
    match std::fs::read_to_string(path) {
        Ok(body) => parse(&body),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(File::default()),
        Err(err) => Err(Error::Protocol(format!("{}: {err}", path.display()))),
    }
}

pub fn parse(body: &str) -> Result<File> {
    let value: Value =
        serde_json::from_str(body).map_err(|err| Error::Protocol(format!("mcp.json: {err}")))?;
    serde_json::from_value(value).map_err(|err| Error::Protocol(format!("mcp.json: {err}")))
}

/// Later names win.
pub fn merge(user: File, project: File) -> File {
    let mut servers = user.mcp_servers;
    servers.extend(project.mcp_servers);
    File {
        mcp_servers: servers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_is_empty_but_unreadable_config_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_path(&dir.path().join("absent.json"))
            .unwrap()
            .mcp_servers
            .is_empty());
        // A directory where the file should be cannot be read on any OS;
        // `exists()` called it present-but-fine before and empty after.
        let blocked = dir.path().join("mcp.json");
        std::fs::create_dir(&blocked).unwrap();
        let err = load_path(&blocked).unwrap_err();
        assert!(err.to_string().contains("mcp.json"), "{err}");
    }

    #[test]
    fn a_stdio_and_http_server_parse() {
        let file = parse(
            r#"{
              "mcpServers": {
                "memory": { "command": "npx", "args": ["-y", "x"], "env": { "A": "1" } },
                "docs": { "url": "https://example.com/mcp", "headers": { "K": "V" } },
                "off": { "command": "x", "disabled": true }
              }
            }"#,
        )
        .unwrap();
        assert_eq!(file.mcp_servers.len(), 3);
        let memory = &file.mcp_servers["memory"];
        match memory.transport().unwrap() {
            TransportConfig::Stdio { command, args, env } => {
                assert_eq!(command, "npx");
                assert_eq!(args, vec!["-y", "x"]);
                assert_eq!(env.get("A").map(String::as_str), Some("1"));
            }
            other => panic!("{other:?}"),
        }
        assert!(file.mcp_servers["off"].disabled);
        match file.mcp_servers["docs"].transport().unwrap() {
            TransportConfig::Http { url, headers } => {
                assert_eq!(url, "https://example.com/mcp");
                assert_eq!(headers.get("K").map(String::as_str), Some("V"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn project_overrides_user_by_name() {
        let user = parse(r#"{"mcpServers":{"a":{"command":"u"},"b":{"command":"u"}}}"#).unwrap();
        let project = parse(r#"{"mcpServers":{"b":{"url":"https://p"}}}"#).unwrap();
        let merged = merge(user, project);
        assert!(merged.mcp_servers["a"].command.is_some());
        assert_eq!(merged.mcp_servers["b"].url.as_deref(), Some("https://p"));
    }

    #[test]
    fn local_execution_policy_defaults_sandboxed_and_transport_is_explicit() {
        let file = parse(
            r#"{"mcpServers":{
                "local":{"command":"tool"},
                "host":{"command":"tool","execution":"host"},
                "remote":{"url":"https://example.com/mcp"},
                "off":{"command":"tool","execution":"disabled"}
            }}"#,
        )
        .unwrap();
        assert_eq!(
            file.mcp_servers["local"].execution_policy().unwrap(),
            McpExecutionPolicy::Sandboxed
        );
        assert_eq!(
            file.mcp_servers["host"].execution_policy().unwrap(),
            McpExecutionPolicy::Host
        );
        assert_eq!(
            file.mcp_servers["remote"].execution_policy().unwrap(),
            McpExecutionPolicy::Remote
        );
        assert_eq!(
            file.mcp_servers["off"].execution_policy().unwrap(),
            McpExecutionPolicy::Disabled
        );
    }

    #[test]
    fn execution_policy_cannot_mismatch_transport() {
        let remote_host =
            parse(r#"{"mcpServers":{"x":{"url":"https://example.com","execution":"host"}}}"#)
                .unwrap();
        assert!(remote_host.mcp_servers["x"].execution_policy().is_err());

        let local_remote =
            parse(r#"{"mcpServers":{"x":{"command":"tool","execution":"remote"}}}"#).unwrap();
        assert!(local_remote.mcp_servers["x"].execution_policy().is_err());
    }

    #[test]
    fn env_values_expand_parent_variables() {
        let lookup = |name: &str| (name == "GITHUB_TOKEN").then(|| "ghp_x".to_string());
        assert_eq!(expand_env("${GITHUB_TOKEN}", lookup), "ghp_x");
        assert_eq!(
            expand_env("Bearer ${GITHUB_TOKEN}!", lookup),
            "Bearer ghp_x!"
        );
        assert_eq!(expand_env("${MISSING}", lookup), "");
        assert_eq!(expand_env("plain $HOME", lookup), "plain $HOME");
    }
}
