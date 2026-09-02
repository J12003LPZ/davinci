# HANDOFF — competitive harness, phases 1 and 2 landed

## Goal

Turn the Rust `pi` rewrite into a harness that competes with Claude Code and
Codex CLI on engineering and interface. The roadmap is
`docs/superpowers/specs/2026-09-01-competitive-harness-roadmap.md`. Phase 1
("turns that are real", commit `2d7162e`) and phase 2 ("trust and control",
spec `docs/superpowers/specs/2026-09-01-trust-and-control-design.md`, plan
`docs/superpowers/plans/2026-09-01-trust-and-control.md`) are done; phases
3–6 are listed in the roadmap and not started.

## State (2026-09-01)

- Branch `rust-rewrite`. `cargo fmt`, `cargo clippy --workspace --all-targets
  -- -D warnings` and `cargo test --workspace` are green (one Node test,
  `js_host::tests::live_autocomplete_queries_get_suggestions_with_prefix`, is
  flaky under the full parallel run and passes alone — it shares the
  persistent Node host with its neighbours).
- Release binary copied to `~/.cargo/bin/pi.exe` and `~/.cargo/bin/davinci.exe`
  (memory note: `cargo install` does not work in this repo).

## What phase 2 changed

- `pi-agent/src/permission.rs` (was dead code, now declared): modes
  `read-only | ask | edits | auto`, rules `tool` / `tool(glob)`, the
  decision order (deny rule → auto → allow rule → read tools → read-only
  refuses → mode table), `ToolApprovalRequest` / `ToolApprovalDecision` /
  `ToolApprover`, and `session_rule_for` (`git status --short` →
  `bash(git status *)`).
- `pi-agent/src/turn.rs`: the gate sits in `execute_one` after the extension
  `tool_call` hook and the unknown-tool check. `Agent.permissions` is an
  `Arc<Mutex<PermissionPolicy>>`, `Agent.approver` an optional callback. The
  library default is `auto` (vendor behaviour); `build_agent` installs the
  configured policy, default `ask`.
- `pi-coding-agent/src/permissions.rs`: `PermissionSources` (user file +
  trusted project file, unioned), `policy_for`, `remember_project_rule`
  (appends to `.pi/settings.json` as JSON, keeps unknown keys), `describe`.
  `settings.rs` gained `permissions: { mode, allow, deny }`. Flags
  `--permission-mode` and `--sandbox` (Codex names) in `args.rs`;
  `PI_PERMISSION_MODE` when neither is given.
- davinci: the `LICENTIA · PERMISSION` panel (`permission_ask`) opens
  mid-turn over the Ask overlay; the worker blocks on a reply channel. Rows:
  once / this session / always here (trusted projects only) / deny. Esc
  denies; ctrl+c denies and interrupts. The ledger row and tool line read
  `awaiting approval` while it waits. `/permissions [mode]` is a davinci-only
  command like `/diff`. The header shows ` · auto` only in auto mode.
- RPC: the approver emits the `select` UI request the JS waiter already
  speaks (`rpc_approval_call`); legacy chrome: extension confirm dialog, yes
  = once, no = deny; `--print` / json: no approver, ask → deny with a message
  naming the flag and the rule to add; `--verbose` prints one stderr line per
  deny.

## Verified

- Unit tests across `pi-agent` (matcher, decision table, gate through
  `run_loop`), `pi-coding-agent` (sources, persistence, flags, panel copy,
  key routing, RPC mapping, the offline fixture) and `pi-tui` (header).
- Live, headless (ConPTY harness `approve.py` / `perms_idle.py` in the
  session scratchpad; recipe in the memory note "driving the davinci TUI
  headlessly"), with `PI_OFFLINE=1 PI_OFFLINE_TOOL_CALL='{"name":"bash",
  "arguments":{"command":"git status --short"}}'` scripting the tool call
  because the Codex usage limit was reached:
  - the `LICENTIA · PERMISSION` panel opens mid-turn with four rows and the
    call on its note line; `allow once` and `allow for this session` run
    the command (17 lines) and the turn finishes; `deny` and `esc` give the
    model "the user declined"; `always allow here` writes
    `.pi/settings.json` `{"permissions":{"allow":["bash(git status *)"]}}`
    and adds `⎿ ✓ remembered bash(git status *) · .pi/settings.json`.
  - `/permissions` lists the mode and rules; `/permissions auto` switches
    and the header reads `gpt-5.6-luna · minimal · auto`.
  - `pi -p` under `ask` denies with the flag/rule message and `--verbose`
    prints `pi: denied bash (no approver in a --print run)`;
    `--permission-mode auto` runs the command; `--sandbox read-only`
    refuses it without asking.
- Harness lesson: write an arrow's escape sequence to the pty in one
  write. Sent byte by byte, ConPTY delivers a lone Esc, which the panel
  reads as deny.

## Next

Phase 3 in the roadmap ("tools that compete": background shell jobs,
`web_fetch` / `web_search`, todo ledger, collapsible tool output, highlighted
diffs). Before it, consider: a `/permissions` sheet once there is a mockup
for it; rule editing from the panel; hooks answering permission questions
(phase 6).
