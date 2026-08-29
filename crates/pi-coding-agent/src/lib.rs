//! Coding-agent library: CLI args, print mode, and built-in tools.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Parser, Subcommand};
use pi_agent::{run_agent_loop, AgentContext, AgentLoopConfig, AgentTool, QueueMode, ToolExecutionMode};
use pi_ai::{test_model, Message, MockProvider, Tool};
use pi_session::{provision_message, SessionCreateOptions, SessionRepository};
use pi_session_sqlite::{SqliteSessionRepository, WriterLeaseOptions};
use serde_json::Value;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Parser)]
#[command(name = "pi", version = VERSION, about = "Pi coding agent (Rust port)")]
pub struct Args {
    /// Print mode: run one prompt and exit.
    #[arg(short = 'p', long = "print")]
    pub print: Option<String>,
    /// Emit print-mode events as JSON lines.
    #[arg(long)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// List sessions in a SQLite database.
    Sessions {
        #[arg(long, default_value = "sessions.sqlite")]
        database: PathBuf,
    },
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
}

pub fn create_coding_tools(cwd: PathBuf) -> Vec<AgentTool> {
    let ctx = ToolContext { cwd };
    vec![
        make_tool("read", "Read a file", read_tool(ctx.clone())),
        make_tool("write", "Write a file", write_tool(ctx.clone())),
        make_tool("edit", "Replace text in a file", edit_tool(ctx.clone())),
        make_tool("bash", "Run a shell command", bash_tool(ctx)),
    ]
}

fn make_tool(name: &'static str, description: &'static str, execute: fn(&Value) -> Result<Value, String>) -> AgentTool {
    AgentTool {
        spec: Tool {
            name: name.into(),
            description: description.into(),
            parameters: serde_json::json!({"required":["path"]}),
        },
        execute,
    }
}

fn resolve_path(cwd: &Path, raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);
    let resolved = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let resolved = if resolved.exists() {
        resolved.canonicalize().map_err(|error| error.to_string())?
    } else {
        resolved
    };
    if !resolved.starts_with(&cwd) {
        return Err("path escapes the working directory".into());
    }
    Ok(resolved)
}

thread_local! {
    static TOOL_CWD: std::cell::RefCell<PathBuf> = std::cell::RefCell::new(PathBuf::from("."));
}

fn with_cwd<T>(cwd: PathBuf, f: impl FnOnce() -> T) -> T {
    TOOL_CWD.with(|slot| {
        *slot.borrow_mut() = cwd;
        f()
    })
}

fn current_cwd() -> PathBuf {
    TOOL_CWD.with(|slot| slot.borrow().clone())
}

fn read_tool(_ctx: ToolContext) -> fn(&Value) -> Result<Value, String> {
    fn exec(args: &Value) -> Result<Value, String> {
        let path = args.get("path").and_then(Value::as_str).ok_or("missing path")?;
        let resolved = resolve_path(&current_cwd(), path)?;
        let contents = fs::read_to_string(&resolved).map_err(|error| error.to_string())?;
        Ok(Value::String(contents))
    }
    exec
}

fn write_tool(_ctx: ToolContext) -> fn(&Value) -> Result<Value, String> {
    fn exec(args: &Value) -> Result<Value, String> {
        let path = args.get("path").and_then(Value::as_str).ok_or("missing path")?;
        let content = args.get("content").and_then(Value::as_str).unwrap_or("");
        let resolved = resolve_path(&current_cwd(), path)?;
        if let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&resolved, content).map_err(|error| error.to_string())?;
        Ok(Value::String(format!("wrote {}", resolved.display())))
    }
    exec
}

fn edit_tool(_ctx: ToolContext) -> fn(&Value) -> Result<Value, String> {
    fn exec(args: &Value) -> Result<Value, String> {
        let path = args.get("path").and_then(Value::as_str).ok_or("missing path")?;
        let old = args.get("oldText").and_then(Value::as_str).ok_or("missing oldText")?;
        let new = args.get("newText").and_then(Value::as_str).ok_or("missing newText")?;
        let resolved = resolve_path(&current_cwd(), path)?;
        let contents = fs::read_to_string(&resolved).map_err(|error| error.to_string())?;
        if !contents.contains(old) {
            return Err("oldText not found".into());
        }
        fs::write(&resolved, contents.replacen(old, new, 1)).map_err(|error| error.to_string())?;
        Ok(Value::String("edited".into()))
    }
    exec
}

fn bash_tool(_ctx: ToolContext) -> fn(&Value) -> Result<Value, String> {
    fn exec(args: &Value) -> Result<Value, String> {
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .ok_or("missing command")?;
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(current_cwd())
            .output()
            .map_err(|error| error.to_string())?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.stderr.is_empty() {
            text.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        Ok(Value::String(text))
    }
    exec
}

pub fn run_print(prompt: &str, json: bool) -> Result<String, String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    with_cwd(cwd.clone(), || {
        let context = AgentContext {
            system_prompt: Some("You are pi.".into()),
            messages: vec![],
            tools: create_coding_tools(cwd),
        };
        let config = AgentLoopConfig {
            model: test_model(),
            tool_execution: ToolExecutionMode::Sequential,
            steering_mode: QueueMode::All,
            follow_up_mode: QueueMode::All,
        };
        let (events, messages) = run_agent_loop(
            vec![Message::User {
                content: Value::String(prompt.into()),
                timestamp: 0,
            }],
            context,
            config,
            &MockProvider::default(),
            Default::default(),
            Default::default(),
        );
        if json {
            let payload: Vec<Value> = events
                .iter()
                .map(|event| serde_json::json!({"type": format!("{event:?}").split(' ').next().unwrap_or("event")}))
                .collect();
            Ok(serde_json::to_string(&payload).unwrap())
        } else {
            let text = messages.iter().rev().find_map(|message| match message {
                Message::Assistant { content, .. } => content.iter().find_map(|block| match block {
                    pi_ai::AssistantContent::Text { text } => Some(text.clone()),
                    _ => None,
                }),
                _ => None,
            });
            Ok(text.unwrap_or_default())
        }
    })
}

pub fn list_sessions(database: &Path) -> Result<Vec<String>, String> {
    let repo = SqliteSessionRepository::open(database, WriterLeaseOptions::default())
        .map_err(|error| error.to_string())?;
    repo.list(None)
        .map_err(|error| error.to_string())
        .map(|sessions| {
            sessions
                .into_iter()
                .map(|session| {
                    format!(
                        "{}\t{}",
                        session.id,
                        session.name.unwrap_or_else(|| session.cwd)
                    )
                })
                .collect()
        })
}

pub fn create_demo_session(database: &Path, cwd: &str, name: &str) -> Result<String, String> {
    let mut repo = SqliteSessionRepository::open(database, WriterLeaseOptions::default())
        .map_err(|error| error.to_string())?;
    let mut session = repo
        .create(SessionCreateOptions {
            cwd: cwd.into(),
            name: Some(name.into()),
            ..SessionCreateOptions::default()
        })
        .map_err(|error| error.to_string())?;
    session
        .append_entry(provision_message("hello"), "main")
        .map_err(|error| error.to_string())?;
    session.release().map_err(|error| error.to_string())?;
    Ok(session.metadata().map_err(|error| error.to_string())?.id)
}

pub fn execute_read(cwd: &Path, path: &str) -> Result<String, String> {
    with_cwd(cwd.to_path_buf(), || {
        read_tool(ToolContext {
            cwd: cwd.to_path_buf(),
        })(&serde_json::json!({"path": path}))
        .map(|value| value.as_str().unwrap_or_default().to_string())
    })
}

pub fn execute_write(cwd: &Path, path: &str, content: &str) -> Result<String, String> {
    with_cwd(cwd.to_path_buf(), || {
        write_tool(ToolContext {
            cwd: cwd.to_path_buf(),
        })(&serde_json::json!({"path": path, "content": content}))
        .map(|value| value.as_str().unwrap_or_default().to_string())
    })
}

pub fn execute_edit(cwd: &Path, path: &str, old: &str, new: &str) -> Result<String, String> {
    with_cwd(cwd.to_path_buf(), || {
        edit_tool(ToolContext {
            cwd: cwd.to_path_buf(),
        })(&serde_json::json!({"path": path, "oldText": old, "newText": new}))
        .map(|value| value.as_str().unwrap_or_default().to_string())
    })
}

pub fn execute_bash(cwd: &Path, command: &str) -> Result<String, String> {
    with_cwd(cwd.to_path_buf(), || {
        bash_tool(ToolContext {
            cwd: cwd.to_path_buf(),
        })(&serde_json::json!({"command": command}))
        .map(|value| value.as_str().unwrap_or_default().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn print_mode_echoes_prompt() {
        let out = run_print("hello", false).unwrap();
        assert!(out.contains("hello"));
    }

    #[test]
    fn tools_read_write_edit_bash() {
        let dir = tempdir().unwrap();
        execute_write(dir.path(), "note.txt", "alpha").unwrap();
        assert_eq!(execute_read(dir.path(), "note.txt").unwrap(), "alpha");
        execute_edit(dir.path(), "note.txt", "alpha", "beta").unwrap();
        assert_eq!(execute_read(dir.path(), "note.txt").unwrap(), "beta");
        let listed = execute_bash(dir.path(), "printf ok").unwrap();
        assert!(listed.contains("ok"));
    }

    #[test]
    fn sessions_list_created_rows() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("sessions.sqlite");
        create_demo_session(&db, dir.path().to_str().unwrap(), "Review session").unwrap();
        let listed = list_sessions(&db).unwrap();
        assert!(listed.iter().any(|row| row.contains("Review session")));
    }
}
