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
        if self.is_plan_mode() || self.abort_requested() {
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
        self.completion_context
            .push(ChatMessage::text("user", text));
        self.invalidate_context_image();
        self.push_event(
            events,
            AgentEvent::CompletionReminder {
                reason_code: reason.into(),
            },
        );
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
