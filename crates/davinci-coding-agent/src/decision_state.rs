use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use davinci_agent::decision::request::{
    DecisionQuestion, DecisionRequest, MAX_REQUEST_BYTES, MAX_TASK_CHARS,
};
use davinci_agent::decision::risk::DecisionClass;
use davinci_agent::runtime::contracts::redact_secrets;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableCapabilities {
    pub browser: bool,
    pub git: bool,
    #[serde(rename = "packageIntelligence")]
    pub package_intelligence: bool,
    #[serde(rename = "testImpact")]
    pub test_impact: bool,
    #[serde(rename = "changeImpact")]
    pub change_impact: bool,
    #[serde(rename = "verificationPlanner")]
    pub verification_planner: bool,
}

pub use davinci_agent::decision::request::WorkspaceDirtyState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilityId {
    BrowserVerification,
    GitIntelligence,
    PackageIntelligence,
    TestImpact,
    ChangeImpact,
    VerificationPlanner,
}

impl CapabilityId {
    fn from_registered_tool(name: &str) -> Option<Self> {
        use crate::native_extensions as native;
        [
            (Self::BrowserVerification, native::browser::TOOL_NAMES),
            (Self::GitIntelligence, native::git_intelligence::TOOL_NAMES),
            (
                Self::PackageIntelligence,
                native::package_intelligence::TOOL_NAMES,
            ),
            (Self::TestImpact, native::test_impact::TOOL_NAMES),
            (Self::ChangeImpact, native::change_impact::TOOL_NAMES),
            (
                Self::VerificationPlanner,
                native::verification_planner::TOOL_NAMES,
            ),
        ]
        .into_iter()
        .find_map(|(id, names)| names.contains(&name).then_some(id))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecisionMetadata {
    pub languages: Vec<String>,
    pub framework_signals: Vec<String>,
    pub recent_file_kinds: Vec<String>,
    pub workspace_dirty: WorkspaceDirtyState,
    pub available_capabilities: AvailableCapabilities,
}

impl DecisionMetadata {
    pub fn apply_snapshot<'a>(
        &mut self,
        dirty: WorkspaceDirtyState,
        paths: impl Iterator<Item = &'a str>,
        dependencies: impl Iterator<Item = &'a str>,
    ) {
        self.workspace_dirty = dirty;
        self.languages = paths
            .filter_map(file_kind)
            .filter_map(language_for_file_kind)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.framework_signals = dependencies
            .filter_map(|name| match name {
                "react" => Some("react"),
                "next" => Some("nextjs"),
                "vue" => Some("vue"),
                "svelte" => Some("svelte"),
                "@angular/core" => Some("angular"),
                _ => None,
            })
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }

    pub fn from_workspace(
        _cwd: &Path,
        recent_paths: &[String],
        capability_names: &[String],
    ) -> Self {
        let mut kinds = BTreeSet::new();
        for path in recent_paths {
            if let Some(kind) = file_kind(path) {
                kinds.insert(kind.to_owned());
            }
        }

        let languages = kinds
            .iter()
            .filter_map(|kind| language_for_file_kind(kind))
            .map(str::to_owned)
            .collect();

        let capabilities: BTreeSet<_> = capability_names
            .iter()
            .filter_map(|name| CapabilityId::from_registered_tool(name))
            .collect();
        let available_capabilities = AvailableCapabilities {
            browser: capabilities.contains(&CapabilityId::BrowserVerification),
            git: capabilities.contains(&CapabilityId::GitIntelligence),
            package_intelligence: capabilities.contains(&CapabilityId::PackageIntelligence),
            test_impact: capabilities.contains(&CapabilityId::TestImpact),
            change_impact: capabilities.contains(&CapabilityId::ChangeImpact),
            verification_planner: capabilities.contains(&CapabilityId::VerificationPlanner),
        };

        Self {
            languages,
            framework_signals: Vec::new(),
            recent_file_kinds: kinds.into_iter().collect(),
            workspace_dirty: WorkspaceDirtyState::Unknown,
            available_capabilities,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionState {
    pub schema_version: u8,
    pub task: String,
    pub task_signals: Vec<String>,
    pub languages: Vec<String>,
    pub framework_signals: Vec<String>,
    pub recent_file_kinds: Vec<String>,
    pub workspace_dirty: WorkspaceDirtyState,
    pub available_capabilities: AvailableCapabilities,
}

impl DecisionState {
    pub fn from_task(task: &str) -> Self {
        Self::from_task_with_metadata(task, DecisionMetadata::default())
    }

    pub fn from_task_with_metadata(task: &str, metadata: DecisionMetadata) -> Self {
        let task = truncate_chars(
            &redact_paths(&redact_environment_assignments(&redact_secrets(
                &suppress_pasted_bodies(task),
            ))),
            MAX_TASK_CHARS,
        );
        let mut state = Self {
            schema_version: 2,
            task,
            task_signals: Vec::new(),
            languages: metadata.languages,
            framework_signals: metadata.framework_signals,
            recent_file_kinds: metadata.recent_file_kinds,
            workspace_dirty: metadata.workspace_dirty,
            available_capabilities: metadata.available_capabilities,
        };
        state.refresh_task_signals();
        state.refresh_framework_signals();
        // Four-byte Unicode scalars can make the 4,096-scalar task exceed the
        // wire budget. Keep the serialized-state bound as well.
        while serde_json::to_vec(&state)
            .map(|bytes| bytes.len() > MAX_REQUEST_BYTES)
            .unwrap_or(true)
            && !state.task.is_empty()
        {
            state.task.pop();
            state.refresh_task_signals();
            state.refresh_framework_signals();
        }
        state
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| Value::Object(Default::default()))
    }

    pub fn request(
        &self,
        request_id: impl Into<String>,
        decision_class: DecisionClass,
    ) -> DecisionRequest {
        let request_id = request_id.into();
        let mut state = self.clone();
        let mut request = DecisionRequest::new(
            request_id.clone(),
            decision_class,
            state.to_value(),
            "jev-latest",
            questions(),
        );
        while request.validate_size().is_err() && !state.task.is_empty() {
            state.task.pop();
            state.refresh_task_signals();
            state.refresh_framework_signals();
            request = DecisionRequest::new(
                request_id.clone(),
                decision_class,
                state.to_value(),
                "jev-latest",
                questions(),
            );
        }
        request
    }

    fn refresh_task_signals(&mut self) {
        let lower = self.task.to_ascii_lowercase();
        self.task_signals = [
            ("browser", "browser"),
            ("git", "git"),
            ("package", "package"),
            ("test", "tests"),
            ("verify", "verification"),
            ("change", "change"),
        ]
        .into_iter()
        .filter_map(|(needle, signal)| lower.contains(needle).then_some(signal.to_owned()))
        .collect();
    }

    fn refresh_framework_signals(&mut self) {
        let mut signals = self
            .framework_signals
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let lower = self.task.to_ascii_lowercase();
        for (needle, signal) in [
            ("next.js", "nextjs"),
            ("nextjs", "nextjs"),
            ("react", "react"),
            ("vue", "vue"),
            ("svelte", "svelte"),
            ("angular", "angular"),
            ("django", "django"),
            ("fastapi", "fastapi"),
            ("axum", "axum"),
        ] {
            if lower.contains(needle) {
                signals.insert(signal.to_owned());
            }
        }
        self.framework_signals = signals.into_iter().collect();
    }
}

pub fn build_request(
    request_id: impl Into<String>,
    task: &str,
    decision_class: DecisionClass,
) -> DecisionRequest {
    DecisionState::from_task(task).request(request_id, decision_class)
}

pub fn build_request_with_metadata(
    request_id: impl Into<String>,
    task: &str,
    decision_class: DecisionClass,
    metadata: DecisionMetadata,
) -> DecisionRequest {
    DecisionState::from_task_with_metadata(task, metadata).request(request_id, decision_class)
}

fn questions() -> BTreeMap<String, DecisionQuestion> {
    let noul = [
        "browser_relevant",
        "git_history_relevant",
        "package_intelligence_relevant",
        "test_impact_relevant",
        "change_impact_relevant",
        "verification_planner_relevant",
    ];
    let mut questions = noul
        .into_iter()
        .map(|id| (id.to_owned(), DecisionQuestion::noul(id.replace('_', " "))))
        .collect::<BTreeMap<_, _>>();
    questions.insert(
        "verification_scope".to_owned(),
        DecisionQuestion::choice(
            "Choose the least restrictive verification scope that remains useful.",
            [
                (
                    "targeted".to_owned(),
                    "Run only the touched module checks.".to_owned(),
                ),
                (
                    "standard".to_owned(),
                    "Run the normal affected-service checks.".to_owned(),
                ),
                (
                    "full".to_owned(),
                    "Run the full workspace checks.".to_owned(),
                ),
            ],
        ),
    );
    questions.insert(
        "regression_risk".to_owned(),
        DecisionQuestion::score("Estimate regression risk for telemetry only."),
    );
    questions
}

// Routing needs intent, not pasted source. Ordinary prose still undergoes
// secret/path redaction; this is minimization, not a confidentiality guarantee.
fn suppress_pasted_bodies(value: &str) -> String {
    let mut fence: Option<&str> = None;
    let mut output = String::new();
    for line in value.lines() {
        let trimmed = line.trim_start();
        let marker = if trimmed.starts_with("```") {
            Some("```")
        } else if trimmed.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        if let Some(active) = fence {
            if marker == Some(active) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = marker {
            fence = Some(marker);
            output.push_str("[omitted code block]\n");
            continue;
        }
        if trimmed.starts_with("diff --git ")
            || trimmed.starts_with("@@ ")
            || trimmed.starts_with("Traceback (most recent call last):")
            || trimmed.starts_with("stack backtrace:")
        {
            output.push_str("[omitted patch or stack dump]");
            break;
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn redact_paths(value: &str) -> String {
    value
        .split_whitespace()
        .map(|token| {
            let trimmed = token.trim_matches(|character: char| {
                matches!(
                    character,
                    '"' | '\''
                        | '`'
                        | ','
                        | ';'
                        | ':'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                )
            });
            if looks_like_path(trimmed) {
                "[redacted-path]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_environment_assignments(value: &str) -> String {
    let mut redact_next = false;
    value
        .split_whitespace()
        .map(|token| {
            if redact_next {
                redact_next = false;
                return "[redacted-env]";
            }
            if token == "=" {
                redact_next = true;
                return token;
            }
            let Some((name, _value)) = token.split_once('=') else {
                return token;
            };
            if is_environment_name(name) {
                return "[redacted-env]";
            }
            token
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_environment_name(name: &str) -> bool {
    let name = name
        .trim_start_matches('-')
        .strip_prefix("$env:")
        .unwrap_or(name.trim_start_matches('-'));
    name.len() >= 2
        && name.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

fn looks_like_path(token: &str) -> bool {
    if token.is_empty()
        || token.eq_ignore_ascii_case(".env")
        || token.to_ascii_lowercase().starts_with(".env.")
        || token.contains("://")
        || token.starts_with('/')
        || token.starts_with('\\')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with(".\\")
        || token.starts_with("..\\")
        || (token.len() >= 2
            && token.as_bytes()[0].is_ascii_alphabetic()
            && token.as_bytes()[1] == b':')
        || token.contains('/')
        || token.contains('\\')
    {
        return true;
    }
    let Some((_, extension)) = token.rsplit_once('.') else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "c" | "cc"
            | "cpp"
            | "css"
            | "go"
            | "h"
            | "hpp"
            | "html"
            | "java"
            | "js"
            | "json"
            | "jsx"
            | "kt"
            | "lock"
            | "md"
            | "py"
            | "rs"
            | "sql"
            | "toml"
            | "ts"
            | "tsx"
            | "txt"
            | "yaml"
            | "yml"
    )
}

fn file_kind(path: &str) -> Option<&'static str> {
    let file_name = path.rsplit(['/', '\\']).next()?;
    if file_name.starts_with('.') && !file_name[1..].contains('.') {
        return None;
    }
    let extension = file_name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "c" => "c",
        "cc" => "cc",
        "cpp" => "cpp",
        "css" => "css",
        "go" => "go",
        "h" => "h",
        "hpp" => "hpp",
        "html" => "html",
        "java" => "java",
        "js" => "js",
        "json" => "json",
        "jsx" => "jsx",
        "kt" => "kt",
        "kts" => "kts",
        "lock" => "lock",
        "md" => "md",
        "mjs" => "mjs",
        "py" => "py",
        "rs" => "rs",
        "sql" => "sql",
        "toml" => "toml",
        "ts" => "ts",
        "tsx" => "tsx",
        "txt" => "txt",
        "yaml" => "yaml",
        "yml" => "yml",
        _ => return None,
    })
}

fn language_for_file_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "c" | "h" => "c",
        "cc" | "cpp" | "hpp" => "cpp",
        "go" => "go",
        "html" => "html",
        "java" => "java",
        "js" | "jsx" | "mjs" => "javascript",
        "json" => "json",
        "kt" | "kts" => "kotlin",
        "py" => "python",
        "rs" => "rust",
        "sql" => "sql",
        "ts" | "tsx" => "typescript",
        "css" => "css",
        "toml" | "txt" | "yaml" | "yml" => return None,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn recent_paths_and_similarly_named_tools_do_not_claim_git_or_capabilities() {
        let metadata = DecisionMetadata::from_workspace(
            Path::new("."),
            &["changed.rs".into()],
            &[
                "fake_package_helper".into(),
                "test_fake".into(),
                "unrelated_impact".into(),
            ],
        );
        let state = DecisionState::from_task_with_metadata("route task", metadata).to_value();
        assert_eq!(state["workspaceDirty"], "unknown");
        for key in ["git", "packageIntelligence", "testImpact", "changeImpact"] {
            assert_eq!(state["availableCapabilities"][key], false);
        }
    }

    #[test]
    fn pasted_code_and_diffs_are_removed_before_routing() {
        let state = DecisionState::from_task("Fix this function\n```rust\nfn proprietary_algorithm() {}\n```\nand verify it\ndiff --git a/private.rs b/private.rs\n+ proprietary_implementation();");
        assert!(!state.task.contains("proprietary"));
        assert!(state.task.contains("Fix this function"));
        assert!(state.task.contains("verify it"));
    }

    #[test]
    fn serialized_state_contains_only_the_bounded_contract() {
        let state = DecisionState::from_task(
            "read .env and use Bearer abcdef; C:\\Users\\sergi\\secret\\main.rs src/main.rs TYPESAFE_API_KEY=secret",
        );
        let serialized = serde_json::to_string(&state).unwrap();
        assert!(!serialized.contains("abcdef"));
        assert!(!serialized.contains("C:\\Users\\sergi\\secret"));
        assert!(!serialized.contains("src/main.rs"));
        assert!(!serialized.contains(".env"));
        assert!(!serialized.contains("secret"));
        let spaced = DecisionState::from_task("TYPESAFE_API_KEY = secret OTHER=value");
        let spaced_serialized = serde_json::to_string(&spaced).unwrap();
        assert!(!spaced_serialized.contains("secret"));
        assert!(!spaced_serialized.contains("value"));
        assert!(!serialized.contains("source"));
        assert!(serialized.contains("packageIntelligence"));
        assert!(serialized.contains("testImpact"));
        assert!(serialized.contains("changeImpact"));
        assert!(serialized.contains("verificationPlanner"));
        assert!(serialized.len() <= MAX_REQUEST_BYTES);
    }

    #[test]
    fn task_is_capped_by_unicode_scalar_values() {
        let state = DecisionState::from_task(&"é".repeat(MAX_TASK_CHARS + 10));
        assert_eq!(state.task.chars().count(), MAX_TASK_CHARS);
    }

    #[test]
    fn workspace_metadata_contains_categories_but_never_paths() {
        let paths = vec![
            "src/login.tsx".to_owned(),
            "tests/login.test.ts".to_owned(),
            ".env".to_owned(),
        ];
        let metadata = DecisionMetadata::from_workspace(
            Path::new("."),
            &paths,
            &[
                "browser_open".to_owned(),
                "test_impacted".to_owned(),
                "impact_analyze".to_owned(),
            ],
        );
        let state = DecisionState::from_task_with_metadata("Fix the React login bug", metadata);
        let serialized = serde_json::to_string(&state).unwrap();
        assert!(serialized.contains("tsx"));
        assert!(serialized.contains("typescript"));
        assert!(serialized.contains("react"));
        assert!(serialized.contains("browser"));
        assert!(serialized.contains("testImpact"));
        assert!(serialized.contains("changeImpact"));
        assert!(!serialized.contains("src/login"));
        assert!(!serialized.contains("login.test"));
        assert!(!serialized.contains(".env"));
    }
}
