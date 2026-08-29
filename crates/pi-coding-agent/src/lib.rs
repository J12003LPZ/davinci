use async_trait::async_trait;
use pi_agent::{AgentError, AgentTool, Result};
use pi_ai::ToolDefinition;
use serde::Deserialize;
use std::path::PathBuf;

pub struct ReadFileTool {
    pub root_dir: PathBuf,
}

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
}

#[async_trait]
impl AgentTool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read contents of a file at relative path".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path to read" }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<String> {
        let parsed: ReadFileArgs = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("Invalid read_file arguments: {}", e)))?;

        let target_path = self.root_dir.join(&parsed.path);
        tokio::fs::read_to_string(&target_path)
            .await
            .map_err(|e| AgentError::Tool(format!("Failed to read file {}: {}", parsed.path, e)))
    }
}

pub struct WriteFileTool {
    pub root_dir: PathBuf,
}

#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}

#[async_trait]
impl AgentTool for WriteFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Write content to a file at relative path".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path to write" },
                    "content": { "type": "string", "description": "File content" }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<String> {
        let parsed: WriteFileArgs = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("Invalid write_file arguments: {}", e)))?;

        let target_path = self.root_dir.join(&parsed.path);
        if let Some(parent) = target_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }

        tokio::fs::write(&target_path, parsed.content)
            .await
            .map_err(|e| {
                AgentError::Tool(format!("Failed to write file {}: {}", parsed.path, e))
            })?;

        Ok(format!(
            "Successfully wrote {} bytes to {}",
            target_path.display(),
            parsed.path
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_read_and_write_file_tools() {
        let temp_dir = tempdir().unwrap();
        let write_tool = WriteFileTool {
            root_dir: temp_dir.path().to_path_buf(),
        };
        let read_tool = ReadFileTool {
            root_dir: temp_dir.path().to_path_buf(),
        };

        let write_args = serde_json::json!({
            "path": "test.txt",
            "content": "Hello Rust Migration!"
        });
        let write_res = write_tool.execute(write_args).await.unwrap();
        assert!(write_res.contains("Successfully wrote"));

        let read_args = serde_json::json!({
            "path": "test.txt"
        });
        let content = read_tool.execute(read_args).await.unwrap();
        assert_eq!(content, "Hello Rust Migration!");
    }
}
