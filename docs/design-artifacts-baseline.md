# Design artifacts integration baseline

Execution began October 2, 2026, from the user's source-only local snapshot,
copied into the isolated `feat/design-artifacts-01a0ff86` worktree. Remote main
was `f24ed1c744a31ee49c95802f982740fe59d2de6d`. The local snapshot contains
additional audit and subscription-budget work; it is preserved separately
from this feature's changes. No source files in the original folder are edited.

The user authorized implementation and PR delivery, and prohibited subagents.
The supplied documents are the feature contract; their historical planning-only
status does not supersede that authorization.

## Integration owners

| Concern | Existing owner |
| --- | --- |
| Session WAL | `davinci_session::JsonlSession`, its custom entries, writer lock and branch entries |
| Exact evidence | `VerificationEvidenceStore::store_artifact/get_artifact`, full SHA-256 |
| Dispatch authority | `ToolContext`, live mutation authority, active task contract, cancellation |
| Subscription admission | `RootBudget`, session binding and existing agent provider dispatch |
| Process cleanup | `jobs::supervisor`, `interaction_testing::browser_process` |
| Browser origins | `native_extensions::browser`, `browser_backend.js`, `browser_network.js` |
| Agent directory | `davinci_session::default_agent_dir`, host overrides |
| Interactive/CLI | Existing command dispatcher and binary modules |
| Embed | `sdk::AgentSession`; stdio RPC is distinct from CBOR |

The alternate `JsonlStoredSession` is not the active CLI's writer. Design must
use the CLI's session owner, not create a second journal.

## Verification ledger

The implementation plan and specification are preserved in the existing
`docs/superpowers` structure. No task is marked complete until its gate runs.
Live model evaluation, human visual acceptance, other operating systems, and
installed-executable verification must be reported separately from local tests.
