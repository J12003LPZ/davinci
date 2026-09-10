# Source Specification — 09-whole-task-budgets-loop-detection

**Original supplied file:** `09-whole-task-budgets-loop-detection.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Whole-task Budgets and Loop Detection
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Create one shared resource budget across the parent agent, subagents, retries, reviews, and verification. Pair it with a progress watchdog that detects repeated work without new evidence.
Budget ──────────────────────────────────────────────────────────
Tokens        81k / 120k
Elapsed       07:18 / 15:00
Workers       3 / 4 concurrent
Retries       2 / 5
Cost          unknown (provider pricing unavailable)
Progress watchdog
! Same test failure observed across 3 attempts with no new diagnosis.
› 1. Continue with remaining budget
  2. Return to Plan Mode with evidence
  3. Stop and preserve checkpoint
Loop signals
Repeated reads of unchanged files without a new hypothesis.
Retrying the same command with unchanged inputs and equivalent output.
Oscillating edits that reintroduce recently removed code.
Repeated failed tests without a changed diagnosis or experiment.
Child agents recursively consuming independent full budgets.
```
