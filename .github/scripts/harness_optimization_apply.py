#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STAGE = (ROOT / ".harness-opt-stage").read_text().strip() if (ROOT / ".harness-opt-stage").exists() else ""


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new, 1)


def append_before_tests_module(path: Path, test_code: str) -> None:
    text = path.read_text()
    marker = "#[cfg(test)]\nmod tests {"
    if test_code.strip() in text:
        return
    if marker not in text:
        raise SystemExit(f"test module marker not found in {path}")
    # Insert inside the existing module immediately after imports/helper area by
    # placing the code just before the module's final closing brace.
    idx = text.rfind("\n}")
    if idx < 0:
        raise SystemExit(f"cannot find test module end in {path}")
    text = text[:idx] + "\n" + test_code.rstrip() + "\n" + text[idx:]
    path.write_text(text)


def governor_tests() -> None:
    path = ROOT / "crates/davinci-coding-agent/src/native_extensions/token_governor.rs"
    text = path.read_text()
    if "fn large_error_output_is_reversibly_compressed()" in text:
        return
    test = r'''
    #[test]
    fn large_error_output_is_reversibly_compressed() {
        let dir = tempdir().unwrap();
        let mut governor = TokenGovernor::with_store(
            "test",
            tiny_thresholds(),
            OutputStore::new(dir.path()),
        );
        let original = (0..500)
            .map(|i| format!("error: compile failure {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = governor.after_tool(
            "exec_command",
            &json!({"command":"cargo check"}),
            ToolResult { content: original.clone(), is_error: true, details: None },
        );
        assert!(result.is_error);
        assert!(result.content.len() < original.len());
        let id = result.details.as_ref().unwrap()["tokenGovernor"]["outputId"]
            .as_str().unwrap();
        let recovered = governor.retrieve(&json!({"id": id})).unwrap();
        assert!(recovered.content.contains("compile failure 499"));
    }
'''
    idx = text.rfind("\n}")
    if idx < 0:
        raise SystemExit("cannot locate governor test module end")
    path.write_text(text[:idx] + "\n" + test.rstrip() + "\n" + text[idx:])


CONTENT_ROUTER_IMPL = r'''use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

const SEARCH_TOOLS: &[&str] = &["grep", "find", "ls"];
const SHELL_TOOLS: &[&str] = &["bash", "powershell", "exec_command"];
const TEST_COMMAND_MARKERS: &[&str] = &[
    "cargo test", "pytest", "python -m pytest", "npm test", "pnpm test", "bun test",
    "go test", "dotnet test", "mvn test", "gradle test", "make test",
];
const LOG_COMMAND_MARKERS: &[&str] = &[
    "cargo build", "cargo check", "cargo clippy", "npm run", "pnpm run", "go build",
];
const LOG_OUTPUT_MARKERS: &[&str] = &[
    "running ", "test result:", "traceback", "stack trace", "panic", "exception",
    "warning:", "error:", "failed",
];
const DIAGNOSTIC_MARKERS: &[&str] = &[
    "error", "failed", "failure", "panic", "exception", "warning", "assertion",
    "expected", "actual",
];
const IDENTITY_KEYS: &[&str] = &[
    "id", "name", "path", "file", "line", "status", "type", "error", "message",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Log,
    CompilerDiagnostics,
    TestOutput,
    JsonArray,
    Ndjson,
    JsonObject,
    SearchResults,
    Tree,
    Table,
    PlainText,
}

impl ContentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::CompilerDiagnostics => "compilerDiagnostics",
            Self::TestOutput => "testOutput",
            Self::JsonArray => "jsonArray",
            Self::Ndjson => "ndjson",
            Self::JsonObject => "jsonObject",
            Self::SearchResults => "searchResults",
            Self::Tree => "tree",
            Self::Table => "table",
            Self::PlainText => "plainText",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecializedView {
    pub kind: ContentKind,
    pub content: String,
}

pub fn classify_content(tool: &str, args: &Value, content: &str) -> ContentKind {
    if SEARCH_TOOLS.contains(&tool) {
        return ContentKind::SearchResults;
    }
    if is_large_json_array(content) {
        return ContentKind::JsonArray;
    }
    if is_large_json_object(content) {
        return ContentKind::JsonObject;
    }
    if is_ndjson(content) {
        return ContentKind::Ndjson;
    }
    if is_compiler_diagnostics(tool, args, content) {
        return ContentKind::CompilerDiagnostics;
    }
    if is_test_output(tool, args, content) {
        return ContentKind::TestOutput;
    }
    if is_log_like(tool, args, content) {
        return ContentKind::Log;
    }
    if is_tree_like(content) {
        return ContentKind::Tree;
    }
    if is_table_like(content) {
        return ContentKind::Table;
    }
    ContentKind::PlainText
}

pub fn build_specialized_view(
    kind: ContentKind,
    _tool: &str,
    args: &Value,
    content: &str,
    output_id: &str,
) -> Option<SpecializedView> {
    let rendered = match kind {
        ContentKind::Log | ContentKind::CompilerDiagnostics | ContentKind::TestOutput => {
            build_log_view(kind, content, output_id)
        }
        ContentKind::JsonArray => build_json_array_view(content, output_id),
        ContentKind::Ndjson => build_ndjson_view(content, output_id),
        ContentKind::JsonObject => build_json_object_view(content, output_id),
        ContentKind::SearchResults => build_search_results_view(args, content, output_id),
        ContentKind::Tree | ContentKind::Table => build_bounded_line_view(kind, content, output_id),
        ContentKind::PlainText => return None,
    }?;
    Some(SpecializedView { kind, content: rendered })
}

fn shell_command(args: &Value) -> String {
    args.get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_large_json_array(content: &str) -> bool {
    matches!(serde_json::from_str::<Value>(content.trim()), Ok(Value::Array(items)) if items.len() > 5)
}

fn is_large_json_object(content: &str) -> bool {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(content.trim()) else {
        return false;
    };
    map.iter().any(|(key, value)| {
        value.as_array().is_some_and(|items| items.len() > 5)
            || DIAGNOSTIC_MARKERS.iter().any(|marker| key.to_ascii_lowercase().contains(marker))
    })
}

fn is_ndjson(content: &str) -> bool {
    let lines = content.lines().filter(|line| !line.trim().is_empty()).collect::<Vec<_>>();
    if lines.len() < 8 {
        return false;
    }
    let parsed = lines.iter().filter(|line| serde_json::from_str::<Value>(line.trim()).is_ok()).count();
    parsed * 100 >= lines.len() * 80
}

fn is_compiler_diagnostics(tool: &str, args: &Value, content: &str) -> bool {
    if !SHELL_TOOLS.contains(&tool) {
        return false;
    }
    let command = shell_command(args);
    let lower = content.to_ascii_lowercase();
    let command_signal = command.contains("cargo check")
        || command.contains("cargo clippy")
        || command.contains("rustc")
        || command.contains("tsc")
        || command.contains("go vet");
    let signals = ["error[e", "--> ", "warning:", "error:"]
        .iter()
        .filter(|marker| lower.contains(**marker))
        .count();
    command_signal && signals >= 2
}

fn is_test_output(tool: &str, args: &Value, content: &str) -> bool {
    if !SHELL_TOOLS.contains(&tool) {
        return false;
    }
    let command = shell_command(args);
    if TEST_COMMAND_MARKERS.iter().any(|marker| command.contains(marker)) {
        return true;
    }
    let lower = content.to_ascii_lowercase();
    lower.contains("test result:") && (lower.contains("passed") || lower.contains("failed"))
}

fn is_log_like(tool: &str, args: &Value, content: &str) -> bool {
    if !SHELL_TOOLS.contains(&tool) {
        return false;
    }
    let command = shell_command(args);
    if command.contains("git diff") || command.contains("git show") {
        return false;
    }
    if LOG_COMMAND_MARKERS.iter().any(|marker| command.contains(marker)) {
        return true;
    }
    let lower = content.to_ascii_lowercase();
    LOG_OUTPUT_MARKERS.iter().filter(|marker| lower.contains(**marker)).count() >= 2
}

fn is_tree_like(content: &str) -> bool {
    let lines = content.lines().filter(|line| !line.trim().is_empty()).collect::<Vec<_>>();
    if lines.len() < 12 {
        return false;
    }
    let structured = lines.iter().filter(|line| {
        line.contains("├──") || line.contains("└──") || line.contains("│  ") || line.starts_with("    ")
    }).count();
    structured * 100 >= lines.len() * 60
}

fn is_table_like(content: &str) -> bool {
    let lines = content.lines().filter(|line| !line.trim().is_empty()).collect::<Vec<_>>();
    if lines.len() < 8 {
        return false;
    }
    for delimiter in ['|', '\t'] {
        let counts = lines.iter().map(|line| line.matches(delimiter).count()).collect::<Vec<_>>();
        let target = counts.iter().copied().filter(|count| *count > 0).max().unwrap_or(0);
        if target > 0 && counts.iter().filter(|count| **count == target).count() * 100 >= lines.len() * 70 {
            return true;
        }
    }
    false
}

fn build_log_view(kind: ContentKind, content: &str, output_id: &str) -> Option<String> {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    let mut selected = BTreeSet::new();
    for index in 0..6.min(lines.len()) { selected.insert(index); }
    for index in lines.len().saturating_sub(12)..lines.len() { selected.insert(index); }
    for (index, line) in lines.iter().enumerate() {
        if is_diagnostic(line) {
            selected.insert(index.saturating_sub(1));
            selected.insert(index);
            if index + 1 < lines.len() { selected.insert(index + 1); }
        }
    }
    let rendered = collapse_adjacent_exact_lines(selected.iter().map(|index| lines[*index].to_string()).collect());
    let shown = rendered.len();
    let mut view = rendered.join("\n");
    view.push_str("\n\n");
    view.push_str(&retrieval_trailer(kind, shown, lines.len(), output_id));
    Some(view)
}

fn selected_json_indices(items: &[Value]) -> BTreeSet<usize> {
    let mut selected = BTreeSet::new();
    if items.is_empty() { return selected; }
    selected.insert(0);
    if items.len() > 1 { selected.insert(items.len() - 1); }
    let mut signal_count = 0;
    let mut identities = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let compact = serde_json::to_string(item).unwrap_or_default();
        if is_diagnostic(&compact) && signal_count < 4 {
            selected.insert(index);
            signal_count += 1;
        }
        if selected.len() < 10 {
            if let Some(identity) = json_identity(item) {
                if identities.insert(identity) { selected.insert(index); }
            }
        }
        if selected.len() >= 10 { break; }
    }
    selected
}

fn json_identity(value: &Value) -> Option<String> {
    let Value::Object(map) = value else { return None; };
    for key in IDENTITY_KEYS {
        if let Some(value) = map.get(*key) {
            if value.is_string() || value.is_number() || value.is_boolean() {
                return Some(format!("{key}={value}"));
            }
        }
    }
    None
}

fn build_json_array_view(content: &str, output_id: &str) -> Option<String> {
    let Value::Array(items) = serde_json::from_str::<Value>(content.trim()).ok()? else { return None; };
    if items.len() <= 5 { return None; }
    let selected = selected_json_indices(&items);
    let shown = selected.len();
    let mut view = format!("JSON array compact view: {} total items", items.len());
    for index in selected {
        view.push_str(&format!("\nitem[{index}]: {}", serde_json::to_string(&items[index]).ok()?));
    }
    view.push_str("\n\n");
    view.push_str(&retrieval_trailer(ContentKind::JsonArray, shown, items.len(), output_id));
    Some(view)
}

fn build_ndjson_view(content: &str, output_id: &str) -> Option<String> {
    let values = content.lines().filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok()).collect::<Vec<_>>();
    if values.len() < 8 { return None; }
    let selected = selected_json_indices(&values);
    let mut view = format!("NDJSON compact view: {} parsed records", values.len());
    for index in selected.iter().copied() {
        view.push_str(&format!("\nrecord[{index}]: {}", serde_json::to_string(&values[index]).ok()?));
    }
    view.push_str("\n\n");
    view.push_str(&retrieval_trailer(ContentKind::Ndjson, selected.len(), values.len(), output_id));
    Some(view)
}

fn build_json_object_view(content: &str, output_id: &str) -> Option<String> {
    let Value::Object(map) = serde_json::from_str::<Value>(content.trim()).ok()? else { return None; };
    let mut view = String::from("JSON object compact view");
    let mut total = 0usize;
    let mut shown = 0usize;
    let mut keys = map.keys().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let value = &map[key];
        if let Some(items) = value.as_array() {
            total += items.len();
            let selected = selected_json_indices(items);
            shown += selected.len();
            view.push_str(&format!("\n{key}: {} total items", items.len()));
            for index in selected { view.push_str(&format!("\n  {key}[{index}]: {}", serde_json::to_string(&items[index]).ok()?)); }
        } else if DIAGNOSTIC_MARKERS.iter().any(|marker| key.to_ascii_lowercase().contains(marker))
            || value.is_string() || value.is_number() || value.is_boolean()
        {
            view.push_str(&format!("\n{key}: {}", serde_json::to_string(value).ok()?));
            shown += 1;
            total += 1;
        }
    }
    view.push_str("\n\n");
    view.push_str(&retrieval_trailer(ContentKind::JsonObject, shown, total.max(shown), output_id));
    Some(view)
}

fn build_search_results_view(args: &Value, content: &str, output_id: &str) -> Option<String> {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() { return None; }
    let mut seen = HashSet::new();
    let unique = lines.iter().enumerate().filter_map(|(index, line)| {
        if seen.insert(*line) { Some((index, (*line).to_string())) } else { None }
    }).collect::<Vec<_>>();
    let query = args.get("pattern").and_then(Value::as_str)
        .or_else(|| args.get("query").and_then(Value::as_str)).unwrap_or_default().trim().to_string();
    let lower_query = query.to_ascii_lowercase();
    let tokens = lower_query.split_whitespace().collect::<Vec<_>>();
    let mut ranked = unique.iter().map(|(index, line)| {
        let lower_line = line.to_ascii_lowercase();
        let mut score = if !lower_query.is_empty() && lower_line.contains(&lower_query) { 10 } else { 0 };
        score += tokens.iter().filter(|token| lower_line.contains(*token)).count() as i32;
        (*index, line.clone(), score)
    }).collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));
    ranked.truncate(40);
    ranked.sort_by_key(|(index, _, _)| *index);
    let mut view = format!("Search results compact view: {} total lines, {} unique, {} kept", lines.len(), unique.len(), ranked.len());
    for (_, line, _) in &ranked { view.push('\n'); view.push_str(line); }
    view.push_str("\n\n");
    view.push_str(&retrieval_trailer(ContentKind::SearchResults, ranked.len(), lines.len(), output_id));
    Some(view)
}

fn build_bounded_line_view(kind: ContentKind, content: &str, output_id: &str) -> Option<String> {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() { return None; }
    let mut selected = BTreeSet::new();
    for index in 0..10.min(lines.len()) { selected.insert(index); }
    for index in lines.len().saturating_sub(12)..lines.len() { selected.insert(index); }
    for (index, line) in lines.iter().enumerate() {
        if is_diagnostic(line) { selected.insert(index); }
    }
    let rendered = collapse_adjacent_exact_lines(selected.iter().map(|index| lines[*index].to_string()).collect());
    let mut view = rendered.join("\n");
    view.push_str("\n\n");
    view.push_str(&retrieval_trailer(kind, rendered.len(), lines.len(), output_id));
    Some(view)
}

fn is_diagnostic(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    DIAGNOSTIC_MARKERS.iter().any(|marker| lower.contains(marker))
}

fn collapse_adjacent_exact_lines(lines: Vec<String>) -> Vec<String> {
    let mut collapsed = Vec::with_capacity(lines.len());
    for line in lines {
        if collapsed.last().is_none_or(|previous| previous != &line) { collapsed.push(line); }
    }
    collapsed
}

fn retrieval_trailer(kind: ContentKind, shown: usize, total: usize, output_id: &str) -> String {
    format!("[… {kind} compact view: showing {shown} of {total}; full exact output is saved as {output_id}. Call retrieve_output with id \"{output_id}\" to inspect omitted data.]", kind = kind.as_str())
}

#[cfg(test)]
mod tests {
    use super::{build_specialized_view, classify_content, ContentKind};

    #[test]
    fn content_router_classifies_and_reduces_supported_shapes() {
        assert_eq!(classify_content("grep", &serde_json::json!({"pattern":"needle"}), "a: needle"), ContentKind::SearchResults);
        let json = serde_json::Value::Array((0..30).map(|i| {
            if i == 17 { serde_json::json!({"id":i,"status":"error","message":"boom"}) }
            else { serde_json::json!({"id":i,"status":"ok","payload":"repetitive payload"}) }
        }).collect()).to_string();
        assert_eq!(classify_content("bash", &serde_json::json!({}), &json), ContentKind::JsonArray);
        assert_eq!(classify_content("bash", &serde_json::json!({"command":"cargo test"}), "running 3 tests\ntest a ... ok\ntest b ... FAILED"), ContentKind::TestOutput);
        assert_eq!(classify_content("exec_command", &serde_json::json!({"command":"cargo test -p davinci-agent"}), "running 3 tests\ntest a ... ok\ntest b ... FAILED"), ContentKind::TestOutput);
        assert_eq!(classify_content("bash", &serde_json::json!({"command":"git diff"}), "diff --git a/a.rs b/a.rs\n@@ -1 +1 @@"), ContentKind::PlainText);
        let view = build_specialized_view(ContentKind::JsonArray, "bash", &serde_json::json!({}), &json, "out-0123456789ab").unwrap();
        assert!(view.content.len() < json.len());
        assert!(view.content.contains("retrieve_output"));
    }

    #[test]
    fn content_router_classifies_extended_shapes() {
        let compiler = (0..12).map(|i| format!("error[E0308]: mismatch {i}\n --> src/lib.rs:{}:1\nwarning: detail", i + 1)).collect::<Vec<_>>().join("\n");
        assert_eq!(classify_content("exec_command", &serde_json::json!({"command":"cargo check"}), &compiler), ContentKind::CompilerDiagnostics);

        let tests = "running 2 tests\ntest alpha ... ok\ntest beta ... FAILED\ntest result: FAILED. 1 passed; 1 failed";
        assert_eq!(classify_content("exec_command", &serde_json::json!({"command":"cargo test -p app"}), tests), ContentKind::TestOutput);

        let ndjson = (0..12).map(|i| serde_json::json!({"id":i,"status":if i == 7 {"error"} else {"ok"}}).to_string()).collect::<Vec<_>>().join("\n");
        assert_eq!(classify_content("exec_command", &serde_json::json!({"command":"cat events.ndjson"}), &ndjson), ContentKind::Ndjson);

        let object = serde_json::json!({"items": (0..12).map(|i| serde_json::json!({"id":i,"status":"ok"})).collect::<Vec<_>>(), "errors": [{"message":"boom"}]}).to_string();
        assert_eq!(classify_content("exec_command", &serde_json::json!({"command":"cat result.json"}), &object), ContentKind::JsonObject);

        let tree = (0..15).map(|i| format!("├── dir{i}/file.rs")).collect::<Vec<_>>().join("\n");
        assert_eq!(classify_content("exec_command", &serde_json::json!({"command":"tree"}), &tree), ContentKind::Tree);

        let table = (0..10).map(|i| format!("{i}|name-{i}|ok")).collect::<Vec<_>>().join("\n");
        assert_eq!(classify_content("exec_command", &serde_json::json!({"command":"report"}), &table), ContentKind::Table);

        assert_eq!(classify_content("exec_command", &serde_json::json!({"command":"echo hello"}), "ordinary short prose"), ContentKind::PlainText);
    }
}
'''


def governor_impl() -> None:
    router = ROOT / "crates/davinci-coding-agent/src/native_extensions/content_router.rs"
    router.write_text(CONTENT_ROUTER_IMPL)

    path = ROOT / "crates/davinci-coding-agent/src/native_extensions/token_governor.rs"
    text = path.read_text()

    text = text.replace("large successful tool outputs", "large successful or failing tool outputs")
    text = replace_once(
        text,
        '    #[serde(default = "default_true")]\n    pub content_aware: bool,\n',
        '    #[serde(default = "default_true")]\n    pub content_aware: bool,\n    #[serde(default = "default_specialized_min_reduction_pct")]\n    pub specialized_min_reduction_pct: u8,\n',
        "governor config field",
    )
    text = replace_once(
        text,
        'fn default_compress_threshold_bytes() -> usize {\n    DEFAULT_COMPRESS_THRESHOLD_BYTES\n}\n',
        'fn default_specialized_min_reduction_pct() -> u8 {\n    10\n}\nfn default_compress_threshold_bytes() -> usize {\n    DEFAULT_COMPRESS_THRESHOLD_BYTES\n}\n',
        "governor default function",
    )
    text = replace_once(
        text,
        '            content_aware: true,\n            compress_threshold_bytes:',
        '            content_aware: true,\n            specialized_min_reduction_pct: default_specialized_min_reduction_pct(),\n            compress_threshold_bytes:',
        "governor default config",
    )
    text = replace_once(
        text,
        '    if let Some(value) = env_usize_any(&[\n        "DAVINCI_GOVERNOR_COMPRESS_THRESHOLD",',
        '    if let Some(value) = env_usize_any(&[\n        "DAVINCI_GOVERNOR_SPECIALIZED_MIN_REDUCTION_PCT",\n        "DAVINCI_TOKEN_GOVERNOR_SPECIALIZED_MIN_REDUCTION_PCT",\n        "PI_GOVERNOR_SPECIALIZED_MIN_REDUCTION_PCT",\n        "PI_TOKEN_GOVERNOR_SPECIALIZED_MIN_REDUCTION_PCT",\n    ]) {\n        config.specialized_min_reduction_pct = value.min(100) as u8;\n    }\n    if let Some(value) = env_usize_any(&[\n        "DAVINCI_GOVERNOR_COMPRESS_THRESHOLD",',
        "governor env threshold",
    )
    text = replace_once(
        text,
        '    pub bytes: usize,\n    pub lines: usize,\n}\n\npub fn call_fingerprint',
        '    pub bytes: usize,\n    pub lines: usize,\n    #[serde(default)]\n    pub content_kind: String,\n    #[serde(default)]\n    pub strategy: String,\n}\n\npub fn call_fingerprint',
        "stored output metadata",
    )
    text = replace_once(
        text,
        'fn collapse_repeated_lines(lines: Vec<String>) -> Vec<String> {',
        'fn clears_specialized_threshold(specialized: usize, generic: usize, pct: u8) -> bool {\n    specialized < generic\n        && specialized.saturating_mul(100)\n            <= generic.saturating_mul(100usize.saturating_sub(pct as usize))\n}\n\nfn collapse_repeated_lines(lines: Vec<String>) -> Vec<String> {',
        "specialized threshold helper",
    )

    old_stats = '''#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl ContentRoutingStats {
    fn record(
        &mut self,
        kind: crate::native_extensions::content_router::ContentKind,
        specialized: bool,
        original_bytes: usize,
        model_view_bytes: usize,
    ) {
        match kind {
            crate::native_extensions::content_router::ContentKind::Log => self.log += 1,
            crate::native_extensions::content_router::ContentKind::JsonArray => {
                self.json_array += 1
            }
            crate::native_extensions::content_router::ContentKind::SearchResults => {
                self.search_results += 1
            }
            crate::native_extensions::content_router::ContentKind::PlainText => {
                self.plain_text += 1
            }
        }
        if specialized {
            self.specialized_views += 1;
        } else {
            self.generic_views += 1;
        }
        self.original_bytes += original_bytes as u64;
        self.model_view_bytes += model_view_bytes as u64;
    }
}
'''
    new_stats = '''#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    fn get_mut(&mut self, kind: crate::native_extensions::content_router::ContentKind) -> &mut KindRetrievalStats {
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
        if specialized { self.specialized_views += 1; } else { self.generic_views += 1; }
        self.original_bytes += original_bytes as u64;
        self.model_view_bytes += model_view_bytes as u64;
        let entry = self.by_kind.get_mut(kind);
        entry.compressed += 1;
        if specialized { entry.specialized += 1; }
        entry.original_bytes += original_bytes as u64;
        entry.model_view_bytes += model_view_bytes as u64;
    }

    fn record_retrieval(&mut self, kind: &str) {
        if let Some(entry) = self.by_kind.get_mut_by_name(kind) {
            entry.retrievals += 1;
        }
    }
}
'''
    text = replace_once(text, old_stats, new_stats, "content routing stats")

    old_early = '''        if result.is_error
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
        if self.config.dedupe_reads && name == "read" {'''
    new_early = '''        let governor_skip = result
            .details
            .as_ref()
            .and_then(|details| details.get("tokenGovernor"))
            .and_then(|details| details.get("skip"))
            .and_then(Value::as_bool)
            == Some(true);
        if governor_skip {
            return result;
        }
        if !result.is_error && self.config.dedupe_reads && name == "read" {'''
    text = replace_once(text, old_early, new_early, "error compression early return")

    text = replace_once(
        text,
        '''        let (chosen_content, strategy) = match specialized {
            Some(view) if view.content.len() < generic.content.len() => {
                (view.content, "specialized")
            }
            _ => (generic.content, "generic"),
        };''',
        '''        let (chosen_content, strategy) = match specialized {
            Some(view)
                if clears_specialized_threshold(
                    view.content.len(),
                    generic.content.len(),
                    self.config.specialized_min_reduction_pct,
                ) => (view.content, "specialized"),
            _ => (generic.content, "generic"),
        };''',
        "specialized choice",
    )
    text = replace_once(
        text,
        '        self.remember_stored(name, args, &reference);',
        '        self.remember_stored(name, args, &reference, kind.as_str(), strategy);',
        "remember stored call",
    )
    text = replace_once(
        text,
        '''    fn remember_stored(&mut self, tool: &str, args: &Value, reference: &StoredOutputRef) {
        self.stored.retain(|entry| entry.id != reference.id);
        self.stored.push_front(StoredOutputEntry {
            id: reference.id.clone(),
            tool: tool.to_string(),
            call: call_summary(tool, args),
            bytes: reference.bytes,
            lines: reference.lines,
        });''',
        '''    fn remember_stored(
        &mut self,
        tool: &str,
        args: &Value,
        reference: &StoredOutputRef,
        content_kind: &str,
        strategy: &str,
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
        });''',
        "remember stored implementation",
    )
    text = replace_once(text, '    pub fn retrieve(&self, args: &Value) -> Result<ToolResult, ToolError> {', '    pub fn retrieve(&mut self, args: &Value) -> Result<ToolResult, ToolError> {', "mutable retrieve")
    text = replace_once(
        text,
        '        self.retrievals.fetch_add(1, Ordering::Relaxed);\n        let start = args',
        '        self.retrievals.fetch_add(1, Ordering::Relaxed);\n        if let Some(kind) = self.stored.iter().find(|entry| entry.id == id).map(|entry| entry.content_kind.clone()) {\n            self.content_routing.record_retrieval(&kind);\n        }\n        let start = args',
        "retrieval attribution",
    )
    text = replace_once(
        text,
        '                "retrieveMaxBytes": self.config.retrieve_max_bytes,\n            },',
        '                "retrieveMaxBytes": self.config.retrieve_max_bytes,\n                "specializedMinReductionPct": self.config.specialized_min_reduction_pct,\n            },',
        "status threshold",
    )
    text = replace_once(
        text,
        '                "retrievalsPerCompressedOutput": retrievals_per_compressed_output,\n            },',
        '                "retrievalsPerCompressedOutput": retrievals_per_compressed_output,\n                "byKind": self.content_routing.by_kind,\n            },',
        "status by kind",
    )

    if "fn specialized_view_must_clear_minimum_reduction_threshold()" not in text:
        idx = text.rfind("\n}")
        tests = r'''

    #[test]
    fn specialized_view_must_clear_minimum_reduction_threshold() {
        assert!(clears_specialized_threshold(80, 100, 10));
        assert!(!clears_specialized_threshold(95, 100, 10));
        assert!(!clears_specialized_threshold(100, 100, 10));
    }

    #[test]
    fn governor_tracks_retrievals_by_content_kind() {
        let dir = tempdir().unwrap();
        let mut governor = TokenGovernor::with_store("test", tiny_thresholds(), OutputStore::new(dir.path()));
        let log = (0..120).map(|i| format!("running task {i}\nwarning: w{i}\nerror: e{i}")).collect::<Vec<_>>().join("\n");
        let log_result = governor.after_tool("exec_command", &json!({"command":"custom runner"}), ok(&log));
        assert!(log_result.details.as_ref().unwrap()["tokenGovernor"]["outputId"].is_string());

        let json_body = serde_json::Value::Array((0..40).map(|i| serde_json::json!({"id":i,"status":if i == 31 {"error"} else {"ok"}})).collect()).to_string();
        let json_result = governor.after_tool("exec_command", &json!({"command":"cat data.json"}), ok(&json_body));
        let json_id = json_result.details.as_ref().unwrap()["tokenGovernor"]["outputId"].as_str().unwrap().to_string();
        let _ = governor.retrieve(&json!({"id":json_id})).unwrap();
        let status = governor.status();
        assert_eq!(status["contentRouting"]["byKind"]["log"]["retrievals"], 0);
        assert_eq!(status["contentRouting"]["byKind"]["jsonArray"]["retrievals"], 1);
    }
'''
        text = text[:idx] + tests + text[idx:]

    path.write_text(text)


def main() -> None:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--tests", action="store_true")
    mode.add_argument("--impl", action="store_true")
    args = parser.parse_args()

    if STAGE == "governor":
        if args.tests:
            governor_tests()
        else:
            governor_impl()
        return
    raise SystemExit(f"stage {STAGE!r} not implemented in patch driver yet")


if __name__ == "__main__":
    main()
