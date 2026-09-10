# Source Specification — 03-execution-tasks-live-task-board

**Original supplied file:** `03-execution-tasks-live-task-board.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Execution Tasks / Live Task Board
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Create a persistent execution checklist separate from the Living Plan. The plan captures intent and design; tasks capture concrete work currently being performed.
Reference UI
Reference: live task/todo progress supplied by the user.
Task model
Fields
id, title, status, owner, dependencies, parent plan step, evidence refs, timestamps.
Statuses
pending, in_progress, completed, blocked, failed, cancelled.
Internal tools
task_create, task_update, task_list, task_get.
Ownership
Only the owning worker/controller may mutate a task unless ownership is explicitly handed off.
Davinci UI
Execution ───────────────────────────────────────────────────────
✓ Fix compilation errors
  verified: cargo check --offline
✓ Add permission classifier
  owner: permission-engine · 18 tests passed
● Wire Shift+Tab mode switching
  owner: mode-keyboard-ui
  editing: crates/davinci-tui/src/input.rs
○ Verify host integration
  depends on: Shift+Tab mode switching
! Release build
  blocked by: host verification
Do not collapse Plan and Tasks
Living Plan = what and why. Tasks = active units of work. Clarifications = explicit user decisions. Evidence = proof that work is valid. Keeping these separate makes state easier to reason about and resume.
```
