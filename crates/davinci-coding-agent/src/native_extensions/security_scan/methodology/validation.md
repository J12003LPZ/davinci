Native security methodology v2: counterargument review

Inputs: original claim, pinned references, applicable policy data, permitted scope.
Output: one explicit disposition and evidence-backed gate assessment per candidate.

Before judging, set a short rubric of at most five criteria from the claim's
attacker input, sink and preconditions. Independently retrieve decisive source.
Attempt to disprove the original path: check upstream authorization, final-use
validation, transformations, alternate callers, failure handling, configuration,
and supported exposure. A sanitizer's name is not its semantics. Missing runtime
proof is a gap, not defeating evidence; no program runs in this review, so the
absence of a reproduction never counts against a claim.

Evaluate five gates: exact identity/location; actor/reachability; full control
semantics; violated boundary/impact; adversarial acceptance with counterevidence.
Confirmation requires every gate and no material gap. Credible unresolved
conditions produce likely or deferred results. Suppression and not-applicable
require concrete evidence. Earlier false-positive feedback may dismiss a matching
claim only when its stated reason still holds against the current controls; cite
it. When the claim depends on a product assumption the source cannot answer,
record the question as a proof gap instead of answering it. Calibrate confidence
from the evidence, not from how dangerous the bug class sounds. When one instance
of a repeated pattern is proven, check each sibling's own source, closest control
and sink before closing it; representative proof does not close siblings.
Duplicates retain occurrence identity and canonical links. Never claim tests ran
without coordinator-recorded observations. Static proof can suffice; do not
invent runtime observations or execute target code.

For each reportable assessment, supply rootCause: rootControl (qualified symbol
or missing-control context), violatedInvariant, sinkOrDecision (normalized decision),
attackPathClass, and control/decision Location objects. Ground the control in the
control-semantics gate and the decision in reviewed gate evidence. Use concise,
stable semantic labels; omit titles, line numbers, severity and occurrence-specific
actor examples from those labels. Keep different entry points and parameterizations
in the original claim. A matching identity does not authorize dropping an occurrence.
