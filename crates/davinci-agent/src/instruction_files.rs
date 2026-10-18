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
    self, file_stamp, governing_instruction_files, is_plugin_context, reload_changed_context_files,
    render_nested_instructions, NESTED_INSTRUCTIONS_CUSTOM_TYPE,
};
use crate::prompt::manifest::hash_text;
use crate::Agent;

impl Agent {
    /// Load the repository instruction files for `cwd` and arm the
    /// request-boundary upkeep (edit refresh, nested delivery). `enabled` is
    /// the host's no-context-files switch: when false nothing is loaded now
    /// or later. Replaces every entry, including `plugin:` ones, so a host
    /// re-adds those afterwards.
    pub fn load_instruction_files(&mut self, enabled: bool) {
        self.instruction_files_enabled = enabled;
        self.context_files = context::load_context_files(&self.cwd, enabled);
        self.context_file_stamps = self
            .context_files
            .iter()
            .filter_map(|file| Some((file.path.clone(), file_stamp(&file.path)?)))
            .collect();
        self.invalidate_context_image();
    }

    /// Run before every provider request. Cheap when nothing changed: one
    /// `stat` per loaded file, and a scan of the last tool batch.
    pub(crate) fn refresh_instruction_context(&mut self) {
        if !self.instruction_files_enabled {
            return;
        }
        let edited =
            reload_changed_context_files(&mut self.context_files, &mut self.context_file_stamps);
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
        for target in &targets {
            for file in governing_instruction_files(&self.cwd, Path::new(target)) {
                let key = file.path.to_string_lossy().into_owned();
                if file.body.trim().is_empty()
                    || loaded.contains(&file.path)
                    || !seen.insert(key.clone())
                    || delivered.get(&key) == Some(&hash_text(&file.body))
                {
                    continue;
                }
                fresh.push(file);
            }
        }
        if fresh.is_empty() {
            return false;
        }
        let details = serde_json::json!({
            "files": fresh
                .iter()
                .map(|file| serde_json::json!({
                    "path": file.path.to_string_lossy(),
                    "hash": hash_text(&file.body),
                }))
                .collect::<Vec<_>>(),
        });
        self.record_custom_message(&serde_json::json!({
            "customType": NESTED_INSTRUCTIONS_CUSTOM_TYPE,
            "content": render_nested_instructions(&fresh, &self.cwd),
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
