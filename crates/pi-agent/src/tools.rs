use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    pub details: Value,
}

pub trait BuiltinTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value;
    fn execute(&self, input: &Value, cwd: &Path) -> Result<ToolResult, ToolError>;
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn BuiltinTool>>,
}

impl ToolRegistry {
    pub fn builtins() -> Self {
        Self {
            tools: vec![
                Box::new(ReadTool),
                Box::new(WriteTool),
                Box::new(EditTool),
                Box::new(BashTool),
            ],
        }
    }

    pub fn with_names(names: &[String]) -> Self {
        let all = Self::builtins();
        Self {
            tools: all
                .tools
                .into_iter()
                .filter(|t| names.iter().any(|n| n == t.name()))
                .collect(),
        }
    }

    pub fn exclude(mut self, names: &[String]) -> Self {
        self.tools.retain(|t| !names.iter().any(|n| n == t.name()));
        self
    }

    pub fn get(&self, name: &str) -> Option<&dyn BuiltinTool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    pub fn schemas(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.parameters(),
                })
            })
            .collect()
    }
}

fn resolve_path(cwd: &Path, path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else {
        cwd.join(p)
    }
}

struct ReadTool;
impl BuiltinTool for ReadTool {
    fn name(&self) -> &'static str {
        "read"
    }
    fn description(&self) -> &'static str {
        "Read file contents"
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})
    }
    fn execute(&self, input: &Value, cwd: &Path) -> Result<ToolResult, ToolError> {
        read_file(cwd, input["path"].as_str().unwrap_or(""))
    }
}

struct WriteTool;
impl BuiltinTool for WriteTool {
    fn name(&self) -> &'static str {
        "write"
    }
    fn description(&self) -> &'static str {
        "Write files (creates/overwrites)"
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})
    }
    fn execute(&self, input: &Value, cwd: &Path) -> Result<ToolResult, ToolError> {
        write_file(
            cwd,
            input["path"].as_str().unwrap_or(""),
            input["content"].as_str().unwrap_or(""),
        )
    }
}

struct EditTool;
impl BuiltinTool for EditTool {
    fn name(&self) -> &'static str {
        "edit"
    }
    fn description(&self) -> &'static str {
        "Edit files with find/replace"
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"oldText":{"type":"string"},"newText":{"type":"string"}},"required":["path","oldText","newText"]})
    }
    fn execute(&self, input: &Value, cwd: &Path) -> Result<ToolResult, ToolError> {
        edit(
            cwd,
            input["path"].as_str().unwrap_or(""),
            input
                .get("oldText")
                .or_else(|| input.get("old_string"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            input
                .get("newText")
                .or_else(|| input.get("new_string"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        )
    }
}

struct BashTool;
impl BuiltinTool for BashTool {
    fn name(&self) -> &'static str {
        "bash"
    }
    fn description(&self) -> &'static str {
        "Execute bash commands"
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]})
    }
    fn execute(&self, input: &Value, cwd: &Path) -> Result<ToolResult, ToolError> {
        bash(cwd, input["command"].as_str().unwrap_or(""))
    }
}

pub fn read_file(cwd: &Path, path: &str) -> Result<ToolResult, ToolError> {
    let resolved = resolve_path(cwd, path);
    match fs::read_to_string(&resolved) {
        Ok(content) => Ok(ToolResult {
            output: content,
            is_error: false,
            details: json!({"path": resolved}),
        }),
        Err(e) => Ok(ToolResult {
            output: format!("Error: {e}"),
            is_error: true,
            details: json!({"path": resolved, "error": e.to_string()}),
        }),
    }
}

pub fn write_file(cwd: &Path, path: &str, content: &str) -> Result<ToolResult, ToolError> {
    let resolved = resolve_path(cwd, path);
    if let Some(parent) = resolved.parent() {
        fs::create_dir_all(parent).map_err(|e| ToolError::Message(e.to_string()))?;
    }
    fs::write(&resolved, content).map_err(|e| ToolError::Message(e.to_string()))?;
    Ok(ToolResult {
        output: format!("Wrote {}", resolved.display()),
        is_error: false,
        details: json!({"path": resolved}),
    })
}

pub fn edit(
    cwd: &Path,
    path: &str,
    old_text: &str,
    new_text: &str,
) -> Result<ToolResult, ToolError> {
    let resolved = resolve_path(cwd, path);
    let current = fs::read_to_string(&resolved).map_err(|e| ToolError::Message(e.to_string()))?;
    if old_text.is_empty() || !current.contains(old_text) {
        return Ok(ToolResult {
            output: "Error: oldText not found in file".into(),
            is_error: true,
            details: json!({"path": resolved}),
        });
    }
    let updated = current.replacen(old_text, new_text, 1);
    fs::write(&resolved, updated).map_err(|e| ToolError::Message(e.to_string()))?;
    Ok(ToolResult {
        output: format!("Edited {}", resolved.display()),
        is_error: false,
        details: json!({"path": resolved}),
    })
}

pub fn bash(cwd: &Path, command: &str) -> Result<ToolResult, ToolError> {
    let output = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .output()
        .map_err(|e| ToolError::Message(e.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = if stderr.is_empty() {
        stdout.clone()
    } else {
        format!("{stdout}{stderr}")
    };
    Ok(ToolResult {
        output: combined,
        is_error: !output.status.success(),
        details: json!({"exitCode": output.status.code(), "stderr": stderr}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn read_write_edit_bash() {
        let dir = tempdir().unwrap();
        write_file(dir.path(), "a.txt", "hello").unwrap();
        let read = read_file(dir.path(), "a.txt").unwrap();
        assert_eq!(read.output, "hello");
        let edited = edit(dir.path(), "a.txt", "hello", "world").unwrap();
        assert!(!edited.is_error);
        assert_eq!(read_file(dir.path(), "a.txt").unwrap().output, "world");
        let missing = edit(dir.path(), "a.txt", "nope", "x").unwrap();
        assert!(missing.is_error);
        assert!(missing.output.contains("oldText not found"));
        let ls = bash(dir.path(), "printf hi").unwrap();
        assert_eq!(ls.output, "hi");
    }
}
