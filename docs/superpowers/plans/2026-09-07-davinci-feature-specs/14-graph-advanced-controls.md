# Source Specification — 14-graph-advanced-controls

**Original supplied file:** `14-graph-advanced-controls.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
/graph Diff, Fork, Rewind, Explain, Dry-run and Verify
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Add engineering-specific control commands around the existing graph model. These increase debuggability and reuse without introducing arbitrary orchestration scripts.
/graph diff
Compare the original graph or prior revision with the current validated graph/state.
/graph fork <node>
Retry from a selected node with a different bounded strategy while preserving earlier evidence.
/graph rewind <node>
Restore task-owned state to the checkpoint before a selected writer node.
/graph explain
Show why each node exists, its dependencies, permissions, artifacts, and exit criteria.
/graph dry-run
Validate topology, budgets, scopes, worker roles, and permissions without executing workers.
/graph verify
Run only the verification/review portion against the current source tree.
/graph budget
Inspect or adjust authorized task-level resource ceilings.
/graph export
Export the validated declarative graph definition for review or reuse.
/graph dry-run security-audit
✓ DAG valid
✓ 6 worker nodes · max concurrency 3
✓ All writer scopes are disjoint
✓ Reviewer cannot approve own mutation
✓ Estimated verification commands are permitted
! Network researcher requires approval in Manual mode
No workers were started.
```
