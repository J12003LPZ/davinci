//! Repository instruction files (`AGENTS.md`) over the life of a session.
//!
//! Two things keep them true after startup, both run at the request boundary
//! of the agent loop ([`Agent::refresh_instruction_context`]):
//!
//! * **Edits** to a loaded file reach the next request. The system prompt is
//!   rebuilt from `context_files`, so a changed body changes the prompt once;
//!   an unchanged tree changes nothing, and the cached prefix stays
//!   byte-identical.
//! * **Nested files** that govern a path the model just touched are delivered
//!   as an appended `davinci.nested_instructions` message, scoped to their
//!   directory. They are not folded into the system prompt: rewriting it
//!   mid-session would invalidate the provider's cached prefix for the whole
//!   conversation, while an appended message extends that prefix. When
//!   nothing new is found nothing is appended.
//!
//! What was already delivered is read back from the message history (the
//! same way `turn_context` recovers its state), so it survives resume and
//! fork, and a compaction that drops the message makes the next touch deliver
//! it again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use davinci_ai::{ChatMessage, MessageContent};
use serde_json::Value;

use crate::context::{
    self, check_instruction_total, discover_created_context_files_with, file_stamp,
    governing_instruction_files_with, is_plugin_context, reload_changed_context_files_with,
    render_nested_instructions, NestedBudget, MAX_INSTRUCTION_TOTAL_BYTES,
    NESTED_INSTRUCTIONS_CUSTOM_TYPE,
};
use crate::prompt::manifest::hash_text;
use crate::Agent;

/// Marker in a delivery's file list recording that the overflow notice was
/// already sent, so it is not repeated on every request.
const NESTED_OVERFLOW_KEY: &str = "<instruction-budget>";

impl Agent {
    /// Whether the current permission policy already lets this agent read
    /// outside the workspace, which is what makes a hard-linked file readable.
    /// A poisoned lock fails closed.
    pub(crate) fn linked_files_allowed(&self) -> bool {
        self.permissions
            .lock()
            .map(|policy| policy.linked_files_allowed())
            .unwrap_or(false)
    }

    /// Load the repository instruction files for `cwd` and arm the
    /// request-boundary upkeep (edit refresh, nested delivery). `enabled` is
    /// the host's no-context-files switch: when false nothing is loaded now
    /// or later. Replaces every entry, including `plugin:` ones, so a host
    /// re-adds those afterwards.
    pub fn load_instruction_files(&mut self, enabled: bool) {
        self.instruction_files_enabled = enabled;
        // A file that cannot be admitted leaves the set empty and the error
        // standing: the next request is refused until it is fixed.
        let loaded = context::load_context_files_checked_with(
            &self.cwd,
            enabled,
            self.linked_files_allowed(),
        );
        self.instruction_error = loaded.as_ref().err().cloned();
        self.context_files = loaded.unwrap_or_default();
        self.context_file_stamps = self
            .context_files
            .iter()
            .filter_map(|file| Some((file.path.clone(), file_stamp(&file.path)?)))
            .collect();
        self.instruction_chain = if enabled {
            context::instruction_chain(&self.cwd)
        } else {
            Vec::new()
        };
        self.invalidate_context_image();
    }

    /// Run before every provider request. Cheap when nothing changed: one
    /// `stat` per loaded file, and a scan of the last tool batch.
    pub(crate) fn refresh_instruction_context(&mut self) {
        if !self.instruction_files_enabled {
            return;
        }
        let mut error = None;
        let allow_linked = self.linked_files_allowed();
        let edited = reload_changed_context_files_with(
            &mut self.context_files,
            &mut self.context_file_stamps,
            allow_linked,
        )
        .unwrap_or_else(|refused| {
            error = Some(refused);
            false
        });
        let created = discover_created_context_files_with(
            &mut self.context_files,
            &mut self.context_file_stamps,
            &self.instruction_chain,
            allow_linked,
        )
        .unwrap_or_else(|refused| {
            error.get_or_insert(refused);
            false
        });
        let edited = edited || created;
        self.instruction_error =
            error.or_else(|| check_instruction_total(&self.context_files).err());
        let delivered = self.deliver_nested_instructions();
        if edited || delivered {
            self.invalidate_context_image();
        }
    }

    fn deliver_nested_instructions(&mut self) -> bool {
        let targets = trailing_tool_paths(&self.messages);
        if targets.is_empty() {
            return false;
        }
        let loaded: HashSet<PathBuf> = self
            .context_files
            .iter()
            .filter(|file| !is_plugin_context(file))
            .filter_map(|file| std::fs::canonicalize(&file.path).ok())
            .collect();
        let delivered = delivered_nested_instructions(&self.messages);
        let mut seen = HashSet::new();
        let mut fresh = Vec::new();
        // The loaded files already hold part of the aggregate allowance; every
        // target of this request draws from what is left, and a file named by
        // several targets is read once.
        let held: u64 = self
            .context_files
            .iter()
            .filter(|file| !is_plugin_context(file))
            .map(|file| file.body.len() as u64)
            .sum();
        let mut budget = NestedBudget::new(MAX_INSTRUCTION_TOTAL_BYTES.saturating_sub(held));
        for target in &targets {
            // A delivered file is still read: an edit shows only in its hash.
            for file in governing_instruction_files_with(
                &self.cwd,
                Path::new(target),
                &loaded,
                self.linked_files_allowed(),
                &mut budget,
            ) {
                let key = file.path.to_string_lossy().into_owned();
                if file.body.trim().is_empty()
                    || !seen.insert(key.clone())
                    || delivered.get(&key) == Some(&hash_text(&file.body))
                {
                    continue;
                }
                fresh.push(file);
            }
        }
        // Said once: files left out because the instruction budget ran out.
        let say_overflow = budget.overflow
            && delivered.get(NESTED_OVERFLOW_KEY) != Some(&NESTED_OVERFLOW_KEY.to_owned());
        if fresh.is_empty() && !say_overflow {
            return false;
        }
        let mut files: Vec<Value> = fresh
            .iter()
            .map(|file| {
                serde_json::json!({
                    "path": file.path.to_string_lossy(),
                    "hash": hash_text(&file.body),
                })
            })
            .collect();
        let mut content = render_nested_instructions(&fresh, &self.cwd);
        if say_overflow {
            content.push_str(&format!(
                "\n[Some nested AGENTS.md files were not delivered: the {MAX_INSTRUCTION_TOTAL_BYTES} byte instruction limit was reached.]"
            ));
            files.push(serde_json::json!({
                "path": NESTED_OVERFLOW_KEY,
                "hash": NESTED_OVERFLOW_KEY,
            }));
        }
        let details = serde_json::json!({ "files": files });
        self.record_custom_message(&serde_json::json!({
            "customType": NESTED_INSTRUCTIONS_CUSTOM_TYPE,
            "content": content,
            "display": false,
            "details": details,
        }));
        // A host message, not a prompt: it must not replay as one.
        let _ = self.pending_prompt_messages.pop();
        true
    }
}

/// Path arguments of the successful path-taking tool calls in the newest
/// assistant message, which the history ends with at a request boundary.
fn trailing_tool_paths(messages: &[ChatMessage]) -> Vec<String> {
    let mut results: HashMap<&str, bool> = HashMap::new();
    for message in messages.iter().rev() {
        match message.role.as_str() {
            "toolResult" => {
                if let Some(id) = message.tool_call_id.as_deref() {
                    results.insert(id, message.is_error != Some(true));
                }
            }
            "assistant" => {
                let mut paths = Vec::new();
                for block in &message.content {
                    let MessageContent::ToolCall {
                        id,
                        name,
                        arguments,
                    } = block
                    else {
                        continue;
                    };
                    if results.get(id.as_str()) == Some(&true) {
                        paths.extend(tool_paths(name, arguments));
                    }
                }
                return paths;
            }
            _ => {}
        }
    }
    Vec::new()
}

/// The files a built-in tool call reads, writes or searches.
fn tool_paths(name: &str, arguments: &Value) -> Vec<String> {
    let path = || {
        arguments
            .get("path")
            .and_then(Value::as_str)
            .map(String::from)
    };
    match name {
        "read" | "write" | "edit" | "notebook_edit" | "ls" | "find" | "grep" => {
            path().into_iter().collect()
        }
        "apply_patch" => arguments
            .get("input")
            .and_then(Value::as_str)
            .map(|patch| {
                patch
                    .lines()
                    .filter_map(|line| {
                        [
                            "*** Add File:",
                            "*** Update File:",
                            "*** Delete File:",
                            "*** Move to:",
                        ]
                        .iter()
                        .find_map(|marker| line.strip_prefix(marker))
                    })
                    .map(|path| path.trim().to_string())
                    .filter(|path| !path.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// `path -> content hash` of every nested file the history already carries.
fn delivered_nested_instructions(messages: &[ChatMessage]) -> HashMap<String, String> {
    let mut delivered = HashMap::new();
    for message in messages {
        if message.role != "custom"
            || message.extra_str("customType") != Some(NESTED_INSTRUCTIONS_CUSTOM_TYPE)
        {
            continue;
        }
        let files = message
            .extra
            .get("details")
            .and_then(|details| details.get("files"))
            .and_then(Value::as_array);
        for file in files.into_iter().flatten() {
            if let (Some(path), Some(hash)) = (
                file.get("path").and_then(Value::as_str),
                file.get("hash").and_then(Value::as_str),
            ) {
                delivered.insert(path.to_string(), hash.to_string());
            }
        }
    }
    delivered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{INSTRUCTION_READS, MAX_INSTRUCTION_FILE_BYTES};
    use davinci_ai::ChatMessage;

    /// Re-check item 11: nested delivery draws from one aggregate allowance
    /// shared with the loaded files and across targets. A deep chain of
    /// maximum-size files is read until the allowance is gone, then stops with
    /// a visible notice, and a path named by two targets is read once.
    #[test]
    fn audit_wp01_b017_nested_delivery_shares_one_aggregate_allowance() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let big = vec![b'n'; MAX_INSTRUCTION_FILE_BYTES as usize];
        std::fs::write(dir.path().join("AGENTS.md"), &big).unwrap();
        let mut deepest = dir.path().to_path_buf();
        for level in 0..40 {
            deepest = deepest.join(format!("d{level}"));
            std::fs::create_dir(&deepest).unwrap();
            std::fs::write(deepest.join("AGENTS.md"), &big).unwrap();
        }
        std::fs::write(deepest.join("x.rs"), "fn x() {}").unwrap();
        let relative = deepest
            .join("x.rs")
            .strip_prefix(dir.path())
            .unwrap()
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");

        let mut agent = Agent::new("base");
        agent.cwd = dir.path().to_path_buf();
        agent.load_instruction_files(true);
        assert!(agent.instruction_error.is_none());
        assert_eq!(agent.context_files.len(), 1, "only the root is loaded");

        // Two successful reads of the same deep file in one assistant turn.
        agent.messages.push(ChatMessage::text("user", "go"));
        let mut call = ChatMessage::text("assistant", "");
        call.content = ["call-1", "call-2"]
            .iter()
            .map(|id| MessageContent::ToolCall {
                id: (*id).into(),
                name: "read".into(),
                arguments: serde_json::json!({"path": relative}),
            })
            .collect();
        agent.messages.push(call);
        for id in ["call-1", "call-2"] {
            let mut result = ChatMessage::text("toolResult", "ok");
            result.tool_call_id = Some(id.into());
            agent.messages.push(result);
        }

        INSTRUCTION_READS.with(|reads| reads.borrow_mut().clear());
        assert!(agent.deliver_nested_instructions());
        let reads = INSTRUCTION_READS.with(|reads| reads.borrow().clone());

        // Root is loaded (256 KiB of the 1 MiB), so three nested files fit; the
        // fourth is probed with no allowance and ends the chain. The second
        // target adds no read at all.
        let nested: Vec<_> = reads
            .iter()
            .filter(|(path, _)| path.components().any(|part| part.as_os_str() == "d0"))
            .collect();
        assert_eq!(nested.len(), 4, "{nested:?}");
        let granted: u64 = nested
            .iter()
            .map(|(_, cap)| *cap)
            .filter(|cap| *cap > 0)
            .sum();
        assert!(granted <= MAX_INSTRUCTION_TOTAL_BYTES - MAX_INSTRUCTION_FILE_BYTES);
        assert_eq!(
            nested[3].1, 0,
            "the allowance was gone before the fourth read"
        );
        let delivered = delivered_nested_instructions(&agent.messages);
        assert_eq!(
            delivered.len(),
            4,
            "three files and the overflow marker: {delivered:?}"
        );
        assert!(delivered.contains_key(NESTED_OVERFLOW_KEY));

        // A second request says nothing new: the overflow notice is sent once.
        agent.messages.push(ChatMessage::text("user", "again"));
        let mut call = ChatMessage::text("assistant", "");
        call.content = vec![MessageContent::ToolCall {
            id: "call-3".into(),
            name: "read".into(),
            arguments: serde_json::json!({"path": relative}),
        }];
        agent.messages.push(call);
        let mut result = ChatMessage::text("toolResult", "ok");
        result.tool_call_id = Some("call-3".into());
        agent.messages.push(result);
        assert!(!agent.deliver_nested_instructions());
    }
}
