use base64::Engine;
use davinci_agent::{ToolError, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

pub const VISUAL_SNAPSHOT_TOOL: &str = "visual_snapshot";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_COMMAND_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_VIEWPORT: u32 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualSnapshotRequest {
    pub url: String,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualSnapshotResult {
    pub image_path: PathBuf,
    pub width: u32,
    pub height: u32,
}

pub trait VisualSnapshotBackend: Send + Sync {
    fn name(&self) -> &str;
    fn available(&self, cwd: &Path) -> bool;
    fn capture(
        &self,
        cwd: &Path,
        request: &VisualSnapshotRequest,
    ) -> Result<VisualSnapshotResult, String>;
}

#[derive(Clone, Default)]
pub struct VisualSnapshotHost {
    backend: Option<Arc<dyn VisualSnapshotBackend>>,
}

impl std::fmt::Debug for VisualSnapshotHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisualSnapshotHost")
            .field(
                "backend",
                &self.backend.as_ref().map(|backend| backend.name()),
            )
            .finish()
    }
}

impl VisualSnapshotHost {
    pub fn discover(cwd: &Path) -> Self {
        if let Some(backend) =
            ExplicitCommandBackend::from_env().filter(|backend| backend.available(cwd))
        {
            return Self::from_backend(Arc::new(backend), cwd);
        }

        // The native host has no safe, typed registry for borrowing an
        // already-connected browser MCP session. Do not guess at a connector
        // or invoke an arbitrary server; a future registry can be inserted at
        // this point without changing the fallback order.
        if let Some(backend) = discover_connected_browser_backend(cwd) {
            return Self::from_backend(backend, cwd);
        }

        if let Some(backend) = ProjectPlaywrightBackend::discover(cwd) {
            return Self::from_backend(Arc::new(backend), cwd);
        }

        Self::default()
    }

    pub fn from_backend(backend: Arc<dyn VisualSnapshotBackend>, cwd: &Path) -> Self {
        if backend.available(cwd) {
            Self {
                backend: Some(backend),
            }
        } else {
            Self::default()
        }
    }

    pub fn is_available(&self) -> bool {
        self.backend.is_some()
    }

    pub fn execute_tool(&self, cwd: &Path, args: &Value) -> Result<ToolResult, ToolError> {
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| ToolError::Failed("visual snapshot backend is unavailable".into()))?;
        let request: VisualSnapshotRequest =
            serde_json::from_value(args.clone()).map_err(|error| {
                ToolError::Failed(format!("invalid visual snapshot request: {error}"))
            })?;
        validate_request(&request)?;

        let snapshot = backend
            .capture(cwd, &request)
            .map_err(|error| ToolError::Failed(format!("visual snapshot failed: {error}")))?;
        if snapshot.width == 0
            || snapshot.height == 0
            || snapshot.width > MAX_VIEWPORT
            || snapshot.height > MAX_VIEWPORT
        {
            return Err(ToolError::Failed(
                "visual snapshot backend returned invalid dimensions".into(),
            ));
        }

        let image_path = safe_image_path(cwd, &snapshot.image_path)?;
        let metadata = std::fs::metadata(&image_path).map_err(|error| {
            ToolError::Failed(format!("cannot inspect snapshot image: {error}"))
        })?;
        if !metadata.is_file() {
            return Err(ToolError::Failed(
                "snapshot image path is not a file".into(),
            ));
        }
        if metadata.len() == 0 || metadata.len() > MAX_IMAGE_BYTES {
            return Err(ToolError::Failed(
                "snapshot image is empty or exceeds the 16 MiB limit".into(),
            ));
        }
        let bytes = std::fs::read(&image_path)
            .map_err(|error| ToolError::Failed(format!("cannot read snapshot image: {error}")))?;
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        let mime_type = image_mime_type(&image_path);
        let project_root = canonicalize_cwd(cwd)?;
        let relative_path = image_path
            .strip_prefix(&project_root)
            .unwrap_or(&image_path)
            .to_string_lossy()
            .into_owned();

        Ok(ToolResult {
            content: format!(
                "Captured {} at {}x{} using {} ({}).",
                request.url,
                snapshot.width,
                snapshot.height,
                backend.name(),
                relative_path
            ),
            is_error: false,
            details: Some(json!({
                "image": {"data": data, "mimeType": mime_type},
                "imagePath": relative_path,
                "width": snapshot.width,
                "height": snapshot.height,
                "backend": backend.name(),
            })),
        })
    }
}

pub fn visual_snapshot_tool_spec() -> davinci_ai::ToolSpec {
    davinci_ai::ToolSpec {
        name: VISUAL_SNAPSHOT_TOOL.to_string(),
        description: "Capture a screenshot through an already available visual backend and return it as image content. Never downloads a browser automatically.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "format": "uri", "minLength": 1},
                "viewportWidth": {"type": "integer", "minimum": 1, "maximum": MAX_VIEWPORT},
                "viewportHeight": {"type": "integer", "minimum": 1, "maximum": MAX_VIEWPORT}
            },
            "required": ["url", "viewportWidth", "viewportHeight"],
            "additionalProperties": false
        }),
        constrained_sampling: None,
    }
}

fn validate_request(request: &VisualSnapshotRequest) -> Result<(), ToolError> {
    if request.url.trim().is_empty() {
        return Err(ToolError::Failed(
            "visual snapshot URL cannot be empty".into(),
        ));
    }
    let parsed = Url::parse(&request.url)
        .map_err(|error| ToolError::Failed(format!("invalid visual snapshot URL: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https" | "file") {
        return Err(ToolError::Failed(
            "visual snapshot URL must use http, https, or file".into(),
        ));
    }
    if request.viewport_width == 0
        || request.viewport_height == 0
        || request.viewport_width > MAX_VIEWPORT
        || request.viewport_height > MAX_VIEWPORT
    {
        return Err(ToolError::Failed(format!(
            "viewport must be between 1 and {MAX_VIEWPORT} pixels"
        )));
    }
    Ok(())
}

fn canonicalize_cwd(cwd: &Path) -> Result<PathBuf, ToolError> {
    cwd.canonicalize()
        .map_err(|error| ToolError::Failed(format!("cannot resolve visual snapshot cwd: {error}")))
}

fn safe_image_path(cwd: &Path, path: &Path) -> Result<PathBuf, ToolError> {
    let cwd = canonicalize_cwd(cwd)?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|error| ToolError::Failed(format!("cannot resolve snapshot image: {error}")))?;
    if canonical != cwd && !canonical.starts_with(&cwd) {
        return Err(ToolError::Failed(
            "snapshot image must be inside the project directory".into(),
        ));
    }
    Ok(canonical)
}

fn image_mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(OsStr::to_str)
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        _ => "image/png",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedCommand {
    program: String,
    args: Vec<String>,
}

fn parse_command_line(raw: &str) -> Result<ParsedCommand, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            Some('"') => {
                if ch == '"' {
                    quote = None;
                } else if ch == '\\' && matches!(chars.peek(), Some('"' | '\\')) {
                    current.push(chars.next().expect("peeked character"));
                } else {
                    current.push(ch);
                }
            }
            Some(_) => current.push(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None => current.push(ch),
        }
    }

    if quote.is_some() {
        return Err("visual snapshot command has an unterminated quote".into());
    }
    if !current.is_empty() {
        words.push(current);
    }
    let Some(program) = words.first().cloned() else {
        return Err("visual snapshot command is empty".into());
    };
    Ok(ParsedCommand {
        program,
        args: words.into_iter().skip(1).collect(),
    })
}

#[derive(Debug, Clone)]
struct ExplicitCommandBackend {
    command: ParsedCommand,
}

impl ExplicitCommandBackend {
    fn from_env() -> Option<Self> {
        let raw = std::env::var("DAVINCI_VISUAL_SNAPSHOT_COMMAND").ok()?;
        parse_command_line(&raw)
            .ok()
            .map(|command| Self { command })
    }

    fn resolved_program(&self, cwd: &Path) -> Option<PathBuf> {
        resolve_program(&self.command.program, cwd)
    }
}

impl VisualSnapshotBackend for ExplicitCommandBackend {
    fn name(&self) -> &str {
        "explicit_command"
    }

    fn available(&self, cwd: &Path) -> bool {
        self.resolved_program(cwd).is_some()
    }

    fn capture(
        &self,
        cwd: &Path,
        request: &VisualSnapshotRequest,
    ) -> Result<VisualSnapshotResult, String> {
        let program = self
            .resolved_program(cwd)
            .ok_or_else(|| "configured visual snapshot command is not executable".to_string())?;
        let args: Vec<OsString> = self.command.args.iter().map(OsString::from).collect();
        run_json_command(cwd, &program, &args, &[], request)
    }
}

#[derive(Debug, Clone)]
struct ProjectPlaywrightBackend {
    node: PathBuf,
    project_root: PathBuf,
}

impl ProjectPlaywrightBackend {
    fn discover(cwd: &Path) -> Option<Self> {
        let node = resolve_program("node", cwd)?;
        for root in cwd.ancestors() {
            let playwright_package = root.join("node_modules").join("playwright");
            if !playwright_package.join("package.json").is_file() {
                continue;
            }
            let has_executable = [
                root.join("node_modules").join(".bin").join("playwright"),
                root.join("node_modules")
                    .join(".bin")
                    .join("playwright.cmd"),
            ]
            .iter()
            .any(|path| path.is_file());
            let has_config = ["js", "ts", "cjs", "mjs"]
                .iter()
                .map(|extension| root.join(format!("playwright.config.{extension}")))
                .any(|path| path.is_file());
            if has_executable || has_config {
                return Some(Self {
                    node,
                    project_root: root.to_path_buf(),
                });
            }
        }
        None
    }

    fn module_path(&self) -> PathBuf {
        self.project_root.join("node_modules")
    }
}

impl VisualSnapshotBackend for ProjectPlaywrightBackend {
    fn name(&self) -> &str {
        "project_playwright"
    }

    fn available(&self, cwd: &Path) -> bool {
        self.node.is_file()
            && self.project_root.is_dir()
            && self
                .module_path()
                .join("playwright")
                .join("package.json")
                .is_file()
            && cwd.starts_with(&self.project_root)
    }

    fn capture(
        &self,
        cwd: &Path,
        request: &VisualSnapshotRequest,
    ) -> Result<VisualSnapshotResult, String> {
        let args = vec![OsString::from("-e"), OsString::from(PLAYWRIGHT_SCRIPT)];
        let env = [(
            OsString::from("NODE_PATH"),
            self.module_path().into_os_string(),
        )];
        run_json_command(cwd, &self.node, &args, &env, request)
    }
}

fn discover_connected_browser_backend(_cwd: &Path) -> Option<Arc<dyn VisualSnapshotBackend>> {
    None
}

fn resolve_program(program: &str, cwd: &Path) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() || program.contains('/') || program.contains('\\') {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        return executable_candidate(candidate);
    }

    let path_var = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path_var) {
        let base = directory.join(program);
        if let Some(candidate) = executable_candidate(base) {
            return Some(candidate);
        }
    }
    None
}

fn executable_candidate(path: PathBuf) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path);
    }
    #[cfg(windows)]
    {
        if path.extension().is_none() {
            for extension in ["exe", "cmd", "bat", "com"] {
                let candidate = path.with_extension(extension);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn command_timeout() -> Duration {
    std::env::var("DAVINCI_VISUAL_SNAPSHOT_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|millis| Duration::from_millis(millis.clamp(1_000, 120_000)))
        .unwrap_or(DEFAULT_TIMEOUT)
}

fn run_json_command(
    cwd: &Path,
    program: &Path,
    args: &[OsString],
    env: &[(OsString, OsString)],
    request: &VisualSnapshotRequest,
) -> Result<VisualSnapshotResult, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start visual snapshot command: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "visual snapshot command has no stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "visual snapshot command has no stderr".to_string())?;
    let stdout_reader = std::thread::spawn(|| read_bounded(stdout));
    let stderr_reader = std::thread::spawn(|| read_bounded(stderr));

    let request_json = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| "visual snapshot command has no stdin".to_string())
        .and_then(|mut stdin| {
            stdin
                .write_all(&request_json)
                .and_then(|_| stdin.write_all(b"\n"))
                .map_err(|error| format!("cannot send visual snapshot request: {error}"))
        });
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        return Err(error);
    }

    let deadline = Instant::now() + command_timeout();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("visual snapshot command timed out".into());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format!("cannot wait for visual snapshot command: {error}"));
            }
        }
    };

    let stdout = stdout_reader
        .join()
        .map_err(|_| "visual snapshot stdout reader panicked".to_string())??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "visual snapshot stderr reader panicked".to_string())??;
    if !status.success() {
        let message = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(if message.is_empty() {
            format!("visual snapshot command exited with {status}")
        } else {
            format!("visual snapshot command exited with {status}: {message}")
        });
    }
    serde_json::from_slice::<VisualSnapshotResult>(&stdout).map_err(|error| {
        format!(
            "visual snapshot command returned invalid JSON: {error} ({})",
            String::from_utf8_lossy(&stdout).trim()
        )
    })
}

fn read_bounded(mut reader: impl Read) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut exceeded = false;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("cannot read visual snapshot command output: {error}"))?;
        if count == 0 {
            break;
        }
        let remaining = MAX_COMMAND_OUTPUT.saturating_sub(output.len());
        if remaining > 0 {
            output.extend_from_slice(&buffer[..count.min(remaining)]);
        }
        if count > remaining {
            exceeded = true;
        }
    }
    if exceeded {
        Err("visual snapshot command output exceeded the 4 MiB limit".into())
    } else {
        Ok(output)
    }
}

const PLAYWRIGHT_SCRIPT: &str = r#"
const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");
let input = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", chunk => { input += chunk; });
process.stdin.on("end", async () => {
  let browser;
  try {
    const request = JSON.parse(input);
    const outputDir = path.join(process.cwd(), ".pi", "visual-snapshots");
    fs.mkdirSync(outputDir, { recursive: true });
    const outputPath = path.join(outputDir, `snapshot-${process.pid}-${Date.now()}.png`);
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage({ viewport: {
      width: request.viewportWidth,
      height: request.viewportHeight
    }});
    await page.goto(request.url, { waitUntil: "domcontentloaded", timeout: 25000 });
    await page.screenshot({ path: outputPath, fullPage: true });
    process.stdout.write(JSON.stringify({
      imagePath: path.relative(process.cwd(), outputPath),
      width: request.viewportWidth,
      height: request.viewportHeight
    }));
  } catch (error) {
    process.stderr.write(String(error && error.stack || error));
    process.exitCode = 1;
  } finally {
    if (browser) await browser.close();
  }
});
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct FakeVisualBackend {
        request: Arc<Mutex<Option<VisualSnapshotRequest>>>,
        image_path: PathBuf,
    }

    impl VisualSnapshotBackend for FakeVisualBackend {
        fn name(&self) -> &str {
            "fake_visual_backend"
        }

        fn available(&self, _cwd: &Path) -> bool {
            true
        }

        fn capture(
            &self,
            _cwd: &Path,
            request: &VisualSnapshotRequest,
        ) -> Result<VisualSnapshotResult, String> {
            *self.request.lock().unwrap() = Some(request.clone());
            Ok(VisualSnapshotResult {
                image_path: self.image_path.clone(),
                width: request.viewport_width,
                height: request.viewport_height,
            })
        }
    }

    #[test]
    fn fake_backend_registers_and_returns_existing_image_as_tool_content() {
        let dir = tempfile::tempdir().unwrap();
        let image_path = dir.path().join("snapshot.png");
        std::fs::write(&image_path, b"fake-png").unwrap();
        let request_log = Arc::new(Mutex::new(None));
        let backend = FakeVisualBackend {
            request: Arc::clone(&request_log),
            image_path,
        };
        let host = VisualSnapshotHost::from_backend(Arc::new(backend), dir.path());

        assert!(host.is_available());
        let result = host
            .execute_tool(
                dir.path(),
                &serde_json::json!({
                    "url": "http://localhost:4173",
                    "viewportWidth": 1024,
                    "viewportHeight": 768
                }),
            )
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(
            result.details.as_ref().unwrap()["image"]["mimeType"],
            "image/png"
        );
        assert_eq!(
            request_log.lock().unwrap().as_ref().unwrap().url,
            "http://localhost:4173"
        );
    }

    #[test]
    fn unavailable_host_does_not_advertise_a_tool() {
        let host = VisualSnapshotHost::default();
        assert!(!host.is_available());
        assert!(host
            .execute_tool(Path::new("."), &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn command_line_parser_keeps_arguments_outside_a_shell() {
        let parsed = parse_command_line(r#"node "capture script.js" --mode=visual"#).unwrap();
        assert_eq!(parsed.program, "node");
        assert_eq!(parsed.args, vec!["capture script.js", "--mode=visual"]);
        assert!(parse_command_line("node 'unterminated").is_err());
    }

    #[test]
    fn image_paths_cannot_escape_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let image_path = outside.path().join("outside.png");
        std::fs::write(&image_path, b"not for this project").unwrap();
        assert!(safe_image_path(dir.path(), &image_path).is_err());
    }
}
