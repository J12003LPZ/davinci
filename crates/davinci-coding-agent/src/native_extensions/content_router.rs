use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

const SEARCH_TOOLS: &[&str] = &["grep", "find", "ls"];
const LOG_COMMAND_MARKERS: &[&str] = &[
    "cargo test",
    "cargo build",
    "cargo check",
    "cargo clippy",
    "pytest",
    "python -m pytest",
    "npm test",
    "pnpm test",
    "bun test",
    "go test",
    "dotnet test",
    "mvn test",
    "gradle test",
    "make test",
];
const LOG_OUTPUT_MARKERS: &[&str] = &[
    "running ",
    "test result:",
    "traceback",
    "stack trace",
    "panic",
    "exception",
    "warning:",
    "error:",
    "failed",
];
const DIAGNOSTIC_MARKERS: &[&str] = &[
    "error",
    "failed",
    "failure",
    "panic",
    "exception",
    "warning",
    "assertion",
    "expected",
    "actual",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Log,
    JsonArray,
    SearchResults,
    PlainText,
}

impl ContentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::JsonArray => "jsonArray",
            Self::SearchResults => "searchResults",
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

    if is_log_like(tool, args, content) {
        return ContentKind::Log;
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
        ContentKind::Log => build_log_view(content, output_id),
        ContentKind::JsonArray => build_json_array_view(content, output_id),
        ContentKind::SearchResults => build_search_results_view(args, content, output_id),
        ContentKind::PlainText => return None,
    }?;

    Some(SpecializedView {
        kind,
        content: rendered,
    })
}

fn is_large_json_array(content: &str) -> bool {
    matches!(
        serde_json::from_str::<Value>(content.trim()),
        Ok(Value::Array(items)) if items.len() > 5
    )
}

fn is_log_like(tool: &str, args: &Value, content: &str) -> bool {
    if !matches!(tool, "bash" | "powershell") {
        return false;
    }

    let command = args
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if command.contains("git diff") || command.contains("git show") {
        return false;
    }
    if LOG_COMMAND_MARKERS
        .iter()
        .any(|marker| command.contains(marker))
    {
        return true;
    }

    let lower = content.to_ascii_lowercase();
    LOG_OUTPUT_MARKERS
        .iter()
        .filter(|marker| lower.contains(*marker))
        .count()
        >= 2
}

fn build_log_view(content: &str, output_id: &str) -> Option<String> {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let mut selected = BTreeSet::new();
    for index in 0..6.min(lines.len()) {
        selected.insert(index);
    }
    for index in lines.len().saturating_sub(12)..lines.len() {
        selected.insert(index);
    }
    for (index, line) in lines.iter().enumerate() {
        if is_diagnostic(line) {
            selected.insert(index.saturating_sub(1));
            selected.insert(index);
            if index + 1 < lines.len() {
                selected.insert(index + 1);
            }
        }
    }

    let rendered = collapse_adjacent_exact_lines(
        selected
            .iter()
            .map(|index| lines[*index].to_string())
            .collect(),
    );
    let shown = rendered.len();
    let mut view = rendered.join("\n");
    view.push_str("\n\n");
    view.push_str(&retrieval_trailer(
        ContentKind::Log,
        shown,
        lines.len(),
        output_id,
    ));
    Some(view)
}

fn build_json_array_view(content: &str, output_id: &str) -> Option<String> {
    let Value::Array(items) = serde_json::from_str::<Value>(content.trim()).ok()? else {
        return None;
    };
    if items.len() <= 5 {
        return None;
    }

    let mut selected = BTreeSet::new();
    selected.insert(0);
    selected.insert(1);
    selected.insert(3);
    selected.insert(items.len() - 2);
    selected.insert(items.len() - 1);
    let mut signal_count = 0;
    for (index, item) in items.iter().enumerate().skip(2) {
        let compact = serde_json::to_string(item).ok()?;
        if contains_any_marker(
            &compact,
            &["error", "fail", "panic", "exception", "warning"],
        ) && signal_count < 4
        {
            selected.insert(index);
            signal_count += 1;
        }
    }

    let shown = selected.len();
    let mut view = format!("JSON array compact view: {} total items", items.len());
    for index in selected {
        let compact = serde_json::to_string(&items[index]).ok()?;
        view.push_str(&format!("\nitem[{index}]: {compact}"));
    }
    view.push_str("\n\n");
    view.push_str(&retrieval_trailer(
        ContentKind::JsonArray,
        shown,
        items.len(),
        output_id,
    ));
    Some(view)
}

fn build_search_results_view(args: &Value, content: &str, output_id: &str) -> Option<String> {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let mut seen = HashSet::new();
    let unique = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            if seen.insert(*line) {
                Some((index, (*line).to_string()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let query = args
        .get("pattern")
        .and_then(Value::as_str)
        .or_else(|| args.get("query").and_then(Value::as_str))
        .unwrap_or_default()
        .trim()
        .to_string();
    let lower_query = query.to_ascii_lowercase();
    let tokens = lower_query.split_whitespace().collect::<Vec<_>>();

    let mut ranked = unique
        .iter()
        .map(|(index, line)| {
            let lower_line = line.to_ascii_lowercase();
            let mut score = if !lower_query.is_empty() && lower_line.contains(&lower_query) {
                10
            } else {
                0
            };
            score += tokens
                .iter()
                .filter(|token| lower_line.contains(*token))
                .count() as i32;
            (*index, line.clone(), score)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));
    ranked.truncate(40);
    ranked.sort_by_key(|(index, _, _)| *index);

    let mut view = format!(
        "Search results compact view: {} total lines, {} unique, {} kept",
        lines.len(),
        unique.len(),
        ranked.len()
    );
    for (_, line, _) in &ranked {
        view.push('\n');
        view.push_str(line);
    }
    view.push_str("\n\n");
    view.push_str(&retrieval_trailer(
        ContentKind::SearchResults,
        ranked.len(),
        lines.len(),
        output_id,
    ));
    Some(view)
}

fn is_diagnostic(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    DIAGNOSTIC_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

fn contains_any_marker(value: &str, markers: &[&str]) -> bool {
    let lower = value.to_ascii_lowercase();
    markers.iter().any(|marker| lower.contains(*marker))
}

fn collapse_adjacent_exact_lines(lines: Vec<String>) -> Vec<String> {
    let mut collapsed = Vec::with_capacity(lines.len());
    for line in lines {
        if collapsed.last().is_none_or(|previous| previous != &line) {
            collapsed.push(line);
        }
    }
    collapsed
}

fn retrieval_trailer(kind: ContentKind, shown: usize, total: usize, output_id: &str) -> String {
    format!(
        "[… {kind} compact view: showing {shown} of {total}; full exact output is saved as {output_id}. Call retrieve_output with id \"{output_id}\" to inspect omitted data.]",
        kind = kind.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::{build_specialized_view, classify_content, ContentKind};

    #[test]
    fn content_router_classifies_and_reduces_supported_shapes() {
        assert_eq!(
            classify_content(
                "grep",
                &serde_json::json!({"pattern": "needle"}),
                "a: needle"
            ),
            ContentKind::SearchResults
        );

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
        assert_eq!(
            classify_content("bash", &serde_json::json!({}), &json),
            ContentKind::JsonArray
        );

        assert_eq!(
            classify_content(
                "bash",
                &serde_json::json!({"command": "cargo test"}),
                "running 3 tests\ntest a ... ok\ntest b ... FAILED"
            ),
            ContentKind::Log
        );

        assert_eq!(
            classify_content(
                "bash",
                &serde_json::json!({"command": "git diff"}),
                "diff --git a/a.rs b/a.rs\n@@ -1 +1 @@"
            ),
            ContentKind::PlainText
        );

        let view = build_specialized_view(
            ContentKind::JsonArray,
            "bash",
            &serde_json::json!({}),
            &json,
            "out-0123456789ab",
        )
        .expect("json array should produce a specialized view");
        assert!(view.content.len() < json.len());
        assert!(view.content.contains("item[3]"));
        assert!(view.content.contains("retrieve_output"));
        assert!(view.content.contains("out-0123456789ab"));
    }
}
