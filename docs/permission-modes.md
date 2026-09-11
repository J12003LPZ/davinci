# Permission modes and living plans

## Keyboard and commands

At an idle composer, **Shift+Tab** cycles the modes below. Ctrl+Tab is also accepted as an alternate binding where the terminal forwards it:

**Manual -> Accept Edits -> Plan Mode -> Auto Mode -> Always Approve -> Manual**.

The native terminal and legacy terminal route the key to the agent's authoritative permission state. The UI does not optimistically change its label. Dialogs, approval prompts, and running turns retain ownership of input; release/repeat events do not advance the mode. The draft, cursor and completion state are preserved. Normal Tab retains its completion behavior. The former thinking-level shortcut moves to **Alt+Shift+T**, avoiding the existing Alt+T tool-output shortcut.

`/permissions` opens the native picker; `/permissions <mode>` selects a mode for the session. CLI `--permission-mode` and the existing `permissions.mode` settings accept human-readable names and compatibility aliases. Stable serialized IDs remain `ask`, `edits`, `read-only`, `auto`, and `always-approve`. The product's unconfigured default remains Manual. The embedding library's no-prompt default is now explicitly Always Approve, preserving its previous behavior rather than redefining Auto as unlimited access. `full-access` and `yolo` aliases resolve to Always Approve.

## Policy semantics

| Mode | Normal behavior |
| --- | --- |
| Manual | Classified safe reads proceed. Implementation edits, shell execution and other meaningful side effects require approval unless a matching user grant exists. |
| Accept Edits | Ordinary, non-destructive workspace file creation/modification proceeds. Sensitive configuration, credentials, deletes, outside-workspace targets and other side effects still require approval. |
| Plan Mode | Classified reads and planning metadata operations proceed. Implementation mutations and shell execution are denied before saved or session allow rules are considered. |
| Auto Mode | Ordinary edits and a deliberately conservative set of literal local commands proceed. Unknown commands, network/destructive actions, sensitive paths, command expansion and wider scope require approval. Cargo build/check/test commands require offline/frozen arguments for automatic approval. |
| Always Approve | Harness approval prompts are bypassed. Explicit deny rules and configured hard filesystem boundaries still apply. The status indicator visibly warns that prompts are disabled. |

This is an **approval policy, not an OS sandbox**. Allowed project checks can execute project code. MCP read-only annotations are declarations from configured servers, not proof of remote behavior. No mode provides missing credentials, overrides OS permissions, or defeats external platform restrictions. User-entered control commands are separate from model tools.

The policy classifies every patch target separately, including deletion actions; a harmless target cannot conceal a secret or outside-workspace target in a compound display string. Path checks include protected harness settings, Git metadata, common credential paths, Windows aliases and existing symlink ancestors. Deny rules take precedence over grants. Subagent profiles cannot request a mode more permissive than their parent's mode. Existing graph subprocesses explicitly request the former non-interactive behavior using `always-approve`, retaining their separate role-tool and shell-policy guards.

## Approval prompts

In print mode, an action requiring approval stops the turn and exits with status 1. Standard output contains an `approval_required` JSON object with the tool call ID, action name, redacted target, permission mode, and resolved global settings path. With `--mode json`, this object follows the normal JSON event stream. Approve the action in an interactive host or configure a narrow `permissions.allow` rule; explicit denies still win. Print mode never records consent, waits for an approval response, or processes later supplied prompts after this result.

The native permission panel offers only choices allowed by the policy. Arrow keys or numbers focus a choice; plain Enter confirms it. Escape denies the call. Mode shortcuts, paste and arriving voice transcripts cannot change the conversation draft while the panel is open.

When offered, **deny with instructions** opens a separate editor. Type what the model should do instead, then press Enter to confirm the denial. Instructions must be non-empty and fit within 4,096 UTF-8 bytes. Escape discards the instructions and denies; Ctrl+C also interrupts the turn. This choice never runs the declined call or saves a permission grant.

RPC clients receive the same denial option through the existing `select`, `input` and `confirm` dialogs. Instructions are attached only after an affirmative confirmation. Cancellation, invalid text, an expired challenge or changed policy denies without attaching the instructions. These approval additions are source-branch changes; the historical installed-build record below does not validate their installation.

## Living Plan Mode

`/plan` enters Plan Mode. The model uses **propose_plan** for a structured, revision-checked plan, while the legacy `update_plan`/todo checklist remains compatible. A plan records the goal, repository evidence and file fingerprints, assumptions, unresolved questions, and stable step IDs. Each step describes its change, files, reason, dependencies and verification. Targeted updates preserve unaffected steps and decisions; revisions invalidate obsolete approval.

Commands:

- `/plan show` and `/plan diff`: inspect the current plan and revision changes.
- `/plan edit <id> <text>`: revise one step and return to Plan Mode.
- `/plan accept <id>` or `/plan reject <id>`: accept/reject a decision (`all` is supported).
- `/plan approve`: approve the complete, fresh plan without changing execution mode.
- `/plan accept [mode]`: approve and select an execution mode. A blank mode uses the previous safe execution mode, never implicitly Always Approve. Explicit Always Approve remains available.
- `/act`: leave Plan Mode without treating the plan as approved.

Handoff rejects incomplete plans, unresolved questions, rejected decisions and changed/unavailable evidence. Plan acceptance is a user command, not a tool the model can call to approve itself. Mode selection alone does not approve a plan or submit an implementation prompt. Re-entering Plan Mode invalidates blanket approval.

Planning state uses session custom entries, not implementation files in the repository. Restoring a stored plan preserves its content and per-step decisions but clears blanket execution consent and selects Plan Mode. Stored records are bounded and validated. Persistence failure is reported and cannot be represented as a successful approval/handoff. Normal permission defaults are configuration-driven; cycling does not silently rewrite global settings.

## Architecture

`crates/davinci-agent/src/permission.rs` owns mode names, aliases, transitions and verdicts; `permission_risk.rs` contains conservative target/command classification. `Agent::set_permission_mode` is the host transition entry point and synchronizes the native Plan prompt with policy state. `living_plan.rs` is the single structured planning store; `planning.rs` adds user commands, revision presentation and session integration. `tools.rs` and `turn.rs` connect tool execution and persistence.

`crates/davinci-tui/src/keybindings.rs`, `interaction.rs`, `session.rs` and `davinci/` own input decoding and presentation. `crates/davinci-coding-agent/src/davinci_interactive.rs`, `permissions.rs` and `main.rs` connect those events to the actual agent. Extension shortcuts reserve the permission-cycle binding. There is no separate mutable UI Plan boolean.

## Reference evidence and intentional differences

Inspected public reference snapshots:

**OpenAI Codex:** `6fee98cc85a69ee44a856121cac0f64505cf48d0`.

- `codex-rs/protocol/src/protocol.rs`: `AskForApproval` and `SandboxPolicy` are separate concepts. Never asking does not remove OS sandboxing.
- `codex-rs/utils/approval-presets/src/lib.rs::builtin_approval_presets`: read-only/default/full-access presets compose approval and sandbox choices.
- `codex-rs/tui/src/chatwidget/interaction.rs::handle_key_event`: Shift+Tab collaboration cycling respects idle state and modal ownership.
- `codex-rs/collaboration-mode-templates/templates/plan.md`: Plan collaboration differs from the `update_plan` checklist and describes complete proposed-plan replacement.
- `codex-rs/tui/src/chatwidget/plan_implementation.rs::selection_view_params` and `chatwidget/tests/plan_mode.rs`: explicit Plan-to-Default handoff choices, including retaining or clearing context.

**Anthropic Claude Code:** `ab9b2cf7bb9e4f98ff264c07a22e46d83c29c558`.

The public repository does **not** expose the core CLI permission engine, keyboard dispatcher or built-in Plan implementation. `CHANGELOG.md` supplies release-level evidence about Shift+Tab, edit acceptance, Plan transitions and Auto edge cases; it is not a substitute for hidden source. Concrete available source includes `plugins/hookify/core/rule_engine.py::RuleEngine.evaluate_rules` (plugin block/deny precedence) and `plugins/feature-dev/commands/feature-dev.md` (plugin exploration and implementation-approval workflow). Neither was mistaken for the core mode implementation.

Davinci intentionally cycles the user's five requested permission modes rather than copying Codex's Plan/Default collaboration cycle. Its planning ledger supports targeted revisions, stable decisions, evidence freshness and resumable state rather than relying solely on whole-plan Markdown replacement. Its shell-denying Plan policy is deliberately stricter than a prompt that permits exploratory builds. These are concrete design differences, not a benchmark claim that plan quality exceeds every reference implementation.

## Verification and installed build

Final focused verification: agent library 365 passed; terminal UI library 577 passed with one ignored diagnostic test; interactive host 85 passed. The full CLI binary suite also completed with 811 passed and one ignored test before the final focused regressions were rerun. The learning-review fixture requires PI_LEARNING_DISABLE_BACKGROUND=0 for that full CLI run; provider/startup network remained disabled. No live-provider request or physical interactive-terminal session was used to claim these results.

Regression coverage includes the five-mode cycle, default Shift+Tab hint, preservation of drafts and completions, modal/running/repeat ownership, session-restored mode labels, Plan mutation denial despite stored grants, protected and compound patch targets, scoped Auto escalation, plan revision freshness, user-only approval, persistence rollback, and post-tool-hook rejection before any plan/session commit.

`cargo build -p davinci-coding-agent --release --offline` and Cargo's offline/locked installation both succeeded. Release `--help` smoke testing succeeded through `cargo run`, showing Shift+Tab and all five modes. The installed `C:\Users\sergi\.cargo\bin\davinci.exe` matched the release executable byte-for-byte by SHA-256:

`76aaee49e58e05b76583205f29be1d261f01013bb06d28be1bc6973eefa1427c`

The previous installed binary was preserved as `target/mode-install-backups/davinci.before-five-modes.87f38bae.bak` (SHA-256 `511151cdb034bb4827e37db98b477af283886e91f6d050c1cd5ee1c4f83d28a6`). Existing running sessions were not terminated. Source changes remain uncommitted alongside the pre-existing workspace edits.
