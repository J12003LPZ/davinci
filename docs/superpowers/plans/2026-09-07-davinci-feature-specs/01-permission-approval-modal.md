# Source Specification — 01-permission-approval-modal

**Original supplied file:** `01-permission-approval-modal.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Claude-style Permission Approval Modal
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Replace plain approval prompts with a consistent, bordered terminal modal that explains exactly what the agent wants to do, why approval is required, which mode/policy caused the stop, and which approval scopes are legal.
Trigger
Any action classified by the centralized permission engine as Ask.
Decision source
Permission engine determines risk class and available choices; TUI only renders them.
Keyboard
Up/Down or number keys select, Enter confirms, Esc denies/cancels.
Mode safety
While open, Shift+Tab mode switching and ordinary composer input must not leak through.
Approval scopes
Allow once; allow for session; project-level rule only where policy permits; deny; deny with instructions.
Reference UI
Reference: Claude-style permission prompt supplied by the user.
Davinci UI
┌ Permission ─────────────────────────────────────────────────────┐
│                                                               │
│  Fetch                                                        │
│  https://www.eesel.ai/blog/elevenlabs-pricing                 │
│                                                               │
│  Agent wants to access content from www.eesel.ai              │
│  Why: verify current pricing against the workspace report.    │
│                                                               │
│  › 1. Allow once                                              │
│    2. Allow www.eesel.ai for this session                     │
│    3. Always allow this action in this project                │
│    4. Deny and tell the agent what to do differently          │
│                                                               │
│  Manual · Network access requires approval                    │
└───────────────────────────────────────────────────────────────┘
Risk-sensitive choices
High-risk classes such as destructive commands, secret access, privileged actions, or externally visible operations should not automatically offer persistent "always allow" choices. The policy engine decides which scopes are legal.
```
