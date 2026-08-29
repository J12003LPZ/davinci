use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const BUILTIN_TOOLS: &[&str] = &[
    "read",
    "write",
    "edit",
    "bash",
    "powershell",
    "grep",
    "find",
    "ls",
];

const DEFAULT_MAX_LINES: usize = 2000;
const DEFAULT_MAX_BYTES: usize = 50 * 1024;
const GREP_MAX_LINE_LENGTH: usize = 500;
const GREP_DEFAULT_LIMIT: usize = 100;
const FIND_DEFAULT_LIMIT: usize = 1000;
const LS_DEFAULT_LIMIT: usize = 500;
const POWERSHELL_UTF8_PREFIX: &str =
    "try { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8 } catch {}\n";

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

impl ToolResult {
    pub fn take_updates(details: &mut Option<serde_json::Value>) -> Vec<serde_json::Value> {
        let Some(Value::Object(map)) = details.as_mut() else {
            return Vec::new();
        };
        match map.remove("_piUpdates") {
            Some(Value::Array(items)) => items,
            _ => Vec::new(),
        }
    }
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
            description: "Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"number"},"limit":{"type":"number"}},"required":["path"]}),
        },
        AgentTool {
            name: "write".into(),
            description: "Write files (creates/overwrites)".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
        },
        AgentTool {
            name: "edit".into(),
            description: "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.".into(),
            parameters: serde_json::json!({
                "type":"object",
                "properties":{
                    "path":{"type":"string","description":"Path to the file to edit (relative or absolute)"},
                    "edits":{
                        "type":"array",
                        "description":"One or more targeted replacements. Each edit is matched against the original file, not incrementally.",
                        "items":{
                            "type":"object",
                            "properties":{
                                "oldText":{"type":"string"},
                                "newText":{"type":"string"}
                            },
                            "required":["oldText","newText"]
                        }
                    },
                    "oldText":{"type":"string"},
                    "newText":{"type":"string"}
                },
                "required":["path"]
            }),
        },
        AgentTool {
            name: "bash".into(),
            description: "Execute bash commands".into(),
            parameters: serde_json::json!({"type":"object","properties":{"command":{"type":"string"},"timeout":{"type":"number","description":"Timeout in seconds (optional, no default timeout)"}},"required":["command"]}),
        },
        AgentTool {
            name: "powershell".into(),
            description: "Execute PowerShell commands".into(),
            parameters: serde_json::json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
        },
        AgentTool {
            name: "grep".into(),
            description: "Search file contents for a pattern. Returns matching lines with file paths and line numbers. Respects .gitignore.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"glob":{"type":"string"},"ignoreCase":{"type":"boolean"},"literal":{"type":"boolean"},"context":{"type":"number"},"limit":{"type":"number"}},"required":["pattern"]}),
        },
        AgentTool {
            name: "find".into(),
            description: "Search for files by glob pattern. Returns matching file paths relative to the search directory. Respects .gitignore.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"limit":{"type":"number"}},"required":["pattern"]}),
        },
        AgentTool {
            name: "ls".into(),
            description: "List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"limit":{"type":"number"}}}),
        },
    ]
}

pub fn execute_tool(
    cwd: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<ToolResult, ToolError> {
    match name {
        "read" => read_tool(cwd, input),
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
        "edit" => edit_tool(cwd, input),
        "bash" => shell_tool(cwd, input),
        "powershell" => powershell_tool(cwd, required_str(input, "command")?),
        "ls" => ls_tool(cwd, input),
        "grep" => grep_tool(cwd, input),
        "find" => find_tool(cwd, input),
        other => Err(ToolError::Unknown(other.to_string())),
    }
}

fn read_tool(cwd: &Path, input: &serde_json::Value) -> Result<ToolResult, ToolError> {
    let raw_path = required_str(input, "path")?;
    let path = resolve(cwd, raw_path)?;
    let bytes = fs::read(&path).map_err(|err| ToolError::Failed(err.to_string()))?;
    if let Some(mime) = detect_image_mime(&path, &bytes) {
        return read_image(&path, &bytes, mime);
    }
    let content = String::from_utf8_lossy(&bytes).into_owned();
    let offset = input
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let (content, truncation) = truncate_read(&content, offset, limit);
    Ok(ToolResult {
        content,
        is_error: false,
        details: Some(serde_json::json!({"path": path, "truncation": truncation})),
    })
}

fn read_image(path: &Path, bytes: &[u8], mime: &str) -> Result<ToolResult, ToolError> {
    match process_image(bytes, mime) {
        Ok(processed) => {
            let mut note = format!("Read image file [{}]", processed.mime_type);
            for hint in &processed.hints {
                note.push('\n');
                note.push_str(hint);
            }
            Ok(ToolResult {
                content: note,
                is_error: false,
                details: Some(serde_json::json!({
                    "path": path,
                    "image": {
                        "type": "image",
                        "data": processed.data,
                        "mimeType": processed.mime_type,
                    }
                })),
            })
        }
        Err(message) => Ok(ToolResult {
            content: format!("Read image file [{mime}]\n{message}"),
            is_error: true,
            details: Some(serde_json::json!({"path": path})),
        }),
    }
}

struct ProcessedImage {
    data: String,
    mime_type: String,
    hints: Vec<String>,
}

fn process_image(bytes: &[u8], mime: &str) -> Result<ProcessedImage, String> {
    let normalized = match mime {
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" => mime.to_string(),
        _ => {
            let png = image::load_from_memory(bytes)
                .map_err(|err| format!("Unsupported image type: {err}"))?;
            let mut out = Vec::new();
            png.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .map_err(|err| err.to_string())?;
            return Ok(ProcessedImage {
                data: base64::engine::general_purpose::STANDARD.encode(&out),
                mime_type: "image/png".into(),
                hints: vec![format!("[Image converted from {mime} to image/png.]")],
            });
        }
    };
    Ok(ProcessedImage {
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        mime_type: normalized,
        hints: Vec::new(),
    })
}

fn detect_image_mime(path: &Path, bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

fn edit_tool(cwd: &Path, input: &serde_json::Value) -> Result<ToolResult, ToolError> {
    let (display_path, edits) =
        crate::edit_diff::prepare_edit_arguments(input).map_err(ToolError::Failed)?;
    let path = resolve(cwd, &display_path)?;
    let raw = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            return Err(ToolError::Failed(format!(
                "Could not edit file: {display_path}. Error code: {}.",
                err.raw_os_error()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| err.to_string())
            )));
        }
    };
    let (bom, content) = crate::edit_diff::split_bom(&raw);
    let ending = crate::edit_diff::detect_line_ending(content);
    let normalized = crate::edit_diff::normalize_to_lf(content);
    let applied =
        crate::edit_diff::apply_edits_to_normalized_content(&normalized, &edits, &display_path)
            .map_err(ToolError::Failed)?;
    let final_content = format!(
        "{bom}{}",
        crate::edit_diff::restore_line_endings(&applied.new_content, ending)
    );
    fs::write(&path, final_content).map_err(|err| ToolError::Failed(err.to_string()))?;
    Ok(ToolResult {
        content: format!("Edited {display_path}"),
        is_error: false,
        details: Some(serde_json::json!({
            "path": display_path,
            "edits": edits.len(),
            "tokensBefore": applied.base_content.len(),
        })),
    })
}

fn resolve_bash_timeout_ms(input: &serde_json::Value) -> Result<Option<u64>, ToolError> {
    let Some(value) = input.get("timeout") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let seconds = value.as_f64().ok_or_else(|| {
        ToolError::Failed("Invalid timeout: must be a finite number of seconds".into())
    })?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(ToolError::Failed(
            "Invalid timeout: must be a finite number of seconds".into(),
        ));
    }
    let timeout_ms = seconds * 1000.0;
    const MAX_TIMEOUT_MS: f64 = 2_147_483_647.0;
    if timeout_ms > MAX_TIMEOUT_MS {
        return Err(ToolError::Failed(format!(
            "Invalid timeout: maximum is {} seconds",
            MAX_TIMEOUT_MS / 1000.0
        )));
    }
    Ok(Some(timeout_ms as u64))
}

fn shell_tool(cwd: &Path, input: &serde_json::Value) -> Result<ToolResult, ToolError> {
    let command = required_str(input, "command")?;
    let timeout_ms = resolve_bash_timeout_ms(input)?;
    let command = match std::env::var("PI_SHELL_COMMAND_PREFIX") {
        Ok(prefix) if !prefix.is_empty() => format!("{prefix}; {command}"),
        _ => command.to_string(),
    };
    let custom = std::env::var("PI_SHELL")
        .ok()
        .filter(|value| !value.is_empty());
    let config = pi_ai::resolve_shell_config(custom.as_deref()).map_err(ToolError::Failed)?;
    let mut process = Command::new(&config.shell);
    process
        .args(&config.args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    match config.command_transport {
        pi_ai::CommandTransport::Argv => {
            process.arg(&command).stdin(std::process::Stdio::null());
        }
        pi_ai::CommandTransport::Stdin => {
            process.stdin(std::process::Stdio::piped());
        }
    }
    let mut child = process
        .spawn()
        .map_err(|err| ToolError::Failed(err.to_string()))?;
    if matches!(config.command_transport, pi_ai::CommandTransport::Stdin) {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin
                .write_all(command.as_bytes())
                .map_err(|err| ToolError::Failed(err.to_string()))?;
        }
    }
    let timeout_label = input.get("timeout").map(|value| match value {
        serde_json::Value::Number(number) => number.to_string(),
        other => other.to_string(),
    });
    let output = wait_shell_output(child, timeout_ms, timeout_label.as_deref())?;
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

fn wait_shell_output(
    mut child: std::process::Child,
    timeout_ms: Option<u64>,
    timeout_label: Option<&str>,
) -> Result<std::process::Output, ToolError> {
    use std::io::Read;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_handle = stdout.map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_handle = stderr.map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    });
    let status = if let Some(timeout_ms) = timeout_ms {
        let start = std::time::Instant::now();
        let limit = std::time::Duration::from_millis(timeout_ms);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() >= limit => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let stdout = stdout_handle
                        .map(|handle| handle.join().unwrap_or_default())
                        .unwrap_or_default();
                    let stderr = stderr_handle
                        .map(|handle| handle.join().unwrap_or_default())
                        .unwrap_or_default();
                    let mut content = String::from_utf8_lossy(&stdout).into_owned();
                    if !stderr.is_empty() {
                        if !content.is_empty() {
                            content.push('\n');
                        }
                        content.push_str(&String::from_utf8_lossy(&stderr));
                    }
                    let seconds = timeout_label.unwrap_or("0");
                    let status = format!("Command timed out after {seconds} seconds");
                    return Err(ToolError::Failed(if content.is_empty() {
                        status
                    } else {
                        format!("{content}\n\n{status}")
                    }));
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                Err(err) => return Err(ToolError::Failed(err.to_string())),
            }
        }
    } else {
        child
            .wait()
            .map_err(|err| ToolError::Failed(err.to_string()))?
    };
    let stdout = stdout_handle
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr = stderr_handle
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn powershell_tool(cwd: &Path, command: &str) -> Result<ToolResult, ToolError> {
    if let Ok(reply) = std::env::var("PI_POWERSHELL_REPLY") {
        return Ok(ToolResult {
            content: reply,
            is_error: false,
            details: Some(serde_json::json!({"exitCode": 0})),
        });
    }
    let wrapped = format!("{POWERSHELL_UTF8_PREFIX}{command}");
    for program in ["pwsh", "powershell"] {
        let result = Command::new(program)
            .args(["-NoProfile", "-NonInteractive", "-Command", &wrapped])
            .current_dir(cwd)
            .output();
        match result {
            Ok(output) => {
                let mut content = String::from_utf8_lossy(&output.stdout).into_owned();
                if !output.stderr.is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&String::from_utf8_lossy(&output.stderr));
                }
                return Ok(ToolResult {
                    content,
                    is_error: !output.status.success(),
                    details: Some(serde_json::json!({"exitCode": output.status.code()})),
                });
            }
            Err(_) => continue,
        }
    }
    Err(ToolError::Failed(
        "PowerShell is not available and could not be launched".into(),
    ))
}

fn ls_tool(cwd: &Path, input: &serde_json::Value) -> Result<ToolResult, ToolError> {
    let path = resolve(
        cwd,
        input.get("path").and_then(|v| v.as_str()).unwrap_or("."),
    )?;
    if !path.exists() {
        return Err(ToolError::Failed(format!(
            "Path not found: {}",
            path.display()
        )));
    }
    if !path.is_dir() {
        return Err(ToolError::Failed(format!(
            "Not a directory: {}",
            path.display()
        )));
    }
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(LS_DEFAULT_LIMIT)
        .max(1);
    let mut entries: Vec<String> = fs::read_dir(&path)
        .map_err(|err| ToolError::Failed(format!("Cannot read directory: {err}")))?
        .flatten()
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_dir() {
                format!("{name}/")
            } else {
                name
            }
        })
        .collect();
    entries.sort_by_key(|name| name.to_ascii_lowercase());
    if entries.is_empty() {
        return Ok(ToolResult {
            content: "(empty directory)".into(),
            is_error: false,
            details: None,
        });
    }
    let entry_limit_reached = entries.len() > limit;
    entries.truncate(limit);
    let mut output = entries.join("\n");
    let mut details = serde_json::Map::new();
    if entry_limit_reached {
        output.push_str(&format!(
            "\n\n[{limit} entries limit reached. Use limit={} for more]",
            limit.saturating_mul(2)
        ));
        details.insert("entryLimitReached".into(), serde_json::json!(limit));
    }
    Ok(ToolResult {
        content: output,
        is_error: false,
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
    })
}

fn grep_tool(cwd: &Path, input: &serde_json::Value) -> Result<ToolResult, ToolError> {
    let pattern = required_str(input, "pattern")?;
    let search_path = resolve(
        cwd,
        input.get("path").and_then(|v| v.as_str()).unwrap_or("."),
    )?;
    if !search_path.exists() {
        return Err(ToolError::Failed(format!(
            "Path not found: {}",
            search_path.display()
        )));
    }
    let glob = input.get("glob").and_then(|v| v.as_str());
    let ignore_case = input
        .get("ignoreCase")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let literal = input
        .get("literal")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let context = input
        .get("context")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(GREP_DEFAULT_LIMIT)
        .max(1);
    let is_dir = search_path.is_dir();
    let mut matches = Vec::new();
    let mut lines_truncated = false;
    walk_files(
        &search_path,
        &IgnoreRules::load(&search_path),
        &mut |file| {
            if matches.len() >= limit {
                return false;
            }
            if let Some(glob) = glob {
                if !path_glob_match(glob, file, &search_path) {
                    return true;
                }
            }
            let Ok(body) = fs::read_to_string(file) else {
                return true;
            };
            let file_lines: Vec<&str> = body.lines().collect();
            for (index, line) in file_lines.iter().enumerate() {
                if line_matches(pattern, line, ignore_case, literal) {
                    let display = format_grep_path(file, &search_path, is_dir);
                    let start = index.saturating_sub(context);
                    let end = (index + context + 1).min(file_lines.len());
                    for (current, line) in file_lines.iter().enumerate().take(end).skip(start) {
                        let (text, truncated) = truncate_line(line);
                        if truncated {
                            lines_truncated = true;
                        }
                        if current == index {
                            matches.push(format!("{display}:{}: {text}", current + 1));
                        } else {
                            matches.push(format!("{display}-{}- {text}", current + 1));
                        }
                    }
                    if matches.len() >= limit {
                        break;
                    }
                }
            }
            true
        },
    );
    if matches.is_empty() {
        return Ok(ToolResult {
            content: "No matches found".into(),
            is_error: false,
            details: None,
        });
    }
    let match_limit_reached = matches.len() >= limit;
    matches.truncate(limit);
    let mut output = matches.join("\n");
    let mut details = serde_json::Map::new();
    let mut notices = Vec::new();
    if match_limit_reached {
        notices.push(format!(
            "{limit} matches limit reached. Use limit={} for more, or refine pattern",
            limit.saturating_mul(2)
        ));
        details.insert("matchLimitReached".into(), serde_json::json!(limit));
    }
    if lines_truncated {
        notices.push("some lines truncated".into());
        details.insert("linesTruncated".into(), serde_json::json!(true));
    }
    if !notices.is_empty() {
        output.push_str("\n\n[");
        output.push_str(&notices.join(". "));
        output.push(']');
    }
    Ok(ToolResult {
        content: output,
        is_error: false,
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
    })
}

fn find_tool(cwd: &Path, input: &serde_json::Value) -> Result<ToolResult, ToolError> {
    let pattern = required_str(input, "pattern")?;
    let search_path = resolve(
        cwd,
        input.get("path").and_then(|v| v.as_str()).unwrap_or("."),
    )?;
    if !search_path.exists() {
        return Err(ToolError::Failed(format!(
            "Path not found: {}",
            search_path.display()
        )));
    }
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(FIND_DEFAULT_LIMIT)
        .max(1);
    let mut hits = Vec::new();
    walk_files(
        &search_path,
        &IgnoreRules::load(&search_path),
        &mut |file| {
            if hits.len() >= limit {
                return false;
            }
            if path_glob_match(pattern, file, &search_path) {
                hits.push(relativize_find_result_path(file, &search_path));
            }
            true
        },
    );
    if hits.is_empty() {
        return Ok(ToolResult {
            content: "No files found matching pattern".into(),
            is_error: false,
            details: None,
        });
    }
    let result_limit_reached = hits.len() >= limit;
    hits.truncate(limit);
    let mut output = hits.join("\n");
    let mut details = serde_json::Map::new();
    if result_limit_reached {
        output.push_str(&format!("\n\n[{limit} results limit reached]"));
        details.insert("resultLimitReached".into(), serde_json::json!(limit));
    }
    Ok(ToolResult {
        content: output,
        is_error: false,
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
    })
}

pub fn relativize_find_result_path(result_path: &Path, search_path: &Path) -> String {
    let display = result_path.to_string_lossy();
    let had_trailing = display.ends_with('/') || display.ends_with('\\');
    let relative = if result_path.is_absolute() {
        result_path
            .strip_prefix(search_path)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| display.into_owned())
    } else {
        display.into_owned()
    };
    let mut posix = relative.replace('\\', "/");
    if posix.is_empty() {
        posix = ".".into();
    }
    if had_trailing && !posix.ends_with('/') {
        posix.push('/');
    }
    posix
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

struct IgnoreRules {
    patterns: Vec<String>,
}

impl IgnoreRules {
    fn load(root: &Path) -> Self {
        let mut patterns = vec![".git".into(), "node_modules".into()];
        let mut current = if root.is_file() {
            root.parent().unwrap_or(root).to_path_buf()
        } else {
            root.to_path_buf()
        };
        loop {
            let gitignore = current.join(".gitignore");
            if let Ok(body) = fs::read_to_string(&gitignore) {
                for line in body.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                        continue;
                    }
                    patterns.push(line.trim_end_matches('/').to_string());
                }
            }
            if current.join(".git").exists() {
                break;
            }
            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => break,
            }
        }
        Self { patterns }
    }

    fn ignored(&self, path: &Path) -> bool {
        let posix = path.to_string_lossy().replace('\\', "/");
        self.patterns.iter().any(|pattern| {
            posix.split('/').any(|part| glob_match(pattern, part))
                || glob_match(pattern, &posix)
                || posix.ends_with(pattern)
        })
    }
}

fn walk_files(root: &Path, ignore: &IgnoreRules, visit: &mut dyn FnMut(&Path) -> bool) {
    if root.is_file() {
        let _ = visit(root);
        return;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if ignore.ignored(&path) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if !visit(&path) {
                return;
            }
        }
    }
}

fn format_grep_path(file: &Path, search_path: &Path, is_dir: bool) -> String {
    if is_dir {
        let relative = file
            .strip_prefix(search_path)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| {
                file.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        if !relative.is_empty() && !relative.starts_with("..") {
            return relative;
        }
    }
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string())
}

fn line_matches(pattern: &str, line: &str, ignore_case: bool, literal: bool) -> bool {
    if literal {
        return if ignore_case {
            line.to_ascii_lowercase()
                .contains(&pattern.to_ascii_lowercase())
        } else {
            line.contains(pattern)
        };
    }
    let haystack = if ignore_case {
        line.to_ascii_lowercase()
    } else {
        line.to_string()
    };
    let needle = if ignore_case {
        pattern.to_ascii_lowercase()
    } else {
        pattern.to_string()
    };
    if let Ok(regex) = regex::Regex::new(&needle) {
        regex.is_match(&haystack)
    } else {
        haystack.contains(&needle)
    }
}

fn truncate_line(line: &str) -> (String, bool) {
    let sanitized = line.replace('\r', "");
    if sanitized.chars().count() > GREP_MAX_LINE_LENGTH {
        (sanitized.chars().take(GREP_MAX_LINE_LENGTH).collect(), true)
    } else {
        (sanitized, false)
    }
}

fn truncate_read(
    content: &str,
    offset: usize,
    limit: Option<usize>,
) -> (String, serde_json::Value) {
    let lines: Vec<&str> = if content.is_empty() {
        Vec::new()
    } else {
        let mut lines: Vec<&str> = content.split('\n').collect();
        if content.ends_with('\n') {
            lines.pop();
        }
        lines
    };
    let start = offset.saturating_sub(1).min(lines.len());
    let max_lines = limit.unwrap_or(DEFAULT_MAX_LINES);
    let mut out = Vec::new();
    let mut bytes = 0usize;
    let mut truncated_by = None;
    for (index, line) in lines.iter().enumerate().skip(start) {
        if out.len() >= max_lines {
            truncated_by = Some("lines");
            break;
        }
        let add = if index > start || !out.is_empty() {
            line.len() + 1
        } else {
            line.len()
        };
        if bytes + add > DEFAULT_MAX_BYTES {
            truncated_by = Some("bytes");
            break;
        }
        out.push(*line);
        bytes += add;
    }
    let output = out.join("\n");
    (
        output.clone(),
        serde_json::json!({
            "truncated": truncated_by.is_some(),
            "truncatedBy": truncated_by,
            "totalLines": lines.len(),
            "totalBytes": content.len(),
            "outputLines": out.len(),
            "outputBytes": output.len(),
            "maxLines": max_lines,
            "maxBytes": DEFAULT_MAX_BYTES,
        }),
    )
}

fn path_glob_match(pattern: &str, file: &Path, search_path: &Path) -> bool {
    let relative = file
        .strip_prefix(search_path)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| file.to_string_lossy().replace('\\', "/"));
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    glob_match(pattern, &relative) || glob_match(pattern, &name)
}

fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == "**" || pattern == "**/*" {
        return true;
    }
    let pattern = pattern.replace('\\', "/");
    let name = name.replace('\\', "/");
    if let Some(stripped) = pattern.strip_prefix("**/") {
        return glob_match(stripped, &name)
            || name
                .rsplit('/')
                .next()
                .is_some_and(|part| glob_match(stripped, part))
            || name.split('/').any(|part| glob_match(stripped, part));
    }
    match_glob_chars(&pattern, &name)
}

fn match_glob_chars(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    fn rec(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) if p.get(1) == Some(&'*') => {
                let rest = if p.get(2) == Some(&'/') {
                    &p[3..]
                } else {
                    &p[2..]
                };
                rec(rest, n)
                    || (!n.is_empty() && rec(p, &n[1..]))
                    || (p.get(2) == Some(&'/') && n.first() == Some(&'/') && rec(&p[3..], &n[1..]))
            }
            (Some('*'), _) => rec(&p[1..], n) || (!n.is_empty() && rec(p, &n[1..])),
            (Some('?'), Some(_)) => rec(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => rec(&p[1..], &n[1..]),
            _ => false,
        }
    }
    rec(&p, &n)
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
        execute_tool(
            dir.path(),
            "write",
            &serde_json::json!({"path":"b.txt","content":"one\ntwo\nthree\n"}),
        )
        .unwrap();
        let sliced = execute_tool(
            dir.path(),
            "read",
            &serde_json::json!({"path":"b.txt","offset":2,"limit":1}),
        )
        .unwrap();
        assert_eq!(sliced.content, "two");
        assert_eq!(
            sliced.details.as_ref().unwrap()["truncation"]["totalLines"],
            3
        );
        execute_tool(
            dir.path(),
            "write",
            &serde_json::json!({"path":"c.txt","content":"alpha\nbeta\ngamma\n"}),
        )
        .unwrap();
        execute_tool(
            dir.path(),
            "edit",
            &serde_json::json!({
                "path":"c.txt",
                "edits":[
                    {"oldText":"alpha","newText":"ALPHA"},
                    {"oldText":"gamma","newText":"GAMMA"}
                ]
            }),
        )
        .unwrap();
        let on_disk = fs::read_to_string(dir.path().join("c.txt")).unwrap();
        assert_eq!(on_disk, "ALPHA\nbeta\nGAMMA\n");
        let missing = execute_tool(
            dir.path(),
            "edit",
            &serde_json::json!({"path":"c.txt","edits":[{"oldText":"nope","newText":"x"}]}),
        )
        .unwrap_err();
        assert!(missing
            .to_string()
            .contains("Could not find the exact text in c.txt"));
    }

    #[test]
    fn bash_timeout_matches_ts_errors() {
        let dir = tempdir().unwrap();
        let invalid = execute_tool(
            dir.path(),
            "bash",
            &serde_json::json!({"command":"true","timeout":0}),
        )
        .unwrap_err();
        assert_eq!(
            invalid.to_string(),
            "Invalid timeout: must be a finite number of seconds"
        );
        let timed_out = execute_tool(
            dir.path(),
            "bash",
            &serde_json::json!({"command":"sleep 2","timeout":0.2}),
        )
        .unwrap_err();
        assert!(timed_out
            .to_string()
            .contains("Command timed out after 0.2 seconds"));
    }

    #[test]
    fn grep_find_ls_match_ts_strings() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/app.ts"), "const needle = 1;\nkeep\n").unwrap();
        fs::write(dir.path().join(".gitignore"), "secret.txt\n").unwrap();
        fs::write(dir.path().join("secret.txt"), "needle hidden").unwrap();
        let grep = execute_tool(
            dir.path(),
            "grep",
            &serde_json::json!({"pattern":"needle","glob":"*.ts"}),
        )
        .unwrap();
        assert!(grep.content.contains("src/app.ts:1: const needle = 1;"));
        assert!(!grep.content.contains("secret.txt"));
        let missing = execute_tool(
            dir.path(),
            "grep",
            &serde_json::json!({"pattern":"nope","path":"missing"}),
        )
        .unwrap_err();
        assert!(missing.to_string().starts_with("Path not found:"));
        let none =
            execute_tool(dir.path(), "grep", &serde_json::json!({"pattern":"zzzz"})).unwrap();
        assert_eq!(none.content, "No matches found");
        let found =
            execute_tool(dir.path(), "find", &serde_json::json!({"pattern":"*.ts"})).unwrap();
        assert_eq!(found.content, "src/app.ts");
        let empty =
            execute_tool(dir.path(), "find", &serde_json::json!({"pattern":"*.rs"})).unwrap();
        assert_eq!(empty.content, "No files found matching pattern");
        let listed = execute_tool(dir.path(), "ls", &serde_json::json!({})).unwrap();
        assert!(listed.content.contains("src/"));
        let empty_dir = dir.path().join("blank");
        fs::create_dir_all(&empty_dir).unwrap();
        let empty_ls =
            execute_tool(dir.path(), "ls", &serde_json::json!({"path":"blank"})).unwrap();
        assert_eq!(empty_ls.content, "(empty directory)");
        let not_dir =
            execute_tool(dir.path(), "ls", &serde_json::json!({"path":"src/app.ts"})).unwrap_err();
        assert!(not_dir.to_string().starts_with("Not a directory:"));
    }

    #[test]
    fn powershell_and_image_read() {
        std::env::set_var("PI_POWERSHELL_REPLY", "ps-ok");
        let dir = tempdir().unwrap();
        let ps = execute_tool(
            dir.path(),
            "powershell",
            &serde_json::json!({"command":"Get-Date"}),
        )
        .unwrap();
        std::env::remove_var("PI_POWERSHELL_REPLY");
        assert_eq!(ps.content, "ps-ok");
        assert!(tool_specs().iter().any(|tool| tool.name == "powershell"));
        let png = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(png)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        fs::write(dir.path().join("dot.png"), &bytes).unwrap();
        let read =
            execute_tool(dir.path(), "read", &serde_json::json!({"path":"dot.png"})).unwrap();
        assert!(read.content.starts_with("Read image file [image/png]"));
        assert!(read.details.as_ref().unwrap()["image"]["data"].is_string());
    }

    #[test]
    fn relativize_find_result_is_posix() {
        assert_eq!(
            relativize_find_result_path(Path::new("/tmp/root/src/app.ts"), Path::new("/tmp/root")),
            "src/app.ts"
        );
    }
}
