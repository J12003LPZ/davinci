# Source Specification — 04-task-level-rewind

**Original supplied file:** `04-task-level-rewind.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Task-level Rewind with Dirty-file Preservation
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Allow the user to restore the repository and/or task state to a previous task checkpoint without silently overwriting edits that existed before the task or changes made afterward.
Core behavior
Checkpoint before substantial task mutations and at verified milestones.
Track the task-owned delta, not merely whole-file snapshots.
Detect pre-existing dirty files and treat them as protected baseline content.
If later manual edits overlap the task delta, show a conflict instead of overwriting.
Separate reversible local effects from non-reversible external effects such as publish/deploy actions.
Support restoring code only, task/conversation state only, or both.
┌ Rewind task ───────────────────────────────────────────────────┐
│ Select checkpoint                                             │
│                                                               │
│ › 1. Before implementation                     19:42          │
│   2. After permission engine tests passed       20:08          │
│   3. Before host integration                    20:31          │
│                                                               │
│ Restore                                                      │
│   [x] Task-owned code changes                                 │
│   [x] Task execution state                                    │
│   [ ] Conversation transcript                                 │
│                                                               │
│ 2 later user edits overlap this checkpoint and need review.   │
└───────────────────────────────────────────────────────────────┘
```
