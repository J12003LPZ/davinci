//! Bounded, prompt-facing observations. No commands or project files are executed.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

pub const ENVIRONMENT_MAX_BYTES: usize = 4096;
pub const ENVIRONMENT_MAX_ENTRIES: usize = 50;
pub const ENVIRONMENT_SCAN_LIMIT: usize = 4096;
const CAPTURE_TIMEOUT: Duration = Duration::from_millis(75);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolShell {
    pub tool: String,
    /// None means the selector could not find this tool's shell.
    pub executable: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingStatus {
    Complete,
    Truncated,
    Unavailable,
    ScanLimitExceeded,
    TimedOut,
    ProbeBusy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentSnapshot {
    pub cwd: String,
    pub os: String,
    pub tool_shells: Vec<ToolShell>,
    pub utc_date: String,
    pub top_level_entries: Vec<String>,
    pub listing_status: ListingStatus,
    pub lossy_names: bool,
    pub fields_truncated: bool,
    /// Executable discovery only; availability is not proof a script will run.
    pub python_executable: Option<String>,
}

impl EnvironmentSnapshot {
    fn empty(cwd: &Path, shells: Vec<ToolShell>, date: &str, status: ListingStatus) -> Self {
        Self {
            cwd: cwd.to_string_lossy().into_owned(),
            os: std::env::consts::OS.into(),
            tool_shells: shells,
            utc_date: date.into(),
            top_level_entries: Vec::new(),
            listing_status: status,
            lossy_names: cwd.to_str().is_none(),
            fields_truncated: false,
            python_executable: None,
        }
    }

    /// JSON strings keep names as data. Escape markup delimiters too, so a
    /// filename cannot close the enclosing runtime/environment section.
    pub fn render(&self) -> String {
        let mut bounded = self.clone();
        bounded.fields_truncated |= clip(&mut bounded.cwd, 768);
        bounded.fields_truncated |= clip(&mut bounded.os, 32);
        bounded.fields_truncated |= clip(&mut bounded.utc_date, 32);
        if bounded.tool_shells.len() > 4 {
            bounded.tool_shells.truncate(4);
            bounded.fields_truncated = true;
        }
        for shell in &mut bounded.tool_shells {
            bounded.fields_truncated |= clip(&mut shell.tool, 32);
            if let Some(executable) = &mut shell.executable {
                bounded.fields_truncated |= clip(executable, 256);
            }
        }
        if let Some(python) = &mut bounded.python_executable {
            bounded.fields_truncated |= clip(python, 256);
        }
        bounded.top_level_entries.sort();
        if bounded.top_level_entries.len() > ENVIRONMENT_MAX_ENTRIES {
            bounded.top_level_entries.truncate(ENVIRONMENT_MAX_ENTRIES);
            bounded.listing_status = ListingStatus::Truncated;
        }
        loop {
            let json = serde_json::to_string(&bounded)
                .expect("environment is serializable")
                .replace('&', "\\u0026")
                .replace('<', "\\u003c")
                .replace('>', "\\u003e");
            let text = format!("<environment>\n{json}\n</environment>");
            if text.len() <= ENVIRONMENT_MAX_BYTES {
                return text;
            }
            if bounded.top_level_entries.pop().is_some() {
                bounded.listing_status = ListingStatus::Truncated;
            } else {
                // JSON escaping can expand a bounded field by up to 6x.
                // Reduce field byte limits until even adversarial data fits.
                bounded.fields_truncated = true;
                let cwd_limit = bounded.cwd.len() / 2;
                clip(&mut bounded.cwd, cwd_limit);
                for shell in &mut bounded.tool_shells {
                    if let Some(executable) = &mut shell.executable {
                        clip(executable, executable.len() / 2);
                    }
                }
                if let Some(python) = &mut bounded.python_executable {
                    clip(python, python.len() / 2);
                }
            }
        }
    }
}

fn clip(value: &mut String, max_bytes: usize) -> bool {
    if value.len() <= max_bytes {
        return false;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    true
}

/// Read only top-level names, with a finite scan and time budget. Callers
/// running on an interactive thread use EnvironmentCapture below, which also
/// bounds a single stalled filesystem operation.
pub fn capture_environment(
    cwd: &Path,
    effective_shell: Vec<ToolShell>,
    utc_date: &str,
) -> EnvironmentSnapshot {
    let mut snapshot =
        EnvironmentSnapshot::empty(cwd, effective_shell, utc_date, ListingStatus::Complete);
    let Ok(entries) = std::fs::read_dir(cwd) else {
        snapshot.listing_status = ListingStatus::Unavailable;
        return snapshot;
    };
    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    for (index, entry) in entries.enumerate() {
        if index >= ENVIRONMENT_SCAN_LIMIT || Instant::now() >= deadline {
            snapshot.top_level_entries.clear();
            snapshot.listing_status = if index >= ENVIRONMENT_SCAN_LIMIT {
                ListingStatus::ScanLimitExceeded
            } else {
                ListingStatus::TimedOut
            };
            return snapshot;
        }
        let Ok(entry) = entry else {
            snapshot.top_level_entries.clear();
            snapshot.listing_status = ListingStatus::Unavailable;
            return snapshot;
        };
        let name = entry.file_name();
        snapshot.lossy_names |= name.to_str().is_none();
        snapshot
            .top_level_entries
            .push(name.to_string_lossy().into_owned());
    }
    snapshot.top_level_entries.sort();
    if snapshot.top_level_entries.len() > ENVIRONMENT_MAX_ENTRIES {
        snapshot.top_level_entries.truncate(ENVIRONMENT_MAX_ENTRIES);
        snapshot.listing_status = ListingStatus::Truncated;
    }
    snapshot
}

/// Cheap cache identity: no filesystem work on the model-round fast path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvironmentKey {
    cwd: PathBuf,
    tools: Vec<String>,
    selectors: Vec<Option<std::ffi::OsString>>,
}

impl EnvironmentKey {
    pub(crate) fn current(cwd: &Path, tools: &[String]) -> Self {
        Self {
            cwd: cwd.into(),
            tools: tools
                .iter()
                .filter(|name| {
                    matches!(
                        name.as_str(),
                        "exec_command" | "bash" | "shell" | "powershell"
                    )
                })
                .cloned()
                .collect(),
            selectors: [
                "PI_SHELL",
                "PATH",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "HOME",
            ]
            .iter()
            .map(std::env::var_os)
            .collect(),
        }
    }
}

/// One admitted worker per agent (including its clones). If a filesystem
/// stalls after the timeout, repeated turns do not accumulate orphan threads.
#[derive(Debug, Clone, Default)]
pub(crate) struct EnvironmentCapture {
    busy: Arc<AtomicBool>,
}

impl EnvironmentCapture {
    pub(crate) fn capture(&self, cwd: &Path, tools: &[String], date: &str) -> EnvironmentSnapshot {
        let owned_cwd = cwd.to_path_buf();
        let owned_tools = tools.to_vec();
        let owned_date = date.to_string();
        self.run(cwd, date, move || {
            let custom_shell = std::env::var("PI_SHELL")
                .ok()
                .filter(|value| !value.is_empty());
            let mut bash = None;
            let mut powershell = None;
            let mut shells = Vec::new();
            for tool in owned_tools {
                let use_powershell =
                    tool == "powershell" || (tool == "exec_command" && cfg!(windows));
                if !use_powershell && !matches!(tool.as_str(), "exec_command" | "bash" | "shell") {
                    continue;
                }
                let resolved = if use_powershell {
                    powershell.get_or_insert_with(|| {
                        crate::tools::resolve_powershell_executable(&owned_cwd)
                            .ok()
                            .map(|path| path.to_string_lossy().into_owned())
                    })
                } else {
                    bash.get_or_insert_with(|| {
                        davinci_ai::resolve_shell_config(custom_shell.as_deref())
                            .ok()
                            .and_then(|config| {
                                crate::process_manager::resolve_native_executable(
                                    &config.shell,
                                    &owned_cwd,
                                )
                                .ok()
                                .map(|path| path.to_string_lossy().into_owned())
                            })
                    })
                };
                shells.push(ToolShell {
                    tool,
                    executable: resolved.clone(),
                });
            }
            shells.sort_by(|a, b| a.tool.cmp(&b.tool));
            let mut snapshot = capture_environment(&owned_cwd, shells, &owned_date);
            snapshot.python_executable = ["python3", "python"].iter().find_map(|name| {
                crate::process_manager::resolve_native_executable(name, &owned_cwd)
                    .ok()
                    .map(|path| path.to_string_lossy().into_owned())
            });
            snapshot
        })
    }

    fn run(
        &self,
        cwd: &Path,
        date: &str,
        work: impl FnOnce() -> EnvironmentSnapshot + Send + 'static,
    ) -> EnvironmentSnapshot {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return EnvironmentSnapshot::empty(cwd, Vec::new(), date, ListingStatus::ProbeBusy);
        }
        struct Release(Arc<AtomicBool>);
        impl Drop for Release {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let release = Release(Arc::clone(&self.busy));
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new()
            .name("prompt-environment".into())
            .spawn(move || {
                let _release = release;
                let _ = tx.send(work());
            });
        if spawned.is_err() {
            return EnvironmentSnapshot::empty(cwd, Vec::new(), date, ListingStatus::Unavailable);
        }
        rx.recv_timeout(CAPTURE_TIMEOUT).unwrap_or_else(|_| {
            EnvironmentSnapshot::empty(cwd, Vec::new(), date, ListingStatus::TimedOut)
        })
    }
}

pub(crate) fn visual_verification_requested(text: &str) -> bool {
    text.to_ascii_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .any(|word| matches!(word, "visual" | "visually" | "screenshot" | "screenshots"))
}

impl crate::Agent {
    pub(crate) fn visual_verification_required(&self) -> bool {
        self.active_contract().is_some_and(|contract| {
            contract
                .verification_requirements
                .iter()
                .any(|requirement| {
                    requirement
                        .to_ascii_lowercase()
                        .split(|ch: char| !ch.is_alphanumeric())
                        .any(|word| matches!(word, "visual" | "screenshot" | "screenshots"))
                })
        })
    }

    pub(crate) fn capture_runtime_environment(&mut self, date: &str) {
        if !self.environment_context {
            self.runtime_environment = None;
            self.environment_key = None;
            return;
        }
        self.environment_key = Some(EnvironmentKey::current(&self.cwd, &self.tools));
        self.runtime_environment = Some(self.environment_capture.capture(
            &self.cwd,
            &self.tools,
            date,
        ));
    }

    /// A model/tool continuation reuses its snapshot and its UTC date. Only a
    /// changed cwd or shell selector triggers new I/O before a later request.
    pub(crate) fn refresh_runtime_environment_for_request(&mut self) -> bool {
        if !self.prompt_session.is_builtin() {
            return false;
        }
        let changed = if self.environment_context {
            self.environment_key.as_ref() != Some(&EnvironmentKey::current(&self.cwd, &self.tools))
        } else {
            self.runtime_environment.is_some()
        };
        if !changed {
            return false;
        }
        let date = self
            .runtime_environment
            .as_ref()
            .map(|snapshot| snapshot.utc_date.clone())
            .unwrap_or_else(|| davinci_session::utc_date_from_unix_ms(davinci_session::now_ms()));
        self.capture_runtime_environment(&date);
        let runtime = self.runtime_prompt_state();
        let capabilities = crate::prompt::CapabilityDecision {
            capabilities: self.capability_run_state().active,
            reasons: Vec::new(),
            evidence: Vec::new(),
        };
        let context = crate::prompt::PromptContext {
            provider: &self.provider,
            model_id: &self.model_id,
            permission_mode: runtime.permission_mode,
            plan_active: self.is_plan_mode(),
        };
        if let Ok(composed) = crate::prompt::compose_turn_prompt(
            &self.prompt_session,
            &context,
            &capabilities,
            &runtime,
        ) {
            self.apply_composed_turn_prompt(&composed);
            self.prompt_manifest = Some(composed.manifest.clone());
            self.prompt_session.last_manifest = Some(composed.manifest);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_probe_times_out_and_does_not_admit_another_worker() {
        let capture = EnvironmentCapture::default();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let started = Instant::now();
        let first = capture.run(Path::new("."), "2026-09-27", move || {
            release_rx.recv().unwrap();
            EnvironmentSnapshot::empty(
                Path::new("."),
                Vec::new(),
                "2026-09-27",
                ListingStatus::Complete,
            )
        });
        assert_eq!(first.listing_status, ListingStatus::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
        let second = capture.run(Path::new("."), "2026-09-27", || {
            panic!("second worker admitted")
        });
        assert_eq!(second.listing_status, ListingStatus::ProbeBusy);
        release_tx.send(()).unwrap();
    }

    #[test]
    fn a_continuation_freezes_the_date_and_listing_until_cwd_or_next_user_turn_changes() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let mut agent = crate::Agent::new_builtin(crate::PromptProfile::Stable);
        agent.environment_context = true;
        agent.cwd = first.path().into();
        agent.capture_runtime_environment("2026-09-27");
        let captured = agent.runtime_environment.clone();
        std::fs::write(first.path().join("new-file"), "").unwrap();
        agent.refresh_runtime_environment_for_request();
        assert_eq!(agent.runtime_environment, captured);
        agent.cwd = second.path().into();
        agent.refresh_runtime_environment_for_request();
        let value = agent.runtime_environment.as_ref().unwrap();
        assert_eq!(value.cwd, second.path().to_string_lossy());
        assert_eq!(value.utc_date, "2026-09-27");
        agent.capture_runtime_environment("2026-09-28");
        assert_eq!(
            agent.runtime_environment.as_ref().unwrap().utc_date,
            "2026-09-28"
        );
    }

    #[test]
    fn advertised_shell_change_refreshes_the_tool_to_shell_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = crate::Agent::new_builtin(crate::PromptProfile::Stable);
        agent.environment_context = true;
        agent.cwd = dir.path().into();
        agent.tools = vec!["bash".into()];
        agent.capture_runtime_environment("2026-09-27");
        assert_eq!(
            agent
                .runtime_environment
                .as_ref()
                .unwrap()
                .tool_shells
                .len(),
            1
        );
        agent.tools.push("exec_command".into());
        agent.refresh_runtime_environment_for_request();
        assert_eq!(
            agent
                .runtime_environment
                .as_ref()
                .unwrap()
                .tool_shells
                .len(),
            2
        );
    }
}
