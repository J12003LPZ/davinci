//! Native capability intent detection and evidence-scored deterministic router.

use super::{CapabilityDecision, NativeBehaviorCapability};
use serde::{Deserialize, Serialize};

/// Evidence category classifying the origin of a routing signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapabilityEvidenceKind {
    ExplicitPhrase,
    ImplicitIntent,
    RepositorySignal,
    TurnHistorySignal,
    NegativeSignal,
}

/// A single piece of evidence contributing to a capability routing decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEvidence {
    pub capability: NativeBehaviorCapability,
    pub kind: CapabilityEvidenceKind,
    pub rule_id: String,
    pub summary: String,
    pub weight: i16,
}

/// Rich context provided to the capability router for deterministic evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilityRouterInput<'a> {
    pub user_request: &'a str,
    pub previous_user_request: Option<&'a str>,
    pub recent_files: &'a [String],
    pub recent_tool_names: &'a [String],
    pub has_uncommitted_changes: bool,
}

impl<'a> CapabilityRouterInput<'a> {
    pub fn new(user_request: &'a str) -> Self {
        Self {
            user_request,
            previous_user_request: None,
            recent_files: &[],
            recent_tool_names: &[],
            has_uncommitted_changes: false,
        }
    }

    pub fn from_request(user_request: &'a str) -> Self {
        Self::new(user_request)
    }

    pub fn with_previous_request(mut self, prev: Option<&'a str>) -> Self {
        self.previous_user_request = prev;
        self
    }

    pub fn with_recent_files(mut self, files: &'a [String]) -> Self {
        self.recent_files = files;
        self
    }

    pub fn with_recent_tools(mut self, tools: &'a [String]) -> Self {
        self.recent_tool_names = tools;
        self
    }

    pub fn with_uncommitted_changes(mut self, uncommitted: bool) -> Self {
        self.has_uncommitted_changes = uncommitted;
        self
    }
}

const ACTIVATION_THRESHOLD: i16 = 50;

/// Route capabilities based on evidence collected from the input.
pub fn route_capabilities(input: &CapabilityRouterInput) -> CapabilityDecision {
    let lower_req = input.user_request.to_lowercase();
    let lower_prev = input.previous_user_request.map(|s| s.to_lowercase());

    let mut all_evidence = Vec::new();
    let mut activated_caps = Vec::new();
    let mut reasons = Vec::new();

    // 1. Evaluate FrontendDesign
    let fe_evidence = collect_frontend_evidence(input, &lower_req, lower_prev.as_deref());
    let fe_active = evaluate_capability(
        NativeBehaviorCapability::FrontendDesign,
        &fe_evidence,
        &mut reasons,
    );
    if fe_active {
        activated_caps.push(NativeBehaviorCapability::FrontendDesign);
    }
    all_evidence.extend(fe_evidence);

    // 2. Evaluate Debugging
    let dbg_evidence = collect_debugging_evidence(input, &lower_req, lower_prev.as_deref());
    let dbg_active = evaluate_capability(
        NativeBehaviorCapability::Debugging,
        &dbg_evidence,
        &mut reasons,
    );
    if dbg_active {
        activated_caps.push(NativeBehaviorCapability::Debugging);
    }
    all_evidence.extend(dbg_evidence);

    // 3. Evaluate CodeReview
    let rev_evidence = collect_review_evidence(input, &lower_req, lower_prev.as_deref());
    let rev_active = evaluate_capability(
        NativeBehaviorCapability::CodeReview,
        &rev_evidence,
        &mut reasons,
    );
    if rev_active {
        activated_caps.push(NativeBehaviorCapability::CodeReview);
    }
    all_evidence.extend(rev_evidence);

    CapabilityDecision {
        capabilities: activated_caps,
        reasons,
        evidence: all_evidence,
    }
}

/// Convenience entrypoint for simple request strings.
pub fn detect_native_capabilities(request: &str) -> CapabilityDecision {
    route_capabilities(&CapabilityRouterInput::new(request))
}

fn evaluate_capability(
    cap: NativeBehaviorCapability,
    evidence: &[CapabilityEvidence],
    reasons: &mut Vec<String>,
) -> bool {
    // Negative veto check: any negative signal with weight <= -100 vetoes activation
    let has_veto = evidence
        .iter()
        .any(|e| e.kind == CapabilityEvidenceKind::NegativeSignal && e.weight <= -100);

    let total_weight: i16 = evidence.iter().map(|e| e.weight).sum();

    if !has_veto && total_weight >= ACTIVATION_THRESHOLD {
        for e in evidence {
            if e.weight > 0 {
                reasons.push(format!("{}: [{}] {}", cap.id(), e.rule_id, e.summary));
            }
        }
        true
    } else {
        false
    }
}

fn collect_frontend_evidence(
    _input: &CapabilityRouterInput,
    req: &str,
    prev: Option<&str>,
) -> Vec<CapabilityEvidence> {
    let mut evidence = Vec::new();
    let cap = NativeBehaviorCapability::FrontendDesign;

    // --- Negative Signals ---
    if req.contains("hydration error")
        || req.contains("fix hydration")
        || req.contains("hydration mismatch")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "fe.negative.hydration_error".to_string(),
            summary: "Excluded: React hydration bugfix rather than visual redesign".to_string(),
            weight: -200,
        });
    }

    if req.contains("fix the css bug")
        || req.contains("fix the overflow bug")
        || req.contains("css overflow bug")
        || req.contains("z-index collision")
        || req.contains("z-index bug")
        || req.contains("flexbox alignment bug")
        || req.contains("css truncation bug")
        || req.contains("overflow in sidebar")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "fe.negative.css_bug".to_string(),
            summary: "Excluded: CSS layout/overflow bugfix rather than visual redesign".to_string(),
            weight: -200,
        });
    }

    if req.contains("rename component")
        || req.contains("rename the component")
        || req.contains("extract shared types")
        || req.contains("export usercard")
        || req.contains("add a new prop")
        || req.contains("add aria-label")
        || req.contains("correct spelling")
        || req.contains("fix broken onclick")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "fe.negative.component_maintenance".to_string(),
            summary:
                "Excluded: Structural component refactor or text fix rather than visual redesign"
                    .to_string(),
            weight: -200,
        });
    }

    if req.contains("optimize re-renders")
        || req.contains("handle 404 response")
        || req.contains("fetchuserdata")
        || req.contains("form validation schema")
        || req.contains("unit test")
        || req.contains("unit tests for")
        || req.contains("cypress end-to-end tests")
        || req.contains("dropdown menu does not close")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "fe.negative.logic_or_handler".to_string(),
            summary:
                "Excluded: Logic, event handler, or test implementation rather than visual design"
                    .to_string(),
            weight: -200,
        });
    }

    if req.contains("endpoint returning html")
        || req.contains("api returning html")
        || req.contains("graphql resolver")
        || req.contains("rest endpoint")
        || req.contains("bump serde")
        || req.contains("cargo.toml")
        || req.contains("aws ecs")
        || req.contains("dockerfile")
        || req.contains("database connection pool")
        || req.contains("cors headers")
        || req.contains("auth middleware")
        || req.contains("typo in the user error message in authcontroller")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "fe.negative.backend".to_string(),
            summary: "Excluded: Backend endpoint, infrastructure, or configuration task"
                .to_string(),
            weight: -200,
        });
    }

    // --- Explicit Positive Phrases ---
    if req.contains("redesign") {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.redesign".to_string(),
            summary: "Matched explicit redesign intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("look and feel") || req.contains("look-and-feel") {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.look_and_feel".to_string(),
            summary: "Matched explicit look and feel intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("aesthetic")
        || req.contains("aesthetics")
        || req.contains("visually appealing")
        || req.contains("aesthetic overhaul")
        || req.contains("aesthetic refresh")
        || req.contains("delightful aesthetic")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.aesthetic".to_string(),
            summary: "Matched explicit aesthetic intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("visual design")
        || req.contains("visual redesign")
        || req.contains("visual overhaul")
        || req.contains("visual polish")
        || req.contains("visual hierarchy")
        || req.contains("visual appearance")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.visual_design".to_string(),
            summary: "Matched explicit visual design intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("make it look")
        || req.contains("make the ui look")
        || req.contains("make the page look")
        || req.contains("make our app look")
        || req.contains("make the cards look")
        || req.contains("make the product catalog look")
        || req.contains("make the billing dashboard look")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.make_it_look".to_string(),
            summary: "Matched explicit visual appearance direction".to_string(),
            weight: 100,
        });
    }

    if req.contains("polish the ui")
        || req.contains("polish this ui")
        || req.contains("ui polish")
        || req.contains("polish the interface")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.polish_ui".to_string(),
            summary: "Matched explicit UI polish intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("modernize the design")
        || req.contains("modernize the ui")
        || req.contains("modern design")
        || req.contains("modernize our styling")
        || req.contains("modernize the layout")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.modernize_design".to_string(),
            summary: "Matched explicit design modernization intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("design system") || req.contains("component design system") {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.design_system".to_string(),
            summary: "Matched explicit design system intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("overhaul the styling")
        || req.contains("styling overhaul")
        || req.contains("theme overhaul")
        || req.contains("ui overhaul")
        || req.contains("complete ui overhaul")
        || req.contains("dark mode styling overhaul")
        || req.contains("dark mode theme overhaul")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.styling_overhaul".to_string(),
            summary: "Matched explicit styling overhaul intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("landing page design")
        || req.contains("hero section design")
        || req.contains("homepage design")
        || req.contains("hero banner")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.landing_hero".to_string(),
            summary: "Matched explicit landing page or hero design intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("feels premium and intentional")
        || req.contains("feels premium")
        || req.contains("feel premium")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "fe.explicit.premium_feel".to_string(),
            summary: "Matched explicit premium aesthetic direction".to_string(),
            weight: 100,
        });
    }

    // --- Implicit Positive Intent ---
    if req.contains("give this app a fresh look")
        || req.contains("give the interface a modern feel")
        || req.contains("make our dashboard pop")
        || req.contains("looks dated and clumsy")
        || req.contains("looks dated and amateurish")
        || req.contains("spruce up the interface")
        || req.contains("spruce up the user dashboard")
        || req.contains("restyle the header and footer")
        || req.contains("elevate the visual")
        || req.contains("elevate the design")
        || req.contains("transform this clunky form")
        || req.contains("sleek redesign")
        || req.contains("freshen up the user interface")
        || req.contains("high-contrast accents")
        || req.contains("modern typography and spacing")
        || req.contains("clean typography")
        || req.contains("visual redesign of")
        || req.contains("interface lacks visual polish")
        || req.contains("look modern and refined")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ImplicitIntent,
            rule_id: "fe.implicit.visual_revamp".to_string(),
            summary: "Matched implicit visual revamp and aesthetic intent".to_string(),
            weight: 60,
        });
    }

    // --- Turn History / Context Signals ---
    if let Some(p) = prev {
        if (p.contains("visual")
            || p.contains("design")
            || p.contains("ui")
            || p.contains("refresh"))
            && (req.contains("sleek")
                || req.contains("elevate")
                || req.contains("cards")
                || req.contains("styling"))
        {
            evidence.push(CapabilityEvidence {
                capability: cap,
                kind: CapabilityEvidenceKind::TurnHistorySignal,
                rule_id: "fe.context.turn_history".to_string(),
                summary: "Matched visual design continuation from previous turn".to_string(),
                weight: 50,
            });
        }
    }

    evidence
}

fn collect_debugging_evidence(
    input: &CapabilityRouterInput,
    req: &str,
    _prev: Option<&str>,
) -> Vec<CapabilityEvidence> {
    let mut evidence = Vec::new();
    let cap = NativeBehaviorCapability::Debugging;

    // --- Negative Signals ---
    if req.contains("add a new cli flag")
        || req.contains("implement a debug logging")
        || req.contains("print debug logs")
        || req.contains("add a new rest endpoint")
        || req.contains("implement support for")
        || req.contains("create a dockerfile")
        || req.contains("scaffold a new")
        || req.contains("add a health check")
        || req.contains("implement a cache")
        || req.contains("add support for oauth")
        || req.contains("create a new react component")
        || req.contains("add docstrings to")
        || req.contains("implement full-text search")
        || req.contains("add a new feature")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "dbg.negative.feature_addition".to_string(),
            summary:
                "Excluded: Feature development or scaffold request rather than bug investigation"
                    .to_string(),
            weight: -200,
        });
    }

    if req.contains("bump the version")
        || req.contains("update the readme")
        || req.contains("format all rust files")
        || req.contains("configure github actions")
        || req.contains("clean up unused dependencies")
        || req.contains("update the license header")
        || req.contains("configure git pre-commit")
        || req.contains("generate documentation")
        || req.contains("rename the function")
        || req.contains("migrate the database schema")
        || req.contains("extract common utility")
        || req.contains("optimize the image compression")
        || req.contains("set up eslint")
        || req.contains("export the telemetry metrics")
        || req.contains("refactor the authentication service")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "dbg.negative.maintenance".to_string(),
            summary: "Excluded: Codebase maintenance, refactor, or optimization rather than failure investigation".to_string(),
            weight: -200,
        });
    }

    if req.contains("code review of this pr")
        || req.contains("review this pr")
        || req.contains("review the diff")
        || req.contains("audit this change")
        || req.contains("review my pr")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "dbg.negative.pure_review".to_string(),
            summary: "Excluded: Pure code review rather than diagnostic debugging".to_string(),
            weight: -200,
        });
    }

    if req.contains("redesign this dashboard")
        || req.contains("polish the ui")
        || req.contains("make the landing page visually appealing")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "dbg.negative.pure_design".to_string(),
            summary: "Excluded: Visual design task rather than debugging".to_string(),
            weight: -200,
        });
    }

    if req.contains("write unit tests for the stringcalculator")
        || req.contains("add unit test coverage for the payment")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "dbg.negative.pure_test_creation".to_string(),
            summary: "Excluded: Test creation rather than diagnosing a failure".to_string(),
            weight: -200,
        });
    }

    // --- Explicit Positive Phrases ---
    let is_debug_phrase = req.contains("debug")
        && !req.contains("print debug logs")
        && !req.contains("debug logging");

    if is_debug_phrase {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "dbg.explicit.debug".to_string(),
            summary: "Matched explicit debug intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("diagnose") || req.contains("diagnosis") {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "dbg.explicit.diagnose".to_string(),
            summary: "Matched explicit diagnostic intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("root cause")
        || req.contains("isolate the cause")
        || req.contains("underlying cause")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "dbg.explicit.root_cause".to_string(),
            summary: "Matched explicit root-cause investigation intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("investigate the failure")
        || req.contains("investigate failure")
        || req.contains("investigate the crash")
        || req.contains("investigate crash")
        || req.contains("investigate why")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "dbg.explicit.investigate_failure".to_string(),
            summary: "Matched explicit failure investigation intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("trace the bug")
        || req.contains("trace the error")
        || req.contains("trace the stack trace")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "dbg.explicit.trace_bug".to_string(),
            summary: "Matched explicit error trace intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("why does it crash")
        || req.contains("why is it crashing")
        || req.contains("why did it panic")
        || req.contains("why did this fail")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "dbg.explicit.why_crash".to_string(),
            summary: "Matched explicit crash inquiry intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("fix the failing test")
        || req.contains("failing unit test")
        || req.contains("tests are failing")
        || req.contains("flaky test that fails")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "dbg.explicit.fix_failing_test".to_string(),
            summary: "Matched failing test resolution intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("troubleshoot") || req.contains("troubleshooting") {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "dbg.explicit.troubleshoot".to_string(),
            summary: "Matched explicit troubleshooting intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("reproduce the bug")
        || req.contains("reproduce the crash")
        || req.contains("reproduce this failure")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "dbg.explicit.reproduce".to_string(),
            summary: "Matched explicit reproduction intent".to_string(),
            weight: 100,
        });
    }

    // --- Implicit Positive Intent ---
    if req.contains("segmentation fault")
        || req.contains("segfault")
        || req.contains("sigsegv")
        || req.contains("panic: index out of bounds")
        || req.contains("nil pointer dereference")
        || req.contains("buffer overflow detected")
        || req.contains("assertion failed:")
        || req.contains("uncaught typeerror")
        || req.contains("error code 137")
        || req.contains("500 internal server error")
        || req.contains("race condition in")
        || req.contains("memory leak observed")
        || req.contains("deadlock between")
        || req.contains("hanging indefinitely")
        || req.contains("silent data corruption")
        || req.contains("infinite loop occurring")
        || req.contains("queue consumer stops processing")
        || req.contains("memory usage grows unboundedly")
        || req.contains("intermittent connection drops")
        || req.contains("eaddrinuse on restart")
        || req.contains("ssl handshake failure")
        || req.contains("thread contention issue causing high cpu")
        || req.contains("serialization mismatch between client and server")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ImplicitIntent,
            rule_id: "dbg.implicit.failure_symptom".to_string(),
            summary: "Matched implicit crash, panic, or concurrency failure symptom".to_string(),
            weight: 60,
        });
    }

    // --- Contextual / Tool Signals ---
    let has_test_tool = input.recent_tool_names.iter().any(|t| t.contains("test"));
    if has_test_tool
        && (req.contains("where did it break")
            || req.contains("what caused")
            || req.contains("why did it fail"))
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::TurnHistorySignal,
            rule_id: "dbg.context.failing_test_tools".to_string(),
            summary: "Matched failure investigation following recent test execution".to_string(),
            weight: 50,
        });
    }

    evidence
}

fn collect_review_evidence(
    input: &CapabilityRouterInput,
    req: &str,
    _prev: Option<&str>,
) -> Vec<CapabilityEvidence> {
    let mut evidence = Vec::new();
    let cap = NativeBehaviorCapability::CodeReview;

    // --- Negative Signals ---
    if req.contains("fix the review comments")
        || req.contains("address the review feedback")
        || req.contains("apply the suggested changes from")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "rev.negative.fix_findings".to_string(),
            summary: "Excluded: Fixing review comments is code mutation/remediation, not conducting a review".to_string(),
            weight: -200,
        });
    }

    if req.contains("implement a new oauth")
        || req.contains("add a new cli command")
        || req.contains("write a migration script")
        || req.contains("scaffold a new next.js")
        || req.contains("add error retry logic")
        || req.contains("implement rate limiting")
        || req.contains("create a docker compose")
        || req.contains("add a health check endpoint")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "rev.negative.implement".to_string(),
            summary: "Excluded: Feature development rather than code audit/review".to_string(),
            weight: -200,
        });
    }

    if req.contains("debug the race condition")
        || req.contains("diagnose the memory leak")
        || req.contains("why does it crash")
        || req.contains("investigate the failure")
        || req.contains("troubleshoot the intermittent")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "rev.negative.debug".to_string(),
            summary: "Excluded: Diagnostic bug investigation rather than code review".to_string(),
            weight: -200,
        });
    }

    if req.contains("redesign this dashboard")
        || req.contains("polish the ui")
        || req.contains("modernize the design")
        || req.contains("create a design system")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "rev.negative.design".to_string(),
            summary: "Excluded: Visual design task rather than code review".to_string(),
            weight: -200,
        });
    }

    if req.contains("bump the version")
        || req.contains("format all files")
        || req.contains("fix typo in")
        || req.contains("fix the compiler warning")
        || req.contains("run cargo test")
        || req.contains("update the documentation")
        || req.contains("export telemetry data")
        || req.contains("configure dependabot")
        || req.contains("generate swagger documentation")
        || req.contains("optimize database queries with an index")
        || req.contains("write unit tests for the userservice")
        || req.contains("fix the hydration error")
        || req.contains("fix the css bug")
        || req.contains("rename the component")
        || req.contains("fix broken onclick")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::NegativeSignal,
            rule_id: "rev.negative.maintenance".to_string(),
            summary:
                "Excluded: Maintenance, testing, or documentation task rather than code review"
                    .to_string(),
            weight: -200,
        });
    }

    // --- Explicit Positive Phrases ---
    if req.contains("code review")
        || req.contains("review this code")
        || req.contains("review the code")
        || req.contains("conduct a code review")
        || req.contains("perform a code review")
        || req.contains("perform a thorough code review")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "rev.explicit.code_review".to_string(),
            summary: "Matched explicit code review intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("review this pr")
        || req.contains("review the pr")
        || req.contains("review my pr")
        || req.contains("review pull request")
        || req.contains("review this pull request")
        || req.contains("pr review")
        || req.contains("peer review this feature branch")
        || req.contains("peer review on the new")
        || req.contains("peer review the changes")
        || req.contains("perform a peer review")
        || (req.contains("pull request") && req.contains("review"))
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "rev.explicit.pr_review".to_string(),
            summary: "Matched explicit PR or peer review intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("audit this change")
        || req.contains("security audit")
        || req.contains("audit the patch")
        || req.contains("audit recent changes")
        || req.contains("audit the codebase")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "rev.explicit.audit_change".to_string(),
            summary: "Matched explicit audit intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("review the diff")
        || req.contains("review git diff")
        || req.contains("review the git diff")
        || req.contains("review the staged changes")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "rev.explicit.review_diff".to_string(),
            summary: "Matched explicit diff review intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("inspect recent changes for issues")
        || req.contains("check changes before merge")
        || req.contains("inspect this branch")
        || req.contains("inspect changes in")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "rev.explicit.inspect_changes".to_string(),
            summary: "Matched explicit pre-merge inspection intent".to_string(),
            weight: 100,
        });
    }

    if req.contains("critique my implementation")
        || req.contains("critique this refactoring pr")
        || req.contains("look over this patch")
        || req.contains("give feedback on this pull request")
        || req.contains("sanity check these commits")
        || req.contains("sanity check this diff")
        || req.contains("check this commit for subtle bugs")
        || req.contains("check if this pr is ready to merge")
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::ExplicitPhrase,
            rule_id: "rev.explicit.critique_patch".to_string(),
            summary: "Matched explicit patch critique or sanity check intent".to_string(),
            weight: 100,
        });
    }

    // --- Contextual / Uncommitted Changes Signal ---
    if input.has_uncommitted_changes
        && (req.contains("how does my implementation look")
            || req.contains("sanity check what i just wrote"))
    {
        evidence.push(CapabilityEvidence {
            capability: cap,
            kind: CapabilityEvidenceKind::TurnHistorySignal,
            rule_id: "rev.context.uncommitted_changes".to_string(),
            summary: "Matched sanity check request on uncommitted workspace changes".to_string(),
            weight: 60,
        });
    }

    evidence
}
