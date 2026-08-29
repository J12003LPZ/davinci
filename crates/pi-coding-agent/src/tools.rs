use async_trait::async_trait;
use pi_agent::agent::{AgentTool, AgentToolResult};
use pi_ai::types::UserContent;
use serde_json::json;

pub struct ReadTool;

#[async_trait]
impl AgentTool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read file contents"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "offset": { "type": "number" },
                "limit": { "type": "number" }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
    ) -> Result<AgentToolResult, String> {
        let path = params
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| "Missing path parameter".to_string())?;

        match std::fs::read_to_string(path) {
            Ok(content) => Ok(AgentToolResult {
                content: vec![UserContent::Text(pi_ai::types::TextContent {
                    content_type: "text".to_string(),
                    text: content,
                    text_signature: None,
                })],
                details: json!({ "path": path }),
                usage: None,
                added_tool_names: None,
                terminate: None,
            }),
            Err(e) => Err(format!("Failed to read file {}: {}", path, e)),
        }
    }
}

pub struct WriteTool;

#[async_trait]
impl AgentTool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Create or overwrite files"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
    ) -> Result<AgentToolResult, String> {
        let path = params
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| "Missing path parameter".to_string())?;
        let content = params
            .get("content")
            .and_then(|c| c.as_str())
            .ok_or_else(|| "Missing content parameter".to_string())?;

        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        match std::fs::write(path, content) {
            Ok(_) => Ok(AgentToolResult {
                content: vec![UserContent::Text(pi_ai::types::TextContent {
                    content_type: "text".to_string(),
                    text: format!("Successfully wrote {} bytes to {}", content.len(), path),
                    text_signature: None,
                })],
                details: json!({ "path": path, "bytes": content.len() }),
                usage: None,
                added_tool_names: None,
                terminate: None,
            }),
            Err(e) => Err(format!("Failed to write file {}: {}", path, e)),
        }
    }
}

pub struct EditTool;

#[async_trait]
impl AgentTool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Make precise file edits"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "oldText": { "type": "string" },
                            "newText": { "type": "string" }
                        },
                        "required": ["oldText", "newText"]
                    }
                }
            },
            "required": ["path", "edits"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
    ) -> Result<AgentToolResult, String> {
        let path = params
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| "Missing path parameter".to_string())?;
        let mut content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read file {}: {}", path, e))?;

        if let Some(edits) = params.get("edits").and_then(|e| e.as_array()) {
            for edit in edits {
                let old_text = edit.get("oldText").and_then(|o| o.as_str()).unwrap_or("");
                let new_text = edit.get("newText").and_then(|n| n.as_str()).unwrap_or("");
                if !content.contains(old_text) {
                    return Err(format!("oldText not found in {}", path));
                }
                content = content.replacen(old_text, new_text, 1);
            }
        }

        std::fs::write(path, &content)
            .map_err(|e| format!("Failed to write modified file {}: {}", path, e))?;

        Ok(AgentToolResult {
            content: vec![UserContent::Text(pi_ai::types::TextContent {
                content_type: "text".to_string(),
                text: format!("Successfully edited {}", path),
                text_signature: None,
            })],
            details: json!({ "path": path }),
            usage: None,
            added_tool_names: None,
            terminate: None,
        })
    }
}

pub struct BashTool;

#[async_trait]
impl AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute bash command"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" }
            },
            "required": ["command"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
    ) -> Result<AgentToolResult, String> {
        let cmd = params
            .get("command")
            .and_then(|c| c.as_str())
            .ok_or_else(|| "Missing command parameter".to_string())?;

        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(cmd)
            .output()
            .map_err(|e| format!("Failed to execute command: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let combined = if stderr.is_empty() {
            stdout
        } else {
            format!("{}\n{}", stdout, stderr)
        };

        Ok(AgentToolResult {
            content: vec![UserContent::Text(pi_ai::types::TextContent {
                content_type: "text".to_string(),
                text: combined,
                text_signature: None,
            })],
            details: json!({ "exit_code": output.status.code() }),
            usage: None,
            added_tool_names: None,
            terminate: None,
        })
    }
}
