# Source Specification — 05-task-scoped-execution-contracts

**Original supplied file:** `05-task-scoped-execution-contracts.docx`, from `davinci_feature_specs(1).zip`.

This is a text transcription of the supplied proposed specification, preserved for local traceability. Paragraph and table text follows in source order; embedded reference-image pixels are not reproduced in Markdown. The original DOCX remains the authority for its visual mockups. This source is not an implementation-completion claim.

## Supplied specification text

```text
DAVINCI
Task-scoped Execution Contracts
Standalone feature specification · PROPOSED
Scope: This file contains only this feature from the Davinci harness expansion. Claude-style dynamic workflows are intentionally excluded.
Bind an accepted plan to an enforceable task scope. Permission modes answer what categories of action are allowed; the execution contract answers what this specific task is allowed to touch.
Task contract · Add password reset
Writable scope     src/auth/, tests/auth/
Protected scope    migrations/, deployment/
Dependencies       No new dependencies
External actions   No publish or deployment
Verification       Auth tests + type check
Any scope expansion requires an explicit contract update.
Scope expansion UX
┌ Scope expansion required ─────────────────────────────────────┐
│ Current approach requires a database migration.               │
│                                                               │
│ Existing contract protects: migrations/                       │
│ Requested expansion: allow migrations/2026_09_password.sql    │
│                                                               │
│ › 1. Approve this scope expansion                             │
│   2. Ask agent for an approach inside current scope           │
│   3. Deny and give instructions                               │
└───────────────────────────────────────────────────────────────┘
Always Approve is not unlimited scope
Always Approve should remove harness approval friction, but it should not silently rewrite an accepted task contract or bypass hard filesystem/platform restrictions.
```
