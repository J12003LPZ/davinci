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
    pub message_renderers: Vec<String>,
    pub entry_renderers: Vec<String>,
    pub markdown_transformers: u32,
}

#[derive(Debug, Default)]
pub struct ExtensionHost {
    pub events: Vec<ExtensionEvent>,
    pub manifests: Vec<ExtensionManifest>,
    pub js: Vec<LoadedJsExtension>,
    pub last_js_result: Option<Value>,
    pub message_renderers: std::collections::HashMap<String, String>,
    pub entry_renderers: std::collections::HashMap<String, String>,
    pub markdown_modules: Vec<String>,
}

impl ExtensionHost {
    pub fn load(agent_dir: &Path, names: &[String]) -> Self {
        let manifests = discover_extensions(agent_dir, names);
        let mut host = Self {
            events: Vec::new(),
            manifests,
            js: Vec::new(),
            last_js_result: None,
            message_renderers: std::collections::HashMap::new(),
            entry_renderers: std::collections::HashMap::new(),
            markdown_modules: Vec::new(),
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
                        let path = module.display().to_string();
                        for custom_type in &loaded.message_renderers {
                            host.message_renderers
                                .insert(custom_type.clone(), path.clone());
                        }
                        for custom_type in &loaded.entry_renderers {
                            host.entry_renderers
                                .insert(custom_type.clone(), path.clone());
                        }
                        if loaded.markdown_transformers > 0 {
                            host.markdown_modules.push(path.clone());
                        }
                        host.js.push(LoadedJsExtension {
                            path,
                            handlers: loaded.handlers,
                            tools: loaded.tools.into_iter().map(|tool| tool.name).collect(),
                            commands: loaded
                                .commands
                                .into_iter()
                                .map(|command| command.name)
                                .collect(),
                            message_renderers: loaded.message_renderers,
                            entry_renderers: loaded.entry_renderers,
                            markdown_transformers: loaded.markdown_transformers,
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

    pub fn get_message_renderer(&self, custom_type: &str) -> Option<&str> {
        self.message_renderers.get(custom_type).map(String::as_str)
    }

    pub fn render_custom_message(
        &self,
        custom_type: &str,
        content: &str,
        expanded: bool,
        output_pad: usize,
        width: usize,
    ) -> Option<Vec<String>> {
        let module = self.message_renderers.get(custom_type)?;
        let rendered = run_js_extension(
            Path::new(module),
            "renderMessage",
            &serde_json::json!({
                "customType": custom_type,
                "content": content,
                "message": {
                    "role": "custom",
                    "customType": custom_type,
                    "content": content,
                    "display": true
                },
                "options": { "expanded": expanded, "outputPad": output_pad },
                "width": width
            }),
        )
        .ok()?;
        rendered.result?.get("lines")?.as_array().map(|lines| {
            lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_string))
                .collect()
        })
    }

    pub fn get_entry_renderer(&self, custom_type: &str) -> Option<&str> {
        self.entry_renderers.get(custom_type).map(String::as_str)
    }

    pub fn render_custom_entry(
        &self,
        custom_type: &str,
        data: &Value,
        expanded: bool,
        width: usize,
    ) -> Option<Vec<String>> {
        let module = self.entry_renderers.get(custom_type)?;
        let rendered = run_js_extension(
            Path::new(module),
            "renderEntry",
            &serde_json::json!({
                "customType": custom_type,
                "entry": {
                    "type": "custom",
                    "customType": custom_type,
                    "data": data
                },
                "options": { "expanded": expanded },
                "width": width
            }),
        )
        .ok()?;
        rendered.result?.get("lines")?.as_array().map(|lines| {
            lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_string))
                .collect()
        })
    }

    pub fn transform_markdown(
        &self,
        markdown: &str,
        message_type: &str,
        is_streaming: bool,
        width: usize,
    ) -> String {
        let mut text = markdown.to_string();
        for module in &self.markdown_modules {
            if let Ok(result) = run_js_extension(
                Path::new(module),
                "transformMarkdown",
                &serde_json::json!({
                    "markdown": text,
                    "context": {
                        "messageType": message_type,
                        "isStreaming": is_streaming,
                        "availableWidth": width
                    },
                    "width": width
                }),
            ) {
                if let Some(next) = result
                    .result
                    .as_ref()
                    .and_then(|value| value.get("markdown"))
                    .and_then(Value::as_str)
                {
                    text = next.to_string();
                }
            }
        }
        text
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
                    "{} handlers={} commands={} renderers={} entries={} md={}",
                    ext.path,
                    ext.handlers.join(","),
                    ext.commands.join(","),
                    ext.message_renderers.join(","),
                    ext.entry_renderers.join(","),
                    ext.markdown_transformers
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
