use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const BUILTIN_TOOLS: &[&str] = &["read", "write", "edit", "bash", "grep", "find", "ls"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Unknown tool: {0}")]
    Unknown(String),
    #[error("{0}")]
    Failed(String),
}

pub fn tool_specs() -> Vec<AgentTool> {
    vec![
        AgentTool {
            name: "read".into(),
            description: "Read file contents".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        },
        AgentTool {
            name: "write".into(),
            description: "Write files (creates/overwrites)".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
        },
        AgentTool {
            name: "edit".into(),
            description: "Edit files with find/replace".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"oldText":{"type":"string"},"newText":{"type":"string"}},"required":["path","oldText","newText"]}),
        },
        AgentTool {
            name: "bash".into(),
            description: "Execute bash commands".into(),
            parameters: serde_json::json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
        },
        AgentTool {
            name: "grep".into(),
            description: "Search file contents".into(),
            parameters: serde_json::json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"}},"required":["pattern"]}),
        },
        AgentTool {
            name: "find".into(),
            description: "Find files by glob pattern".into(),
            parameters: serde_json::json!({"type":"object","properties":{"pattern":{"type":"string"}},"required":["pattern"]}),
        },
        AgentTool {
            name: "ls".into(),
            description: "List directory contents".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
        },
    ]
}

pub fn execute_tool(
    cwd: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<ToolResult, ToolError> {
    match name {
        "read" => {
            let path = resolve(cwd, required_str(input, "path")?)?;
            let content =
                fs::read_to_string(&path).map_err(|err| ToolError::Failed(err.to_string()))?;
            Ok(ToolResult {
                content,
                is_error: false,
                details: Some(serde_json::json!({"path": path})),
            })
        }
        "write" => {
            let path = resolve(cwd, required_str(input, "path")?)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|err| ToolError::Failed(err.to_string()))?;
            }
            let content = required_str(input, "content")?;
            fs::write(&path, content).map_err(|err| ToolError::Failed(err.to_string()))?;
            Ok(ToolResult {
                content: format!("Wrote {}", path.display()),
                is_error: false,
                details: None,
            })
        }
        "edit" => {
            let path = resolve(cwd, required_str(input, "path")?)?;
            let old = required_str(input, "oldText")?;
            let new = required_str(input, "newText")?;
            let original =
                fs::read_to_string(&path).map_err(|err| ToolError::Failed(err.to_string()))?;
            if !original.contains(old) {
                return Ok(ToolResult {
                    content: format!("oldText not found in {}", path.display()),
                    is_error: true,
                    details: None,
                });
            }
            fs::write(&path, original.replacen(old, new, 1))
                .map_err(|err| ToolError::Failed(err.to_string()))?;
            Ok(ToolResult {
                content: format!("Edited {}", path.display()),
                is_error: false,
                details: None,
            })
        }
        "bash" => {
            let command = required_str(input, "command")?;
            let output = Command::new("bash")
                .arg("-lc")
                .arg(command)
                .current_dir(cwd)
                .output()
                .map_err(|err| ToolError::Failed(err.to_string()))?;
            let mut content = String::from_utf8_lossy(&output.stdout).into_owned();
            if !output.stderr.is_empty() {
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            Ok(ToolResult {
                content,
                is_error: !output.status.success(),
                details: Some(serde_json::json!({"exitCode": output.status.code()})),
            })
        }
        "ls" => {
            let path = resolve(
                cwd,
                input.get("path").and_then(|v| v.as_str()).unwrap_or("."),
            )?;
            let mut names = Vec::new();
            for entry in fs::read_dir(&path)
                .map_err(|err| ToolError::Failed(err.to_string()))?
                .flatten()
            {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            names.sort();
            Ok(ToolResult {
                content: names.join("\n"),
                is_error: false,
                details: None,
            })
        }
        "grep" => {
            let pattern = required_str(input, "pattern")?;
            let root = resolve(
                cwd,
                input.get("path").and_then(|v| v.as_str()).unwrap_or("."),
            )?;
            let mut hits = Vec::new();
            walk_grep(&root, pattern, &mut hits, 0);
            Ok(ToolResult {
                content: hits.join("\n"),
                is_error: false,
                details: None,
            })
        }
        "find" => {
            let pattern = required_str(input, "pattern")?;
            let mut hits = Vec::new();
            walk_find(cwd, pattern, &mut hits, 0);
            Ok(ToolResult {
                content: hits.join("\n"),
                is_error: false,
                details: None,
            })
        }
        other => Err(ToolError::Unknown(other.to_string())),
    }
}

fn required_str<'a>(input: &'a serde_json::Value, field: &str) -> Result<&'a str, ToolError> {
    input
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::Failed(format!("Missing {field}")))
}

fn resolve(cwd: &Path, path: &str) -> Result<PathBuf, ToolError> {
    let path = PathBuf::from(path);
    Ok(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

fn walk_grep(path: &Path, pattern: &str, hits: &mut Vec<String>, depth: usize) {
    if depth > 6 || hits.len() > 200 {
        return;
    }
    if path.is_file() {
        if let Ok(body) = fs::read_to_string(path) {
            for (index, line) in body.lines().enumerate() {
                if line.contains(pattern) {
                    hits.push(format!("{}:{}:{line}", path.display(), index + 1));
                }
            }
        }
        return;
    }
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            walk_grep(&entry.path(), pattern, hits, depth + 1);
        }
    }
}

fn walk_find(path: &Path, pattern: &str, hits: &mut Vec<String>, depth: usize) {
    if depth > 6 || hits.len() > 200 {
        return;
    }
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let child = entry.path();
            if let Some(name) = child.file_name().and_then(|n| n.to_str()) {
                if glob_match(pattern, name) {
                    hits.push(child.display().to_string());
                }
            }
            if child.is_dir() {
                walk_find(&child, pattern, hits, depth + 1);
            }
        }
    }
}

fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    name.contains(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn read_write_edit_semantics() {
        let dir = tempdir().unwrap();
        execute_tool(
            dir.path(),
            "write",
            &serde_json::json!({"path":"a.txt","content":"hello"}),
        )
        .unwrap();
        let read = execute_tool(dir.path(), "read", &serde_json::json!({"path":"a.txt"})).unwrap();
        assert_eq!(read.content, "hello");
        execute_tool(
            dir.path(),
            "edit",
            &serde_json::json!({"path":"a.txt","oldText":"hello","newText":"world"}),
        )
        .unwrap();
        let read = execute_tool(dir.path(), "read", &serde_json::json!({"path":"a.txt"})).unwrap();
        assert_eq!(read.content, "world");
    }
}
