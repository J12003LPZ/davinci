use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

use crate::extensions::{discover_extensions, ExtensionManifest};
use crate::js_host::{
    execute_command_tool, node_available, resolve_extension_module, run_js_extension,
    JsExtensionResult,
};

/// Extension event bus matching `vendor/pi/packages/coding-agent/src/core/extensions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ExtensionEvent {
    #[serde(rename = "before_agent_start")]
    BeforeAgentStart,
    #[serde(rename = "agent_start")]
    AgentStart,
    #[serde(rename = "agent_end")]
    AgentEnd,
    #[serde(rename = "agent_settled")]
    AgentSettled,
    #[serde(rename = "session_start")]
    SessionStart,
    #[serde(rename = "session_shutdown")]
    SessionShutdown,
    #[serde(rename = "session_before_compact")]
    SessionBeforeCompact,
    #[serde(rename = "tool_call")]
    ToolCall {
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    #[serde(rename = "input")]
    Input { text: String },
}

#[derive(Debug, Clone)]
pub struct LoadedJsExtension {
    pub path: String,
    pub handlers: Vec<String>,
    pub tools: Vec<String>,
    pub commands: Vec<String>,
}

#[derive(Debug, Default)]
pub struct ExtensionHost {
    pub events: Vec<ExtensionEvent>,
    pub manifests: Vec<ExtensionManifest>,
    pub js: Vec<LoadedJsExtension>,
    pub last_js_result: Option<Value>,
}

impl ExtensionHost {
    pub fn load(agent_dir: &Path, names: &[String]) -> Self {
        let manifests = discover_extensions(agent_dir, names);
        let mut host = Self {
            events: Vec::new(),
            manifests,
            js: Vec::new(),
            last_js_result: None,
        };
        if node_available() {
            for manifest in &host.manifests {
                let Some(dir) = manifest.path.as_ref().map(Path::new) else {
                    continue;
                };
                let Some(module) = resolve_extension_module(dir) else {
                    continue;
                };
                if let Ok(loaded) = run_js_extension(&module, "load", &serde_json::json!({})) {
                    if loaded.ok {
                        host.js.push(LoadedJsExtension {
                            path: module.display().to_string(),
                            handlers: loaded.handlers,
                            tools: loaded.tools.into_iter().map(|tool| tool.name).collect(),
                            commands: loaded
                                .commands
                                .into_iter()
                                .map(|command| command.name)
                                .collect(),
                        });
                    }
                }
            }
        }
        host
    }

    pub fn emit(&mut self, event: ExtensionEvent) {
        self.dispatch_js(&event);
        self.events.push(event);
    }

    fn dispatch_js(&mut self, event: &ExtensionEvent) {
        let Ok(payload) = serde_json::to_value(event) else {
            return;
        };
        for ext in &self.js {
            if let Ok(result) = run_js_extension(Path::new(&ext.path), "emit", &payload) {
                if result.ok {
                    self.last_js_result = result.result;
                }
            }
        }
    }

    pub fn tool_call_blocked(&self) -> bool {
        self.last_js_result
            .as_ref()
            .and_then(|value| value.get("block"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn execute_named_tool(&self, name: &str, cwd: &Path) -> Option<Result<String, String>> {
        for manifest in &self.manifests {
            for tool in &manifest.tools {
                if tool.name == name {
                    if let Some(command) = &tool.command {
                        return Some(execute_command_tool(command, cwd));
                    }
                }
            }
        }
        None
    }

    pub fn kinds(&self) -> Vec<&'static str> {
        self.events
            .iter()
            .map(|event| match event {
                ExtensionEvent::BeforeAgentStart => "before_agent_start",
                ExtensionEvent::AgentStart => "agent_start",
                ExtensionEvent::AgentEnd => "agent_end",
                ExtensionEvent::AgentSettled => "agent_settled",
                ExtensionEvent::SessionStart => "session_start",
                ExtensionEvent::SessionShutdown => "session_shutdown",
                ExtensionEvent::SessionBeforeCompact => "session_before_compact",
                ExtensionEvent::ToolCall { .. } => "tool_call",
                ExtensionEvent::ToolResult { .. } => "tool_result",
                ExtensionEvent::Input { .. } => "input",
            })
            .collect()
    }

    pub fn js_summary(result: &JsExtensionResult) -> String {
        format!(
            "handlers={} tools={}",
            result.handlers.join(","),
            result
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    pub fn describe_js(&self) -> String {
        self.js
            .iter()
            .map(|ext| {
                format!(
                    "{} handlers={} commands={}",
                    ext.path,
                    ext.handlers.join(","),
                    ext.commands.join(",")
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_match_ts() {
        let mut host = ExtensionHost::default();
        host.emit(ExtensionEvent::BeforeAgentStart);
        host.emit(ExtensionEvent::ToolCall {
            tool_name: "read".into(),
            args: serde_json::json!({}),
        });
        host.emit(ExtensionEvent::SessionStart);
        assert_eq!(
            host.kinds(),
            ["before_agent_start", "tool_call", "session_start"]
        );
    }
}
