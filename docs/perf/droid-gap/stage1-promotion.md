# Stage 1 promotion record — controlled Script and cache work

**Status:** Stage 1 executed on branch `feat/droid-controlled-script-parity`
(PR #274). **Default remains `off`/direct.** Controlled Script is opt-in.
Measured Stage 0 facts stay in the preregistered
[stage0-report.md](stage0-report.md); this file does not re-measure them.

**Spec:** [2026-10-31-droid-script-controlled-parity-design.md](../../superpowers/specs/2026-10-31-droid-script-controlled-parity-design.md)
**Plans:** [controlled script](../../superpowers/plans/2026-10-31-droid-controlled-script-implementation.md),
[cache and context](../../superpowers/plans/2026-10-31-droid-cache-and-context-implementation.md),
[parity benchmark](../../superpowers/plans/2026-10-31-droid-parity-benchmark-implementation.md)
**Owner gap plan:** [2026-10-31-droid-gap.md](../../superpowers/plans/2026-10-31-droid-gap.md)
**Crosswalk:** [controlled-script-parity-crosswalk.csv](controlled-script-parity-crosswalk.csv)

## 1. What is proven, and at which level

| Claim | Evidence | Level |
| --- | --- | --- |
| 12 deterministic Droid Script reference scenarios | `reference_script_fixture_suite_passes` + `codemode_reference_fixtures.rs` (05 syntax error, 09 long job, real guest) | `BEHAVIOR_PARITY` (fixtures) |
| Nested Read/Execute, aliases, patch-once, structured exit codes | broker fixtures in `tests/common/codemode.rs` | `BEHAVIOR_PARITY` |
| Mutation authorization, revocation-while-queued, ambiguous effect closes admission, no replay | `queued_mutation_revocation_is_denied`, `unknown_mutation_never_auto_replays`, MCP lost-reply fixture | `BEHAVIOR_PARITY` |
| Detached/background Script lifecycle | `background_script_returns_running_and_can_finish`, `killed_background_script_is_cancelled_and_not_replayed`; owner approval in `config::BACKGROUND_SCRIPTS_APPROVED` | `BEHAVIOR_PARITY` (in-process ownership; no cross-process recovery) |
| Tool discovery never rewrites an existing request prefix | `deferred_tool_load_does_not_rewrite_history` | `BEHAVIOR_PARITY` |
| Cache-drop attribution without a fabricated local cause | `cache_drop_classification_requires_evidence` | `BEHAVIOR_PARITY` |
| Unverified Codex route receives no cache TTL or hosted tool shape | `cache_ttl_not_sent_to_unverified_codex_route`, `unsupported_hosted_tools_fail_closed` | `BEHAVIOR_PARITY` |
| Controlled Script/`wait_for_script` model surface, prompt reference | `controlled_mode_advertises_script_and_wait_tools_with_the_prompt`, `script_reference_reaches_the_stable_prefix_only_when_supplied` | `BEHAVIOR_PARITY` |
| Image output | fails closed `UNSUPPORTED_OUTPUT` | **OPEN parity gap** |
| Task-success or performance parity with Droid | none | **`UNTESTED`** |
| Subscription longevity benefit | none (no allowance window observed) | **`UNKNOWN`** |

## 2. Behavior parity vs performance parity

- **Behavior:** the deterministic surface is proven, including detached run
  ownership. The program-level claim is not made: image output is a declared
  gap and no matched behavioural suite against Droid was run.
- **Performance:** Stage 0 was a 17-task pilot; see
  [stage0-report.md](stage0-report.md) for the measured numbers and their
  stated limits. No matched suite (≥150 scenarios × 3 repetitions) or
  independent allowance measurement was run, and every such claim is refused
  by `scripts/bench/parity_report.py`.
- **Transport confound:** DaVinci runs native WebSocket, Droid through a proxy.
  Any comparison is `product-system` unless that confound is bounded.

## 3. Promotion decision

`rollout_gate(PromotionDecision { behavior_parity: false, security_pass: true,
success_noninferior: false, resource_benefit: false, unknowns: [...] })`
returns **`off`**:

1. Keep `off`/direct default; read-only Codemode unchanged.
2. Controlled Script is opt-in (`--codemode controlled` or the trusted user
   setting) after its recovery gate; never default-enabled.
3. Do not claim Droid parity, performance parity or subscription savings.
4. Rollback stays one setting: `off`.

## 4. Rollback

Set `"codemode": {"mode": "off"}` or remove the key. Later turns no longer
offer or run Codemode. Durable run records under `<evidence dir>/script-runs`
stay in place and are never replayed; an unresolved mutation stays
`recoveryRequired` and must be reconciled from the operation journal. See
[`docs/codemode.md`](../../codemode.md).

## 5. Remaining blockers

- No matched live pilot has produced a result yet; the Droid arm now exists in
  this repository (`scripts/bench/droid_arm.py`, `gap_study.py`), so a bounded
  pilot is a run, not a build.
- Trusted bounded image projection before image parity can be claimed.
- Detached runs have in-process ownership only; a killed host leaves a
  `recoveryRequired` record rather than a resumable run.
