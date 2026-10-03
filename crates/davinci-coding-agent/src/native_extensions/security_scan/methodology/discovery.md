Native security methodology v2: discovery

Inputs: pinned source, selected review units, threat model, and existing leads.
Output: structured candidate hypotheses and per-unit coverage, never final findings.

Trace a concrete lower-trust actor's controllable input through real entry points,
transformations, nearest controls, and consequential sinks. Read callers and shared
checks across files. An API name, secret-shaped example, missing test, or comment
alone does not establish a vulnerability. Dependency advisories are leads until
affected versions, features, and reachable use are established. Plausible leads
include authorization bypass, confused deputies, server-side request forgery,
path traversal, injection into a real sink, cross-tenant exposure, sensitive state
changes without enforcement, and sandbox or trust-boundary escape. Generic calls
for more validation without a path, and maintainability complaints, are not leads.

For each hypothesis record the actor, prerequisites, source, control semantics,
sink, violated invariant, narrow impact, exact hashed locations, and proof gaps.
Preserve distinct affected occurrences even when they share a root cause. Give
each call site of a dangerous sink its own source and closest control; keep
independently reachable operations, handlers and concrete implementations as
separate candidates, and split a candidate whose sink, control or impact differs.
A claim about "every" implementation must name the implementations. A deprecated,
opt-in or documented-dangerous API is a precondition, not a reason to drop a lead.
Do not substitute a nearby different weakness for the original claim.

In diff and changed reviews, cover every changed file, including files deleted
from the baseline side, and look for controls the change removed or weakened.
Stay anchored to the change and the supporting source it relies on; commit
messages and titles are claims, not evidence. When a change alters a shared
helper, guard or sink wrapper, follow it to the call sites it now affects and keep
each vulnerable instance addressable. Continue until no further distinct
plausible candidates remain within budget, and mark untouched units deferred.
