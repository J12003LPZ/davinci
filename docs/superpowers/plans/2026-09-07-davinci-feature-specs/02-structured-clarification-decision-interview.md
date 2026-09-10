# Source Specification — 02-structured-clarification-decision-interview

**Original supplied file:** `02-structured-clarification-decision-interview.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Structured Clarification / Decision Interview
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Add a first-class ask_user_question interaction for material ambiguity. The agent investigates first, then asks only when an unresolved choice can materially change architecture, scope, behavior, permissions, cost, persistence, or an irreversible decision.
Behavior
Questions are structured, not merely open-ended.
Offer 2-4 concrete options when possible, with a concise explanation for each.
One option may be marked Recommended, but recommendation is never treated as user consent.
Always include a custom-answer path when appropriate.
Capture why the question is material and the repository evidence that led to it.
Store the user answer as explicit decision state in the Living Plan.
Reference UI
Reference: structured planning question with a recommended option, supplied by the user.
Davinci UI
┌ Decision required ─────────────────────────────────────────────┐
│ Cache scope                                                   │
│                                                               │
│ Which caching strategy should this feature use?               │
│                                                               │
│ › 1. In-memory LRU (Recommended)                              │
│      No new infrastructure; best for process-local workload.  │
│   2. SQLite                                                   │
│      Persistent across restarts; adds disk I/O.               │
│   3. Redis                                                    │
│      Shared cache; adds external infrastructure.               │
│   4. Type something                                           │
│                                                               │
│ Why I am asking: persistence semantics change materially.     │
│ Evidence: SQLite exists; no Redis config is present.          │
└───────────────────────────────────────────────────────────────┘
Internal tool
ask_user_question
Decision states
Open, AnsweredByUser, Deferred, Cancelled.
Critical rule
A model recommendation never becomes AnsweredByUser unless the user explicitly selects it.
```
