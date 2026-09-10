# Source Specification — 13-graph-monitoring-intervention

**Original supplied file:** `13-graph-monitoring-intervention.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
/graph Monitoring and Intervention
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Make graph execution observable and controllable from a dedicated terminal view: phases, worker state, time/usage, outputs, dependencies, and intervention controls.
GRAPH · Implement OAuth refresh
────────────────────────────────────────────────────────────────
● Research                 3/3    21k tokens   0:42
● Architecture             1/1     7k tokens   0:19
◉ Implementation           2/4    31k tokens   1:08
○ Verification             0/2
○ Review                   0/1
Budget  59k/120k tokens · 02:09/10:00
↑↓ Select  Enter Inspect  p Pause  x Stop  r Retry  d Diff
Graph runtime controls
Pause/resume the graph at safe execution boundaries.
Stop or retry a selected node without restarting unrelated successful nodes.
Inspect a node prompt contract, tool activity, artifacts, and verification evidence.
Show dependencies and why a node is blocked.
Preserve deterministic worker ownership and bounded fan-out.
```
