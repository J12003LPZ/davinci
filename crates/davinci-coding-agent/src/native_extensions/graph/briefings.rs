//! Prompt construction for every model node, plus the JOIN node.
//!
//! Rules baked in here:
//!  - Tasks state outcomes, not procedures.
//!  - Context is pointers and digests, never whole transcripts.
//!  - The output contract lives in `graph_submit`; briefings say when to submit.
//!  - The reviewer receives evidence, never the writer's reasoning.

use super::types::{
    Confidence, EvidenceArtifact, ImplementationPlan, ResearchKind, ReviewDecision, Role, Verdict,
    VerificationResult,
};
use std::collections::BTreeSet;

const SUBMIT_RULE: &str =
    "When your work is complete, call the graph_submit tool exactly once with your result. \
That call is your entire deliverable; prose outside it is ignored. \
If graph_submit reports a validation error, fix the listed fields and call it again.";

pub fn role_system_prompt(role: Role) -> String {
    let body = match role {
        Role::Classifier => {
            "You are the task classifier in a graph-engineered coding pipeline. \
             You never touch the repository; you only judge the request. "
        }
        Role::Researcher => {
            "You are a read-only code researcher in a graph-engineered coding pipeline. \
             You gather evidence: verifiable claims with file:line references. \
             You cannot and must not modify anything (read-only role). Do not propose a patch; report what IS. "
        }
        Role::TestAnalyzer => {
            "You are a read-only test analyst in a graph-engineered coding pipeline. \
             You establish the current test baseline by running existing test and typecheck commands, \
             and report which tests exist, pass, and fail. You cannot modify anything (read-only role plus test execution). \
             Do not propose a patch; report what IS. "
        }
        Role::Historian => {
            "You are a read-only repository historian in a graph-engineered coding pipeline. \
             You use git log, git show, git blame and file reads to explain how the relevant code evolved \
             and what past changes are related. You cannot modify anything (read-only role). "
        }
        Role::Planner => {
            "You are the implementation planner in a graph-engineered coding pipeline. \
             You turn goal plus evidence into an explicit, bounded implementation plan with executable \
             completion criteria. You never touch the repository. Plan only what the goal requires; \
             list everything else under outOfScope. "
        }
        Role::Writer => {
            "You are the implementation writer in a graph-engineered coding pipeline - the sole mutation node. \
             Exactly one writer exists; every other node is read-only. Implement the approved plan. \
             Do not redesign the task, do not expand scope, do not refactor beyond the plan. \
             Never run git commit, git push, or any git state change; a human owns version control. \
             If evidence shows the plan is wrong, stop and submit planInvalidated=true with the reason \
             instead of inventing a different project. "
        }
        Role::Reviewer => {
            "You are the independent reviewer in a graph-engineered coding pipeline. \
             You judge the resulting artifacts - diff, changed files, verification output - on their own merits. \
             You were deliberately not shown the writer's reasoning; do not try to reconstruct or excuse it. \
             You cannot modify anything (read-only role plus test execution). \
             The diff and verification results ARE your evidence: read the specific files the diff touches to \
             check context, but never re-survey the repository with broad greps or file sweeps - a wide \
             search costs far more than it tells you, and the change in front of you is the evidence. \
             Approve only if the change satisfies the goal and plan without blocker or major issues. "
        }
    };
    format!("{body}{SUBMIT_RULE}")
}

pub struct ClassifyInput<'a> {
    pub goal: &'a str,
    pub is_git_repo: bool,
    pub package_scripts: Vec<String>,
    pub max_researchers: u32,
}

pub fn classify_briefing(input: &ClassifyInput<'_>) -> String {
    let scripts = if input.package_scripts.is_empty() {
        "none found".to_string()
    } else {
        input.package_scripts.join(", ")
    };
    let cap = input.max_researchers;
    [
        "# Classify this coding request",
        "",
        "## Request",
        input.goal,
        "",
        "## Repository signals",
        &format!(
            "- git repository: {}",
            if input.is_git_repo { "yes" } else { "no" }
        ),
        &format!("- package.json scripts: {scripts}"),
        "",
        "## Your call",
        "Decide taskClass and complexity:",
        "- \"trivial\": one obvious, localized change (typo, rename, small config tweak). No investigation needed.",
        "- \"standard\": clear goal, one subsystem, moderate uncertainty. 1-2 research tasks.",
        "- \"complex\": multiple subsystems, unknown root cause, or architectural impact. Up to the cap below.",
        "",
        &format!(
            "Propose researchTasks (0 to {cap}) ONLY where each answers a distinct question you cannot"
        ),
        "answer from the request alone. Kinds: code_search (locate and understand relevant code),",
        "test_baseline (what tests exist and currently pass or fail), history (how this code evolved - only useful",
        "in a git repository), docs (project documentation and conventions). Each focus must be a concrete question,",
        "not a topic. Fewer, sharper tasks beat broad sweeps.",
        "",
        "Also decide milestones. If the request bundles SEVERAL SEPARABLE deliverables (numbered gaps,",
        "multiple independent features or fixes), list 2-8 ordered milestones. Each milestone must be",
        "independently implementable and verifiable - a change that could ship on its own. The pipeline",
        "will plan, implement, verify, and review each milestone separately, in the order you give.",
        "Milestones are deliverables, not implementation steps: never split one coherent change into",
        "stages. For a single deliverable, return an empty milestones array.",
    ]
    .join("\n")
}

/// The goal text a milestone's planner/writer/reviewer sees: the full request
/// for context, the current milestone as the only actionable scope.
pub fn milestone_goal(goal: &str, milestones: &[String], index: usize) -> String {
    let mut lines = vec![
        goal.to_string(),
        String::new(),
        format!(
            "## Current milestone ({} of {}) - deliver ONLY this now",
            index + 1,
            milestones.len()
        ),
        milestones
            .get(index)
            .cloned()
            .unwrap_or_else(|| goal.to_string()),
    ];
    let delivered = &milestones[..index.min(milestones.len())];
    if !delivered.is_empty() {
        lines.push(String::new());
        lines.push("## Already delivered by earlier milestones (do not redo)".into());
        lines.extend(delivered.iter().map(|entry| format!("- {entry}")));
    }
    let later = milestones.get(index + 1..).unwrap_or(&[]);
    if !later.is_empty() {
        lines.push(String::new());
        lines.push("## Later milestones (do NOT start these)".into());
        lines.extend(later.iter().map(|entry| format!("- {entry}")));
    }
    lines.join("\n")
}

pub fn research_briefing(goal: &str, kind: ResearchKind, focus: &str) -> String {
    [
        "# Research assignment",
        "",
        "## Overall goal (context only - you are NOT implementing it)",
        goal,
        "",
        &format!("## Your question (kind: {kind})"),
        focus,
        "",
        "## Bar for findings",
        "Every finding needs file:line refs a stranger could open and verify. State confidence honestly.",
        "List real risks you noticed and real gaps you could not answer. Do not propose a patch.",
    ]
    .join("\n")
}

pub fn build_evidence_digest(
    artifacts: &[EvidenceArtifact],
    failed_kinds: &[ResearchKind],
    max_chars: usize,
) -> String {
    let mut seen_claims = BTreeSet::new();
    let mut findings: Vec<(u8, String)> = Vec::new();
    let mut risks: Vec<String> = Vec::new();
    let mut gaps: Vec<String> = Vec::new();
    let mut test_baseline: Option<String> = None;

    for artifact in artifacts {
        for finding in &artifact.findings {
            if !seen_claims.insert(finding.claim.clone()) {
                continue;
            }
            let rank = match finding.confidence {
                Confidence::High => 0,
                Confidence::Medium => 1,
                Confidence::Low => 2,
            };
            findings.push((
                rank,
                format!(
                    "- [{}] {} ({})",
                    finding.confidence,
                    finding.claim,
                    finding.refs.join(", ")
                ),
            ));
        }
        for risk in &artifact.risks {
            if !risks.contains(risk) {
                risks.push(risk.clone());
            }
        }
        for gap in &artifact.gaps {
            if !gaps.contains(gap) {
                gaps.push(gap.clone());
            }
        }
        if let (Some(baseline), None) = (&artifact.test_baseline, &test_baseline) {
            test_baseline = Some(format!(
                "`{}` exited {}: {}",
                baseline.command, baseline.exit_code, baseline.summary
            ));
        }
    }
    for kind in failed_kinds {
        let gap = format!("research task \"{kind}\" failed; treat that area as unverified");
        if !gaps.contains(&gap) {
            gaps.push(gap);
        }
    }

    findings.sort_by_key(|(rank, _)| *rank);
    let mut kept: Vec<String> = Vec::new();
    let mut budget = max_chars;
    for (_, line) in findings {
        let cost = line.chars().count() + 1;
        if cost > budget {
            continue;
        }
        budget -= cost;
        kept.push(line);
    }

    let mut sections = vec![
        "### Findings".to_string(),
        if kept.is_empty() {
            "- none".to_string()
        } else {
            kept.join("\n")
        },
        String::new(),
        "### Risks".to_string(),
        if risks.is_empty() {
            "- none reported".to_string()
        } else {
            risks
                .iter()
                .map(|risk| format!("- {risk}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
        String::new(),
        "### Gaps (unverified areas)".to_string(),
        if gaps.is_empty() {
            "- none reported".to_string()
        } else {
            gaps.iter()
                .map(|gap| format!("- {gap}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
    ];
    if let Some(baseline) = test_baseline {
        sections.push(String::new());
        sections.push("### Test baseline".to_string());
        sections.push(baseline);
    }
    sections.join("\n")
}

pub fn plan_briefing(goal: &str, evidence_digest: &str, replan_reason: Option<&str>) -> String {
    let mut parts = vec![
        "# Produce the implementation plan".to_string(),
        String::new(),
        "## Goal".to_string(),
        goal.to_string(),
        String::new(),
        "## Evidence (gathered by read-only researchers)".to_string(),
        evidence_digest.to_string(),
    ];
    if let Some(reason) = replan_reason {
        parts.extend([
            String::new(),
            "## Previous plan was invalidated".to_string(),
            format!("The writer stopped because: {reason}"),
            "Produce a corrected plan that accounts for this.".to_string(),
        ]);
    }
    parts.extend([
        String::new(),
        "## Requirements on the plan".to_string(),
        "- steps reference the actual files evidence points to".to_string(),
        "- testsToRun are exact commands runnable from the project root".to_string(),
        "- completionCriteria are objectively checkable, not vibes".to_string(),
        "- invariants name what must NOT change (public APIs, schemas, generated files)"
            .to_string(),
    ]);
    parts.join("\n")
}

fn bullet_list(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        empty.to_string()
    } else {
        items
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn plan_section(plan: &ImplementationPlan) -> String {
    let steps = plan
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let files = if step.files.is_empty() {
                "n/a".to_string()
            } else {
                step.files.join(", ")
            };
            format!("{}. {} (files: {files})", index + 1, step.description)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let tests_to_add = if plan.tests_to_add.is_empty() {
        "- none".to_string()
    } else {
        plan.tests_to_add
            .iter()
            .map(|test| format!("- {}: {}", test.file, test.behavior))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let tests_to_run = if plan.tests_to_run.is_empty() {
        "- none".to_string()
    } else {
        plan.tests_to_run
            .iter()
            .map(|command| format!("- `{command}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    [
        "### Steps".to_string(),
        steps,
        String::new(),
        format!("### Tests to add\n{tests_to_add}"),
        String::new(),
        format!("### Tests to run\n{tests_to_run}"),
        String::new(),
        format!(
            "### Completion criteria\n{}",
            bullet_list(&plan.completion_criteria, "- none")
        ),
        String::new(),
        format!(
            "### Invariants (must not change)\n{}",
            bullet_list(&plan.invariants, "- none")
        ),
        String::new(),
        format!(
            "### Out of scope\n{}",
            bullet_list(&plan.out_of_scope, "- none")
        ),
    ]
    .join("\n")
}

pub fn implement_briefing(
    goal: &str,
    plan: Option<&ImplementationPlan>,
    evidence_digest: &str,
    revision_notes: Option<&str>,
) -> String {
    let mut parts = vec![
        "# Implement".to_string(),
        String::new(),
        "## Goal".to_string(),
        goal.to_string(),
    ];
    match plan {
        Some(plan) => parts.extend([
            String::new(),
            "## Approved plan (implement THIS, not your own)".to_string(),
            plan_section(plan),
        ]),
        None => parts.extend([
            String::new(),
            "## Mode".to_string(),
            "This was classified trivial: implement the goal directly, smallest reasonable change."
                .to_string(),
        ]),
    }
    if !evidence_digest.is_empty() {
        parts.extend([
            String::new(),
            "## Evidence".to_string(),
            evidence_digest.to_string(),
        ]);
    }
    if let Some(notes) = revision_notes.filter(|notes| !notes.is_empty()) {
        parts.extend([
            String::new(),
            "## Revision - previous attempt did not pass".to_string(),
            notes.to_string(),
            "Fix these concrete failures. Do not start over; the working tree already contains the previous attempt."
                .to_string(),
        ]);
    }
    parts.extend([
        String::new(),
        "## Hard rules".to_string(),
        "- You are the only process allowed to modify files.".to_string(),
        "- Never run git commit, git push, or any git state change.".to_string(),
        "- Run the plan's tests yourself before submitting; submit an honest changedFiles list."
            .to_string(),
        "- If the plan cannot work as written, submit planInvalidated=true with the reason."
            .to_string(),
    ]);
    parts.join("\n")
}

pub struct ReviewInput<'a> {
    pub goal: &'a str,
    pub plan: Option<&'a ImplementationPlan>,
    pub diff: &'a str,
    pub changed_files: &'a [String],
    pub verification: &'a VerificationResult,
}

pub fn review_briefing(input: &ReviewInput<'_>) -> String {
    let verification_lines = input
        .verification
        .commands
        .iter()
        .map(|command| {
            format!(
                "- {}: `{}` -> exit {} ({}ms)\n```\n{}\n```",
                command.name,
                command.command,
                command.exit_code,
                command.duration_ms,
                command.output_tail
            )
        })
        .collect::<Vec<_>>();
    let mut parts = vec![
        "# Review this change".to_string(),
        String::new(),
        "## Original goal".to_string(),
        input.goal.to_string(),
    ];
    if let Some(plan) = input.plan {
        parts.extend([
            String::new(),
            "## Approved plan".to_string(),
            plan_section(plan),
        ]);
    }
    parts.extend([
        String::new(),
        "## Changed files".to_string(),
        bullet_list(input.changed_files, "- (none reported)"),
        String::new(),
        "## Diff".to_string(),
        if input.diff.is_empty() {
            "(no git diff available - not a git repository; review the changed files by reading them)"
                .to_string()
        } else {
            format!("```diff\n{}\n```", input.diff)
        },
        String::new(),
        "## Deterministic verification results".to_string(),
        if verification_lines.is_empty() {
            "- no verification commands were configured".to_string()
        } else {
            verification_lines.join("\n")
        },
        String::new(),
        "## Your judgment".to_string(),
        "Judge whether the change satisfies the goal and plan. Look for: correctness bugs, plan deviations,".to_string(),
        "invariant violations, missing tests the plan promised, and scope creep. You may run the listed test".to_string(),
        "commands yourself to confirm. Severity: blocker = must not ship; major = should not ship; minor = note.".to_string(),
        "Budget your time: start from the diff above, open only files it touches or directly references,".to_string(),
        "and submit your verdict as soon as you have judged every changed file - do not explore beyond the change.".to_string(),
    ]);
    parts.join("\n")
}

pub fn revision_notes_from(
    verification: Option<&VerificationResult>,
    review: Option<&ReviewDecision>,
) -> String {
    let mut notes: Vec<String> = Vec::new();
    if let Some(verification) = verification.filter(|verification| !verification.passed) {
        for command in &verification.commands {
            if command.exit_code != 0 && !command.skipped {
                notes.push(format!(
                    "Verification failed: `{}` exited {}. Output tail:\n{}",
                    command.command, command.exit_code, command.output_tail
                ));
            }
        }
    }
    if let Some(review) = review.filter(|review| review.verdict == Verdict::ChangesRequired) {
        for issue in &review.issues {
            if issue.severity == super::types::Severity::Minor {
                continue;
            }
            let file = issue
                .file
                .as_ref()
                .map(|file| format!(", {file}"))
                .unwrap_or_default();
            notes.push(format!(
                "Reviewer ({}{file}): {}",
                issue.severity, issue.description
            ));
        }
    }
    notes.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::types::{
        EvidenceFinding, Severity, VerificationCommandResult,
    };

    #[test]
    fn every_role_prompt_demands_a_single_graph_submit() {
        for role in Role::ALL {
            assert!(role_system_prompt(*role).contains("graph_submit tool exactly once"));
        }
    }

    #[test]
    fn the_reviewer_is_never_told_the_writers_reasoning() {
        let prompt = role_system_prompt(Role::Reviewer);
        assert!(prompt.contains("deliberately not shown the writer's reasoning"));
    }

    #[test]
    fn the_digest_ranks_high_confidence_first_and_records_failed_research() {
        let evidence = EvidenceArtifact {
            kind: ResearchKind::CodeSearch,
            findings: vec![
                EvidenceFinding {
                    claim: "low one".into(),
                    refs: vec!["a.rs:1".into()],
                    confidence: Confidence::Low,
                },
                EvidenceFinding {
                    claim: "high one".into(),
                    refs: vec!["b.rs:2".into()],
                    confidence: Confidence::High,
                },
            ],
            risks: vec![],
            gaps: vec![],
            test_baseline: None,
        };
        let digest = build_evidence_digest(&[evidence], &[ResearchKind::History], 10_000);
        let high = digest.find("high one").expect("high finding present");
        let low = digest.find("low one").expect("low finding present");
        assert!(high < low);
        assert!(digest.contains("research task \"history\" failed"));
    }

    #[test]
    fn the_digest_respects_its_character_budget() {
        let evidence = EvidenceArtifact {
            kind: ResearchKind::CodeSearch,
            findings: (0..50)
                .map(|index| EvidenceFinding {
                    claim: format!("claim number {index} with padding text"),
                    refs: vec![format!("file{index}.rs:1")],
                    confidence: Confidence::High,
                })
                .collect(),
            risks: vec![],
            gaps: vec![],
            test_baseline: None,
        };
        let digest = build_evidence_digest(&[evidence], &[], 120);
        assert!(digest.len() < 500, "digest was {} chars", digest.len());
    }

    #[test]
    fn a_milestone_goal_names_what_is_done_and_what_is_off_limits() {
        let milestones = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];
        let text = milestone_goal("overall", &milestones, 1);
        assert!(text.contains("## Current milestone (2 of 3)"));
        assert!(text.contains("Already delivered"));
        assert!(text.contains("- first"));
        assert!(text.contains("Later milestones"));
        assert!(text.contains("- third"));
    }

    #[test]
    fn revision_notes_carry_failures_but_drop_minor_review_nits() {
        let verification = VerificationResult {
            commands: vec![VerificationCommandResult {
                name: "test".into(),
                command: "cargo test".into(),
                exit_code: 1,
                duration_ms: 10,
                output_tail: "boom".into(),
                skipped: false,
            }],
            passed: false,
        };
        let review = ReviewDecision {
            verdict: Verdict::ChangesRequired,
            issues: vec![
                super::super::types::ReviewIssue {
                    severity: Severity::Blocker,
                    file: Some("src/lib.rs".into()),
                    description: "unsound".into(),
                },
                super::super::types::ReviewIssue {
                    severity: Severity::Minor,
                    file: None,
                    description: "naming".into(),
                },
            ],
            notes: String::new(),
        };
        let notes = revision_notes_from(Some(&verification), Some(&review));
        assert!(notes.contains("cargo test"));
        assert!(notes.contains("unsound"));
        assert!(!notes.contains("naming"));
    }

    #[test]
    fn a_passing_verification_contributes_no_revision_notes() {
        let verification = VerificationResult {
            commands: vec![],
            passed: true,
        };
        assert!(revision_notes_from(Some(&verification), None).is_empty());
    }
}
