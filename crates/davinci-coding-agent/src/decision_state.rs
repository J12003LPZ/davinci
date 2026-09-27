use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use davinci_agent::decision::request::{
    DecisionQuestion, DecisionRequest, MAX_REQUEST_BYTES, MAX_TASK_CHARS,
};
use davinci_agent::decision::risk::DecisionClass;
use davinci_agent::runtime::contracts::redact_secrets;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MAX_REQUIREMENTS: usize = 8;
pub const MAX_REQUIREMENT_CHARS: usize = 240;

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

/// A bounded, deterministic projection of explicit user requirements. The
/// wording has already passed the task redaction pipeline before it reaches
/// this type; it is evidence for routing, never completion authority.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequirementLedger {
    pub entries: Vec<RequirementRecord>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequirementRecord {
    pub id: String,
    pub wording: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementEvidenceKind {
    Check,
    Observation,
    Diff,
    Comment,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementEvidence {
    pub requirement_id: String,
    pub kind: RequirementEvidenceKind,
    pub current: bool,
    pub passed: bool,
    pub references: Vec<String>,
}

/// Completion remains deterministic: a complete ledger needs current,
/// concrete evidence for every requirement. A provider judgment can add a
/// reminder, but cannot turn stale, failed, missing, unavailable, or comment
/// evidence into completion.
pub fn deterministic_requirements_satisfied(
    ledger: &RequirementLedger,
    evidence: &[RequirementEvidence],
) -> bool {
    if !ledger.is_complete() {
        return false;
    }
    ledger.entries.iter().all(|requirement| {
        evidence.iter().any(|item| {
            item.requirement_id == requirement.id
                && item.current
                && item.passed
                && !item.references.is_empty()
                && matches!(
                    item.kind,
                    RequirementEvidenceKind::Check
                        | RequirementEvidenceKind::Observation
                        | RequirementEvidenceKind::Diff
                )
        })
    })
}

impl RequirementLedger {
    pub fn from_task(task: &str) -> Self {
        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();
        let mut truncated = false;
        for line in task.lines() {
            for fragment in line.split(['.', '?', '!', ';']) {
                let (fragment, is_bullet) = strip_requirement_marker(fragment);
                let wording = fragment.split_whitespace().collect::<Vec<_>>().join(" ");
                if wording.is_empty() || (!is_bullet && !looks_like_requirement(&wording)) {
                    continue;
                }
                let normalized = wording.to_ascii_lowercase();
                if !seen.insert(normalized) {
                    continue;
                }
                if entries.len() >= MAX_REQUIREMENTS {
                    truncated = true;
                    continue;
                }
                let bounded = truncate_chars(&wording, MAX_REQUIREMENT_CHARS);
                if bounded.chars().count() < wording.chars().count() {
                    truncated = true;
                }
                entries.push(RequirementRecord {
                    id: requirement_id(&wording),
                    wording: bounded,
                });
            }
        }
        Self {
            entries,
            truncated,
        }
    }

    pub fn is_complete(&self) -> bool {
        !self.truncated && !self.entries.is_empty()
    }
}

fn requirement_id(wording: &str) -> String {
    let digest = Sha256::digest(wording.as_bytes());
    let short = digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("req-{short}")
}

fn strip_requirement_marker(value: &str) -> (&str, bool) {
    let trimmed = value.trim();
    let without_bullet = trimmed
        .strip_prefix('-')
        .or_else(|| trimmed.strip_prefix('*'))
        .or_else(|| trimmed.strip_prefix('•'))
        .map(str::trim_start);
    if let Some(value) = without_bullet {
        return (value, true);
    }
    let bytes = trimmed.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index > 0 && trimmed[index..].starts_with('.') {
        return (trimmed[index + 1..].trim_start(), true);
    }
    (trimmed, false)
}

fn looks_like_requirement(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "must ",
        "should ",
        "need to ",
        "required",
        "requirement",
        "ensure ",
        "verify ",
        "test ",
        "preserve ",
        "keep ",
        "support ",
        "without ",
        "do not ",
        "don't ",
        "make sure ",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
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
    #[serde(default)]
    pub requirements: RequirementLedger,
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
        let requirements = RequirementLedger::from_task(&task);
        let mut state = Self {
            schema_version: 2,
            task,
            task_signals: Vec::new(),
            languages: metadata.languages,
            framework_signals: metadata.framework_signals,
            recent_file_kinds: metadata.recent_file_kinds,
            workspace_dirty: metadata.workspace_dirty,
            available_capabilities: metadata.available_capabilities,
            requirements,
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
            state.requirements = RequirementLedger::from_task(&state.task);
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
            questions(&state.requirements),
        );
        // Only a size failure is fixed by shortening the task. A malformed
        // question must surface as-is, not strip the task to nothing.
        while request.validate_questions().is_ok()
            && request.validate_size().is_err()
            && !state.task.is_empty()
        {
            state.task.pop();
            state.refresh_task_signals();
            state.refresh_framework_signals();
            state.requirements = RequirementLedger::from_task(&state.task);
            request = DecisionRequest::new(
                request_id.clone(),
                decision_class,
                state.to_value(),
                "jev-latest",
                questions(&state.requirements),
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

/// Question ids never reach the model, so each question carries its full
/// meaning and names the state fields it should read.
fn questions(requirements: &RequirementLedger) -> BTreeMap<String, DecisionQuestion> {
    let capability = |what: &str, field: &str, yes: &str| {
        DecisionQuestion::noul_with(
            format!(
                "A coding agent is about to work on `task`. Would {what} materially help \
                 complete `task` correctly? Consider `task`, `taskSignals`, `languages`, \
                 `frameworkSignals` and `recentFileKinds`. If `{field}` is false the tool \
                 is unavailable, which does not change whether it would help."
            ),
            yes,
            "The task can be done well without it, or it would add only noise.",
        )
    };
    let mut questions = BTreeMap::from([
        (
            "browser_relevant".to_owned(),
            capability(
                "opening the running app in a real browser to click through and inspect the page",
                "availableCapabilities.browser",
                "The task changes or debugs something a user sees or does in a web page.",
            ),
        ),
        (
            "git_history_relevant".to_owned(),
            capability(
                "reading git history (who changed a symbol, related commits, branch diffs)",
                "availableCapabilities.git",
                "The task depends on why or when code changed, a regression window, or branch differences.",
            ),
        ),
        (
            "package_intelligence_relevant".to_owned(),
            capability(
                "looking up a third-party package's version, exports or dependents",
                "availableCapabilities.packageIntelligence",
                "The task involves adding, upgrading, or correctly using an external dependency.",
            ),
        ),
        (
            "test_impact_relevant".to_owned(),
            capability(
                "finding which existing tests cover the code being changed",
                "availableCapabilities.testImpact",
                "The task changes behavior that existing tests should confirm or that could break them.",
            ),
        ),
        (
            "change_impact_relevant".to_owned(),
            capability(
                "tracing which callers and modules depend on the code being changed",
                "availableCapabilities.changeImpact",
                "The task edits a function, type or interface that other code relies on.",
            ),
        ),
        (
            "verification_planner_relevant".to_owned(),
            capability(
                "planning which build, lint and test commands prove the change works",
                "availableCapabilities.verificationPlanner",
                "The task produces a code change whose correctness needs more than one kind of check.",
            ),
        ),
    ]);
    questions.insert(
        "verification_scope".to_owned(),
        DecisionQuestion::choice(
            "A coding agent will finish `task` by running checks. Choose the smallest \
             verification scope that would still catch the mistakes this change could \
             realistically cause.",
            [
                (
                    "targeted".to_owned(),
                    "Only the checks for the module being touched: a local, self-contained change."
                        .to_owned(),
                ),
                (
                    "standard".to_owned(),
                    "The test suite of the affected service or crate: behavior changes other code in it may see."
                        .to_owned(),
                ),
                (
                    "full".to_owned(),
                    "Every check in the workspace: a shared contract, build config, or cross-service change."
                        .to_owned(),
                ),
            ],
        ),
    );
    questions.insert(
        "regression_risk".to_owned(),
        DecisionQuestion::score(
            "How likely is completing `task` to break behavior that currently works \
             somewhere other than the intended change?",
            [
                "Almost none: text, comments, docs, or an isolated new file nothing uses yet.",
                "Low: a small local edit inside one function with obvious effects.",
                "Moderate: changes behavior of a module other code calls.",
                "High: changes a shared interface, data format, concurrency, or security-sensitive path.",
            ],
        ),
    );
    for requirement in &requirements.entries {
        questions.insert(
            requirement_question_id(&requirement.id),
            DecisionQuestion::choice(
                format!(
                    "Using only current public evidence, classify whether this explicit user requirement is supported: `{}`. Do not infer a passing check from a comment, stale result, or an unavailable observation.",
                    requirement.wording
                ),
                [
                    (
                        "supported".to_owned(),
                        "A current, concrete check or observable directly supports the requirement.".to_owned(),
                    ),
                    (
                        "possibly_missing".to_owned(),
                        "The requirement may be unmet or the current evidence is insufficient.".to_owned(),
                    ),
                    (
                        "uncertain".to_owned(),
                        "The evidence is stale, unavailable, contradictory, or cannot identify the requirement.".to_owned(),
                    ),
                ],
            ),
        );
    }
    questions
}

pub fn requirement_question_id(requirement_id: &str) -> String {
    format!("requirement_{requirement_id}")
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
    fn routing_questions_match_the_api_shape_and_carry_their_own_meaning() {
        let request = build_request("shape", "Fix the React login bug", DecisionClass::Ranking);
        request
            .validate_size()
            .expect("routing questions pass the API shape check");
        for (id, question) in &request.questions {
            // Ids are not sent to the model; the text must stand alone.
            assert!(
                question.instructions.len() > 60 && question.instructions.contains("`task`"),
                "{id} instructions do not explain the judgment: {:?}",
                question.instructions
            );
            assert!(question.criteria.is_some(), "{id} has no criteria");
        }
        let wire = serde_json::to_value(&request.questions).unwrap();
        assert_eq!(
            wire["regression_risk"]["criteria"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert!(wire["browser_relevant"]["criteria"]["true"].is_string());
        assert!(wire["browser_relevant"]["criteria"]["false"].is_string());
    }

    #[test]
    fn a_maximal_task_still_fits_the_request_bound_with_full_questions() {
        let request = build_request("big", &"word ".repeat(5000), DecisionClass::Ranking);
        let encoded = request.validate_size().expect("bounded request");
        assert!(encoded.len() <= MAX_REQUEST_BYTES);
        assert!(!request.state["task"].as_str().unwrap().is_empty());
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

    #[test]
    fn requirement_ledger_keeps_redacted_explicit_items_with_stable_ids() {
        let task = "Must preserve the public API. Verify empty input.\n- Keep the caller input unchanged.";
        let ledger = RequirementLedger::from_task(task);
        assert_eq!(ledger.entries.len(), 3);
        assert!(ledger.is_complete());
        assert!(ledger.entries[0].id.starts_with("req-"));
        assert_eq!(
            ledger.entries[0].id,
            RequirementLedger::from_task(task).entries[0].id
        );
        assert_eq!(ledger.entries[2].wording, "Keep the caller input unchanged");
    }

    #[test]
    fn requirement_ledger_marks_overflow_instead_of_claiming_completeness() {
        let task = (0..(MAX_REQUIREMENTS + 2))
            .map(|index| format!("- Verify case {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let ledger = RequirementLedger::from_task(&task);
        assert_eq!(ledger.entries.len(), MAX_REQUIREMENTS);
        assert!(ledger.truncated);
        assert!(!ledger.is_complete());
    }

    #[test]
    fn requirement_ledger_is_inside_the_redacted_state_contract() {
        let state = DecisionState::from_task(
            "Must read C:\\Users\\sergi\\private.rs and use TOKEN=secret; verify it",
        );
        let serialized = serde_json::to_string(&state).unwrap();
        assert!(serialized.contains("requirements"));
        assert!(!serialized.contains("C:\\Users\\sergi\\private.rs"));
        assert!(!serialized.contains("secret"));
    }

    #[test]
    fn requirement_completion_cases_fail_closed_for_missing_or_bad_evidence() {
        let ledger = RequirementLedger::from_task("Must preserve the API");
        let id = ledger.entries[0].id.clone();
        let good = RequirementEvidence {
            requirement_id: id.clone(),
            kind: RequirementEvidenceKind::Check,
            current: true,
            passed: true,
            references: vec!["test:api-contract".into()],
        };
        assert!(deterministic_requirements_satisfied(
            &ledger,
            std::slice::from_ref(&good)
        ));

        for evidence in [
            vec![],
            vec![RequirementEvidence {
                requirement_id: id.clone(),
                kind: RequirementEvidenceKind::Check,
                current: false,
                passed: true,
                references: vec!["test:old".into()],
            }],
            vec![RequirementEvidence {
                requirement_id: id.clone(),
                kind: RequirementEvidenceKind::Check,
                current: true,
                passed: false,
                references: vec!["test:failed".into()],
            }],
            vec![RequirementEvidence {
                requirement_id: id.clone(),
                kind: RequirementEvidenceKind::Comment,
                current: true,
                passed: true,
                references: vec!["comment:looks-good".into()],
            }],
            vec![RequirementEvidence {
                requirement_id: id.clone(),
                kind: RequirementEvidenceKind::Unavailable,
                current: false,
                passed: false,
                references: vec![],
            }],
        ] {
            assert!(!deterministic_requirements_satisfied(&ledger, &evidence));
        }

        let mut truncated = ledger.clone();
        truncated.truncated = true;
        assert!(!deterministic_requirements_satisfied(&truncated, &[good]));
    }

    #[test]
    fn labeled_requirement_cases_keep_completion_authority_deterministic() {
        let complete = RequirementLedger::from_task("Must preserve the API");
        let complete_id = complete.entries[0].id.clone();
        let evidence = |
            kind: RequirementEvidenceKind,
            current: bool,
            passed: bool,
            reference: &[&str],
        | RequirementEvidence {
            requirement_id: complete_id.clone(),
            kind,
            current,
            passed,
            references: reference.iter().map(|value| (*value).to_owned()).collect(),
        };
        let cases = [
            (
                "complete-change",
                complete.clone(),
                vec![evidence(RequirementEvidenceKind::Check, true, true, &["test:api"])],
                true,
            ),
            (
                "omitted-requirement",
                RequirementLedger::from_task("Must preserve the API\n- Verify empty input"),
                vec![evidence(RequirementEvidenceKind::Check, true, true, &["test:api"])],
                false,
            ),
            (
                "incorrect-boundary-after-smoke-pass",
                complete.clone(),
                vec![evidence(
                    RequirementEvidenceKind::Check,
                    true,
                    false,
                    &["test:smoke-only"],
                )],
                false,
            ),
            (
                "stale-diff",
                complete.clone(),
                vec![evidence(
                    RequirementEvidenceKind::Diff,
                    false,
                    true,
                    &["diff:old"],
                )],
                false,
            ),
            (
                "failed-command",
                complete.clone(),
                vec![evidence(
                    RequirementEvidenceKind::Check,
                    true,
                    false,
                    &["cmd:exit-1"],
                )],
                false,
            ),
            (
                "truncated-requirement-set",
                RequirementLedger {
                    entries: complete.entries.clone(),
                    truncated: true,
                },
                vec![evidence(RequirementEvidenceKind::Check, true, true, &["test:api"])],
                false,
            ),
            (
                "misleading-diff-comment",
                complete.clone(),
                vec![evidence(
                    RequirementEvidenceKind::Comment,
                    true,
                    true,
                    &["comment:looks-good"],
                )],
                false,
            ),
            (
                "unavailable-evidence",
                complete,
                vec![evidence(
                    RequirementEvidenceKind::Unavailable,
                    false,
                    false,
                    &[],
                )],
                false,
            ),
        ];
        for (label, ledger, evidence, expected) in cases {
            assert_eq!(
                deterministic_requirements_satisfied(&ledger, &evidence),
                expected,
                "labeled requirement case {label}"
            );
        }
    }
}
