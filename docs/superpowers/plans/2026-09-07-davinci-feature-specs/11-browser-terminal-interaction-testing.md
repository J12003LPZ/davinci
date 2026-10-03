# Source Specification — 11-browser-terminal-interaction-testing

**Original supplied file:** `11-browser-terminal-interaction-testing.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Browser and Terminal Interaction Testing
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Add managed interaction testing for the actual user surface. For web tasks, capture browser traces. For Davinci itself, run the TUI in a controlled terminal, send key events, resize it, and inspect rendered state.
Terminal interaction test · permission mode cycling
✓ Launch davinci in PTY
✓ Type unfinished draft
✓ Shift+Tab -> Accept Edits
✓ Shift+Tab -> Plan Mode
✓ Shift+Tab -> Auto Mode
✓ Shift+Tab -> Always Approve
✓ Shift+Tab -> Manual
✓ Draft text unchanged
✓ Normal Tab behavior unchanged
Artifacts: screen frames + event log + process exit status
Web interaction evidence
Functional assertions against the actual browser state.
Screenshots or DOM snapshots for relevant states.
Console errors and failed network requests.
Trace artifacts attached to completion evidence when useful.
```
