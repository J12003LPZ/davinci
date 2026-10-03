# Source Specification — 07-live-task-agent-control-panel

**Original supplied file:** `07-live-task-agent-control-panel.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Live Task and Agent Control Panel
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Provide an inspectable control surface for every active worker: what it is doing, what it owns, what it is waiting on, and how to intervene safely.
Agents ──────────────────────────────────────────────────────────
● permission-engine     implementing classifier      00:42
  owns: crates/davinci-agent/src/permission*.rs
  tools: 18 · tokens: 24k · status: working
● mode-keyboard-ui      wiring Shift+Tab             00:31
  owns: crates/davinci-tui/src/*
  tools: 11 · tokens: 16k · status: working
○ host-review           waiting on implementation
[Enter] Inspect   [s] Steer   [x] Stop   [r] Retry   [d] Diff
Required controls
Steer one active worker without restarting the entire task.
Stop one worker or the task-owned process tree.
Inspect recent tool calls and owned changes.
Show whether a new user message redirects active work or is queued for after the current boundary.
Enforce write ownership rather than relying on prompt instructions alone.
Require explicit handoff when ownership changes.
```
