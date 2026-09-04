//! Hand-rolled artifact and config validators.
//!
//! These run both in the controller (parent process) and inside worker
//! processes. Error strings are shown to worker models verbatim, so each one
//! names the offending field and the expected shape — the worker uses them to
//! correct its own `graph_submit` call without a respawn.

use super::types::{
    Artifact, ArtifactContract, ArtifactKind, Complexity, Confidence, FieldKind, FieldRule,
    ResearchKind, Role, Severity, TaskClass, Verdict,
};
use serde_json::Value;

pub type ValidationResult = Result<Artifact, Vec<String>>;

static RESEARCH_TASK_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "kind",
        required: true,
        allow_null: false,
        kind: FieldKind::Enum(&["code_search", "test_baseline", "history", "docs"]),
        description: None,
    },
    FieldRule {
        name: "focus",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 1 },
        description: Some("A concrete question, not a topic"),
    },
];

static CLASSIFICATION_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "taskClass",
        required: true,
        allow_null: false,
        kind: FieldKind::Enum(&["trivial", "bug", "feature", "refactor", "investigation"]),
        description: None,
    },
    FieldRule {
        name: "complexity",
        required: true,
        allow_null: false,
        kind: FieldKind::Enum(&["trivial", "standard", "complex"]),
        description: None,
    },
    FieldRule {
        name: "rationale",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 1 },
        description: None,
    },
    FieldRule {
        name: "researchTasks",
        required: true,
        allow_null: false,
        kind: FieldKind::ObjectArray(RESEARCH_TASK_FIELDS, 0),
        description: None,
    },
    FieldRule {
        name: "milestones",
        required: false,
        allow_null: true,
        kind: FieldKind::StringArray { min_items: 0 },
        description: Some("Empty for one deliverable; otherwise 2 to 8 ordered, independently shippable deliverables"),
    },
];

static FINDING_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "claim",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 1 },
        description: None,
    },
    FieldRule {
        name: "refs",
        required: true,
        allow_null: false,
        kind: FieldKind::StringArray { min_items: 0 },
        description: Some("path:line references that support the claim"),
    },
    FieldRule {
        name: "confidence",
        required: true,
        allow_null: false,
        kind: FieldKind::Enum(&["high", "medium", "low"]),
        description: None,
    },
];

static TEST_BASELINE_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "command",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 0 },
        description: None,
    },
    FieldRule {
        name: "exitCode",
        required: true,
        allow_null: false,
        kind: FieldKind::Integer,
        description: None,
    },
    FieldRule {
        name: "summary",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 0 },
        description: None,
    },
];

static EVIDENCE_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "kind",
        required: true,
        allow_null: false,
        kind: FieldKind::Enum(&["code_search", "test_baseline", "history", "docs"]),
        description: None,
    },
    FieldRule {
        name: "findings",
        required: true,
        allow_null: false,
        kind: FieldKind::ObjectArray(FINDING_FIELDS, 0),
        description: None,
    },
    FieldRule {
        name: "risks",
        required: true,
        allow_null: false,
        kind: FieldKind::StringArray { min_items: 0 },
        description: None,
    },
    FieldRule {
        name: "gaps",
        required: true,
        allow_null: false,
        kind: FieldKind::StringArray { min_items: 0 },
        description: None,
    },
    FieldRule {
        name: "testBaseline",
        required: false,
        allow_null: true,
        kind: FieldKind::Object(TEST_BASELINE_FIELDS),
        description: None,
    },
];

static PLAN_STEP_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "description",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 1 },
        description: None,
    },
    FieldRule {
        name: "files",
        required: true,
        allow_null: false,
        kind: FieldKind::StringArray { min_items: 0 },
        description: None,
    },
];

static TEST_TO_ADD_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "file",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 0 },
        description: None,
    },
    FieldRule {
        name: "behavior",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 0 },
        description: None,
    },
];

static PLAN_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "steps",
        required: true,
        allow_null: false,
        kind: FieldKind::ObjectArray(PLAN_STEP_FIELDS, 1),
        description: None,
    },
    FieldRule {
        name: "testsToAdd",
        required: true,
        allow_null: false,
        kind: FieldKind::ObjectArray(TEST_TO_ADD_FIELDS, 0),
        description: None,
    },
    FieldRule {
        name: "testsToRun",
        required: true,
        allow_null: false,
        kind: FieldKind::StringArray { min_items: 0 },
        description: Some("Exact shell commands runnable from the project root"),
    },
    FieldRule {
        name: "completionCriteria",
        required: true,
        allow_null: false,
        kind: FieldKind::StringArray { min_items: 1 },
        description: None,
    },
    FieldRule {
        name: "invariants",
        required: true,
        allow_null: false,
        kind: FieldKind::StringArray { min_items: 0 },
        description: None,
    },
    FieldRule {
        name: "outOfScope",
        required: true,
        allow_null: false,
        kind: FieldKind::StringArray { min_items: 0 },
        description: None,
    },
];

static PATCH_REPORT_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "changedFiles",
        required: true,
        allow_null: false,
        kind: FieldKind::StringArray { min_items: 0 },
        description: None,
    },
    FieldRule {
        name: "summary",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 1 },
        description: None,
    },
    FieldRule {
        name: "deviations",
        required: true,
        allow_null: false,
        kind: FieldKind::StringArray { min_items: 0 },
        description: None,
    },
    FieldRule {
        name: "planInvalidated",
        required: true,
        allow_null: false,
        kind: FieldKind::Boolean,
        description: None,
    },
    FieldRule {
        name: "invalidationReason",
        required: false,
        allow_null: true,
        kind: FieldKind::String { min_length: 1 },
        description: Some("Required when planInvalidated is true"),
    },
];

static REVIEW_ISSUE_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "severity",
        required: true,
        allow_null: false,
        kind: FieldKind::Enum(&["blocker", "major", "minor"]),
        description: None,
    },
    FieldRule {
        name: "file",
        required: false,
        allow_null: true,
        kind: FieldKind::String { min_length: 0 },
        description: None,
    },
    FieldRule {
        name: "description",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 1 },
        description: None,
    },
];

static REVIEW_FIELDS: &[FieldRule] = &[
    FieldRule {
        name: "verdict",
        required: true,
        allow_null: false,
        kind: FieldKind::Enum(&["approve", "changes_required"]),
        description: None,
    },
    FieldRule {
        name: "issues",
        required: true,
        allow_null: false,
        kind: FieldKind::ObjectArray(REVIEW_ISSUE_FIELDS, 0),
        description: None,
    },
    FieldRule {
        name: "notes",
        required: true,
        allow_null: false,
        kind: FieldKind::String { min_length: 0 },
        description: None,
    },
];

const CLASSIFICATION_EXAMPLE: &str = r#"{
  "taskClass": "bug",
  "complexity": "standard",
  "rationale": "One subsystem, root cause unknown until the call site is read.",
  "researchTasks": [
    {
      "kind": "code_search",
      "focus": "Where is the session file written, and what sets its permissions?"
    }
  ],
  "milestones": []
}"#;

const EVIDENCE_EXAMPLE: &str = r#"{
  "kind": "code_search",
  "findings": [
    {
      "claim": "Session files are opened with mode 0644.",
      "refs": [
        "src/store.rs:118"
      ],
      "confidence": "high"
    }
  ],
  "risks": [
    "The umask may already restrict this on some hosts."
  ],
  "gaps": [],
  "testBaseline": null
}"#;

const PLAN_EXAMPLE: &str = r#"{
  "steps": [
    {
      "description": "Open session files with mode 0600.",
      "files": [
        "src/store.rs"
      ]
    }
  ],
  "testsToAdd": [
    {
      "file": "src/store.rs",
      "behavior": "a new session file is not group- or world-readable"
    }
  ],
  "testsToRun": [
    "cargo test -p pi-session"
  ],
  "completionCriteria": [
    "cargo test -p pi-session passes",
    "new session files are mode 0600"
  ],
  "invariants": [
    "existing session files are not rewritten"
  ],
  "outOfScope": [
    "Windows ACLs"
  ]
}"#;

const PATCH_REPORT_EXAMPLE: &str = r#"{
  "changedFiles": [
    "src/store.rs"
  ],
  "summary": "Session files are now created with mode 0600; test added.",
  "deviations": [],
  "planInvalidated": false
}"#;

const REVIEW_EXAMPLE: &str = r#"{
  "verdict": "changes_required",
  "issues": [
    {
      "severity": "major",
      "file": "src/store.rs",
      "description": "The mode is set after the file is created, leaving a window where it is world-readable."
    }
  ],
  "notes": "Otherwise matches the plan."
}"#;

static CLASSIFICATION_PROMPT_RULES: &[&str] = &[
    "taskClass: one of trivial|bug|feature|refactor|investigation",
    "complexity: one of trivial|standard|complex",
    "rationale: non-empty string",
    "researchTasks: array (may be empty) of { kind: one of code_search|test_baseline|history|docs, focus: non-empty string }",
    "milestones: array of strings — empty for a single deliverable, otherwise 2 to 8 entries",
];

static EVIDENCE_PROMPT_RULES: &[&str] = &[
    "kind: one of code_search|test_baseline|history|docs",
    "findings: array of { claim: non-empty string, refs: non-empty array of \"path:line\" strings, confidence: one of high|medium|low }",
    "risks: array of strings (may be empty)",
    "gaps: array of strings (may be empty)",
    "testBaseline: optional { command: string, exitCode: number, summary: string }",
];

static PLAN_PROMPT_RULES: &[&str] = &[
    "steps: non-empty array of { description: non-empty string, files: array of paths }",
    "testsToAdd: array (may be empty) of { file: string, behavior: string }",
    "testsToRun: array of exact shell commands runnable from the project root",
    "completionCriteria: non-empty array of measurable statements",
    "invariants: array of strings (may be empty)",
    "outOfScope: array of strings (may be empty)",
];

static PATCH_REPORT_PROMPT_RULES: &[&str] = &[
    "changedFiles: array of file paths (may be empty)",
    "summary: non-empty string",
    "deviations: array of strings (may be empty)",
    "planInvalidated: boolean",
    "invalidationReason: non-empty string, required when planInvalidated is true",
];

static REVIEW_PROMPT_RULES: &[&str] = &[
    "verdict: one of approve|changes_required",
    "issues: array of { severity: one of blocker|major|minor, file?: string, description: non-empty string } — at least one when verdict is changes_required",
    "notes: string (may be empty)",
];

pub fn artifact_contract(kind: ArtifactKind) -> ArtifactContract {
    match kind {
        ArtifactKind::Classification => ArtifactContract {
            kind,
            fields: CLASSIFICATION_FIELDS,
            example: CLASSIFICATION_EXAMPLE,
            prompt_rules: CLASSIFICATION_PROMPT_RULES,
        },
        ArtifactKind::Evidence => ArtifactContract {
            kind,
            fields: EVIDENCE_FIELDS,
            example: EVIDENCE_EXAMPLE,
            prompt_rules: EVIDENCE_PROMPT_RULES,
        },
        ArtifactKind::Plan => ArtifactContract {
            kind,
            fields: PLAN_FIELDS,
            example: PLAN_EXAMPLE,
            prompt_rules: PLAN_PROMPT_RULES,
        },
        ArtifactKind::PatchReport => ArtifactContract {
            kind,
            fields: PATCH_REPORT_FIELDS,
            example: PATCH_REPORT_EXAMPLE,
            prompt_rules: PATCH_REPORT_PROMPT_RULES,
        },
        ArtifactKind::Review => ArtifactContract {
            kind,
            fields: REVIEW_FIELDS,
            example: REVIEW_EXAMPLE,
            prompt_rules: REVIEW_PROMPT_RULES,
        },
    }
}

/// The JSON schema of one artifact kind, generated directly from ArtifactContract.
pub fn artifact_schema(kind: ArtifactKind) -> Value {
    artifact_contract(kind).to_json_schema()
}

fn is_non_empty_string(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::String(text)) if !text.is_empty())
}

fn is_string_array(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Array(items)) if items.iter().all(Value::is_string))
}

/// True when `value` is a string that names a member of the enum.
fn is_enum_member(value: Option<&Value>, parses: fn(&str) -> bool) -> bool {
    value.and_then(Value::as_str).map(parses).unwrap_or(false)
}

fn push_if(errors: &mut Vec<String>, condition: bool, message: impl Into<String>) {
    if condition {
        errors.push(message.into());
    }
}

fn validate_classification(value: &serde_json::Map<String, Value>, errors: &mut Vec<String>) {
    push_if(
        errors,
        !is_enum_member(value.get("taskClass"), |text| {
            TaskClass::parse(text).is_some()
        }),
        format!("taskClass must be one of {}", TaskClass::names()),
    );
    push_if(
        errors,
        !is_enum_member(value.get("complexity"), |text| {
            Complexity::parse(text).is_some()
        }),
        format!("complexity must be one of {}", Complexity::names()),
    );
    push_if(
        errors,
        !is_non_empty_string(value.get("rationale")),
        "rationale must be a non-empty string",
    );
    match value.get("researchTasks") {
        Some(Value::Array(tasks)) => {
            for (index, task) in tasks.iter().enumerate() {
                let Some(task) = task.as_object() else {
                    errors.push(format!("researchTasks[{index}] must be an object"));
                    continue;
                };
                push_if(
                    errors,
                    !is_enum_member(task.get("kind"), |text| ResearchKind::parse(text).is_some()),
                    format!(
                        "researchTasks[{index}].kind must be one of {}",
                        ResearchKind::names()
                    ),
                );
                push_if(
                    errors,
                    !is_non_empty_string(task.get("focus")),
                    format!("researchTasks[{index}].focus must be a non-empty string"),
                );
            }
        }
        _ => errors.push("researchTasks must be an array (may be empty)".into()),
    }
    match value.get("milestones") {
        None | Some(Value::Null) => {}
        Some(milestones) if is_string_array(Some(milestones)) => {
            let milestones = milestones.as_array().expect("checked above");
            for (index, milestone) in milestones.iter().enumerate() {
                push_if(
                    errors,
                    milestone.as_str().unwrap_or_default().trim().is_empty(),
                    format!("milestones[{index}] must be a non-empty string"),
                );
            }
            push_if(
                errors,
                milestones.len() == 1,
                "milestones must be empty or contain 2+ entries; a single deliverable needs no milestones",
            );
            push_if(
                errors,
                milestones.len() > 8,
                "milestones must contain at most 8 entries; merge related deliverables",
            );
        }
        _ => errors.push(
            "milestones must be an array of strings (may be empty; omit for a single deliverable)"
                .into(),
        ),
    }
}

fn validate_non_empty_refs(value: &serde_json::Value) -> Result<(), String> {
    let refs = value.as_array().ok_or_else(|| "refs must be an array".to_string())?;
    if refs.is_empty() || refs.iter().any(|v| v.as_str().map_or(true, |s| s.trim().is_empty())) {
        return Err("evidence finding requires at least one non-empty ref".into());
    }
    Ok(())
}

fn validate_evidence(value: &serde_json::Map<String, Value>, errors: &mut Vec<String>) {
    push_if(
        errors,
        !is_enum_member(value.get("kind"), |text| {
            ResearchKind::parse(text).is_some()
        }),
        format!("kind must be one of {}", ResearchKind::names()),
    );
    match value.get("findings") {
        Some(Value::Array(findings)) => {
            for (index, finding) in findings.iter().enumerate() {
                let Some(finding) = finding.as_object() else {
                    errors.push(format!("findings[{index}] must be an object"));
                    continue;
                };
                push_if(
                    errors,
                    !is_non_empty_string(finding.get("claim")),
                    format!("findings[{index}].claim must be a non-empty string"),
                );
                if let Some(refs) = finding.get("refs") {
                    if !is_string_array(Some(refs)) {
                        errors.push(format!(
                            "findings[{index}].refs must be an array of \"path:line\" strings (evidence without refs is not evidence)"
                        ));
                    } else if let Err(msg) = validate_non_empty_refs(refs) {
                        errors.push(format!("findings[{index}].refs: {msg}"));
                    }
                } else {
                    errors.push(format!(
                        "findings[{index}].refs must be an array of \"path:line\" strings (evidence without refs is not evidence)"
                    ));
                }
                push_if(
                    errors,
                    !is_enum_member(finding.get("confidence"), |text| {
                        Confidence::parse(text).is_some()
                    }),
                    format!(
                        "findings[{index}].confidence must be one of {}",
                        Confidence::names()
                    ),
                );
            }
        }
        _ => errors.push("findings must be an array".into()),
    }
    push_if(
        errors,
        !is_string_array(value.get("risks")),
        "risks must be an array of strings (may be empty)",
    );
    push_if(
        errors,
        !is_string_array(value.get("gaps")),
        "gaps must be an array of strings (may be empty)",
    );
    match value.get("testBaseline") {
        None | Some(Value::Null) => {}
        Some(Value::Object(baseline)) => {
            push_if(
                errors,
                !baseline.get("command").is_some_and(Value::is_string),
                "testBaseline.command must be a string",
            );
            push_if(
                errors,
                !baseline.get("exitCode").is_some_and(Value::is_number),
                "testBaseline.exitCode must be a number",
            );
            push_if(
                errors,
                !baseline.get("summary").is_some_and(Value::is_string),
                "testBaseline.summary must be a string",
            );
        }
        _ => errors.push("testBaseline must be an object when present".into()),
    }
}

fn validate_plan(value: &serde_json::Map<String, Value>, errors: &mut Vec<String>) {
    match value.get("steps") {
        Some(Value::Array(steps)) if !steps.is_empty() => {
            for (index, step) in steps.iter().enumerate() {
                let Some(step) = step.as_object() else {
                    errors.push(format!("steps[{index}] must be an object"));
                    continue;
                };
                push_if(
                    errors,
                    !is_non_empty_string(step.get("description")),
                    format!("steps[{index}].description must be a non-empty string"),
                );
                push_if(
                    errors,
                    !is_string_array(step.get("files")),
                    format!("steps[{index}].files must be an array of file paths"),
                );
            }
        }
        _ => errors.push("steps must be a non-empty array".into()),
    }
    match value.get("testsToAdd") {
        Some(Value::Array(tests)) => {
            for (index, test) in tests.iter().enumerate() {
                let valid = test.as_object().is_some_and(|test| {
                    test.get("file").is_some_and(Value::is_string)
                        && test.get("behavior").is_some_and(Value::is_string)
                });
                push_if(
                    errors,
                    !valid,
                    format!("testsToAdd[{index}] must be {{ file: string; behavior: string }}"),
                );
            }
        }
        _ => errors.push("testsToAdd must be an array (may be empty)".into()),
    }
    push_if(
        errors,
        !is_string_array(value.get("testsToRun")),
        "testsToRun must be an array of exact runnable shell commands",
    );
    push_if(
        errors,
        !matches!(value.get("completionCriteria"), Some(Value::Array(items)) if !items.is_empty() && items.iter().all(Value::is_string)),
        "completionCriteria must be a non-empty array of measurable statements",
    );
    push_if(
        errors,
        !is_string_array(value.get("invariants")),
        "invariants must be an array of strings (may be empty)",
    );
    push_if(
        errors,
        !is_string_array(value.get("outOfScope")),
        "outOfScope must be an array of strings (may be empty)",
    );
}

fn validate_patch_report(value: &serde_json::Map<String, Value>, errors: &mut Vec<String>) {
    push_if(
        errors,
        !is_string_array(value.get("changedFiles")),
        "changedFiles must be an array of file paths (may be empty)",
    );
    push_if(
        errors,
        !is_non_empty_string(value.get("summary")),
        "summary must be a non-empty string",
    );
    push_if(
        errors,
        !is_string_array(value.get("deviations")),
        "deviations must be an array of strings (may be empty)",
    );
    let invalidated = value.get("planInvalidated");
    push_if(
        errors,
        !invalidated.is_some_and(Value::is_boolean),
        "planInvalidated must be a boolean",
    );
    push_if(
        errors,
        invalidated == Some(&Value::Bool(true))
            && !is_non_empty_string(value.get("invalidationReason")),
        "invalidationReason is required (non-empty string) when planInvalidated is true",
    );
}

fn validate_review(value: &serde_json::Map<String, Value>, errors: &mut Vec<String>) {
    let verdict = value.get("verdict").and_then(Value::as_str);
    push_if(
        errors,
        !verdict
            .map(|text| Verdict::parse(text).is_some())
            .unwrap_or(false),
        format!("verdict must be one of {}", Verdict::names()),
    );
    match value.get("issues") {
        Some(Value::Array(issues)) => {
            for (index, issue) in issues.iter().enumerate() {
                let Some(issue) = issue.as_object() else {
                    errors.push(format!("issues[{index}] must be an object"));
                    continue;
                };
                push_if(
                    errors,
                    !is_enum_member(issue.get("severity"), |text| {
                        Severity::parse(text).is_some()
                    }),
                    format!(
                        "issues[{index}].severity must be one of {}",
                        Severity::names()
                    ),
                );
                push_if(
                    errors,
                    !is_non_empty_string(issue.get("description")),
                    format!("issues[{index}].description must be a non-empty string"),
                );
                push_if(
                    errors,
                    !matches!(
                        issue.get("file"),
                        None | Some(Value::Null) | Some(Value::String(_))
                    ),
                    format!("issues[{index}].file must be a string when present"),
                );
            }
            push_if(
                errors,
                verdict == Some("changes_required") && issues.is_empty(),
                "issues must contain at least one issue when verdict is changes_required",
            );
        }
        _ => errors.push("issues must be an array".into()),
    }
    push_if(
        errors,
        !value.get("notes").is_some_and(Value::is_string),
        "notes must be a string (may be empty)",
    );
}

pub fn validate_artifact(expect: ArtifactKind, value: &Value) -> ValidationResult {
    let Some(object) = value.as_object() else {
        return Err(vec![format!(
            "artifact must be a JSON object matching the \"{expect}\" contract"
        )]);
    };
    let mut errors = Vec::new();
    match expect {
        ArtifactKind::Classification => validate_classification(object, &mut errors),
        ArtifactKind::Evidence => validate_evidence(object, &mut errors),
        ArtifactKind::Plan => validate_plan(object, &mut errors),
        ArtifactKind::PatchReport => validate_patch_report(object, &mut errors),
        ArtifactKind::Review => validate_review(object, &mut errors),
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let decoded = match expect {
        ArtifactKind::Classification => {
            serde_json::from_value(value.clone()).map(Artifact::Classification)
        }
        ArtifactKind::Evidence => serde_json::from_value(value.clone())
            .map(|evidence| Artifact::Evidence(Box::new(evidence))),
        ArtifactKind::Plan => {
            serde_json::from_value(value.clone()).map(|plan| Artifact::Plan(Box::new(plan)))
        }
        ArtifactKind::PatchReport => serde_json::from_value(value.clone())
            .map(|report| Artifact::PatchReport(Box::new(report))),
        ArtifactKind::Review => {
            serde_json::from_value(value.clone()).map(|review| Artifact::Review(Box::new(review)))
        }
    };
    decoded.map_err(|error| vec![format!("artifact could not be decoded: {error}")])
}

const BUDGET_KEYS: &[&str] = &[
    "maxResearchers",
    "maxParallelWorkers",
    "maxWorkers",
    "maxRevisionCycles",
    "maxReplans",
    "maxCostUsd",
    "runDeadlineMs",
    "verifyCommandTimeoutMs",
];

/// Returns `[]` when valid. Only checks the SHAPE of user-provided overrides;
/// merging is `config.rs`'s job.
pub fn validate_config_shape(value: &Value) -> Vec<String> {
    let Some(object) = value.as_object() else {
        return vec!["config must be a JSON object".into()];
    };
    let mut errors = Vec::new();
    match object.get("budgets") {
        None | Some(Value::Null) => {}
        Some(Value::Object(budgets)) => {
            for key in BUDGET_KEYS {
                match budgets.get(*key) {
                    None | Some(Value::Null) => {}
                    Some(value) if value.as_f64().is_some_and(|number| number >= 0.0) => {}
                    Some(_) => errors.push(format!("budgets.{key} must be a non-negative number")),
                }
            }
            match budgets.get("workerTimeoutMs") {
                None | Some(Value::Null) => {}
                Some(Value::Object(timeouts)) => {
                    for (role, timeout) in timeouts {
                        push_if(
                            &mut errors,
                            Role::parse(role).is_none(),
                            format!("budgets.workerTimeoutMs.{role}: unknown role"),
                        );
                        push_if(
                            &mut errors,
                            !timeout.as_f64().is_some_and(|number| number >= 0.0),
                            format!(
                                "budgets.workerTimeoutMs.{role} must be a non-negative number (0 = no timeout)"
                            ),
                        );
                    }
                }
                Some(_) => errors.push(
                    "budgets.workerTimeoutMs must be an object mapping role to milliseconds".into(),
                ),
            }
        }
        Some(_) => errors.push("budgets must be an object".into()),
    }
    match object.get("models") {
        None | Some(Value::Null) => {}
        Some(Value::Object(models)) => {
            for (role, model) in models {
                push_if(
                    &mut errors,
                    Role::parse(role).is_none(),
                    format!("models.{role}: unknown role"),
                );
                push_if(
                    &mut errors,
                    !model.is_string(),
                    format!("models.{role} must be a string like \"provider/modelId\""),
                );
            }
        }
        Some(_) => {
            errors.push("models must be an object mapping role to \"provider/modelId\"".into())
        }
    }
    match object.get("verifyCommands") {
        None | Some(Value::Null) => {}
        Some(Value::Array(commands)) => {
            for (index, command) in commands.iter().enumerate() {
                let valid = command.as_object().is_some_and(|command| {
                    command.get("name").is_some_and(Value::is_string)
                        && command.get("command").is_some_and(Value::is_string)
                });
                push_if(
                    &mut errors,
                    !valid,
                    format!("verifyCommands[{index}] must be {{ name: string; command: string }}"),
                );
            }
        }
        Some(_) => errors.push("verifyCommands must be an array".into()),
    }
    for key in ["workerExtensions", "workerExtraTools"] {
        match object.get(key) {
            None | Some(Value::Null) => {}
            Some(value) if is_string_array(Some(value)) => {}
            Some(_) => errors.push(format!("{key} must be an array of strings")),
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_contract_example_validates_and_every_schema_names_its_fields() {
        for kind in [
            ArtifactKind::Classification,
            ArtifactKind::Evidence,
            ArtifactKind::Plan,
            ArtifactKind::PatchReport,
            ArtifactKind::Review,
        ] {
            let contract = artifact_contract(kind);
            let contract_str = contract.to_string();
            let json = contract_str
                .split("```json\n")
                .nth(1)
                .and_then(|rest| rest.split("\n```").next())
                .expect("the contract carries an example");
            let example: Value = serde_json::from_str(json).expect("example is JSON");
            assert!(
                validate_artifact(kind, &example).is_ok(),
                "{kind}: {:?}",
                validate_artifact(kind, &example).err()
            );
            let schema = artifact_schema(kind);
            let required = schema["required"].as_array().expect("required");
            for field in required {
                assert!(
                    schema["properties"].get(field.as_str().unwrap()).is_some(),
                    "{kind}: {field} is required but has no schema"
                );
                assert!(
                    example.get(field.as_str().unwrap()).is_some(),
                    "{kind}: the example lacks required {field}"
                );
            }
        }
    }
    use serde_json::json;

    #[test]
    fn a_valid_classification_decodes() {
        let value = json!({
            "taskClass": "bug",
            "complexity": "standard",
            "rationale": "because",
            "researchTasks": [{"kind": "code_search", "focus": "where is it"}],
        });
        let artifact = validate_artifact(ArtifactKind::Classification, &value).expect("valid");
        let classification = artifact.as_classification().expect("classification");
        assert_eq!(classification.task_class, TaskClass::Bug);
        assert_eq!(classification.research_tasks.len(), 1);
    }

    #[test]
    fn validation_errors_name_the_offending_field() {
        let value = json!({"taskClass": "nope", "complexity": "standard", "rationale": ""});
        let errors = validate_artifact(ArtifactKind::Classification, &value).unwrap_err();
        assert!(errors.iter().any(|error| error.starts_with("taskClass")));
        assert!(errors.iter().any(|error| error.starts_with("rationale")));
        assert!(errors
            .iter()
            .any(|error| error.starts_with("researchTasks")));
    }

    #[test]
    fn evidence_without_refs_is_not_evidence() {
        let value = json!({
            "kind": "code_search",
            "findings": [{"claim": "x", "confidence": "high"}],
            "risks": [],
            "gaps": [],
        });
        let errors = validate_artifact(ArtifactKind::Evidence, &value).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("refs")));
    }

    #[test]
    fn a_single_milestone_is_rejected_as_a_non_decomposition() {
        let value = json!({
            "taskClass": "feature",
            "complexity": "complex",
            "rationale": "r",
            "researchTasks": [],
            "milestones": ["only one"],
        });
        let errors = validate_artifact(ArtifactKind::Classification, &value).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("2+ entries")));
    }

    #[test]
    fn a_changes_required_review_must_list_an_issue() {
        let value = json!({"verdict": "changes_required", "issues": [], "notes": ""});
        let errors = validate_artifact(ArtifactKind::Review, &value).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("at least one")));
    }

    #[test]
    fn an_invalidated_plan_must_say_why() {
        let value = json!({
            "changedFiles": [],
            "summary": "s",
            "deviations": [],
            "planInvalidated": true,
        });
        let errors = validate_artifact(ArtifactKind::PatchReport, &value).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.contains("invalidationReason")));
    }

    #[test]
    fn a_zero_worker_timeout_is_accepted_as_no_timeout() {
        let value = json!({"budgets": {"workerTimeoutMs": {"writer": 0}, "maxCostUsd": 0}});
        assert!(validate_config_shape(&value).is_empty());
    }

    #[test]
    fn negative_budgets_and_unknown_roles_are_rejected() {
        let value = json!({"budgets": {"maxCostUsd": -1, "workerTimeoutMs": {"wizard": 5}}});
        let errors = validate_config_shape(&value);
        assert!(errors.iter().any(|error| error.contains("maxCostUsd")));
        assert!(errors.iter().any(|error| error.contains("unknown role")));
    }

    #[test]
    fn classification_schema_and_runtime_acceptance_match_for_milestones() {
        let artifact = serde_json::json!({
            "complexity": "simple",
            "reason": "small change"
        });
        let contract = artifact_contract(ArtifactKind::Classification);
        assert_eq!(
            contract.accepts(&artifact),
            validate_artifact(ArtifactKind::Classification, &artifact).is_ok()
        );
    }

    #[test]
    fn artifact_schema_and_runtime_acceptance_parity_for_all_kinds() {
        for kind in [
            ArtifactKind::Classification,
            ArtifactKind::Evidence,
            ArtifactKind::Plan,
            ArtifactKind::PatchReport,
            ArtifactKind::Review,
        ] {
            let contract = artifact_contract(kind);
            let contract_str = contract.to_string();
            let json = contract_str
                .split("```json\n")
                .nth(1)
                .and_then(|rest| rest.split("\n```").next())
                .expect("the contract carries an example");
            let valid_example: Value = serde_json::from_str(json).expect("valid JSON example");

            assert!(contract.accepts(&valid_example), "{kind} contract should accept example");
            assert!(validate_artifact(kind, &valid_example).is_ok(), "{kind} validator should accept example");

            let empty = json!({});
            assert!(!contract.accepts(&empty), "{kind} contract should reject empty object");
            assert!(validate_artifact(kind, &empty).is_err(), "{kind} validator should reject empty object");
        }
    }

    #[test]
    fn evidence_finding_rejects_empty_refs() {
        let value = serde_json::json!({
            "findings": [{"summary": "auth lives here", "refs": []}]
        });
        assert!(validate_artifact(ArtifactKind::Evidence, &value).is_err());
    }

    #[test]
    fn evidence_finding_rejects_blank_ref() {
        let value = serde_json::json!({
            "findings": [{"summary": "auth lives here", "refs": ["  "]}]
        });
        assert!(validate_artifact(ArtifactKind::Evidence, &value).is_err());
    }
}
