# Source Specification — 12-graph-save-reuse

**Original supplied file:** `12-graph-save-reuse.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
/graph Save and Reuse
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Allow a successful engineering graph configuration to be saved and reused without turning /graph into a general-purpose dynamic workflow language.
/graph save security-audit
Saved validated graph:
  .davinci/graphs/security-audit.yaml
Run again:
  /graph run security-audit
Saved content includes topology, roles, artifact contracts,
budgets, verification policy, and safe configurable parameters.
What is not saved
Do not persist transient chain-of-thought, hidden reasoning, or every ad-hoc worker prompt. Save the validated graph definition and explicit execution configuration.
```
