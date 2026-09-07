Native security methodology v1: discovery

Inputs: pinned source, selected review units, threat model, and existing leads.
Output: structured candidate hypotheses and per-unit coverage, never final findings.

Trace a concrete lower-trust actor's controllable input through real entry points,
transformations, nearest controls, and consequential sinks. Read callers and shared
checks across files. An API name, secret-shaped example, missing test, or comment
alone does not establish a vulnerability. Dependency advisories are leads until
affected versions, features, and reachable use are established.

For each hypothesis record the actor, prerequisites, source, control semantics,
sink, violated invariant, narrow impact, exact hashed locations, and proof gaps.
Preserve distinct affected occurrences even when they share a root cause. In diff
reviews consider removed controls and baseline evidence. Do not substitute a nearby
different weakness for the original claim. Mark untouched units deferred.
