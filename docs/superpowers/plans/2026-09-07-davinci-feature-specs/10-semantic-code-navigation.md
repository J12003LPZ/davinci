# Source Specification — 10-semantic-code-navigation

**Original supplied file:** `10-semantic-code-navigation.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Semantic Code Navigation
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Add language-aware navigation so the agent can operate on symbols and diagnostics rather than depending only on text search.
Capabilities
Go to definition, find references, symbol outline, diagnostics, rename preview, call hierarchy where supported.
Fallback
Ordinary file/text search remains available when semantic support is unavailable or incomplete.
Safety
A language-server edit is evidence, not proof of semantic correctness; normal verification still applies.
Symbol: Agent::cycle_permission_mode
Definition   crates/davinci-agent/src/lib.rs:742
References   8
  davinci_interactive.rs:418
  main.rs:1261
  model.rs:773
  ...
Diagnostics  0 errors · 1 warning
[Enter] Open   [r] References   [n] Rename preview
```
