//! Immutable server-to-client request policy for language servers.

use super::servers::{ServerCommand, ServerKind};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub(super) struct ClientRequestConfig {
    pub workspace_folders: Vec<Value>,
    pub sections: BTreeMap<String, Value>,
}

impl ClientRequestConfig {
    pub(super) fn for_command(command: &ServerCommand, workspace_uri: &str) -> Self {
        let mut sections = BTreeMap::new();
        match command.kind {
            ServerKind::RustAnalyzer => {
                sections.insert("rust-analyzer".into(), command.initialization_options.clone());
                if let Some(value) = command.initialization_options.get("cargo") {
                    sections.insert("rust-analyzer.cargo".into(), value.clone());
                }
                if let Some(value) = command.initialization_options.get("procMacro") {
                    sections.insert("rust-analyzer.procMacro".into(), value.clone());
                }
                if let Some(value) = command.initialization_options.get("checkOnSave") {
                    sections.insert("rust-analyzer.checkOnSave".into(), value.clone());
                }
            }
            ServerKind::Basedpyright => {
                if let Some(value) = command.initialization_options.get("python") {
                    sections.insert("python".into(), value.clone());
                }
                sections.insert(
                    "basedpyright".into(),
                    json!({"analysis":command.initialization_options.get("analysis").cloned().unwrap_or(Value::Null)}),
                );
            }
            ServerKind::Pyright => {
                if let Some(value) = command.initialization_options.get("python") {
                    sections.insert("python".into(), value.clone());
                }
                sections.insert(
                    "pyright".into(),
                    json!({"analysis":command.initialization_options.get("analysis").cloned().unwrap_or(Value::Null)}),
                );
            }
            ServerKind::TypeScriptNative | ServerKind::TypeScriptLanguageServer => {}
        }
        Self {
            workspace_folders: vec![json!({
                "uri": workspace_uri,
                "name": command
                    .workspace
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("project")
            })],
            sections,
        }
    }

    pub(super) fn configuration_response(&self, params: &Value) -> Option<Vec<Value>> {
        let items = params.get("items")?.as_array()?;
        if items.len() > 64 {
            return None;
        }
        Some(
            items
                .iter()
                .map(|item| {
                    let scope_allowed = item
                        .get("scopeUri")
                        .and_then(Value::as_str)
                        .is_none_or(|scope| {
                            self.workspace_folders
                                .iter()
                                .filter_map(|folder| folder.get("uri").and_then(Value::as_str))
                                .any(|root| scope.starts_with(root))
                        });
                    if !scope_allowed {
                        return Value::Null;
                    }
                    item.get("section")
                        .and_then(Value::as_str)
                        .and_then(|section| self.sections.get(section))
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::language_intelligence::servers::ServerCommand;
    use std::path::PathBuf;

    #[test]
    fn configuration_reply_preserves_scope_and_order() {
        let command = ServerCommand {
            kind: ServerKind::Basedpyright,
            program: PathBuf::from("server"),
            args: Vec::new(),
            workspace: PathBuf::from("/fixture/service"),
            version: None,
            typescript_version: None,
            initialization_options: json!({
                "python":{"pythonPath":"/fixture/service/.venv/bin/python"},
                "analysis":{"diagnosticMode":"openFilesOnly","baselineMode":"discard"}
            }),
        };
        let config = ClientRequestConfig::for_command(&command, "file:///fixture/service");
        let response = config
            .configuration_response(&json!({"items":[
                {"scopeUri":"file:///fixture/service/app.py","section":"python"},
                {"scopeUri":"file:///fixture/service/app.py","section":"basedpyright"},
                {"scopeUri":"file:///outside/secret.py","section":"python"},
                {"section":"unknown"}
            ]}))
            .unwrap();
        assert_eq!(response.len(), 4);
        assert_eq!(response[0]["pythonPath"], "/fixture/service/.venv/bin/python");
        assert_eq!(response[1]["analysis"]["baselineMode"], "discard");
        assert!(response[2].is_null());
        assert!(response[3].is_null());
    }
}
