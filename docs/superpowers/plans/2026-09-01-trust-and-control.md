# Trust and control — implementation plan

Spec: `docs/superpowers/specs/2026-09-01-trust-and-control-design.md`.
Every task ends with `cargo test -p <crate>` green; the last with `make fmt`,
`make clippy`, `make test`.

## 1. Policy engine (`pi-agent`)

- Rewrite `crates/pi-agent/src/permission.rs`: `PermissionMode`,
  `PermissionRule` (parse `tool` / `tool(pattern)`), glob matcher,
  `PermissionPolicy::decide(tool, args, cwd) -> PermissionVerdict`,
  `ToolApprovalRequest`, `ToolApprovalDecision`, `ToolApprover`,
  `session_rule_for(tool, subject)`. Declare `mod permission` and re-export.
- Tests: matcher, subject normalisation, decision table, deny-wins, rule
  derivation.

## 2. The gate (`pi-agent/src/turn.rs`, `lib.rs`)

- `Agent { permissions: Arc<Mutex<PermissionPolicy>>, approver: Option<ToolApprover> }`.
- `execute_one`: after the `pre_tool` hook, run the gate; `Ask` → approver
  or the non-interactive deny; append session rules on
  `AllowForSession` / `AllowAlways`.
- Tests: counting approver; `read` never asks; `read-only` denies without
  asking; deny result text; no approver text.

## 3. Settings, flags, wiring (`pi-coding-agent`)

- `settings.rs`: `PermissionSettings`, `permissions` field.
- New `permissions.rs`: `policy_for(agent_dir, cwd, override_trust, flag_mode)`,
  `remember_project_rule(cwd, rule)`, `describe(policy)` for `/permissions`.
- `args.rs` + `help.txt`: `--permission-mode`, `--sandbox`; `PI_PERMISSION_MODE`.
- `build_agent`: install the policy.
- Tests: union/untrusted/precedence/persist; flag parsing.

## 4. davinci

- `Question::Permission` → `Ask` copy; `model.permission_mode` for the
  header's ` · auto`.
- `run_turn`: approver channel, overlay routing through `app::handle_key`,
  ledger detail `awaiting approval`, esc = deny, persist on always.
- `/permissions [mode]` davinci-only command; Instrumenta corpus rows.
- Tests: Ask rows; key → decision mapping; ledger text.

## 5. Other surfaces

- RPC approver via `js_host::dispatch_ui_waiter` (made `pub`) with a
  `select` call; map answers.
- Legacy chrome approver via `open_extension_confirm` + a reply channel.
- `--print` / json: `--verbose` stderr line per deny.

## 6. Finish

- fmt, clippy, tests; release build copied to `~/.cargo/bin/{pi,davinci}.exe`.
- Drive one real approval through the ConPTY harness by hand.
- CLAUDE.md (architecture paragraph), HANDOFF.md, memory note. Commit.
