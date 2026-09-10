# Source Specification — 06-evidence-backed-completion

**Original supplied file:** `06-evidence-backed-completion.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Evidence-backed Completion
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Replace generic "done" claims with a completion state backed by concrete evidence tied to the exact source state that was verified.
Completion evidence ─────────────────────────────────────────────
IMPLEMENTATION      ✓ Complete
TARGETED TESTS      ✓ Passed on current source
BUILD               ✓ Passed on current source
INSTALLED APP       ✓ Matches verified build
LIVE UI CHECK       ○ Not performed
REMAINING GAP       Physical keyboard interaction
Source fingerprint  76aaee49…
Last verified       23:04:17
Evidence rules
Record command, exit status, relevant source fingerprint, tool/runtime version where useful, and produced artifact.
If relevant source changes afterward, mark the associated verification stale.
Distinguish implemented, tested, built, installed, and live-verified.
Reserve enough task budget for verification and handoff; do not consume the entire budget on implementation.
Completion cannot silently substitute an earlier test result for current-source verification.
```
