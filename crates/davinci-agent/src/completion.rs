//! Davinci completion wiring; no TypeScript counterpart. The Stable prompt and
//! tools remain frozen: continuations are bounded request suffixes, never WAL.
use crate::{Agent, AgentEvent, PermissionVerdict};
use davinci_ai::ChatMessage;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const MAX_HOOK_BLOCKS: u32 = 3;
const MAX_CHANGED_PATHS: usize = 64;
const MAX_HOOK_REASON_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct CompletionHook(pub Arc<dyn Fn(bool) -> Option<String> + Send + Sync>);

impl std::fmt::Debug for CompletionHook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CompletionHook(..)")
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CompletionState {
    request: String,
    before: Option<crate::verification::workspace::Snapshot>,
    git_before: Option<BTreeMap<PathBuf, [u8; 2]>>,
    pub(crate) mutations: BTreeSet<PathBuf>,
    requirement_sent: bool,
    hook_blocks: u32,
    hook_limit_noticed: bool,
    initial_mutation_generation: u64,
    initial_tool_calls: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct CompletionContextMessage {
    pub(crate) after_message: usize,
    pub(crate) message: ChatMessage,
}

impl Agent {
    pub(crate) fn begin_completion_prompt(&mut self, text: &str) {
        self.completion_context.clear();
        let before = self.completion_file_snapshot(&[]);
        let git_before = self.completion_git_status();
        *self
            .completion_state
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = CompletionState {
            request: text.into(),
            before,
            git_before,
            initial_mutation_generation: self.mutation_verification_state().mutation_generation,
            initial_tool_calls: self.stats.tool_calls,
            ..CompletionState::default()
        };
        self.invalidate_context_image();
    }

    fn completion_observation_allowed(&self, path: &Path) -> bool {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        };
        let Ok(relative) = absolute.strip_prefix(&self.cwd) else {
            return false;
        };
        if relative
            .components()
            .next()
            .is_some_and(|part| part.as_os_str() == ".davinci-transactions")
        {
            return false;
        }
        let (outside, escape) = crate::is_outside_or_symlink_escape(&self.cwd, &absolute);
        if relative
            .components()
            .any(|part| matches!(part, Component::ParentDir))
            || crate::is_sensitive_file_path(&relative.to_string_lossy())
            || outside
            || escape
        {
            return false;
        }
        self.permissions.lock().is_ok_and(|policy| {
            [relative.to_string_lossy(), absolute.to_string_lossy()]
                .iter()
                .all(|spelling| {
                    matches!(
                        policy.decide(
                            "completion-observation",
                            "read",
                            &serde_json::json!({"path":spelling}),
                            &self.cwd
                        ),
                        PermissionVerdict::Allow
                    )
                })
        })
    }

    fn completion_auxiliary_observation_allowed(&self) -> bool {
        self.active_contract().is_none()
            && !self
                .runtime
                .as_ref()
                .is_some_and(|runtime| runtime.parent_agent_id.is_some())
            && std::env::var_os("PI_GRAPH_ROLE").is_none()
    }

    fn completion_file_snapshot(
        &self,
        inputs: &[PathBuf],
    ) -> Option<crate::verification::workspace::Snapshot> {
        // A hook/subscriber can deny a read. Keep content identity observation on
        // the tool path in that case; successful mutation facts still accumulate.
        if !self.completion_auxiliary_observation_allowed()
            || self.pre_tool.is_some()
            || self.named_file_hooks_active
            || self.runtime.is_some()
            || !self.tools.iter().any(|tool| tool == "read")
        {
            return None;
        }
        Some(crate::verification::workspace::Snapshot::capture_guarded(
            &self.cwd,
            inputs,
            &|path| self.completion_observation_allowed(path),
        ))
    }

    fn completion_git_status(&self) -> Option<BTreeMap<PathBuf, [u8; 2]>> {
        if !self.completion_auxiliary_observation_allowed() {
            return None;
        }
        // Read-only metadata authority is required before the sanitized Git
        // observer. Project config cannot invoke fsmonitor or remote helpers.
        let metadata = self.cwd.join(".git");
        if !self.permissions.lock().is_ok_and(|policy| {
            [".git".to_string(), metadata.to_string_lossy().into_owned()]
                .iter()
                .all(|path| {
                    matches!(
                        policy.decide(
                            "completion-git",
                            "read",
                            &serde_json::json!({"path":path}),
                            &self.cwd
                        ),
                        PermissionVerdict::Allow
                    )
                })
        }) {
            return None;
        }
        let top = crate::runtime::transactions::observe_git(
            &self.cwd,
            &["rev-parse", "--show-toplevel"],
            8192,
        )
        .ok()?;
        let top = std::str::from_utf8(&top).ok()?.trim();
        if Path::new(top).canonicalize().ok()? != self.cwd.canonicalize().ok()? {
            return None;
        }
        let bytes = crate::runtime::transactions::observe_git(
            &self.cwd,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            2 * 1024 * 1024,
        )
        .ok()?;
        let mut status = parse_git_status(&bytes)?;
        status.retain(|path, _| self.completion_observation_allowed(path));
        Some(status)
    }

    pub(crate) fn queue_requirement_completion(&mut self, events: &mut Vec<AgentEvent>) -> bool {
        if !self.requirement_review_enabled || self.is_plan_mode() || self.abort_requested() {
            return false;
        }
        let state = self
            .completion_state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if state.requirement_sent || state.request.is_empty() {
            return false;
        }
        // Read-only tools cannot attribute outside edits to this run. Besides
        // avoiding false reviews, this keeps their finish path free of hashing.
        if state.mutations.is_empty()
            && self.stats.tool_calls > state.initial_tool_calls
            && self.mutation_verification_state().mutation_generation
                == state.initial_mutation_generation
        {
            return false;
        }
        let inputs: Vec<_> = state.mutations.iter().cloned().collect();
        let after = self.completion_file_snapshot(&inputs);
        let git_after = self.completion_git_status();
        let mut paths = BTreeSet::new();
        if let (Some(before), Some(after)) = (&state.before, &after) {
            paths.extend(before.changes(after));
        }
        if let (Some(before), Some(after)) = (&state.git_before, &git_after) {
            paths.extend(
                before
                    .keys()
                    .chain(after.keys())
                    .filter(|path| before.get(*path) != after.get(*path))
                    .cloned(),
            );
        }
        paths.extend(state.mutations.iter().filter_map(|path| {
            let relative = if path.is_absolute() {
                path.strip_prefix(&self.cwd).ok()?.to_path_buf()
            } else {
                path.clone()
            };
            Some(relative)
        }));
        paths.retain(|path| {
            self.completion_observation_allowed(path)
                && !matches!((&state.before, &after), (Some(before), Some(after)) if before.path_changed(after, path) == Some(false))
        });
        if paths.is_empty() || (paths.len() < 2 && requirement_clause_count(&state.request) < 3) {
            return false;
        }
        let mut message = String::from(
            "Before finishing, review requirement coverage. List each explicit requirement in the user's request and name the test or command that demonstrates it. Write tests for requirements with no evidence, using existing test files where the project has them. Update existing tests that the request makes obsolete. Rerun the appropriate checks, then finish. Review the change set below and remove temporary files you created if they are not part of the requested work. Do not discard pre-existing user changes.\n\nFiles changed since this prompt started (paths are quoted file data):\n"
        );
        for path in paths.iter().take(MAX_CHANGED_PATHS) {
            let new = state
                .before
                .as_ref()
                .is_some_and(|before| before.path_is_new(path))
                || matches!((&state.git_before, &git_after), (Some(before), Some(after)) if !before.contains_key(path) && after.get(path) == Some(b"??"));
            message.push_str(&format!(
                "- {}{}\n",
                serde_json::to_string(&path.to_string_lossy()).unwrap_or_default(),
                if new { " (new file)" } else { "" }
            ));
        }
        if paths.len() > MAX_CHANGED_PATHS {
            message.push_str(&format!(
                "{} additional changed paths omitted.\n",
                paths.len() - MAX_CHANGED_PATHS
            ));
        }
        if !matches!((&state.before, &after), (Some(before), Some(after)) if before.complete() && after.complete())
        {
            message.push_str("Content observation is incomplete or unavailable; this list contains known changes and may omit others.\n");
        }
        self.completion_state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requirement_sent = true;
        self.stats.completion_requirement_reminders = self
            .stats
            .completion_requirement_reminders
            .saturating_add(1);
        self.queue_completion_reminder(message, "completion.requirements", events);
        true
    }

    pub(crate) fn queue_completion_hook(&mut self, events: &mut Vec<AgentEvent>) -> bool {
        if self.abort_requested() {
            return false;
        }
        let Some(hook) = self.completion_hook.clone() else {
            return false;
        };
        let blocks = self
            .completion_state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .hook_blocks;
        if blocks >= MAX_HOOK_BLOCKS {
            let mut state = self
                .completion_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !state.hook_limit_noticed {
                state.hook_limit_noticed = true;
                drop(state);
                self.stats.completion_hook_limit_hits =
                    self.stats.completion_hook_limit_hits.saturating_add(1);
                self.push_event(events, AgentEvent::CompletionNotice {
                    reason_code: "completion.hook_limit".into(),
                    text: format!("Completion hooks blocked {MAX_HOOK_BLOCKS} consecutive attempts. Finishing with the hook's checks still unresolved."),
                });
            }
            return false;
        }
        let reason = (hook.0)(blocks > 0);
        if self.abort_requested() {
            return false;
        }
        let Some(mut reason) = reason else {
            self.completion_state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .hook_blocks = 0;
            return false;
        };
        if reason.len() > MAX_HOOK_REASON_BYTES {
            let mut end = MAX_HOOK_REASON_BYTES;
            while !reason.is_char_boundary(end) {
                end -= 1;
            }
            reason.truncate(end);
            reason.push_str("\n[Hook feedback truncated]");
        }
        self.completion_state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .hook_blocks += 1;
        self.stats.completion_hook_blocks = self.stats.completion_hook_blocks.saturating_add(1);
        self.queue_completion_reminder(format!("A completion hook blocked this attempt. Resolve its feedback before finishing:\n{reason}"), "completion.hook_block", events);
        true
    }

    fn queue_completion_reminder(
        &mut self,
        text: String,
        reason: &str,
        events: &mut Vec<AgentEvent>,
    ) {
        let mut message = ChatMessage::text("user", text);
        message.extra.insert(
            "davinciCompletionOverlay".into(),
            serde_json::Value::Bool(true),
        );
        self.completion_context.push(CompletionContextMessage {
            after_message: self.messages.len(),
            message,
        });
        self.invalidate_context_image();
        self.push_event(
            events,
            AgentEvent::CompletionReminder {
                reason_code: reason.into(),
            },
        );
    }

    pub(crate) fn completion_provider_history(&self, history: &[ChatMessage]) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(history.len() + self.completion_context.len());
        let mut overlays = self.completion_context.iter().peekable();
        for index in 0..=history.len() {
            while overlays
                .peek()
                .is_some_and(|overlay| overlay.after_message.min(history.len()) <= index)
            {
                messages.push(overlays.next().unwrap().message.clone());
            }
            if let Some(message) = history.get(index) {
                messages.push(message.clone());
            }
        }
        messages
    }
}

fn requirement_clause_count(request: &str) -> usize {
    request
        .split([',', ';', '.', '!', '?', '\n'])
        .filter(|clause| clause.chars().any(char::is_alphanumeric))
        .count()
}

fn parse_git_status(bytes: &[u8]) -> Option<BTreeMap<PathBuf, [u8; 2]>> {
    let mut fields = bytes.split(|byte| *byte == 0);
    let mut paths = BTreeMap::new();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        if field.len() < 4 || field[2] != b' ' {
            return None;
        }
        let status = [field[0], field[1]];
        let path = PathBuf::from(std::str::from_utf8(&field[3..]).ok()?);
        paths.insert(path, status);
        if status.contains(&b'R') || status.contains(&b'C') {
            let source = fields.next()?;
            if source.is_empty() {
                return None;
            }
            paths.insert(PathBuf::from(std::str::from_utf8(source).ok()?), status);
        }
    }
    Some(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_ai::AssistantMessage;

    fn reply(_: &Agent) -> Result<AssistantMessage, String> {
        serde_json::from_value(serde_json::json!({"id":"answer", "role":"assistant",
            "model":"fixture", "content":[{"type":"text", "text":"Done"}], "stopReason":"stop"}))
        .map_err(|error| error.to_string())
    }

    fn fixture(root: &Path) -> Agent {
        let mut agent = Agent::new("fixture");
        agent.cwd = root.into();
        agent.auto_compaction = false;
        agent.auto_verify = false;
        agent
    }

    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn change_set_detects_dirty_to_dirty_edits_and_marks_only_new_files() {
        let root = tempfile::tempdir().unwrap();
        for path in ["app.py", "user.py"] {
            std::fs::write(root.path().join(path), "clean").unwrap();
        }
        git(root.path(), &["init", "-q"]);
        git(root.path(), &["add", "."]);
        git(
            root.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "initial",
            ],
        );
        for path in ["app.py", "user.py", "preexisting.py"] {
            std::fs::write(root.path().join(path), "dirty before").unwrap();
        }
        let mut agent = fixture(root.path());
        agent.prompt("Handle normal input, reject invalid input; preserve the format.");
        std::fs::write(root.path().join("app.py"), "dirty after").unwrap();
        std::fs::write(root.path().join("preexisting.py"), "dirty after").unwrap();
        std::fs::write(root.path().join("fresh.py"), "new").unwrap();
        let mut reminder = None;
        agent
            .run_loop(|agent| {
                if let Some(last) = agent.completion_context.last() {
                    reminder = Some(davinci_ai::content_text(&last.message.content));
                }
                reply(agent)
            })
            .unwrap();
        let reminder = reminder.unwrap();
        assert!(reminder.contains("- \"app.py\"\n"));
        assert!(reminder.contains("- \"preexisting.py\"\n"));
        assert!(reminder.contains("- \"fresh.py\" (new file)"));
        assert!(!reminder.contains("user.py"));
        assert_eq!(agent.stats.completion_requirement_reminders, 1);
    }

    #[test]
    fn reverted_successful_mutations_are_omitted_from_change_set() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("app.py"), "original").unwrap();
        let mut agent = fixture(root.path());
        agent.prompt("Handle normal input, reject invalid input; preserve the format.");
        std::fs::write(root.path().join("app.py"), "edited").unwrap();
        agent.record_successful_mutation_paths(vec!["app.py".into()]);
        std::fs::write(root.path().join("app.py"), "original").unwrap();
        agent.record_successful_mutation_paths(vec!["app.py".into()]);
        let events = agent.run_loop(reply).unwrap();
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::CompletionReminder { .. })));
    }

    #[test]
    fn denied_reads_do_not_enter_requirement_change_list() {
        let root = tempfile::tempdir().unwrap();
        let mut agent = fixture(root.path());
        agent
            .permissions
            .lock()
            .unwrap()
            .deny
            .push(crate::PermissionRule::parse("read(private.py)").unwrap());
        agent.prompt("Handle normal input, reject invalid input; preserve the format.");
        for path in ["private.py", "public.py"] {
            std::fs::write(root.path().join(path), "changed").unwrap();
        }
        agent.record_successful_mutation_paths(vec!["private.py".into(), "public.py".into()]);
        let mut text = String::new();
        agent
            .run_loop(|agent| {
                if let Some(message) = agent.completion_context.last() {
                    text = davinci_ai::content_text(&message.message.content);
                }
                reply(agent)
            })
            .unwrap();
        assert!(text.contains("public.py"));
        assert!(!text.contains("private.py"));
        assert!(text.contains("incomplete"));
    }

    #[test]
    fn steering_starts_a_fresh_requirement_budget_and_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let mut agent = fixture(root.path());
        agent.prompt("Handle normal input, reject invalid input; preserve the format.");
        std::fs::write(root.path().join("app.py"), "initial").unwrap();
        let remote = agent.remote_queue();
        let mut injected = false;
        agent
            .run_loop(|agent| {
                if !injected && !agent.completion_context.is_empty() {
                    remote.push_steer(
                        "Handle normal input, reject invalid input; also preserve encoding.".into(),
                        vec![],
                    );
                    injected = true;
                } else if injected && agent.completion_context.is_empty() {
                    std::fs::write(root.path().join("app.py"), "steered").unwrap();
                    agent.record_successful_mutation_paths(vec!["app.py".into()]);
                }
                reply(agent)
            })
            .unwrap();
        assert_eq!(agent.stats.completion_requirement_reminders, 2);
        assert_eq!(agent.stats.user_steers, 1);
    }

    #[test]
    fn active_context_vm_includes_ephemeral_completion_suffix_and_accounts_for_it() {
        let root = tempfile::tempdir().unwrap();
        let mut agent = fixture(root.path());
        agent.set_runtime(crate::RuntimeHandle::new(
            crate::RunId::new(),
            crate::AgentId::new(),
            crate::RuntimeBus::new(),
        ));
        agent.set_context_vm_mode(crate::ContextVmMode::Active);
        agent.prompt("Handle normal input, reject invalid input; preserve the format.");
        std::fs::write(root.path().join("app.py"), "after").unwrap();
        agent.record_successful_mutation_paths(vec!["app.py".into()]);
        let mut saw = false;
        agent
            .run_loop(|agent| {
                let image = agent.prepared_context_image().unwrap();
                if !agent.completion_context.is_empty() {
                    saw = true;
                    assert!(
                        davinci_ai::content_text(&image.messages.last().unwrap().content)
                            .contains("List each explicit requirement")
                    );
                    assert!(image.entries.last().unwrap().mandatory);
                    assert!(
                        image.estimated_tokens
                            >= crate::provider_budget::message_token_ceiling(
                                &agent.completion_context.last().unwrap().message
                            )
                    );
                    assert!(agent
                        .context_vm_events_for_runtime()
                        .iter()
                        .all(|event| !event
                            .visible_text
                            .contains("List each explicit requirement")));
                }
                reply(agent)
            })
            .unwrap();
        assert!(saw);
        assert!(
            !serde_json::to_string(&agent.prepared_context_image().unwrap().messages)
                .unwrap()
                .contains("List each explicit requirement")
        );
    }

    #[test]
    fn git_porcelain_tracks_both_rename_paths_and_literal_newlines() {
        let parsed =
            parse_git_status(b"R  new.py\0old.py\0?? odd\nname.py\0 M dirty.py\0").unwrap();
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[Path::new("new.py")], *b"R ");
        assert_eq!(parsed[Path::new("old.py")], *b"R ");
        assert_eq!(parsed[Path::new("odd\nname.py")], *b"??");
        assert!(parse_git_status(b"R  new.py\0").is_none());
    }

    #[test]
    fn requirement_rule_counts_separate_clauses_without_inferring_requirements() {
        assert_eq!(requirement_clause_count("Fix a typo."), 1);
        assert_eq!(
            requirement_clause_count("Normal input, invalid input; old format."),
            3
        );
        assert_eq!(requirement_clause_count("One. Two. Three."), 3);
    }

    #[test]
    fn runtime_bookkeeping_does_not_turn_a_trivial_edit_into_multiple_changed_files() {
        let root = tempfile::tempdir().unwrap();
        let mut agent = fixture(root.path());
        agent.prompt("Fix the typo");
        std::fs::write(root.path().join("app.py"), "after").unwrap();
        std::fs::create_dir(root.path().join(".davinci-transactions")).unwrap();
        std::fs::write(root.path().join(".davinci-transactions/journal.json"), "{}").unwrap();
        assert!(!agent
            .run_loop(reply)
            .unwrap()
            .iter()
            .any(|event| matches!(event, AgentEvent::CompletionReminder { .. })));
    }

    #[test]
    fn requirement_then_verification_feedback_preserves_the_previous_provider_prefix() {
        let root = tempfile::tempdir().unwrap();
        let mut agent = fixture(root.path());
        agent.prompt("Handle normal input, reject invalid input; preserve the format.");
        std::fs::write(root.path().join("app.py"), "after").unwrap();
        agent.record_successful_mutation_paths(vec!["app.py".into()]);
        let mut previous = Vec::new();
        agent
            .run_loop(|agent| {
                let messages = agent.messages_for_provider();
                assert!(
                    messages.starts_with(&previous),
                    "completion feedback reordered existing provider input"
                );
                previous = messages;
                reply(agent)
            })
            .unwrap();
        assert_eq!(agent.stats.model_turns, 3);
    }

    #[test]
    fn native_responses_resume_records_cannot_save_transient_completion_input() {
        let root = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let mut agent = fixture(root.path());
        agent.session =
            Some(crate::JsonlSession::create(sessions.path(), "native-completion", None).unwrap());
        agent.prompt("Handle normal input, reject invalid input; preserve the format.");
        std::fs::write(root.path().join("app.py"), "after").unwrap();
        agent.record_successful_mutation_paths(vec!["app.py".into()]);
        let model = crate::cache_stability::codex_test_model();
        agent
            .run_loop(|agent| {
                let prepared = davinci_ai::PreparedProviderRequest::new(
                    crate::cache_stability::wire_body_for_next_request(agent, &model),
                );
                let turn = davinci_ai::NativeResponsesTurn::from_prepared(
                    &prepared,
                    davinci_ai::NativeResponsesOutput {
                        response_id: Some("fixture".into()),
                        output_items: vec![],
                        final_response: None,
                        terminal_event_type: "response.completed".into(),
                    },
                )
                .unwrap();
                Ok::<_, String>(crate::CompleteOutput {
                    message: reply(agent)?,
                    stream_events: None,
                    streamed_live: false,
                    native_responses_resume: Some(davinci_ai::NativeResponsesResumeRecord {
                        turn,
                        resume_provider_message_count: 0,
                        resume_provider_messages_fingerprint: String::new(),
                    }),
                })
            })
            .unwrap();
        let saved = std::fs::read_to_string(&agent.session.as_ref().unwrap().path).unwrap();
        assert!(!saved.contains("List each explicit requirement"));
        assert_eq!(
            agent
                .session
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .filter(|entry| entry.custom_type.as_deref()
                    == Some(davinci_ai::NATIVE_RESPONSES_TURN_ENTRY_TYPE))
                .count(),
            1
        );
    }

    #[test]
    fn read_only_tools_do_not_attribute_external_workspace_edits_to_the_run() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("app.py"), "before").unwrap();
        let mut agent = fixture(root.path());
        agent.prompt("Inspect the source");
        let mut step = 0;
        let events = agent
            .run_loop(|agent| {
                step += 1;
                let mut message = reply(agent)?;
                if step == 1 {
                    message.content = vec![davinci_ai::ContentBlock::ToolCall {
                        id: "read".into(),
                        name: "read".into(),
                        arguments: serde_json::json!({"path":"app.py"}),
                    }];
                } else {
                    for path in ["external.py", "other.py"] {
                        std::fs::write(root.path().join(path), "external change").unwrap();
                    }
                }
                Ok::<_, String>(message)
            })
            .unwrap();
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::CompletionReminder { .. })));
    }
}
