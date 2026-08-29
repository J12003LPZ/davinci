//! Coding-agent print mode, JSONL session subset, and built-in tools.

use async_trait::async_trait;
use pi_agent::{Agent, AgentError, AgentTool, Result};
use pi_ai::{LanguageModel, MockLanguageModel, ToolDefinition};
use pi_core::{Role, WriterLeaseOptions};
use pi_session_sqlite::SqliteSessionRepository;
use pi_tui::{Component, ConversationView, DifferentialRenderer};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

pub struct ReadTool {
    pub root: PathBuf,
}

#[derive(Deserialize)]
struct PathArgs {
    path: String,
}

#[async_trait]
impl AgentTool for ReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".into(),
            description: "Read a file relative to the workspace".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<String> {
        let parsed: PathArgs =
            serde_json::from_value(args).map_err(|e| AgentError::Tool(e.to_string()))?;
        let target = sanitize(&self.root, &parsed.path)?;
        tokio::fs::read_to_string(target)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))
    }
}

pub struct WriteTool {
    pub root: PathBuf,
}

#[derive(Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

#[async_trait]
impl AgentTool for WriteTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write".into(),
            description: "Write a file relative to the workspace".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<String> {
        let parsed: WriteArgs =
            serde_json::from_value(args).map_err(|e| AgentError::Tool(e.to_string()))?;
        let target = sanitize(&self.root, &parsed.path)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AgentError::Tool(e.to_string()))?;
        }
        tokio::fs::write(&target, parsed.content)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(format!("wrote {}", parsed.path))
    }
}

pub struct EditTool {
    pub root: PathBuf,
}

#[derive(Deserialize)]
struct EditArgs {
    path: String,
    old_string: String,
    new_string: String,
}

#[async_trait]
impl AgentTool for EditTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "edit".into(),
            description: "Replace one occurrence of text in a file".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<String> {
        let parsed: EditArgs =
            serde_json::from_value(args).map_err(|e| AgentError::Tool(e.to_string()))?;
        let target = sanitize(&self.root, &parsed.path)?;
        let current = tokio::fs::read_to_string(&target)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        let updated = current.replacen(&parsed.old_string, &parsed.new_string, 1);
        if updated == current {
            return Err(AgentError::Tool("old_string not found".into()));
        }
        tokio::fs::write(&target, updated)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        Ok(format!("edited {}", parsed.path))
    }
}

pub struct BashTool {
    pub root: PathBuf,
}

#[async_trait]
impl AgentTool for BashTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "bash".into(),
            description: "Run a shell command in the workspace".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, args: serde_json::Value) -> Result<String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("command required".into()))?;
        let output = tokio::process::Command::new("bash")
            .arg("-lc")
            .arg(command)
            .current_dir(&self.root)
            .output()
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.stderr.is_empty() {
            text.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        if text.len() > 256 * 1024 {
            text.truncate(256 * 1024);
            text.push_str("\n[truncated]");
        }
        Ok(text)
    }
}

fn sanitize(root: &Path, rel: &str) -> Result<PathBuf> {
    let target = root.join(rel);
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if let Ok(canon) = target.canonicalize() {
        if !canon.starts_with(&canon_root) {
            return Err(AgentError::Tool("path escapes workspace".into()));
        }
        return Ok(canon);
    }
    if target
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AgentError::Tool("path escapes workspace".into()));
    }
    Ok(target)
}

pub struct PrintSession {
    pub store: Arc<SqliteSessionRepository>,
    pub agent: Agent,
    pub session_id: String,
}

impl PrintSession {
    pub fn open(root: impl AsRef<Path>, model: Option<Arc<dyn LanguageModel>>) -> Result<Self> {
        let root = root.as_ref();
        let db_path = root.join(".pi").join("sessions.sqlite");
        let store = Arc::new(
            SqliteSessionRepository::open_path(db_path, WriterLeaseOptions::default())
                .map_err(AgentError::Session)?,
        );
        let session_id = Uuid::now_v7().to_string();
        store
            .create(Some(&session_id), &root.display().to_string(), None, None)
            .map_err(AgentError::Session)?;
        let model = model.unwrap_or_else(|| Arc::new(MockLanguageModel::new("Pi: ")));
        let mut agent = Agent::new(store.clone(), model);
        agent.register_tool(Arc::new(ReadTool {
            root: root.to_path_buf(),
        }));
        agent.register_tool(Arc::new(WriteTool {
            root: root.to_path_buf(),
        }));
        agent.register_tool(Arc::new(EditTool {
            root: root.to_path_buf(),
        }));
        agent.register_tool(Arc::new(BashTool {
            root: root.to_path_buf(),
        }));
        Ok(Self {
            store,
            agent,
            session_id,
        })
    }

    pub async fn prompt(&self, text: &str) -> Result<String> {
        self.agent.run(&self.session_id, text, None).await
    }

    pub fn render_preview(&self) -> Vec<String> {
        let mut view = ConversationView::new(&self.session_id);
        if let Ok(entries) = self.store.entries(&self.session_id) {
            for entry in entries {
                if let pi_core::Entry::Message {
                    id,
                    timestamp,
                    message,
                    ..
                } = entry
                {
                    view.messages.push(pi_core::Message {
                        id,
                        session_id: self.session_id.clone(),
                        role: message.role,
                        content: message.content,
                        tool_calls: message.tool_calls,
                        tool_call_id: message.tool_call_id,
                        timestamp,
                    });
                }
            }
        }
        let lines = view.render(80);
        let mut renderer = DifferentialRenderer::new();
        let _ = renderer.frame(lines.clone(), 80);
        lines
    }
}

pub fn append_jsonl(path: impl AsRef<Path>, role: Role, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let rec = serde_json::json!({
        "type": "message",
        "role": role,
        "content": content
    });
    writeln!(file, "{rec}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn print_session_echoes_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let session = PrintSession::open(tmp.path(), None).unwrap();
        let reply = session.prompt("migrate writer leases").await.unwrap();
        assert_eq!(reply, "Pi: migrate writer leases");
        assert!(session
            .render_preview()
            .join("\n")
            .contains("migrate writer leases"));
    }

    #[tokio::test]
    async fn write_and_read_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = WriteTool {
            root: tmp.path().to_path_buf(),
        };
        tool.execute(serde_json::json!({"path":"note.txt","content":"hello"}))
            .await
            .unwrap();
        let reader = ReadTool {
            root: tmp.path().to_path_buf(),
        };
        let text = reader
            .execute(serde_json::json!({"path":"note.txt"}))
            .await
            .unwrap();
        assert_eq!(text, "hello");
    }
}
