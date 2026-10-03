//! Branch-tree summarization matching
//! `vendor/pi/packages/coding-agent/src/core/compaction/branch-summarization.ts`.

use davinci_ai::{ChatMessage, StopReason};
use davinci_protocol::Usage;
use davinci_session::{branch_entries, SessionEntry};

use crate::compaction::{
    compute_file_lists, convert_to_llm, env_summarizer, estimate_tokens, extract_file_ops,
    format_file_operations, get_summarization_failure, serialize_conversation, CompactionDetails,
    FileOperations, SummarizeRequest, Summarizer, SUMMARIZATION_SYSTEM_PROMPT,
};

pub const BRANCH_SUMMARY_PREAMBLE: &str = "The user explored a different conversation branch before returning here.\nSummary of that exploration:\n\n";

pub const BRANCH_SUMMARY_PROMPT: &str = "Create a structured summary of this conversation branch for context when returning later.\n\nUse this EXACT format:\n\n## Goal\n[What was the user trying to accomplish in this branch?]\n\n## Constraints & Preferences\n- [Any constraints, preferences, or requirements mentioned]\n- [Or \"(none)\" if none were mentioned]\n\n## Progress\n### Done\n- [x] [Completed tasks/changes]\n\n### In Progress\n- [ ] [Work that was started but not finished]\n\n### Blocked\n- [Issues preventing progress, if any]\n\n## Key Decisions\n- **[Decision]**: [Brief rationale]\n\n## Next Steps\n1. [What should happen next to continue this work]\n\nKeep each section concise. Preserve exact file paths, function names, and error messages.";

#[derive(Debug, Clone, Default)]
pub struct BranchSummaryResult {
    pub summary: Option<String>,
    pub usage: Option<Usage>,
    pub details: CompactionDetails,
    pub aborted: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BranchPreparation {
    pub messages: Vec<ChatMessage>,
    pub file_ops: FileOperations,
    pub total_tokens: u64,
}

pub fn collect_entries_for_branch_summary<'a>(
    entries: &'a [SessionEntry],
    old_leaf_id: Option<&str>,
    target_id: &str,
) -> (Vec<&'a SessionEntry>, Option<String>) {
    let Some(old_leaf_id) = old_leaf_id else {
        return (Vec::new(), None);
    };
    let old_path = branch_entries(entries, Some(old_leaf_id));
    let target_path = branch_entries(entries, Some(target_id));
    let old_ids: std::collections::HashSet<&str> =
        old_path.iter().map(|entry| entry.id.as_str()).collect();
    let common_ancestor_id = target_path
        .iter()
        .rev()
        .find(|entry| old_ids.contains(entry.id.as_str()))
        .map(|entry| entry.id.clone());

    let by_id: std::collections::HashMap<&str, &SessionEntry> = entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect();
    let mut collected = Vec::new();
    let mut current = Some(old_leaf_id);
    while let Some(id) = current {
        if Some(id) == common_ancestor_id.as_deref() {
            break;
        }
        let Some(entry) = by_id.get(id) else {
            break;
        };
        collected.push(*entry);
        current = entry.parent_id.as_deref();
    }
    collected.reverse();
    (collected, common_ancestor_id)
}

pub fn message_from_branch_entry(entry: &SessionEntry) -> Option<ChatMessage> {
    match entry.entry_type.as_str() {
        "message" => {
            let message = entry.message.as_ref()?;
            let role = message.get("role")?.as_str()?;
            if role == "toolResult" {
                return None;
            }
            serde_json::from_value(message.clone()).ok()
        }
        "custom_message" => crate::custom_message_from_session_entry(entry),
        "branch_summary" => {
            let summary = entry.extra.get("summary")?.as_str()?;
            Some(ChatMessage::text("branchSummary", summary))
        }
        "compaction" => {
            let summary = entry.extra.get("summary")?.as_str()?;
            Some(ChatMessage::text("compactionSummary", summary))
        }
        _ => None,
    }
}

fn content_text_from_value(value: &serde_json::Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
            .collect::<Vec<_>>()
            .join("");
    }
    String::new()
}

pub fn prepare_branch_entries(entries: &[&SessionEntry], token_budget: u64) -> BranchPreparation {
    let mut file_ops = FileOperations::default();
    for entry in entries {
        if entry.entry_type == "branch_summary"
            && entry.extra.get("fromHook") != Some(&serde_json::Value::Bool(true))
        {
            if let Some(details) = entry.extra.get("details") {
                if let Some(read) = details.get("readFiles").and_then(|value| value.as_array()) {
                    for path in read.iter().filter_map(|value| value.as_str()) {
                        file_ops.read.push(path.to_string());
                    }
                }
                if let Some(modified) = details
                    .get("modifiedFiles")
                    .and_then(|value| value.as_array())
                {
                    for path in modified.iter().filter_map(|value| value.as_str()) {
                        file_ops.edited.push(path.to_string());
                    }
                }
            }
        }
    }

    let mut messages = Vec::new();
    let mut total_tokens = 0_u64;
    for entry in entries.iter().rev() {
        let Some(message) = message_from_branch_entry(entry) else {
            continue;
        };
        let extracted = extract_file_ops(std::slice::from_ref(&message));
        file_ops.read.extend(extracted.read);
        file_ops.written.extend(extracted.written);
        file_ops.edited.extend(extracted.edited);
        let tokens = estimate_tokens(&message);
        if token_budget > 0 && total_tokens + tokens > token_budget {
            if matches!(entry.entry_type.as_str(), "compaction" | "branch_summary")
                && total_tokens < (token_budget as f64 * 0.9) as u64
            {
                messages.insert(0, message);
                total_tokens += tokens;
            }
            break;
        }
        messages.insert(0, message);
        total_tokens += tokens;
    }
    file_ops.read.sort();
    file_ops.read.dedup();
    file_ops.written.sort();
    file_ops.written.dedup();
    file_ops.edited.sort();
    file_ops.edited.dedup();
    BranchPreparation {
        messages,
        file_ops,
        total_tokens,
    }
}

pub fn build_branch_summary_prompt(
    messages: &[ChatMessage],
    custom_instructions: Option<&str>,
    replace_instructions: bool,
) -> String {
    let instructions = if replace_instructions {
        if let Some(custom) = custom_instructions {
            custom.to_string()
        } else {
            BRANCH_SUMMARY_PROMPT.to_string()
        }
    } else if let Some(custom) = custom_instructions {
        format!("{BRANCH_SUMMARY_PROMPT}\n\nAdditional focus: {custom}")
    } else {
        BRANCH_SUMMARY_PROMPT.to_string()
    };
    let conversation = serialize_conversation(&convert_to_llm(messages));
    format!("<conversation>\n{conversation}\n</conversation>\n\n{instructions}")
}

pub fn generate_branch_summary(
    entries: &[&SessionEntry],
    context_window: u64,
    reserve_tokens: u64,
    custom_instructions: Option<&str>,
    replace_instructions: bool,
    summarizer: Option<&Summarizer>,
) -> BranchSummaryResult {
    let token_budget = context_window.saturating_sub(reserve_tokens);
    let prepared = prepare_branch_entries(entries, token_budget);
    if prepared.messages.is_empty() {
        return BranchSummaryResult {
            summary: Some("No content to summarize".into()),
            ..BranchSummaryResult::default()
        };
    }
    let env = env_summarizer();
    let Some(summarizer) = summarizer.or(env.as_ref()) else {
        let details = compute_file_lists(&prepared.file_ops);
        let mut summary = BRANCH_SUMMARY_PREAMBLE.to_string();
        summary.push_str(&serialize_conversation(&convert_to_llm(&prepared.messages)));
        summary.push_str(&format_file_operations(
            &details.read_files,
            &details.modified_files,
        ));
        return BranchSummaryResult {
            summary: Some(summary),
            details,
            ..BranchSummaryResult::default()
        };
    };
    let prompt = build_branch_summary_prompt(
        &prepared.messages,
        custom_instructions,
        replace_instructions,
    );
    let response = match summarizer.summarize(&SummarizeRequest {
        system: SUMMARIZATION_SYSTEM_PROMPT.to_string(),
        prompt,
        max_tokens: 2048,
        label: "Branch summarization".into(),
        provider: String::new(),
        model_id: String::new(),
    }) {
        Ok(response) => response,
        Err(error) => {
            return BranchSummaryResult {
                error: Some(error),
                ..BranchSummaryResult::default()
            };
        }
    };
    if response.stop_reason == Some(StopReason::Aborted) {
        return BranchSummaryResult {
            aborted: true,
            ..BranchSummaryResult::default()
        };
    }
    if let Some(failure) = get_summarization_failure(&response, "Branch summarization") {
        return BranchSummaryResult {
            error: Some(failure),
            ..BranchSummaryResult::default()
        };
    }
    if response.has_tool_call {
        return BranchSummaryResult {
            error: Some("Branch summarization attempted to call a tool".into()),
            ..BranchSummaryResult::default()
        };
    }
    let details = compute_file_lists(&prepared.file_ops);
    let mut summary = format!("{BRANCH_SUMMARY_PREAMBLE}{}", response.text);
    summary.push_str(&format_file_operations(
        &details.read_files,
        &details.modified_files,
    ));
    if summary.trim().is_empty() {
        summary = "No summary generated".into();
    }
    BranchSummaryResult {
        summary: Some(summary),
        usage: Some(response.usage),
        details,
        aborted: false,
        error: None,
    }
}

pub fn navigation_target(entry: &SessionEntry) -> (Option<String>, Option<String>) {
    match entry.entry_type.as_str() {
        "message" => {
            let role = entry
                .message
                .as_ref()
                .and_then(|message| message.get("role"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if role == "user" {
                let text = entry
                    .message
                    .as_ref()
                    .and_then(|message| message.get("content"))
                    .map(content_text_from_value)
                    .unwrap_or_default();
                (entry.parent_id.clone(), Some(text))
            } else {
                (Some(entry.id.clone()), None)
            }
        }
        "custom_message" => {
            let text = entry
                .extra
                .get("content")
                .or(entry.message.as_ref())
                .map(content_text_from_value)
                .unwrap_or_default();
            (entry.parent_id.clone(), Some(text))
        }
        _ => (Some(entry.id.clone()), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compaction::SummarizeResponse;
    use davinci_ai::StopReason;

    fn message_entry(id: &str, parent: Option<&str>, role: &str, text: &str) -> SessionEntry {
        SessionEntry {
            id: id.into(),
            entry_type: "message".into(),
            parent_id: parent.map(str::to_string),
            seq: 0,
            timestamp: 0,
            message: Some(serde_json::json!({
                "role": role,
                "content": [{"type": "text", "text": text}],
            })),
            custom_type: None,
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn collect_and_prepare_keep_recent_branch_context() {
        let entries = vec![
            message_entry("root", None, "user", "start"),
            message_entry("a", Some("root"), "assistant", "old work"),
            message_entry("b", Some("a"), "user", "abandoned"),
            message_entry("c", Some("root"), "user", "other"),
        ];
        let (collected, ancestor) = collect_entries_for_branch_summary(&entries, Some("b"), "c");
        assert_eq!(ancestor.as_deref(), Some("root"));
        assert_eq!(
            collected
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        let prepared = prepare_branch_entries(&collected, 20_000);
        assert_eq!(prepared.messages.len(), 2);
        assert_eq!(prepared.messages[0].role, "assistant");
        assert_eq!(prepared.messages[1].role, "user");
        let overflow = prepare_branch_entries(&collected, 1);
        assert!(overflow.messages.is_empty());
    }

    #[test]
    fn generate_branch_summary_uses_ts_prompt_and_preamble() {
        let entries = [message_entry(
            "branch-user",
            None,
            "user",
            "Abandoned request",
        )];
        let refs: Vec<&SessionEntry> = entries.iter().collect();
        let summarizer = Summarizer::new(|request| {
            assert_eq!(request.system, SUMMARIZATION_SYSTEM_PROMPT);
            assert!(request.prompt.contains("## Goal"));
            assert!(request.prompt.contains("[User]: Abandoned request"));
            assert_eq!(request.max_tokens, 2048);
            Ok(SummarizeResponse {
                text: "## Goal\nexplore".into(),
                usage: Usage {
                    input: 2,
                    output: 3,
                    total_tokens: 5,
                    ..Usage::default()
                },
                stop_reason: Some(StopReason::Stop),
                error_message: None,
                has_tool_call: false,
            })
        });
        let result =
            generate_branch_summary(&refs, 200_000, 16_384, None, false, Some(&summarizer));
        assert!(result
            .summary
            .as_deref()
            .unwrap()
            .starts_with(BRANCH_SUMMARY_PREAMBLE));
        assert!(result
            .summary
            .as_deref()
            .unwrap()
            .contains("## Goal\nexplore"));
        assert_eq!(
            result.usage.as_ref().map(|usage| usage.total_tokens),
            Some(5)
        );
    }

    #[test]
    fn generate_branch_summary_rejects_tools_and_length() {
        let entries = [message_entry("u", None, "user", "hi")];
        let refs: Vec<&SessionEntry> = entries.iter().collect();
        let tools = Summarizer::new(|_| {
            Ok(SummarizeResponse {
                text: String::new(),
                usage: Usage::default(),
                stop_reason: Some(StopReason::ToolUse),
                error_message: None,
                has_tool_call: true,
            })
        });
        assert_eq!(
            generate_branch_summary(&refs, 200_000, 16_384, None, false, Some(&tools)).error,
            Some("Branch summarization attempted to call a tool".into())
        );
        let length = Summarizer::new(|_| {
            Ok(SummarizeResponse {
                text: "partial".into(),
                usage: Usage::default(),
                stop_reason: Some(StopReason::Length),
                error_message: None,
                has_tool_call: false,
            })
        });
        assert_eq!(
            generate_branch_summary(&refs, 200_000, 16_384, None, false, Some(&length)).error,
            Some(
                "Branch summarization failed: generation hit the token cap and the summary is incomplete".into()
            )
        );
    }
}
