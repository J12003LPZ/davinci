//! Lossless token-governor middleware.
//!
//! The governor is intentionally deterministic and fail-open.  It keeps a
//! reversible copy of large successful tool outputs, returns a compact digest
//! to the model, and records enough metadata for `retrieve_output` to recover
//! the original text.  No network access is required.
//!
//! Only the `content` of a tool result reaches the model (`details` is for
//! the UI and events), so everything the model needs to undo a compression —
//! the output id and how to call `retrieve_output` — is written into the
//! digest itself.

use davinci_agent::{ToolError, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const DEFAULT_COMPRESS_THRESHOLD_BYTES: usize = 8_192;
const DEFAULT_COMPRESS_THRESHOLD_LINES: usize = 200;
const DEFAULT_KEEP_HEAD_LINES: usize = 15;
const DEFAULT_KEEP_TAIL_LINES: usize = 30;
const DEFAULT_MAX_IMPORTANT_LINES: usize = 60;
const DEFAULT_MAX_LEDGER_ENTRIES: usize = 200;
/// A short window limits read suppression. The host also clears visibility
/// ledgers on actual pruning and compaction, including non-default settings.
const DEFAULT_DEDUPE_WINDOW: usize = 6;
/// `retrieve_output` answers at most this many bytes per call and says where
/// to continue; the whole point of the store is to not flood the context.
const DEFAULT_RETRIEVE_MAX_BYTES: usize = 48_000;
/// How many stored outputs the status payload lists, newest first.
const STORED_MANIFEST_ENTRIES: usize = 64;
/// Stored outputs of sessions untouched for this long are swept on startup.
pub const STORE_RETENTION: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// Tools whose output the model needs verbatim: `read` has its own
/// offset/limit paging and `edit` must match the bytes it was shown; `batch`
/// already caps and structures its sub-results; the rest are small or are
/// themselves the governor's or memory's answer. These are never digested
/// (the `read` dedupe still applies).
const LOSSLESS_TOOLS: &[&str] = &[
    "read",
    "edit",
    "write",
    "notebook_edit",
    "batch",
    "todo",
    "agent",
    "retrieve_output",
    "memory_search",
    "graph_submit",
];

/// Returns whether a tool's output may be compressed by the token governor.
pub fn tool_may_be_compressed(name: &str) -> bool {
    !LOSSLESS_TOOLS.contains(&name)
}

/// Tools the anti-loop ledger watches: pure queries whose answer only changes
/// when the repository does.
const SEARCH_TOOLS: &[&str] = &["grep", "find", "ls"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenGovernorConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_compress_threshold_bytes")]
    pub compress_threshold_bytes: usize,
    #[serde(default = "default_compress_threshold_lines")]
    pub compress_threshold_lines: usize,
    #[serde(default = "default_keep_head_lines")]
    pub keep_head_lines: usize,
    #[serde(default = "default_keep_tail_lines")]
    pub keep_tail_lines: usize,
    #[serde(default = "default_max_important_lines")]
    pub max_important_lines: usize,
    #[serde(default = "default_true")]
    pub dedupe_reads: bool,
    #[serde(default = "default_dedupe_window")]
    pub dedupe_window: usize,
    #[serde(default = "default_true")]
    pub anti_loop: bool,
    #[serde(default = "default_retrieve_max_bytes")]
    pub retrieve_max_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_dir: Option<PathBuf>,
}

fn default_true() -> bool {
    true
}
fn default_compress_threshold_bytes() -> usize {
    DEFAULT_COMPRESS_THRESHOLD_BYTES
}
fn default_compress_threshold_lines() -> usize {
    DEFAULT_COMPRESS_THRESHOLD_LINES
}
fn default_keep_head_lines() -> usize {
    DEFAULT_KEEP_HEAD_LINES
}
fn default_keep_tail_lines() -> usize {
    DEFAULT_KEEP_TAIL_LINES
}
fn default_max_important_lines() -> usize {
    DEFAULT_MAX_IMPORTANT_LINES
}
fn default_dedupe_window() -> usize {
    DEFAULT_DEDUPE_WINDOW
}
fn default_retrieve_max_bytes() -> usize {
    DEFAULT_RETRIEVE_MAX_BYTES
}

impl Default for TokenGovernorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            compress_threshold_bytes: DEFAULT_COMPRESS_THRESHOLD_BYTES,
            compress_threshold_lines: DEFAULT_COMPRESS_THRESHOLD_LINES,
            keep_head_lines: DEFAULT_KEEP_HEAD_LINES,
            keep_tail_lines: DEFAULT_KEEP_TAIL_LINES,
            max_important_lines: DEFAULT_MAX_IMPORTANT_LINES,
            dedupe_reads: true,
            dedupe_window: DEFAULT_DEDUPE_WINDOW,
            anti_loop: true,
            retrieve_max_bytes: DEFAULT_RETRIEVE_MAX_BYTES,
            store_dir: None,
        }
    }
}

impl TokenGovernorConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        apply_env(&mut config);
        config
    }

    pub fn from_file(path: &Path) -> Self {
        let mut config = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default();
        apply_env(&mut config);
        config
    }
}

fn apply_env(config: &mut TokenGovernorConfig) {
    if let Some(value) = env_bool_any(&[
        "DAVINCI_GOVERNOR_ENABLED",
        "DAVINCI_TOKEN_GOVERNOR_ENABLED",
        "PI_GOVERNOR_ENABLED",
        "PI_TOKEN_GOVERNOR_ENABLED",
    ]) {
        config.enabled = value;
    }
    if let Some(value) = env_usize_any(&[
        "DAVINCI_GOVERNOR_COMPRESS_THRESHOLD",
        "DAVINCI_TOKEN_GOVERNOR_COMPRESS_THRESHOLD_BYTES",
        "PI_GOVERNOR_COMPRESS_THRESHOLD",
        "PI_TOKEN_GOVERNOR_COMPRESS_THRESHOLD_BYTES",
    ]) {
        config.compress_threshold_bytes = value.max(1);
    }
    if let Some(value) = env_usize_any(&[
        "DAVINCI_TOKEN_GOVERNOR_COMPRESS_THRESHOLD_LINES",
        "PI_TOKEN_GOVERNOR_COMPRESS_THRESHOLD_LINES",
    ]) {
        config.compress_threshold_lines = value.max(1);
    }
    if let Some(value) = env_usize_any(&[
        "DAVINCI_TOKEN_GOVERNOR_KEEP_HEAD_LINES",
        "PI_TOKEN_GOVERNOR_KEEP_HEAD_LINES",
    ]) {
        config.keep_head_lines = value;
    }
    if let Some(value) = env_usize_any(&[
        "DAVINCI_TOKEN_GOVERNOR_KEEP_TAIL_LINES",
        "PI_TOKEN_GOVERNOR_KEEP_TAIL_LINES",
    ]) {
        config.keep_tail_lines = value;
    }
    if let Some(value) = env_usize_any(&[
        "DAVINCI_TOKEN_GOVERNOR_MAX_IMPORTANT_LINES",
        "PI_TOKEN_GOVERNOR_MAX_IMPORTANT_LINES",
    ]) {
        config.max_important_lines = value.max(1);
    }
    if let Some(value) = env_bool_any(&[
        "DAVINCI_TOKEN_GOVERNOR_DEDUPE_READS",
        "PI_TOKEN_GOVERNOR_DEDUPE_READS",
    ]) {
        config.dedupe_reads = value;
    }
    if let Some(value) = env_usize_any(&[
        "DAVINCI_TOKEN_GOVERNOR_DEDUPE_WINDOW",
        "PI_TOKEN_GOVERNOR_DEDUPE_WINDOW",
    ]) {
        config.dedupe_window = value;
    }
    if let Some(value) = env_bool_any(&[
        "DAVINCI_TOKEN_GOVERNOR_ANTI_LOOP",
        "PI_TOKEN_GOVERNOR_ANTI_LOOP",
    ]) {
        config.anti_loop = value;
    }
    if let Some(value) = env_usize_any(&[
        "DAVINCI_TOKEN_GOVERNOR_RETRIEVE_MAX_BYTES",
        "PI_TOKEN_GOVERNOR_RETRIEVE_MAX_BYTES",
    ]) {
        config.retrieve_max_bytes = value.max(1_024);
    }
    if let Some(value) = env_string_any(&[
        "DAVINCI_GOVERNOR_STORE_DIR",
        "DAVINCI_TOKEN_GOVERNOR_DIR",
        "PI_GOVERNOR_STORE_DIR",
        "PI_TOKEN_GOVERNOR_DIR",
    ]) {
        config.store_dir = Some(PathBuf::from(value));
    }
}

fn env_string_any(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn env_bool_any(names: &[&str]) -> Option<bool> {
    names.iter().find_map(|name| env_bool(name))
}

fn env_usize_any(names: &[&str]) -> Option<usize> {
    names.iter().find_map(|name| env_usize(name))
}

fn env_bool(name: &str) -> Option<bool> {
    match std::env::var(name)
        .ok()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.trim().parse().ok()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredOutputRef {
    pub id: String,
    pub bytes: usize,
    pub lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompressionInfo {
    pub compressed: bool,
    pub original_bytes: usize,
    pub original_lines: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored: Option<StoredOutputRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressedOutput {
    pub content: String,
    pub info: CompressionInfo,
}

/// One entry of the status payload's `stored` manifest: what a stored output
/// came from, so `/governor-status` can name it (`bash · cargo test`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredOutputEntry {
    pub id: String,
    pub tool: String,
    pub call: String,
    pub bytes: usize,
    pub lines: usize,
}

pub fn call_fingerprint(tool_name: &str, args: &Value, state_hash: &str) -> String {
    let normalized = normalize_json(args);
    sha256_hex(format!("{tool_name}\0{normalized}\0{state_hash}").as_bytes())
}

fn call_key(tool_name: &str, args: &Value) -> String {
    format!("{tool_name}\0{}", normalize_json(args))
}

pub fn normalize_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut out = String::from("{");
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                out.push_str(&normalize_json(&map[key]));
            }
            out.push('}');
            out
        }
        Value::Array(values) => {
            let items = values.iter().map(normalize_json).collect::<Vec<_>>();
            format!("[{}]", items.join(","))
        }
        _ => value.to_string(),
    }
}

pub fn file_content_hash(content: &str) -> String {
    sha256_hex(content.as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, Default)]
struct BoundedSet {
    values: HashSet<String>,
    order: VecDeque<String>,
}

impl BoundedSet {
    fn insert(&mut self, value: String) -> bool {
        if !self.values.insert(value.clone()) {
            return false;
        }
        self.order.push_back(value);
        while self.order.len() > DEFAULT_MAX_LEDGER_ENTRIES {
            if let Some(old) = self.order.pop_front() {
                self.values.remove(&old);
            }
        }
        true
    }

    fn contains(&self, value: &str) -> bool {
        self.values.contains(value)
    }

    fn clear(&mut self) {
        self.values.clear();
        self.order.clear();
    }
}

/// What the model last saw of each read target, and at which tool call.
#[derive(Debug, Clone, Default)]
struct ReadLedger {
    reads: HashMap<String, (String, usize)>,
}

impl ReadLedger {
    /// `call_index` is the ordinal of the read being recorded; the status
    /// says whether an identical output was served within `window` calls.
    fn status(&mut self, key: String, hash: &str, call_index: usize, window: usize) -> ReadStatus {
        let status = match self.reads.get(&key) {
            None => ReadStatus::New,
            Some((previous, at)) if previous == hash => {
                if call_index.saturating_sub(*at) <= window {
                    ReadStatus::Unchanged
                } else {
                    ReadStatus::Stale
                }
            }
            Some(_) => ReadStatus::Changed,
        };
        self.reads.insert(key, (hash.to_string(), call_index));
        status
    }

    fn clear(&mut self) {
        self.reads.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStatus {
    New,
    /// Same bytes, and the earlier output is still in the model's view.
    Unchanged,
    /// Same bytes, but the earlier output may have been pruned: serve it again.
    Stale,
    Changed,
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

/// The dedupe key: the file plus the window the model asked for. Two reads
/// of different ranges of one file are different outputs.
fn read_key(args: &Value) -> Option<String> {
    let path = args.get("path").and_then(Value::as_str)?;
    let window = |name: &str| {
        args.get(name)
            .and_then(Value::as_u64)
            .map(|value| value.to_string())
            .unwrap_or_default()
    };
    Some(format!(
        "{}\0{}\0{}",
        normalize_path(path),
        window("offset"),
        window("limit")
    ))
}

#[derive(Debug, Clone)]
pub struct OutputStore {
    root: PathBuf,
}

impl OutputStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn for_session(session_key: &str) -> Self {
        let base = std::env::var_os("DAVINCI_TOKEN_GOVERNOR_DIR")
            .or_else(|| std::env::var_os("PI_TOKEN_GOVERNOR_DIR"))
            .map(PathBuf::from)
            .or_else(|| {
                let home = std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .map(PathBuf::from);
                home.map(|h| {
                    let davinci = h.join(".davinci");
                    if davinci.exists() {
                        davinci
                    } else {
                        let pi = h.join(".pi");
                        if pi.exists() {
                            pi
                        } else {
                            davinci
                        }
                    }
                })
            })
            .unwrap_or_else(std::env::temp_dir)
            .join("agent")
            .join("token-governor")
            .join("outputs")
            .join(sanitize_component(session_key));
        Self::new(base)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save(&self, content: &str) -> Result<StoredOutputRef, ToolError> {
        let digest = file_content_hash(content);
        let id = format!("out-{}", &digest[..12]);
        fs::create_dir_all(&self.root).map_err(|err| ToolError::Failed(err.to_string()))?;
        let path = self.root.join(format!("{id}.txt"));
        if !path.exists() {
            fs::write(&path, content).map_err(|err| ToolError::Failed(err.to_string()))?;
        }
        Ok(StoredOutputRef {
            id,
            bytes: content.len(),
            lines: line_count(content),
        })
    }

    pub fn load(&self, id: &str) -> Result<String, ToolError> {
        if !is_valid_output_id(id) {
            return Err(ToolError::Failed("invalid output id".into()));
        }
        fs::read_to_string(self.root.join(format!("{id}.txt"))).map_err(|err| {
            ToolError::Failed(format!(
                "no stored output {id} in this session ({err}); only ids named in a compressed result can be retrieved"
            ))
        })
    }

    /// Remove sibling session directories under the store's parent that no
    /// file has touched for `max_age`. Nothing else ever deletes them, and a
    /// session's outputs are useless once that session is gone. Best effort:
    /// a failure to remove one directory is not an error. Returns how many
    /// directories went.
    pub fn sweep_stale_sessions(&self, max_age: Duration) -> usize {
        let Some(parent) = self.root.parent() else {
            return 0;
        };
        let Ok(entries) = fs::read_dir(parent) else {
            return 0;
        };
        let now = SystemTime::now();
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path == self.root || !path.is_dir() {
                continue;
            }
            let Some(newest) = newest_modification(&path) else {
                continue;
            };
            let stale = now
                .duration_since(newest)
                .map(|age| age > max_age)
                .unwrap_or(false);
            if stale && fs::remove_dir_all(&path).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

/// The most recent mtime among a directory's files; the directory's own
/// mtime only when it holds none.
fn newest_modification(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) {
                newest = Some(newest.map_or(modified, |current| current.max(modified)));
            }
        }
    }
    newest.or_else(|| fs::metadata(dir).and_then(|meta| meta.modified()).ok())
}

fn sanitize_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "default".into()
    } else {
        sanitized.chars().take(96).collect()
    }
}

fn is_valid_output_id(id: &str) -> bool {
    id.len() == 16 && id.starts_with("out-") && id[4..].chars().all(|ch| ch.is_ascii_hexdigit())
}

fn line_count(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
    }
}

/// Compress a tool result while preserving the head, tail, and notable lines.
/// The trailer names no output id; `compress_with_reference` is what the
/// governor uses once the original is on disk.
pub fn compress_output(content: &str, config: &TokenGovernorConfig) -> CompressedOutput {
    compress_with_reference(content, config, None)
}

/// Like `compress_output`, but the trailer tells the model the id under which
/// the full text was stored and how to get it back.
pub fn compress_with_reference(
    content: &str,
    config: &TokenGovernorConfig,
    stored: Option<&StoredOutputRef>,
) -> CompressedOutput {
    let bytes = content.len();
    let lines = line_count(content);
    let should_compress =
        bytes >= config.compress_threshold_bytes || lines >= config.compress_threshold_lines;
    let info = CompressionInfo {
        compressed: should_compress,
        original_bytes: bytes,
        original_lines: lines,
        stored: stored.cloned(),
    };
    if !should_compress {
        return CompressedOutput {
            content: content.to_string(),
            info,
        };
    }
    let source = content.lines().collect::<Vec<_>>();
    let mut selected = Vec::new();
    let head_end = config.keep_head_lines.min(source.len());
    selected.extend(source[..head_end].iter().map(|line| (*line).to_string()));
    let tail_start = source.len().saturating_sub(config.keep_tail_lines);
    let notable_budget = config.max_important_lines.min(source.len());
    if notable_budget > 0 {
        let mut kept_notable = 0;
        for line in source
            .iter()
            .skip(head_end)
            .take(tail_start.saturating_sub(head_end))
        {
            if kept_notable >= notable_budget {
                break;
            }
            if is_notable(line) {
                selected.push((*line).to_string());
                kept_notable += 1;
            }
        }
    }
    let tail_copy_start = tail_start.max(head_end);
    if tail_copy_start < source.len() {
        selected.extend(
            source[tail_copy_start..]
                .iter()
                .map(|line| (*line).to_string()),
        );
    }
    // Lines folded by the repeat collapse were shown once, not omitted.
    let omitted = lines.saturating_sub(selected.len());
    let selected = collapse_repeated_lines(selected);
    let mut digest = selected.join("\n");
    if omitted > 0 || stored.is_some() {
        digest.push_str("\n\n");
        digest.push_str(&compression_trailer(omitted, lines, stored));
    }
    CompressedOutput {
        content: digest,
        info,
    }
}

fn compression_trailer(omitted: usize, total: usize, stored: Option<&StoredOutputRef>) -> String {
    match stored {
        Some(stored) => format!(
            "[… {omitted} of {total} lines omitted; the full output ({} bytes) is saved as {}. \
             Call retrieve_output with id \"{}\" — optionally startLine/endLine or grep — to read what you need.]",
            stored.bytes, stored.id, stored.id
        ),
        None => format!("[… {omitted} of {total} lines omitted]"),
    }
}

fn is_notable(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "error",
        "warn",
        "fail",
        "panic",
        "exception",
        "todo",
        "fixme",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn collapse_repeated_lines(lines: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    let mut previous: Option<String> = None;
    let mut repeats = 0usize;
    for line in lines {
        if previous.as_deref() == Some(line.as_str()) {
            repeats += 1;
            if repeats == 3 {
                output.push(format!("[… repeated line omitted: {}]", line));
            }
            continue;
        }
        repeats = 1;
        previous = Some(line.clone());
        output.push(line);
    }
    output
}

/// A one-line account of a call for the stored manifest: the command for a
/// shell, the pattern and path for a search, the normalized args otherwise.
fn call_summary(tool: &str, args: &Value) -> String {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    let summary = match tool {
        "bash" | "powershell" => text("command"),
        "grep" => {
            let path = text("path");
            if path.is_empty() {
                format!("\"{}\"", text("pattern"))
            } else {
                format!("\"{}\" in {path}", text("pattern"))
            }
        }
        "find" => {
            let path = text("path");
            if path.is_empty() {
                text("pattern")
            } else {
                format!("{} in {path}", text("pattern"))
            }
        }
        "ls" | "web_fetch" | "job_output" => {
            let value = text("path");
            if value.is_empty() {
                let url = text("url");
                if url.is_empty() {
                    text("id")
                } else {
                    url
                }
            } else {
                value
            }
        }
        _ => normalize_json(args),
    };
    let summary = summary.lines().next().unwrap_or_default().trim();
    let mut clipped = summary.chars().take(72).collect::<String>();
    if summary.chars().count() > 72 {
        clipped.push('…');
    }
    clipped
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GovernorStats {
    pub bytes_withheld: u64,
    pub retrievals: u64,
    pub compressed_outputs: u64,
    pub deduplicated_reads: u64,
    pub blocked_calls: u64,
    #[serde(default)]
    pub prunings: u64,
}

#[derive(Debug, Clone)]
pub struct TokenGovernor {
    pub config: TokenGovernorConfig,
    pub session_key: String,
    store: OutputStore,
    calls: BoundedSet,
    pending_calls: HashMap<String, VecDeque<String>>,
    reads: ReadLedger,
    stored: VecDeque<StoredOutputEntry>,
    tool_calls: usize,
    compressed_outputs: usize,
    deduplicated_reads: usize,
    blocked_calls: usize,
    /// Bytes the model was not sent: the difference between each original
    /// and what replaced it.
    bytes_withheld: usize,
    retrievals: Arc<AtomicU64>,
    prunings: usize,
}

impl Default for TokenGovernor {
    fn default() -> Self {
        Self::new("default", TokenGovernorConfig::default())
    }
}

impl TokenGovernor {
    pub fn new(session_key: impl Into<String>, config: TokenGovernorConfig) -> Self {
        let session_key = session_key.into();
        let store = config
            .store_dir
            .as_ref()
            .map(|root| OutputStore::new(root.join("outputs").join(&session_key)))
            .unwrap_or_else(|| OutputStore::for_session(&session_key));
        Self {
            store,
            session_key,
            config,
            calls: BoundedSet::default(),
            pending_calls: HashMap::new(),
            reads: ReadLedger::default(),
            stored: VecDeque::new(),
            tool_calls: 0,
            compressed_outputs: 0,
            deduplicated_reads: 0,
            blocked_calls: 0,
            bytes_withheld: 0,
            retrievals: Arc::new(AtomicU64::new(0)),
            prunings: 0,
        }
    }

    pub fn stats(&self) -> GovernorStats {
        GovernorStats {
            bytes_withheld: self.bytes_withheld as u64,
            retrievals: self.retrievals.load(Ordering::Relaxed),
            compressed_outputs: self.compressed_outputs as u64,
            deduplicated_reads: self.deduplicated_reads as u64,
            blocked_calls: self.blocked_calls as u64,
            prunings: self.prunings as u64,
        }
    }

    pub fn record_pruning(&mut self) {
        self.prunings += 1;
        self.session_start();
    }

    #[allow(dead_code)]
    pub fn prunings(&self) -> u64 {
        self.prunings as u64
    }

    #[cfg(test)]
    pub fn with_store(
        session_key: impl Into<String>,
        config: TokenGovernorConfig,
        store: OutputStore,
    ) -> Self {
        let mut governor = Self::new(session_key, config);
        governor.store = store;
        governor
    }

    /// Drop other sessions' stored outputs that have aged past the retention
    /// window. The product host calls this once per process; the library
    /// constructor never deletes anything.
    pub fn sweep_stale_outputs(&self) -> usize {
        self.store.sweep_stale_sessions(STORE_RETENTION)
    }

    /// The ledgers only: a new session, or a compaction that rewrote what the
    /// model can see, means no earlier output is known to be in view.
    pub fn session_start(&mut self) {
        self.calls.clear();
        self.pending_calls.clear();
        self.reads.clear();
    }

    pub fn session_compact(&mut self) {
        self.session_start();
    }

    /// `/governor-reset`: the ledgers and the counters.
    pub fn reset(&mut self) {
        self.session_start();
        self.stored.clear();
        self.tool_calls = 0;
        self.compressed_outputs = 0;
        self.deduplicated_reads = 0;
        self.blocked_calls = 0;
        self.bytes_withheld = 0;
        self.retrievals.store(0, Ordering::Relaxed);
    }

    /// `state_hash` must cover the complete search domain's content, not just
    /// Git status. Empty means freshness is unknown: execute rather than suppress.
    pub fn before_tool(
        &mut self,
        name: &str,
        args: &Value,
        state_hash: impl FnOnce() -> String,
    ) -> Option<String> {
        if !self.config.enabled || !self.config.anti_loop {
            return None;
        }
        if !SEARCH_TOOLS.contains(&name) {
            return None;
        }
        let state = state_hash();
        if state.is_empty() {
            return None;
        }
        let fingerprint = call_fingerprint(name, args, &state);
        if self.calls.contains(&fingerprint) {
            self.blocked_calls += 1;
            return Some(format!(
                "Repeated {name} call blocked by token governor: the repository has not changed since the identical call, so its result still stands. Change the query or inspect the previous result."
            ));
        }
        self.pending_calls
            .entry(call_key(name, args))
            .or_default()
            .push_back(fingerprint);
        None
    }

    pub fn after_tool(&mut self, name: &str, args: &Value, mut result: ToolResult) -> ToolResult {
        if !self.config.enabled {
            return result;
        }
        self.tool_calls += 1;

        // A call only enters the anti-loop ledger after a successful result.
        // Failed searches remain retryable, including after transient cwd or
        // provider errors.
        if SEARCH_TOOLS.contains(&name) {
            let key = call_key(name, args);
            let fingerprint = self
                .pending_calls
                .get_mut(&key)
                .and_then(|pending| pending.pop_front());
            if self.pending_calls.get(&key).is_some_and(VecDeque::is_empty) {
                self.pending_calls.remove(&key);
            }
            if !result.is_error {
                if let Some(fingerprint) = fingerprint {
                    self.calls.insert(fingerprint);
                }
            }
        }

        if result.is_error
            || result
                .details
                .as_ref()
                .and_then(|details| details.get("tokenGovernor"))
                .and_then(|details| details.get("skip"))
                .and_then(Value::as_bool)
                == Some(true)
        {
            return result;
        }
        if self.config.dedupe_reads && name == "read" {
            if let Some(key) = read_key(args) {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let hash = file_content_hash(&result.content);
                let status =
                    self.reads
                        .status(key, &hash, self.tool_calls, self.config.dedupe_window);
                if status == ReadStatus::Unchanged {
                    self.deduplicated_reads += 1;
                    let marker = format!(
                        "[unchanged read: {path} is byte-identical to the read you made a moment ago; that output is still valid]"
                    );
                    self.bytes_withheld += result.content.len().saturating_sub(marker.len());
                    result.content = marker;
                    result.details = merge_details(
                        result.details,
                        json!({"tokenGovernor": {"deduplicated": true, "path": path}}),
                    );
                    return result;
                }
            }
        }
        if LOSSLESS_TOOLS.contains(&name) {
            return result;
        }
        let probe = compress_output(&result.content, &self.config);
        if !probe.info.compressed {
            return result;
        }
        // Fail open: if the original cannot be stored, the model keeps the
        // whole output rather than a digest it could never expand.
        let Ok(reference) = self.store.save(&result.content) else {
            return result;
        };
        let compressed = compress_with_reference(&result.content, &self.config, Some(&reference));
        self.remember_stored(name, args, &reference);
        self.bytes_withheld += result
            .content
            .len()
            .saturating_sub(compressed.content.len());
        result.content = compressed.content;
        result.details = merge_details(
            result.details,
            json!({
                "tokenGovernor": {
                    "compressed": true,
                    "originalBytes": compressed.info.original_bytes,
                    "originalLines": compressed.info.original_lines,
                    "outputId": reference.id,
                    "reference": format!("governor://{}", reference.id),
                }
            }),
        );
        self.compressed_outputs += 1;
        result
    }

    fn remember_stored(&mut self, tool: &str, args: &Value, reference: &StoredOutputRef) {
        self.stored.retain(|entry| entry.id != reference.id);
        self.stored.push_front(StoredOutputEntry {
            id: reference.id.clone(),
            tool: tool.to_string(),
            call: call_summary(tool, args),
            bytes: reference.bytes,
            lines: reference.lines,
        });
        self.stored.truncate(STORED_MANIFEST_ENTRIES);
    }

    pub fn retrieve(&self, args: &Value) -> Result<ToolResult, ToolError> {
        let id = args
            .get("id")
            .or_else(|| args.get("outputId"))
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("retrieve_output requires id".into()))?;
        let content = self.store.load(id)?;
        self.retrievals.fetch_add(1, Ordering::Relaxed);
        let start = args
            .get("startLine")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        let end = args
            .get("endLine")
            .and_then(Value::as_u64)
            .map(|line| line as usize);
        let pattern = args.get("grep").and_then(Value::as_str);
        let total = line_count(&content);
        let budget = self.config.retrieve_max_bytes;
        let mut selected = Vec::new();
        let mut used = 0usize;
        let mut matched = 0usize;
        let mut stopped_at: Option<usize> = None;
        for (index, line) in content.lines().enumerate() {
            let line_no = index + 1;
            if line_no < start {
                continue;
            }
            if end.is_some_and(|last| line_no > last) {
                break;
            }
            if pattern.is_some_and(|pattern| !line.contains(pattern)) {
                continue;
            }
            matched += 1;
            let rendered = format!("{line_no}: {line}");
            if used + rendered.len() + 1 > budget && !selected.is_empty() {
                stopped_at = Some(line_no);
                break;
            }
            used += rendered.len() + 1;
            selected.push(rendered);
        }
        let mut text = selected.join("\n");
        match stopped_at {
            Some(line_no) => text.push_str(&format!(
                "\n\n[… stopped before line {line_no} of {total} to stay under {budget} bytes; call retrieve_output again with startLine {line_no}{}]",
                end.map(|last| format!(" and endLine {last}")).unwrap_or_default()
            )),
            None if matched == 0 => text.push_str(&match pattern {
                Some(pattern) => format!("[no line of {id} matches \"{pattern}\" in that range; {total} lines total]"),
                None => format!("[no lines in that range; {id} has {total} lines]"),
            }),
            None => {}
        }
        Ok(ToolResult {
            content: text,
            is_error: false,
            details: Some(json!({"tokenGovernor": {
                "outputId": id,
                "totalLines": total,
                "returnedLines": selected.len(),
                "truncated": stopped_at.is_some(),
            }})),
        })
    }

    pub fn status(&self) -> Value {
        json!({
            "enabled": self.config.enabled,
            "sessionKey": self.session_key,
            "toolCalls": self.tool_calls,
            "compressedOutputs": self.compressed_outputs,
            "deduplicatedReads": self.deduplicated_reads,
            "blockedCalls": self.blocked_calls,
            "bytesWithheld": self.bytes_withheld,
            "store": self.store.root(),
            "stored": self.stored,
            "thresholds": {
                "compressBytes": self.config.compress_threshold_bytes,
                "compressLines": self.config.compress_threshold_lines,
                "keepHeadLines": self.config.keep_head_lines,
                "keepTailLines": self.config.keep_tail_lines,
                "maxImportantLines": self.config.max_important_lines,
                "dedupeReads": self.config.dedupe_reads,
                "dedupeWindow": self.config.dedupe_window,
                "antiLoop": self.config.anti_loop,
                "retrieveMaxBytes": self.config.retrieve_max_bytes,
            },
        })
    }
}

fn merge_details(existing: Option<Value>, addition: Value) -> Option<Value> {
    match (existing, addition) {
        (Some(Value::Object(mut existing)), Value::Object(addition)) => {
            for (key, value) in addition {
                existing.insert(key, value);
            }
            Some(Value::Object(existing))
        }
        (None, value) => Some(value),
        (Some(existing), _) => Some(existing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn ok(content: &str) -> ToolResult {
        ToolResult {
            content: content.into(),
            is_error: false,
            details: None,
        }
    }

    fn tiny_thresholds() -> TokenGovernorConfig {
        TokenGovernorConfig {
            compress_threshold_bytes: 1,
            compress_threshold_lines: 1,
            ..Default::default()
        }
    }

    #[test]
    fn fingerprint_is_stable_for_object_key_order() {
        let left = json!({"path":"a", "limit": 10});
        let right = json!({"limit": 10, "path":"a"});
        assert_eq!(
            call_fingerprint("grep", &left, "state"),
            call_fingerprint("grep", &right, "state")
        );
    }

    #[test]
    fn config_file_merges_defaults_and_supports_store_directory() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("token-governor.json");
        fs::write(
            &path,
            r#"{"compressThresholdBytes": 1024, "storeDir": "custom-store"}"#,
        )
        .unwrap();
        let config = TokenGovernorConfig::from_file(&path);
        assert_eq!(config.compress_threshold_bytes, 1024);
        assert_eq!(
            config.compress_threshold_lines,
            DEFAULT_COMPRESS_THRESHOLD_LINES
        );
        assert_eq!(config.dedupe_window, DEFAULT_DEDUPE_WINDOW);
        assert_eq!(config.store_dir, Some(PathBuf::from("custom-store")));
    }

    #[test]
    fn compression_preserves_notable_lines_and_is_reversible() {
        let dir = tempdir().unwrap();
        let config = TokenGovernorConfig {
            compress_threshold_bytes: 1,
            compress_threshold_lines: 1,
            keep_head_lines: 1,
            keep_tail_lines: 1,
            max_important_lines: 2,
            ..Default::default()
        };
        let original = "head\nwarning: keep this\nnoise\ntail";
        let mut governor = TokenGovernor::with_store("test", config, OutputStore::new(dir.path()));
        let result = governor.after_tool("bash", &json!({}), ok(original));
        assert!(result.content.contains("head"));
        assert!(result.content.contains("warning"));
        let details = result.details.unwrap();
        let id = details["tokenGovernor"]["outputId"].as_str().unwrap();
        assert_eq!(
            governor.retrieve(&json!({"id": id})).unwrap().content,
            "1: head\n2: warning: keep this\n3: noise\n4: tail"
        );
    }

    #[test]
    fn the_digest_names_its_output_id_because_details_never_reach_the_model() {
        let dir = tempdir().unwrap();
        let mut governor =
            TokenGovernor::with_store("test", tiny_thresholds(), OutputStore::new(dir.path()));
        let original = (1..=400)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = governor.after_tool("bash", &json!({"command": "ls"}), ok(&original));
        let id = result.details.as_ref().unwrap()["tokenGovernor"]["outputId"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            result.content.contains(&format!("saved as {id}")),
            "{}",
            result.content
        );
        assert!(result.content.contains("retrieve_output"));
        assert!(result.content.contains("of 400 lines omitted"));
        // The id is also in the status manifest with the call that made it.
        let status = governor.status();
        assert_eq!(status["stored"][0]["id"], id);
        assert_eq!(status["stored"][0]["tool"], "bash");
        assert_eq!(status["stored"][0]["call"], "ls");
        assert!(status["bytesWithheld"].as_u64().unwrap() > 0);
    }

    #[test]
    fn omitted_count_ignores_lines_folded_by_the_repeat_collapse() {
        let config = TokenGovernorConfig {
            compress_threshold_bytes: 1,
            compress_threshold_lines: 1,
            keep_head_lines: 10,
            keep_tail_lines: 0,
            max_important_lines: 1,
            ..Default::default()
        };
        // Ten identical head lines all "kept": nothing omitted, some folded.
        let compressed = compress_output(&["same"; 10].join("\n"), &config);
        assert!(
            !compressed.content.contains("omitted;"),
            "{}",
            compressed.content
        );
        assert!(compressed.content.contains("repeated line omitted"));
    }

    #[test]
    fn lossless_tools_are_never_digested() {
        let dir = tempdir().unwrap();
        let mut governor =
            TokenGovernor::with_store("test", tiny_thresholds(), OutputStore::new(dir.path()));
        let big = (1..=500)
            .map(|n| format!("{n}: fn line_{n}() {{}}"))
            .collect::<Vec<_>>()
            .join("\n");
        for tool in ["read", "edit", "write", "batch", "agent"] {
            let result =
                governor.after_tool(tool, &json!({"path": format!("{tool}.rs")}), ok(&big));
            assert_eq!(result.content, big, "{tool} must stay verbatim");
            assert!(result.details.is_none(), "{tool} must not be marked");
        }
        let shell = governor.after_tool("bash", &json!({"command": "cat"}), ok(&big));
        assert_ne!(shell.content, big);
        assert_eq!(governor.status()["compressedOutputs"], 1);
    }

    #[test]
    fn anti_loop_only_blocks_repeated_search_calls() {
        let mut governor = TokenGovernor::new("test", TokenGovernorConfig::default());
        let args = json!({"path":"src"});
        assert!(governor
            .before_tool("ls", &args, || "state".into())
            .is_none());
        let _ = governor.after_tool("ls", &args, ok("src"));
        assert!(governor
            .before_tool("ls", &args, || "state".into())
            .is_some());
        assert!(governor
            .before_tool("read", &args, || "state".into())
            .is_none());
    }

    #[test]
    fn pruning_invalidates_read_and_search_visibility() {
        let mut governor = TokenGovernor::new("test", TokenGovernorConfig::default());
        let args = json!({"path": "src"});
        governor.before_tool("ls", &args, || "state".into());
        governor.after_tool("ls", &args, ok("src"));
        governor.after_tool("read", &args, ok("important source"));
        governor.record_pruning();
        assert!(governor
            .before_tool("ls", &args, || "state".into())
            .is_none());
        assert_eq!(
            governor
                .after_tool("read", &args, ok("important source"))
                .content,
            "important source"
        );
        assert_eq!(governor.prunings(), 1);
    }

    #[test]
    fn unknown_search_state_never_suppresses_a_fresh_search() {
        let mut governor = TokenGovernor::new("test", TokenGovernorConfig::default());
        let args = json!({"pattern": "needle"});
        governor.before_tool("grep", &args, String::new);
        governor.after_tool("grep", &args, ok("old result"));
        assert!(governor.before_tool("grep", &args, String::new).is_none());
        assert_eq!(
            governor
                .after_tool("grep", &args, ok("changed result"))
                .content,
            "changed result"
        );
    }

    #[test]
    fn the_state_hash_is_only_computed_for_search_tools() {
        let mut governor = TokenGovernor::new("test", TokenGovernorConfig::default());
        let mut computed = 0;
        let _ = governor.before_tool("read", &json!({"path": "a"}), || {
            computed += 1;
            "state".into()
        });
        let _ = governor.before_tool("bash", &json!({"command": "ls"}), || {
            computed += 1;
            "state".into()
        });
        assert_eq!(computed, 0);
        let _ = governor.before_tool("grep", &json!({"pattern": "a"}), || {
            computed += 1;
            "state".into()
        });
        assert_eq!(computed, 1);
    }

    #[test]
    fn failed_searches_and_retrieval_are_not_blocked_or_compressed() {
        let mut governor = TokenGovernor::with_store(
            "test",
            tiny_thresholds(),
            OutputStore::new(tempdir().unwrap().path()),
        );
        let args = json!({"path":"src"});
        assert!(governor
            .before_tool("grep", &args, || "state".into())
            .is_none());
        let failed = governor.after_tool(
            "grep",
            &args,
            ToolResult {
                content: "error".into(),
                is_error: true,
                details: None,
            },
        );
        assert!(failed.is_error);
        assert!(governor
            .before_tool("grep", &args, || "state".into())
            .is_none());

        let retrieved = governor.after_tool(
            "retrieve_output",
            &json!({"id":"out-000000000000"}),
            ok("full output"),
        );
        assert_eq!(retrieved.content, "full output");
        assert!(retrieved.details.is_none());
    }

    #[test]
    fn repeated_reads_return_a_small_marker() {
        let mut governor = TokenGovernor::new("test", TokenGovernorConfig::default());
        let args = json!({"path":"README.md"});
        let first = governor.after_tool("read", &args, ok("same"));
        let second = governor.after_tool("read", &args, ok("same"));
        assert_eq!(first.content, "same");
        assert!(second.content.contains("unchanged read"));
    }

    #[test]
    fn a_read_is_served_again_once_its_twin_may_have_been_pruned() {
        let mut governor = TokenGovernor::new(
            "test",
            TokenGovernorConfig {
                dedupe_window: 2,
                ..Default::default()
            },
        );
        let args = json!({"path":"README.md"});
        assert_eq!(
            governor.after_tool("read", &args, ok("same")).content,
            "same"
        );
        // Two unrelated calls push the first read to the edge of the window…
        let _ = governor.after_tool("bash", &json!({}), ok("x"));
        let _ = governor.after_tool("bash", &json!({}), ok("x"));
        // …a third puts it past it: the earlier output may be a placeholder
        // by now, so the model gets the bytes back.
        let _ = governor.after_tool("bash", &json!({}), ok("x"));
        assert_eq!(
            governor.after_tool("read", &args, ok("same")).content,
            "same"
        );
        // And right after that, the marker again.
        assert!(governor
            .after_tool("read", &args, ok("same"))
            .content
            .contains("unchanged read"));
        assert_eq!(governor.status()["deduplicatedReads"], 1);
    }

    #[test]
    fn reads_of_different_ranges_are_different_outputs() {
        let mut governor = TokenGovernor::new("test", TokenGovernorConfig::default());
        let head = json!({"path":"a.rs", "offset": 1, "limit": 10});
        let tail = json!({"path":"a.rs", "offset": 11, "limit": 10});
        assert_eq!(governor.after_tool("read", &head, ok("h")).content, "h");
        assert_eq!(governor.after_tool("read", &tail, ok("t")).content, "t");
        assert!(governor
            .after_tool("read", &head, ok("h"))
            .content
            .contains("unchanged read"));
    }

    #[test]
    fn retrieval_is_paged_and_says_where_to_continue() {
        let dir = tempdir().unwrap();
        let config = TokenGovernorConfig {
            retrieve_max_bytes: 1_024,
            ..tiny_thresholds()
        };
        let mut governor = TokenGovernor::with_store("test", config, OutputStore::new(dir.path()));
        let original = (1..=300)
            .map(|n| format!("row {n:04} {}", "x".repeat(20)))
            .collect::<Vec<_>>()
            .join("\n");
        let result = governor.after_tool("bash", &json!({}), ok(&original));
        let id = result.details.unwrap()["tokenGovernor"]["outputId"]
            .as_str()
            .unwrap()
            .to_string();
        let page = governor.retrieve(&json!({"id": id})).unwrap();
        assert!(page.content.len() < 1_024 + 200, "{}", page.content.len());
        assert!(page
            .content
            .contains("call retrieve_output again with startLine"));
        assert_eq!(
            page.details.as_ref().unwrap()["tokenGovernor"]["truncated"],
            true
        );
        let filtered = governor
            .retrieve(&json!({"id": id, "grep": "row 0299"}))
            .unwrap();
        assert!(filtered.content.starts_with("299: row 0299"));
        let empty = governor
            .retrieve(&json!({"id": id, "grep": "absent"}))
            .unwrap();
        assert!(empty.content.contains("no line of"));
        let missing = governor.retrieve(&json!({"id": "out-ffffffffffff"}));
        assert!(missing.is_err());
    }

    #[test]
    fn compression_does_not_duplicate_head_and_tail_when_they_overlap() {
        let config = TokenGovernorConfig {
            compress_threshold_bytes: 1,
            compress_threshold_lines: 1,
            keep_head_lines: 10,
            keep_tail_lines: 10,
            ..Default::default()
        };
        let compressed = compress_output("one\ntwo\nthree", &config);
        assert_eq!(compressed.content.matches("one").count(), 1);
        assert_eq!(compressed.content.matches("two").count(), 1);
        assert_eq!(compressed.content.matches("three").count(), 1);
    }

    #[test]
    fn compression_keeps_tail_when_head_and_tail_partition_the_input() {
        let config = TokenGovernorConfig {
            compress_threshold_bytes: 1,
            compress_threshold_lines: 1,
            keep_head_lines: 2,
            keep_tail_lines: 2,
            ..Default::default()
        };
        let compressed = compress_output("one\ntwo\nthree\nfour", &config);
        assert!(compressed.content.contains("one"));
        assert!(compressed.content.contains("two"));
        assert!(compressed.content.contains("three"));
        assert!(compressed.content.contains("four"));
    }

    #[test]
    fn reset_clears_counters_but_compaction_keeps_them() {
        let dir = tempdir().unwrap();
        let mut governor =
            TokenGovernor::with_store("test", tiny_thresholds(), OutputStore::new(dir.path()));
        let _ = governor.after_tool("bash", &json!({}), ok(&"line\n".repeat(50)));
        assert_eq!(governor.status()["compressedOutputs"], 1);
        governor.session_compact();
        assert_eq!(governor.status()["compressedOutputs"], 1);
        governor.reset();
        assert_eq!(governor.status()["compressedOutputs"], 0);
        assert_eq!(governor.status()["stored"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn stale_session_stores_are_swept_and_the_live_one_is_kept() {
        let dir = tempdir().unwrap();
        let outputs = dir.path().join("outputs");
        let old = outputs.join("old-session");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("out-000000000000.txt"), "x").unwrap();
        let stale = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
        fs::OpenOptions::new()
            .write(true)
            .open(old.join("out-000000000000.txt"))
            .unwrap()
            .set_modified(stale)
            .unwrap();
        let fresh = outputs.join("fresh-session");
        fs::create_dir_all(&fresh).unwrap();
        let live = OutputStore::new(outputs.join("live-session"));
        live.save("live").unwrap();

        let removed = live.sweep_stale_sessions(STORE_RETENTION);
        assert_eq!(removed, 1);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(live.root().exists());
    }

    #[test]
    fn call_summaries_name_the_command_or_query() {
        assert_eq!(
            call_summary("bash", &json!({"command": "cargo test --workspace\n"})),
            "cargo test --workspace"
        );
        assert_eq!(
            call_summary(
                "grep",
                &json!({"pattern": "SessionManager", "path": "crates"})
            ),
            "\"SessionManager\" in crates"
        );
        assert_eq!(call_summary("ls", &json!({"path": "src"})), "src");
        let long = call_summary("bash", &json!({"command": "x".repeat(100)}));
        assert_eq!(long.chars().count(), 73);
        assert!(long.ends_with('…'));
    }
}
