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
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    fn execute(&self, input: &Value, cwd: &Path) -> Result<ToolResult, ToolError>;
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn BuiltinTool>>,
}

impl ToolRegistry {
    pub fn builtins() -> Self {
        let mut tools: Vec<Box<dyn BuiltinTool>> = vec![
            Box::new(ReadTool),
            Box::new(WriteTool),
            Box::new(EditTool),
            Box::new(BashTool),
            Box::new(LsTool),
            Box::new(GrepTool),
            Box::new(FindTool),
        ];
        if cfg!(windows) || std::env::var("PI_ENABLE_POWERSHELL").is_ok() {
            tools.push(Box::new(PowerShellTool));
        }
        Self { tools }
    }

    pub fn register(&mut self, tool: Box<dyn BuiltinTool>) {
        self.tools.retain(|t| t.name() != tool.name());
        self.tools.push(tool);
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

    pub fn names(&self) -> Vec<&str> {
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

struct LsTool;
impl BuiltinTool for LsTool {
    fn name(&self) -> &'static str {
        "ls"
    }
    fn description(&self) -> &'static str {
        "List directory contents"
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"limit":{"type":"number"}}})
    }
    fn execute(&self, input: &Value, cwd: &Path) -> Result<ToolResult, ToolError> {
        ls(
            cwd,
            input.get("path").and_then(|v| v.as_str()).unwrap_or("."),
            input.get("limit").and_then(|v| v.as_u64()).unwrap_or(500) as usize,
        )
    }
}

struct GrepTool;
impl BuiltinTool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }
    fn description(&self) -> &'static str {
        "Search file contents for patterns (respects .gitignore)"
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"glob":{"type":"string"},"ignoreCase":{"type":"boolean"},"literal":{"type":"boolean"},"limit":{"type":"number"}},"required":["pattern"]})
    }
    fn execute(&self, input: &Value, cwd: &Path) -> Result<ToolResult, ToolError> {
        grep(
            cwd,
            input.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
            input.get("path").and_then(|v| v.as_str()),
            input.get("glob").and_then(|v| v.as_str()),
            input
                .get("ignoreCase")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            input.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize,
        )
    }
}

struct FindTool;
impl BuiltinTool for FindTool {
    fn name(&self) -> &'static str {
        "find"
    }
    fn description(&self) -> &'static str {
        "Find files by glob pattern (respects .gitignore)"
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"limit":{"type":"number"}},"required":["pattern"]})
    }
    fn execute(&self, input: &Value, cwd: &Path) -> Result<ToolResult, ToolError> {
        find_files(
            cwd,
            input.get("pattern").and_then(|v| v.as_str()).unwrap_or("*"),
            input.get("path").and_then(|v| v.as_str()),
            input.get("limit").and_then(|v| v.as_u64()).unwrap_or(1000) as usize,
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

pub fn ls(cwd: &Path, path: &str, limit: usize) -> Result<ToolResult, ToolError> {
    let resolved = resolve_path(cwd, path);
    let mut names = Vec::new();
    let entries = fs::read_dir(&resolved).map_err(|e| ToolError::Message(e.to_string()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let suffix = if entry.path().is_dir() { "/" } else { "" };
        names.push(format!("{name}{suffix}"));
        if names.len() >= limit {
            break;
        }
    }
    names.sort();
    Ok(ToolResult {
        output: names.join("\n"),
        is_error: false,
        details: json!({"path": resolved, "count": names.len()}),
    })
}

pub fn grep(
    cwd: &Path,
    pattern: &str,
    path: Option<&str>,
    glob_pat: Option<&str>,
    ignore_case: bool,
    limit: usize,
) -> Result<ToolResult, ToolError> {
    let root = resolve_path(cwd, path.unwrap_or("."));
    let flags = if ignore_case { "(?i)" } else { "" };
    let re = regex::Regex::new(&format!("{flags}{pattern}"))
        .map_err(|e| ToolError::Message(format!("Invalid regex: {e}")))?;
    let matcher = glob_pat.map(|g| glob::Pattern::new(g).ok()).unwrap_or(None);
    let mut hits = Vec::new();
    if root.is_file() {
        grep_file(&root, cwd, &re, &mut hits, limit);
    } else {
        for entry in walkdir::WalkDir::new(&root).into_iter().flatten() {
            if hits.len() >= limit {
                break;
            }
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(pat) = &matcher {
                if !pat.matches_path(path) {
                    continue;
                }
            }
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.') || n == "target")
            {
                continue;
            }
            grep_file(path, cwd, &re, &mut hits, limit);
        }
    }
    Ok(ToolResult {
        output: hits.join("\n"),
        is_error: false,
        details: json!({"matches": hits.len()}),
    })
}

fn grep_file(path: &Path, cwd: &Path, re: &regex::Regex, hits: &mut Vec<String>, limit: usize) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let rel = path.strip_prefix(cwd).unwrap_or(path);
    for (i, line) in content.lines().enumerate() {
        if hits.len() >= limit {
            return;
        }
        if re.is_match(line) {
            hits.push(format!("{}:{}:{line}", rel.display(), i + 1));
        }
    }
}

pub fn find_files(
    cwd: &Path,
    pattern: &str,
    path: Option<&str>,
    limit: usize,
) -> Result<ToolResult, ToolError> {
    let root = resolve_path(cwd, path.unwrap_or("."));
    let pat = glob::Pattern::new(pattern)
        .map_err(|e| ToolError::Message(format!("Invalid glob: {e}")))?;
    let mut hits = Vec::new();
    for entry in walkdir::WalkDir::new(&root).into_iter().flatten() {
        if hits.len() >= limit {
            break;
        }
        let p = entry.path();
        let rel = p.strip_prefix(&root).unwrap_or(p);
        let rel_posix = rel.to_string_lossy().replace('\\', "/");
        if pat.matches(&rel_posix)
            || pat.matches(p.file_name().and_then(|n| n.to_str()).unwrap_or(""))
        {
            let suffix = if p.is_dir() { "/" } else { "" };
            hits.push(format!("{rel_posix}{suffix}"));
        }
    }
    Ok(ToolResult {
        output: hits.join("\n"),
        is_error: false,
        details: json!({"count": hits.len()}),
    })
}

struct PowerShellTool;
impl BuiltinTool for PowerShellTool {
    fn name(&self) -> &'static str {
        "powershell"
    }
    fn description(&self) -> &'static str {
        "Execute PowerShell commands"
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]})
    }
    fn execute(&self, input: &Value, cwd: &Path) -> Result<ToolResult, ToolError> {
        powershell(cwd, input["command"].as_str().unwrap_or(""))
    }
}

pub fn powershell(cwd: &Path, command: &str) -> Result<ToolResult, ToolError> {
    let exe = if cfg!(windows) { "powershell" } else { "pwsh" };
    let output = Command::new(exe)
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .current_dir(cwd)
        .output();
    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = if stderr.is_empty() {
                stdout
            } else {
                format!("{stdout}{stderr}")
            };
            Ok(ToolResult {
                output: combined,
                is_error: !output.status.success(),
                details: json!({"exitCode": output.status.code(), "shell": exe}),
            })
        }
        Err(e) => Ok(ToolResult {
            output: format!("Error: {e}"),
            is_error: true,
            details: json!({"error": e.to_string()}),
        }),
    }
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
        let out = bash(dir.path(), "printf hi").unwrap();
        assert_eq!(out.output, "hi");
        write_file(dir.path(), "src/a.rs", "fn main() {}").unwrap();
        let listed = ls(dir.path(), ".", 50).unwrap();
        assert!(listed.output.contains("a.txt") || listed.output.contains("src/"));
        let found = find_files(dir.path(), "*.rs", None, 50).unwrap();
        assert!(found.output.contains("a.rs"));
        let grepped = grep(dir.path(), "main", None, Some("*.rs"), false, 20).unwrap();
        assert!(grepped.output.contains("main"));
    }
}
