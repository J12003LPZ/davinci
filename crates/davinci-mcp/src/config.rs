use serde::Deserialize;
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
    /// Explicit local attestation that this server's read-only annotations may
    /// authorize tools. Discovery and project trust do not imply this opt-in.
    #[serde(default, rename = "trustReadOnlyHints")]
    pub trust_read_only_hints: bool,
}

impl ServerConfig {
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
fn expand_env(value: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
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

pub fn load_path(path: &Path) -> Result<File> {
    if !path.exists() {
        return Ok(File::default());
    }
    let body = std::fs::read_to_string(path)?;
    parse(&body)
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
