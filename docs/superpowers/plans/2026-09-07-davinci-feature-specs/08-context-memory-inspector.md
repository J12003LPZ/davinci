# Source Specification — 08-context-memory-inspector

**Original supplied file:** `08-context-memory-inspector.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Context and Memory Inspector
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Expose what information will be sent to the model and why it is present. This turns context/memory mistakes into something the user can inspect and correct instead of repeatedly fighting hidden state.
Context inspector ───────────────────────────────────────────────
  3.2k  System + harness instructions            mandatory
  1.4k  Living Plan rev 7                        pinned
  2.1k  Retrieved code: permission.rs            fresh
  0.7k  Project memory: "uses pnpm"              user-approved
  1.9k  Tool result: cargo test                   stale
Total prepared context: 9.3k tokens
[Enter] Inspect   [p] Pin   [x] Exclude   [r] Refresh evidence
Memory provenance
User-approved decision
Highest confidence; explicitly editable/removable by the user.
Repository fact
Must carry source location and freshness/fingerprint.
Agent inference
Clearly labeled as inference; never silently promoted to durable fact.
Tool evidence
Carries command/tool provenance and staleness state.
Mandatory policy context is not optional
The inspector may let the user exclude irrelevant retrieved context or correct memory, but it must not disable mandatory system, permission, trust, or safety instructions.
```
