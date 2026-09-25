//! Lossless token-governor middleware.
//!
//! The governor is intentionally deterministic and fail-open.  It keeps a
//! reversible copy of large successful or failing tool outputs, returns a compact digest
//! to the model, and records enough metadata for `retrieve_output` to recover
//! the original text.  No network access is required.
//!
//! Only the `content` of a tool result reaches the model (`details` is for
//! the UI and events), so everything the model needs to undo a compression —
//! the output id and how to call `retrieve_output` — is written into the
//! digest itself.

use davinci_agent::{PermissionState, PermissionVerdict, ToolError, ToolResult};
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
/// Returns whether a tool's output may be compressed by the token governor.
pub fn tool_may_be_compressed(name: &str) -> bool {
    !matches!(
        davinci_agent::runtime::output_policy_for_tool(name, davinci_agent::tool_class(name)),
        davinci_agent::OutputPolicy::LosslessRequired
    )
}

/// Guarantees that if any tool in the list can generate compressible output under
/// the token governor, `retrieve_output` is automatically included so the worker
/// never loses access to compressed data.
pub fn ensure_governor_recovery_tool(tools: &mut Vec<String>) {
    let has_compressible = tools.iter().any(|t| tool_may_be_compressed(t));
    if has_compressible && !tools.iter().any(|t| t == "retrieve_output") {
        tools.push("retrieve_output".into());
    }
}

/// Tools the anti-loop ledger watches: pure queries whose answer only changes
/// when the repository does.
const SEARCH_TOOLS: &[&str] = &["grep", "find", "ls"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenGovernorConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub content_aware: bool,
    #[serde(default = "default_specialized_min_reduction_pct")]
    pub specialized_min_reduction_pct: u8,
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
fn default_specialized_min_reduction_pct() -> u8 {
    10
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
            content_aware: true,
            specialized_min_reduction_pct: default_specialized_min_reduction_pct(),
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
    if let Some(value) = env_bool_any(&[
        "DAVINCI_GOVERNOR_CONTENT_AWARE",
        "DAVINCI_TOKEN_GOVERNOR_CONTENT_AWARE",
        "PI_GOVERNOR_CONTENT_AWARE",
        "PI_TOKEN_GOVERNOR_CONTENT_AWARE",
    ]) {
        config.content_aware = value;
    }
    if let Some(value) = env_usize_any(&[
        "DAVINCI_GOVERNOR_SPECIALIZED_MIN_REDUCTION_PCT",
        "DAVINCI_TOKEN_GOVERNOR_SPECIALIZED_MIN_REDUCTION_PCT",
        "PI_GOVERNOR_SPECIALIZED_MIN_REDUCTION_PCT",
        "PI_TOKEN_GOVERNOR_SPECIALIZED_MIN_REDUCTION_PCT",
    ]) {
        config.specialized_min_reduction_pct = value.min(100) as u8;
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
    #[serde(default)]
    pub content_kind: String,
    #[serde(default)]
    pub strategy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lsp_authorization: Option<LspAuthorization>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LspAuthorization {
    pub workspace: PathBuf,
    pub permission_scope: u64,
    pub permission_revision: u64,
    pub paths: Vec<PathBuf>,
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
        if !stored_output_is_intact(&path, &digest) {
            davinci_sys::fs::atomic_write(&path, content.as_bytes())
                .map_err(|err| ToolError::Failed(err.to_string()))?;
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
fn stored_output_is_intact(path: &Path, digest: &str) -> bool {
    fs::read_to_string(path)
        .map(|content| file_content_hash(&content) == digest)
        .unwrap_or(false)
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

fn truncate_to_bytes(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let end = text
        .char_indices()
        .take_while(|(index, _)| *index <= max_bytes)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0);
    &text[..end]
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

fn clears_specialized_threshold(specialized: usize, generic: usize, pct: u8) -> bool {
    specialized < generic
        && specialized.saturating_mul(100)
            <= generic.saturating_mul(100usize.saturating_sub(pct as usize))
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
pub struct KindRetrievalStats {
    pub compressed: u64,
    pub specialized: u64,
    pub original_bytes: u64,
    pub model_view_bytes: u64,
    pub retrievals: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentKindStats {
    pub log: KindRetrievalStats,
    pub compiler_diagnostics: KindRetrievalStats,
    pub test_output: KindRetrievalStats,
    pub json_array: KindRetrievalStats,
    pub ndjson: KindRetrievalStats,
    pub json_object: KindRetrievalStats,
    pub search_results: KindRetrievalStats,
    pub tree: KindRetrievalStats,
    pub table: KindRetrievalStats,
    pub plain_text: KindRetrievalStats,
}

impl ContentKindStats {
    fn get_mut(
        &mut self,
        kind: crate::native_extensions::content_router::ContentKind,
    ) -> &mut KindRetrievalStats {
        use crate::native_extensions::content_router::ContentKind;
        match kind {
            ContentKind::Log => &mut self.log,
            ContentKind::CompilerDiagnostics => &mut self.compiler_diagnostics,
            ContentKind::TestOutput => &mut self.test_output,
            ContentKind::JsonArray => &mut self.json_array,
            ContentKind::Ndjson => &mut self.ndjson,
            ContentKind::JsonObject => &mut self.json_object,
            ContentKind::SearchResults => &mut self.search_results,
            ContentKind::Tree => &mut self.tree,
            ContentKind::Table => &mut self.table,
            ContentKind::PlainText => &mut self.plain_text,
        }
    }

    fn get_mut_by_name(&mut self, name: &str) -> Option<&mut KindRetrievalStats> {
        match name {
            "log" => Some(&mut self.log),
            "compilerDiagnostics" => Some(&mut self.compiler_diagnostics),
            "testOutput" => Some(&mut self.test_output),
            "jsonArray" => Some(&mut self.json_array),
            "ndjson" => Some(&mut self.ndjson),
            "jsonObject" => Some(&mut self.json_object),
            "searchResults" => Some(&mut self.search_results),
            "tree" => Some(&mut self.tree),
            "table" => Some(&mut self.table),
            "plainText" => Some(&mut self.plain_text),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentRoutingStats {
    pub log: u64,
    pub json_array: u64,
    pub search_results: u64,
    pub plain_text: u64,
    pub specialized_views: u64,
    pub generic_views: u64,
    pub original_bytes: u64,
    pub model_view_bytes: u64,
    #[serde(default)]
    pub by_kind: ContentKindStats,
}

impl ContentRoutingStats {
    fn record(
        &mut self,
        kind: crate::native_extensions::content_router::ContentKind,
        specialized: bool,
        original_bytes: usize,
        model_view_bytes: usize,
    ) {
        use crate::native_extensions::content_router::ContentKind;
        match kind {
            ContentKind::Log => self.log += 1,
            ContentKind::JsonArray => self.json_array += 1,
            ContentKind::SearchResults => self.search_results += 1,
            ContentKind::PlainText => self.plain_text += 1,
            _ => {}
        }
        if specialized {
            self.specialized_views += 1;
        } else {
            self.generic_views += 1;
        }
        self.original_bytes += original_bytes as u64;
        self.model_view_bytes += model_view_bytes as u64;
        let entry = self.by_kind.get_mut(kind);
        entry.compressed += 1;
        if specialized {
            entry.specialized += 1;
        }
        entry.original_bytes += original_bytes as u64;
        entry.model_view_bytes += model_view_bytes as u64;
    }

    fn record_retrieval(&mut self, kind: &str) {
        if let Some(entry) = self.by_kind.get_mut_by_name(kind) {
            entry.retrievals += 1;
        }
    }
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
    #[serde(default)]
    pub content_routing: ContentRoutingStats,
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
    content_routing: ContentRoutingStats,
    lsp_permissions: Option<Arc<PermissionState>>,
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
            content_routing: ContentRoutingStats::default(),
            lsp_permissions: None,
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
            content_routing: self.content_routing,
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
        self.content_routing = ContentRoutingStats::default();
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

        let governor_skip = result
            .details
            .as_ref()
            .and_then(|details| details.get("tokenGovernor"))
            .and_then(|details| details.get("skip"))
            .and_then(Value::as_bool)
            == Some(true);
        if governor_skip {
            return result;
        }
        if !result.is_error && self.config.dedupe_reads && name == "read" {
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
        if !tool_may_be_compressed(name) {
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
        let generic = compress_with_reference(&result.content, &self.config, Some(&reference));
        let kind =
            crate::native_extensions::content_router::classify_content(name, args, &result.content);
        let specialized = if self.config.content_aware {
            crate::native_extensions::content_router::build_specialized_view(
                kind,
                name,
                args,
                &result.content,
                &reference.id,
            )
        } else {
            None
        };
        let original_bytes = generic.info.original_bytes;
        let original_lines = generic.info.original_lines;
        let (chosen_content, strategy) = match specialized {
            Some(view)
                if clears_specialized_threshold(
                    view.content.len(),
                    generic.content.len(),
                    self.config.specialized_min_reduction_pct,
                ) =>
            {
                (view.content, "specialized")
            }
            _ => (generic.content, "generic"),
        };
        let view_bytes = chosen_content.len();
        let content_hash = file_content_hash(&result.content);
        self.remember_stored(name, args, &reference, kind.as_str(), strategy, None);
        self.bytes_withheld += result.content.len().saturating_sub(view_bytes);
        result.content = chosen_content;
        result.details = merge_details(
            result.details,
            json!({
                "tokenGovernor": {
                    "compressed": true,
                    "contentKind": kind.as_str(),
                    "strategy": strategy,
                    "originalBytes": original_bytes,
                    "originalLines": original_lines,
                    "viewBytes": view_bytes,
                    "outputId": reference.id,
                    "reference": format!("governor://{}", reference.id),
                    "contentHash": content_hash,
                }
            }),
        );
        self.content_routing
            .record(kind, strategy == "specialized", original_bytes, view_bytes);
        self.compressed_outputs += 1;
        result
    }

    fn remember_stored(
        &mut self,
        tool: &str,
        args: &Value,
        reference: &StoredOutputRef,
        content_kind: &str,
        strategy: &str,
        lsp_authorization: Option<LspAuthorization>,
    ) {
        self.stored.retain(|entry| entry.id != reference.id);
        self.stored.push_front(StoredOutputEntry {
            id: reference.id.clone(),
            tool: tool.to_string(),
            call: call_summary(tool, args),
            bytes: reference.bytes,
            lines: reference.lines,
            content_kind: content_kind.to_string(),
            strategy: strategy.to_string(),
            lsp_authorization,
        });
        self.stored.truncate(STORED_MANIFEST_ENTRIES);
    }

    /// Retain normalized LSP evidence with the permission generation that
    /// authorized it. Retrieval fails closed after a policy revision.
    pub(crate) fn retain_lsp_output(
        &mut self,
        name: &str,
        args: &Value,
        content: &str,
        workspace: &Path,
        source: Option<&Path>,
        permissions: Arc<PermissionState>,
    ) -> Result<String, ToolError> {
        let revision = permissions
            .lock()
            .ok()
            .and_then(|guard| guard.revision())
            .ok_or_else(|| ToolError::Failed("language evidence permission revision unavailable".into()))?;
        let scope = Arc::as_ptr(&permissions) as usize as u64;
        let workspace = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        let mut paths = Vec::new();
        if let Some(source) = source {
            if let Ok(source) = source.canonicalize() {
                if source.starts_with(&workspace) {
                    paths.push(source);
                }
            }
        }
        if let Ok(value) = serde_json::from_str::<Value>(content) {
            collect_lsp_paths(&value, &workspace, &mut paths);
        }
        paths.sort();
        paths.dedup();
        let reference = self.store.save(content)?;
        self.lsp_permissions = Some(permissions);
        self.remember_stored(
            name,
            args,
            &reference,
            "json",
            "semantic-cap",
            Some(LspAuthorization {
                workspace,
                permission_scope: scope,
                permission_revision: revision,
                paths,
            }),
        );
        Ok(reference.id)
    }

    fn authorize_lsp_retrieval(&self, id: &str) -> Result<(), ToolError> {
        let Some(entry) = self.stored.iter().find(|entry| entry.id == id) else {
            return Ok(());
        };
        let Some(auth) = &entry.lsp_authorization else {
            return Ok(());
        };
        let permissions = self
            .lsp_permissions
            .as_ref()
            .ok_or_else(|| ToolError::Failed("stored language evidence is no longer authorized".into()))?;
        if Arc::as_ptr(permissions) as usize as u64 != auth.permission_scope {
            return Err(ToolError::Failed("stored language evidence permission scope changed".into()));
        }
        let guard = permissions
            .lock()
            .map_err(|_| ToolError::Failed("stored language evidence permission state unavailable".into()))?;
        if guard.revision() != Some(auth.permission_revision) {
            return Err(ToolError::Failed("stored language evidence permission revision changed".into()));
        }
        for path in &auth.paths {
            if !path.starts_with(&auth.workspace) {
                return Err(ToolError::Failed("stored language evidence path escaped its workspace".into()));
            }
            let relative = path
                .strip_prefix(&auth.workspace)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            if !matches!(
                guard.decide(
                    "language-intelligence",
                    &entry.tool,
                    &json!({"path": relative}),
                    &auth.workspace,
                ),
                PermissionVerdict::Allow
            ) {
                return Err(ToolError::Failed("stored language evidence is no longer permitted".into()));
            }
        }
        Ok(())
    }

    pub fn retrieve(&mut self, args: &Value) -> Result<ToolResult, ToolError> {
        let id = args
            .get("id")
            .or_else(|| args.get("outputId"))
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("retrieve_output requires id".into()))?;
        self.authorize_lsp_retrieval(id)?;
        let content = self.store.load(id)?;
        self.retrievals.fetch_add(1, Ordering::Relaxed);
        if let Some(kind) = self
            .stored
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.content_kind.clone())
        {
            self.content_routing.record_retrieval(&kind);
        }
        let start = args
            .get("startLine")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        let line_byte_offset = args
            .get("lineByteOffset")
            .and_then(Value::as_u64)
            .map(|offset| {
                usize::try_from(offset)
                    .map_err(|_| ToolError::Failed("lineByteOffset is too large".into()))
            })
            .transpose()?
            .unwrap_or(0);
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
        let mut next_line_byte_offset: Option<usize> = None;
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
            let line_offset = if line_no == start {
                line_byte_offset
            } else {
                0
            };
            if line_offset > line.len() || !line.is_char_boundary(line_offset) {
                return Err(ToolError::Failed(format!(
                    "lineByteOffset {line_offset} is not a valid UTF-8 boundary for line {line_no}"
                )));
            }
            let rendered = format!("{line_no}: {}", &line[line_offset..]);
            let rendered_bytes = rendered.len().saturating_add(1);
            if selected.is_empty() && rendered_bytes > budget {
                let prefix = format!("{line_no}: ");
                let available = budget.saturating_sub(prefix.len().saturating_add(1));
                let fragment = truncate_to_bytes(&line[line_offset..], available);
                if fragment.is_empty() {
                    return Err(ToolError::Failed(
                        "retrieve_max_bytes is too small to make cursor progress".into(),
                    ));
                }
                selected.push(format!("{prefix}{fragment}…"));
                stopped_at = Some(line_no);
                next_line_byte_offset = Some(line_offset + fragment.len());
                break;
            }
            if used.saturating_add(rendered_bytes) > budget && !selected.is_empty() {
                stopped_at = Some(line_no);
                next_line_byte_offset = Some(0);
                break;
            }
            used = used.saturating_add(rendered_bytes);
            selected.push(rendered);
        }
        let mut text = selected.join("\n");
        match stopped_at {
            Some(line_no) => text.push_str(&format!(
                "\n\n[… output truncated at line {line_no} of {total} to stay under {budget} bytes; see tokenGovernor.nextCursor for continuation]"
            )),
            None if matched == 0 => text.push_str(&match pattern {
                Some(pattern) => format!("[no line of {id} matches \"{pattern}\" in that range; {total} lines total]"),
                None => format!("[no lines in that range; {id} has {total} lines]"),
            }),
            None => {}
        }
        let mut governor_details = json!({
            "outputId": id,
            "totalLines": total,
            "returnedLines": selected.len(),
            "truncated": stopped_at.is_some(),
        });
        if let (Some(line_no), Some(line_byte_offset)) = (stopped_at, next_line_byte_offset) {
            governor_details["nextCursor"] = json!({
                "startLine": line_no,
                "lineByteOffset": line_byte_offset,
            });
        }
        Ok(ToolResult {
            content: text,
            is_error: false,
            details: Some(json!({"tokenGovernor": governor_details})),
        })
    }

    /// Convert a stored governor output into the Context VM's lossless
    /// artifact reference without copying the output into context state.
    // Public library API, also compiled into the binary's private module.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn artifact_ref(
        &self,
        id: &str,
        source_ref: impl Into<String>,
    ) -> Result<davinci_agent::runtime::context_vm::ArtifactRef, ToolError> {
        let content = self.store.load(id)?;
        Ok(davinci_agent::runtime::context_vm::ArtifactRef {
            uri: format!("governor://output/{id}"),
            content_hash: file_content_hash(&content),
            source_ref: source_ref.into(),
        })
    }

    /// Resolve a `governor://output/out-...` artifact through the existing output
    /// store. This is the exact retrieval path used by retrieve_output, with
    /// no second in-memory copy or alternate storage format.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn retrieve_artifact(&mut self, uri: &str) -> Result<String, ToolError> {
        let id = uri
            .strip_prefix("governor://output/")
            .or_else(|| uri.strip_prefix("governor://"))
            .ok_or_else(|| ToolError::Failed("invalid governor artifact URI".into()))?;
        self.authorize_lsp_retrieval(id)?;
        let content = self.store.load(id)?;
        self.retrievals.fetch_add(1, Ordering::Relaxed);
        Ok(content)
    }

    pub fn status(&self) -> Value {
        let content_original_bytes = self.content_routing.original_bytes;
        let content_model_view_bytes = self.content_routing.model_view_bytes;
        let estimated_byte_reduction_pct = if content_original_bytes == 0 {
            0.0
        } else {
            (1.0 - (content_model_view_bytes as f64 / content_original_bytes as f64)) * 100.0
        };
        let retrievals_per_compressed_output = if self.compressed_outputs == 0 {
            0.0
        } else {
            self.retrievals.load(Ordering::Relaxed) as f64 / self.compressed_outputs as f64
        };
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
                "specializedMinReductionPct": self.config.specialized_min_reduction_pct,
            },
            "contentRouting": {
                "log": self.content_routing.log,
                "jsonArray": self.content_routing.json_array,
                "searchResults": self.content_routing.search_results,
                "plainText": self.content_routing.plain_text,
                "specializedViews": self.content_routing.specialized_views,
                "genericViews": self.content_routing.generic_views,
                "originalBytes": content_original_bytes,
                "modelViewBytes": content_model_view_bytes,
                "estimatedByteReductionPct": estimated_byte_reduction_pct,
                "retrievalsPerCompressedOutput": retrievals_per_compressed_output,
                "byKind": self.content_routing.by_kind,
            },
        })
    }
}

fn collect_lsp_paths(value: &Value, workspace: &Path, out: &mut Vec<PathBuf>) {
    match value {
        Value::Object(map) => {
            if let Some(path) = map.get("path").and_then(Value::as_str) {
                let candidate = workspace.join(path);
                if let Ok(candidate) = candidate.canonicalize() {
                    if candidate.starts_with(workspace) {
                        out.push(candidate);
                    }
                }
            }
            for value in map.values() {
                collect_lsp_paths(value, workspace, out);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_lsp_paths(value, workspace, out);
            }
        }
        _ => {}
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
    fn torn_stored_output_is_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let store = OutputStore::new(dir.path().to_path_buf());
        let saved = store.save("full content").unwrap();
        let path = dir.path().join(format!("{}.txt", saved.id));
        fs::write(&path, "full con").unwrap();
        store.save("full content").unwrap();
        assert_eq!(store.load(&saved.id).unwrap(), "full content");
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
    fn context_artifact_ref_reuses_the_existing_output_store() {
        let dir = tempdir().unwrap();
        let store = OutputStore::new(dir.path());
        let original = "exact governor bytes\nwith evidence";
        let reference = store.save(original).unwrap();
        let mut governor = TokenGovernor::with_store("test", TokenGovernorConfig::default(), store);

        let artifact = governor
            .artifact_ref(&reference.id, "session:tool-result")
            .unwrap();
        assert_eq!(artifact.uri, format!("governor://output/{}", reference.id));
        assert_eq!(governor.retrieve_artifact(&artifact.uri).unwrap(), original);
        assert_eq!(
            governor
                .retrieve_artifact(&format!("governor://{}", reference.id))
                .unwrap(),
            original
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
        let page_details = &page.details.as_ref().unwrap()["tokenGovernor"];
        assert_eq!(page_details["truncated"], true);
        assert!(page_details["nextCursor"]["startLine"]
            .as_u64()
            .is_some_and(|line| line > 1));
        assert_eq!(page_details["nextCursor"]["lineByteOffset"], 0);
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
    fn retrieval_caps_an_oversized_first_line() {
        let dir = tempdir().unwrap();
        let config = TokenGovernorConfig {
            retrieve_max_bytes: 1_024,
            ..tiny_thresholds()
        };
        let mut governor = TokenGovernor::with_store("test", config, OutputStore::new(dir.path()));
        let original = "é".repeat(4_000);
        let result = governor.after_tool("bash", &json!({}), ok(&original));
        let id = result.details.unwrap()["tokenGovernor"]["outputId"]
            .as_str()
            .unwrap()
            .to_string();

        let page = governor.retrieve(&json!({"id": id})).unwrap();

        assert!(page.content.len() < 1_024 + 200, "{}", page.content.len());
        assert_eq!(
            page.details.as_ref().unwrap()["tokenGovernor"]["truncated"],
            true
        );
        assert!(std::str::from_utf8(page.content.as_bytes()).is_ok());

        let first_cursor = page.details.as_ref().unwrap()["tokenGovernor"]["nextCursor"].clone();
        assert_eq!(first_cursor["startLine"], 1);
        let first_offset = first_cursor["lineByteOffset"]
            .as_u64()
            .expect("oversized line should return a byte cursor");
        assert!(first_offset > 0);

        let next_page = governor
            .retrieve(&json!({
                    "id": id,
                    "startLine": first_cursor["startLine"],
                    "lineByteOffset": first_cursor["lineByteOffset"],
            }))
            .unwrap();
        assert!(std::str::from_utf8(next_page.content.as_bytes()).is_ok());
        assert_ne!(next_page.details, page.details);
        if next_page.details.as_ref().unwrap()["tokenGovernor"]["truncated"] == true {
            let second_cursor = &next_page.details.as_ref().unwrap()["tokenGovernor"]["nextCursor"];
            assert_eq!(second_cursor["startLine"], 1);
            assert!(second_cursor["lineByteOffset"]
                .as_u64()
                .is_some_and(|offset| offset > first_offset));
        }
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

    #[test]
    fn content_aware_views_are_smaller_and_exactly_retrievable() {
        let dir = tempdir().unwrap();
        let config = TokenGovernorConfig {
            compress_threshold_bytes: 1,
            compress_threshold_lines: 1,
            content_aware: true,
            ..Default::default()
        };
        let mut governor =
            TokenGovernor::with_store("router-e2e", config, OutputStore::new(dir.path()));

        let log = (0..120)
            .map(|i| {
                if i == 67 {
                    "test auth_refresh ... FAILED\nassertion failed: expected 200 actual 401"
                        .to_string()
                } else {
                    format!("test case_{i} ... ok")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let result = governor.after_tool("bash", &json!({"command": "cargo test"}), ok(&log));

        assert!(result.content.len() < log.len());
        assert!(result.content.contains("FAILED"));
        assert!(result.content.contains("expected 200 actual 401"));
        assert!(result.content.contains("retrieve_output"));

        let details = result.details.as_ref().unwrap();
        assert_eq!(details["tokenGovernor"]["contentKind"], "testOutput");
        assert_eq!(details["tokenGovernor"]["strategy"], "specialized");

        let id = details["tokenGovernor"]["outputId"].as_str().unwrap();
        let recovered = governor.retrieve(&json!({"id": id})).unwrap().content;
        let reconstructed = recovered
            .lines()
            .map(|line| line.split_once(": ").map(|(_, body)| body).unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(reconstructed, log);
    }

    #[test]
    fn content_aware_ablation_reports_reversible_byte_measurement() {
        let logs = (0..12)
            .map(|task| {
                (0..120)
                    .map(|i| {
                        if i == 60 + task {
                            format!(
                                "test auth_refresh_{task} ... FAILED\nassertion failed: expected 200 actual {}",
                                401 + task
                            )
                        } else {
                            format!("test task_{task}_case_{i} ... ok")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>();

        let run = |content_aware| {
            let dir = tempdir().unwrap();
            let config = TokenGovernorConfig {
                compress_threshold_bytes: 1,
                compress_threshold_lines: 1,
                content_aware,
                ..Default::default()
            };
            let mut governor = TokenGovernor::with_store(
                format!("ablation-{content_aware}"),
                config,
                OutputStore::new(dir.path()),
            );
            let mut original_bytes = 0;
            let mut view_bytes = 0;
            let mut generic_views = 0;
            let mut specialized_views = 0;
            for (task, log) in logs.iter().enumerate() {
                let args = json!({"command": format!("cargo test --task {task}")});
                let result = governor.after_tool("bash", &args, ok(log));
                let details = result.details.as_ref().unwrap()["tokenGovernor"].clone();
                let id = details["outputId"].as_str().unwrap();
                let recovered = governor.retrieve(&json!({"id": id})).unwrap().content;
                let reconstructed = recovered
                    .lines()
                    .map(|line| line.split_once(": ").map(|(_, body)| body).unwrap_or(line))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert_eq!(reconstructed, *log);

                original_bytes += details["originalBytes"].as_u64().unwrap();
                view_bytes += details["viewBytes"].as_u64().unwrap();
                match details["strategy"].as_str().unwrap() {
                    "generic" => generic_views += 1,
                    "specialized" => specialized_views += 1,
                    strategy => panic!("unexpected governor strategy: {strategy}"),
                }
            }
            println!(
                "governor_ab tasks={} content_aware={content_aware} original_bytes={original_bytes} view_bytes={view_bytes} withheld_bytes={} generic_views={generic_views} specialized_views={specialized_views}",
                logs.len(),
                original_bytes.saturating_sub(view_bytes)
            );
            (view_bytes, generic_views, specialized_views)
        };

        let (generic_bytes, generic_views, generic_specialized_views) = run(false);
        let (specialized_bytes, specialized_views, specialized_specialized_views) = run(true);
        assert_eq!(generic_views, 12);
        assert_eq!(generic_specialized_views, 0);
        assert_eq!(specialized_views, 0);
        assert_eq!(specialized_specialized_views, 12);
        assert!(
            specialized_bytes < generic_bytes,
            "content-aware view should be smaller: specialized={specialized_bytes}, generic={generic_bytes}"
        );
    }

    #[test]
    fn governor_status_reports_content_routing() {
        let dir = tempdir().unwrap();
        let config = TokenGovernorConfig {
            compress_threshold_bytes: 1,
            compress_threshold_lines: 1,
            content_aware: true,
            ..Default::default()
        };
        let mut governor =
            TokenGovernor::with_store("routing-status", config, OutputStore::new(dir.path()));

        let json = serde_json::Value::Array(
            (0..30)
                .map(|i| {
                    if i == 17 {
                        serde_json::json!({"id": i, "status": "error", "message": "boom"})
                    } else {
                        serde_json::json!({"id": i, "status": "ok", "payload": "repetitive payload"})
                    }
                })
                .collect(),
        )
        .to_string();

        let _ = governor.after_tool("bash", &json!({"command": "printf json"}), ok(&json));
        let status = governor.status();

        assert_eq!(status["contentRouting"]["jsonArray"], 1);
        assert_eq!(status["contentRouting"]["specializedViews"], 1);
        assert!(status["contentRouting"]["originalBytes"].as_u64().unwrap() > 0);
        assert!(status["contentRouting"]["modelViewBytes"].as_u64().unwrap() > 0);
        assert!(
            status["contentRouting"]["estimatedByteReductionPct"]
                .as_f64()
                .unwrap()
                > 0.0
        );
    }

    #[test]
    fn large_error_output_is_reversibly_compressed() {
        let dir = tempdir().unwrap();
        let mut governor =
            TokenGovernor::with_store("test", tiny_thresholds(), OutputStore::new(dir.path()));
        let original = (0..500)
            .map(|i| format!("error: compile failure {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = governor.after_tool(
            "exec_command",
            &json!({"command":"cargo check"}),
            ToolResult {
                content: original.clone(),
                is_error: true,
                details: None,
            },
        );
        assert!(result.is_error);
        assert!(result.content.len() < original.len());
        let id = result.details.as_ref().unwrap()["tokenGovernor"]["outputId"]
            .as_str()
            .unwrap();
        let recovered = governor.retrieve(&json!({"id": id})).unwrap();
        assert!(recovered.content.contains("compile failure 499"));
    }

    #[test]
    fn specialized_view_must_clear_minimum_reduction_threshold() {
        assert!(clears_specialized_threshold(80, 100, 10));
        assert!(!clears_specialized_threshold(95, 100, 10));
        assert!(!clears_specialized_threshold(100, 100, 10));
    }

    #[test]
    fn governor_tracks_retrievals_by_content_kind() {
        let dir = tempdir().unwrap();
        let mut governor =
            TokenGovernor::with_store("test", tiny_thresholds(), OutputStore::new(dir.path()));
        let log = (0..120)
            .map(|i| format!("running task {i}\nwarning: w{i}\nerror: e{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let log_result = governor.after_tool(
            "exec_command",
            &json!({"command":"custom runner"}),
            ok(&log),
        );
        assert!(log_result.details.as_ref().unwrap()["tokenGovernor"]["outputId"].is_string());

        let json_body = serde_json::Value::Array(
            (0..40)
                .map(|i| serde_json::json!({"id":i,"status":if i == 31 {"error"} else {"ok"}}))
                .collect(),
        )
        .to_string();
        let json_result = governor.after_tool(
            "exec_command",
            &json!({"command":"cat data.json"}),
            ok(&json_body),
        );
        let json_id = json_result.details.as_ref().unwrap()["tokenGovernor"]["outputId"]
            .as_str()
            .unwrap()
            .to_string();
        let _ = governor.retrieve(&json!({"id":json_id})).unwrap();
        let status = governor.status();
        assert_eq!(status["contentRouting"]["byKind"]["log"]["retrievals"], 0);
        assert_eq!(
            status["contentRouting"]["byKind"]["jsonArray"]["retrievals"],
            1
        );
    }
}
