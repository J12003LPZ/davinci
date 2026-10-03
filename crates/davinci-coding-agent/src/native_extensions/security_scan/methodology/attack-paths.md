Native security methodology v2: attack paths and severity

Inputs: source-backed candidate and counterevidence. Output: causal path, supported
prerequisites, impact, severity and confidence rationales, residual caveats.

First decide whether the claim is a security vulnerability at all, rather than a
correctness defect or a false positive, and whether the affected code belongs to a
shipped product surface or production workflow in the mapped threat model. Test,
example and tooling code is not a product finding unless a runtime path uses it.

Connect actor to entry, transformations, closest checks, crossed boundary, and
specific asset impact. Separate observed facts from deployment assumptions.
Establish exposure, identity and privilege, trust boundaries, sensitive data or
credential flow, reachability and existing mitigations from source, not intuition.
Before settling exposure, authentication scope, boundary crossing, preconditions
and impact surface, name the strongest source evidence that the path is internal
only, administrator only, not cross-boundary or unreachable, and say why it does
or does not defeat the claim. Absent public-ingress or deployment evidence lowers
confidence; on its own it is not a reason to suppress. Never invent a chain the
code does not support, and keep the root control's exact location even when a
wrapper or route is easier to explain. A cleaner neighbouring finding does not
replace this row: assess it or record why it fails.

Keep three steps apart: the factual path, then severity calibration, then the
reportability decision applied from those settled facts without re-arguing them.
Critical requires demonstrated extreme or system-wide impact, such as
unauthenticated remote code execution or broad cross-tenant compromise. High
requires a substantial authorization or isolation breach, sensitive disclosure,
credential theft or execution. Medium needs meaningful preconditions or exposes
narrower data or actions. Low is minor impact or defence in depth; informational
is hardening only. Confidence is high, medium, or low according to evidence
strength, separately from severity. Do not invent numeric probabilities or CVSS
vectors. CWE is optional. Source proof does not establish released affected
versions, production prevalence, or online credential validity.
