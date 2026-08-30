//! Lossless token-governor middleware.
//!
//! The governor is intentionally deterministic and fail-open.  It keeps a
//! reversible copy of large successful tool outputs, returns a compact digest
//! to the model, and records enough metadata for `retrieve_output` to recover
//! the original text.  No network access is required.

use pi_agent::{ToolError, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;

const DEFAULT_COMPRESS_THRESHOLD_BYTES: usize = 8_192;
const DEFAULT_COMPRESS_THRESHOLD_LINES: usize = 200;
const DEFAULT_KEEP_HEAD_LINES: usize = 15;
const DEFAULT_KEEP_TAIL_LINES: usize = 30;
const DEFAULT_MAX_IMPORTANT_LINES: usize = 60;
const DEFAULT_MAX_LEDGER_ENTRIES: usize = 200;

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
    #[serde(default = "default_true")]
    pub anti_loop: bool,
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
            anti_loop: true,
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

    pub fn from_file(path: &std::path::Path) -> Self {
        let mut config = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default();
        apply_env(&mut config);
        config
    }
}

fn apply_env(config: &mut TokenGovernorConfig) {
    if let Some(value) = env_bool_any(&["PI_GOVERNOR_ENABLED", "PI_TOKEN_GOVERNOR_ENABLED"]) {
        config.enabled = value;
    }
    if let Some(value) = env_usize_any(&[
        "PI_GOVERNOR_COMPRESS_THRESHOLD",
        "PI_TOKEN_GOVERNOR_COMPRESS_THRESHOLD_BYTES",
    ]) {
        config.compress_threshold_bytes = value.max(1);
    }
    if let Some(value) = env_usize("PI_TOKEN_GOVERNOR_COMPRESS_THRESHOLD_LINES") {
        config.compress_threshold_lines = value.max(1);
    }
    if let Some(value) = env_usize("PI_TOKEN_GOVERNOR_KEEP_HEAD_LINES") {
        config.keep_head_lines = value;
    }
    if let Some(value) = env_usize("PI_TOKEN_GOVERNOR_KEEP_TAIL_LINES") {
        config.keep_tail_lines = value;
    }
    if let Some(value) = env_usize("PI_TOKEN_GOVERNOR_MAX_IMPORTANT_LINES") {
        config.max_important_lines = value.max(1);
    }
    if let Some(value) = env_bool("PI_TOKEN_GOVERNOR_DEDUPE_READS") {
        config.dedupe_reads = value;
    }
    if let Some(value) = env_bool("PI_TOKEN_GOVERNOR_ANTI_LOOP") {
        config.anti_loop = value;
    }
    if let Some(value) = env_string_any(&["PI_GOVERNOR_STORE_DIR", "PI_TOKEN_GOVERNOR_DIR"]) {
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

#[derive(Debug, Clone)]
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

impl Default for BoundedSet {
    fn default() -> Self {
        Self {
            values: HashSet::new(),
            order: VecDeque::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ReadLedger {
    paths: HashMap<String, String>,
}

impl ReadLedger {
    fn status(&mut self, path: &str, hash: &str) -> ReadStatus {
        let key = normalize_path(path);
        let status = match self.paths.get(&key) {
            None => ReadStatus::New,
            Some(previous) if previous == hash => ReadStatus::Unchanged,
            Some(_) => ReadStatus::Changed,
        };
        self.paths.insert(key, hash.to_string());
        status
    }

    fn clear(&mut self) {
        self.paths.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStatus {
    New,
    Unchanged,
    Changed,
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
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
        let base = std::env::var_os("PI_TOKEN_GOVERNOR_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".pi")))
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".pi")))
            .unwrap_or_else(std::env::temp_dir)
            .join("agent/token-governor/outputs")
            .join(sanitize_component(session_key));
        Self::new(base)
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
        fs::read_to_string(self.root.join(format!("{id}.txt")))
            .map_err(|err| ToolError::Failed(err.to_string()))
    }

    pub fn path(&self, id: &str) -> Option<PathBuf> {
        is_valid_output_id(id).then(|| self.root.join(format!("{id}.txt")))
    }
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
pub fn compress_output(content: &str, config: &TokenGovernorConfig) -> CompressedOutput {
    let bytes = content.len();
    let lines = line_count(content);
    let should_compress =
        bytes >= config.compress_threshold_bytes || lines >= config.compress_threshold_lines;
    let info = CompressionInfo {
        compressed: should_compress,
        original_bytes: bytes,
        original_lines: lines,
        stored: None,
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
        for line in source
            .iter()
            .skip(head_end)
            .take(tail_start.saturating_sub(head_end))
        {
            if is_notable(line) && selected.len() < head_end + notable_budget {
                selected.push((*line).to_string());
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
    selected = collapse_repeated_lines(selected);
    let omitted = lines.saturating_sub(selected.len());
    let mut digest = selected.join("\n");
    if omitted > 0 {
        digest.push_str(&format!(
            "\n\n[… {omitted} lines omitted; use retrieve_output to inspect the full result]"
        ));
    }
    CompressedOutput {
        content: digest,
        info,
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

#[derive(Debug, Clone)]
pub struct TokenGovernor {
    pub config: TokenGovernorConfig,
    pub session_key: String,
    store: OutputStore,
    calls: BoundedSet,
    pending_calls: HashMap<String, VecDeque<String>>,
    reads: ReadLedger,
    compressed_outputs: usize,
    deduplicated_reads: usize,
    blocked_calls: usize,
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
            compressed_outputs: 0,
            deduplicated_reads: 0,
            blocked_calls: 0,
        }
    }

    pub fn with_store(
        session_key: impl Into<String>,
        config: TokenGovernorConfig,
        store: OutputStore,
    ) -> Self {
        let mut governor = Self::new(session_key, config);
        governor.store = store;
        governor
    }

    pub fn session_start(&mut self) {
        self.calls.clear();
        self.pending_calls.clear();
        self.reads.clear();
    }

    pub fn session_compact(&mut self) {
        self.session_start();
    }

    pub fn before_tool(&mut self, name: &str, args: &Value, state_hash: &str) -> Option<String> {
        if !self.config.enabled || !self.config.anti_loop {
            return None;
        }
        if !matches!(name, "grep" | "find" | "ls") {
            return None;
        }
        let fingerprint = call_fingerprint(name, args, state_hash);
        if self.calls.contains(&fingerprint) {
            self.blocked_calls += 1;
            return Some(format!(
                "Repeated {name} call blocked by token governor; change the query or inspect the previous result."
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

        // A call only enters the anti-loop ledger after a successful result.
        // Failed searches remain retryable, including after transient cwd or
        // provider errors.
        if matches!(name, "grep" | "find" | "ls") {
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
            || matches!(name, "retrieve_output" | "memory_search")
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
        let original = result.content.clone();
        if self.config.dedupe_reads && name == "read" {
            if let Some(path) = args.get("path").and_then(Value::as_str) {
                let hash = file_content_hash(&original);
                if self.reads.status(path, &hash) == ReadStatus::Unchanged {
                    self.deduplicated_reads += 1;
                    result.content =
                        format!("[unchanged read: {path}; the previous output is still valid]");
                    result.details = merge_details(
                        result.details,
                        json!({"tokenGovernor": {"deduplicated": true, "path": path}}),
                    );
                    return result;
                }
            }
        }
        let mut compressed = compress_output(&original, &self.config);
        if compressed.info.compressed {
            if let Ok(reference) = self.store.save(&original) {
                compressed.info.stored = Some(reference.clone());
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
            }
        }
        result
    }

    pub fn retrieve(&self, args: &Value) -> Result<ToolResult, ToolError> {
        let id = args
            .get("id")
            .or_else(|| args.get("outputId"))
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("retrieve_output requires id".into()))?;
        let content = self.store.load(id)?;
        let start = args
            .get("startLine")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        let end = args
            .get("endLine")
            .and_then(Value::as_u64)
            .map(|line| line as usize);
        let mut lines = content.lines().enumerate().filter_map(|(index, line)| {
            let line_no = index + 1;
            (line_no >= start && end.is_none_or(|last| line_no <= last))
                .then_some(format!("{line_no}: {line}"))
        });
        let mut selected = lines.by_ref().collect::<Vec<_>>();
        if let Some(pattern) = args.get("grep").and_then(Value::as_str) {
            selected.retain(|line| line.contains(pattern));
        }
        Ok(ToolResult {
            content: selected.join("\n"),
            is_error: false,
            details: Some(json!({"tokenGovernor": {"outputId": id}})),
        })
    }

    pub fn status(&self) -> Value {
        json!({
            "enabled": self.config.enabled,
            "sessionKey": self.session_key,
            "compressedOutputs": self.compressed_outputs,
            "deduplicatedReads": self.deduplicated_reads,
            "blockedCalls": self.blocked_calls,
            "store": self.store.root,
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
        let result = governor.after_tool(
            "bash",
            &json!({}),
            ToolResult {
                content: original.into(),
                is_error: false,
                details: None,
            },
        );
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
    fn anti_loop_only_blocks_repeated_search_calls() {
        let mut governor = TokenGovernor::new("test", TokenGovernorConfig::default());
        let args = json!({"path":"src"});
        assert!(governor.before_tool("ls", &args, "state").is_none());
        let _ = governor.after_tool(
            "ls",
            &args,
            ToolResult {
                content: "src".into(),
                is_error: false,
                details: None,
            },
        );
        assert!(governor.before_tool("ls", &args, "state").is_some());
        assert!(governor.before_tool("read", &args, "state").is_none());
    }

    #[test]
    fn failed_searches_and_retrieval_are_not_blocked_or_compressed() {
        let mut governor = TokenGovernor::with_store(
            "test",
            TokenGovernorConfig {
                compress_threshold_bytes: 1,
                compress_threshold_lines: 1,
                ..Default::default()
            },
            OutputStore::new(tempdir().unwrap().path()),
        );
        let args = json!({"path":"src"});
        assert!(governor.before_tool("grep", &args, "state").is_none());
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
        assert!(governor.before_tool("grep", &args, "state").is_none());

        let retrieved = governor.after_tool(
            "retrieve_output",
            &json!({"id":"out-000000000000"}),
            ToolResult {
                content: "full output".into(),
                is_error: false,
                details: None,
            },
        );
        assert_eq!(retrieved.content, "full output");
        assert!(retrieved.details.is_none());
    }

    #[test]
    fn repeated_reads_return_a_small_marker() {
        let mut governor = TokenGovernor::new("test", TokenGovernorConfig::default());
        let args = json!({"path":"README.md"});
        let first = governor.after_tool(
            "read",
            &args,
            ToolResult {
                content: "same".into(),
                is_error: false,
                details: None,
            },
        );
        let second = governor.after_tool(
            "read",
            &args,
            ToolResult {
                content: "same".into(),
                is_error: false,
                details: None,
            },
        );
        assert_eq!(first.content, "same");
        assert!(second.content.contains("unchanged read"));
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
}
