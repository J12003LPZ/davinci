# Production harness readiness evidence

The current exported-tree implementation is recorded in [OpenAI harness implementation — 2026-10-02](openai-harness-implementation.md). The branch, CI and benchmark statements below are the historical September 30 record, not evidence for the October 2 source export.

This branch implements the engineering portions of the [authoritative plan](production-harness-readiness-plan.md), based on `2e52d0f68f9cc989fc69a6114409e237e1990158`. The [requirement ledger](requirements-evidence.json) retains every checkbox and its source line. It distinguishes code verification from acceptance evidence. The product is not certified as better than Codex or Claude Code.

The work is isolated in the cloud branch `codex/production-readiness-cloud-20260930`, reviewed in [draft PR #74](https://github.com/J12003LPZ/davinci/pull/74). No release tag, installation, deployment, private campaign or paid model call was made. Copilot's quota-blocked review is not independent review evidence.

## Coverage and acceptance

| Task | Engineering result | Outstanding acceptance evidence |
| --- | --- | --- |
| 0.1 Windows failure | Two full parallel native runs for PR branch head `d6c2ab6c82f4a4a0a1446da37dedebee40e52b15` (GitHub synthetic merge checkout): each 1,385 passed, 0 failed, 17 existing ignored. The graph test passed both. [Actions evidence](https://github.com/J12003LPZ/davinci/actions/runs/36687936657). | Original baseline graph failure did not reproduce; its cause is unproven. Green PR checks do not make main green. |
| 0.2 Release identity | Push-CI proof verified on actual green source CI `46229662`, every required shard, immutable source/binary hashes, tag and clean-tree checks, staged install, manual green-tag workflow. [Release discipline](release-discipline.md). | No tag or release created; final source CI must pass before release. |
| 0.3 Self-update | CLI self-update names the source installers; extension package parity remains intact. | None beyond final checks. |
| 0.4 Freeze | Existing agent loop, hooks, rewind, sandbox broker and bench reused; no dependency added. | Phase 1/2 measurements do not exist, so no new architecture or measured promotion. |
| 1.1 Private tasks | Git-native immutable importer, human-prompt attestation, source/test validation, starter/reference/regression grading, split isolation, provenance and labels. [Evaluation guide](evaluation.md). | Owner must supply 3–5 repositories, human prompts, 40–60 dev and at least 150 holdout tasks. Attestations cannot prove human authorship. |
| 1.2 Metrics | Per-arm/per-size success, regressions, unrelated edits, separate tokens, dated pinned price estimates, all-attempt spend, latency, requests and tools. Missing usage is unknown. | Live print and interactive/RPC measurements. |
| 1.3 Baselines | Existing runners and Claude differential workflow freeze source/model/binary identities before execution. | Same-model/same-effort OpenAI and Claude campaigns; independent grading. |
| 1.4 Protocol | Paired 3-rep counterbalance, size-class and holdout gates, token/latency/unrelated-edit ceilings, clustered bootstrap intervals. | Actual dev/holdout campaigns and owner-approved token ceiling. |
| 2.1 Completion | Once-per-prompt requirement reminder after mutation, bounded deterministic trigger, cached-prefix preservation, reason events/stats, mode parity and session ephemerality. [Completion guide](../completion.md). | Large-task improvement, small-task non-regression, token ceiling and holdout confirmation. |
| 2.2 Change set | Tool mutations plus Git delta from prompt start; new files flagged; no automatic deletion. | Original one DaVinci/three Codex unrelated-edit row artifacts were unavailable; actual-cause review and rate measurement remain pending. |
| 2.3 Preview | Stable/off, Stable/on and Preview/on arms documented and selectable. | No winner selected, modules removed, or prompt promoted without measurements. |
| 2.4 Receipts | Conditional work remains inactive. | Requires traces of fabricated checks before implementation. |
| 3.1 Hooks | In-loop plugin Stop and distinct user completion hooks, three-block cap, continuation flag, tool feedback, current trust/approval checks and sandbox guard. [Recipe](../completion-hooks.md). | Final native and integrated fixtures; no live competitor claim. |
| 3.2 Rewind | Prompt checkpoints, durable preimages, code/conversation/both, native and legacy UI, double-Escape, RPC preview/apply; conflicts refuse user edits, aliases and unreadable replacements. [Guide](../rewind.md). | Final integrated fixtures; shell mutations remain explicitly untracked. |
| 4.1 Seatbelt | Native macOS job at `46229662`: 10 passed, 0 failed, 0 ignored; profile/startup/filesystem/environment/network controls enforced. | Seatbelt does not prevent process-group escape, so deterministic whole-tree teardown is unsupported. Tested macOS 26.6.2 arm64; other versions/architectures unverified. |
| 4.2 Auto | Capable Linux backend defaults to workspace-write/network-denied; unsupported hosts retain permission behavior and disclose enforcement. Existing raw MCP/JS sessions refuse unsafe in-place activation. | macOS Auto cannot meet required ownership capabilities. Dev-split approval/success/latency measurement pending. Native Windows isolation is not claimed; WSL2 is the documented route. |
| 5.1 Background cost | Learning/security-watch off by default, opt-in; separate measured/unknown foreground and background receipts, failed attempts and process/session ownership. [Diagnostics](../session-diagnostics.md). | Interactive/RPC benefit and cost campaign. |
| 5.2 Learning scope | Project/global auto-apply require approval by default; project policy may narrow global policy. | No measured justification for automatic promotion. |
| 5.3 Tool surface | Full stays default; Lean plus tool_search remains selectable. | Both-provider Full/Lean A/B before any default change. |
| 5.4 Measure-or-cut | Complete on/off matrix below; Context VM and JEV stay off. | All live subsystem arms and long-session Context VM set pending. |
| 5.5 Status | Fourteen native status commands folded into /status; internal RPC/sheet handlers retained; read-only /doctor checks observed health and local install identity. | Final integrated checks; a matching local sidecar alone cannot prove release CI. |
| 5.6 Facts | Crate counts and new behavior documented in CLAUDE.md. | Counts refreshed after final integration. |
| 6 Claude adapter | Conditional work remains inactive. | Requires same-Claude-model traces showing a deficit. |
| Definition of done | Controls and evidence plumbing implemented. | Green released tag, private holdout superiority, separate large-task/background results and capable macOS isolation remain unmet. |

## Decisions and measure-or-cut table

Learning background review and security watch are **off by default, opt-in**. Both project and global learned-skill activation require approval by default. Windows sandbox use is documented through WSL2; native Job Object ownership does not imply filesystem/network isolation. The 25% uncached-token ceiling is a proposed gate parameter, not an owner-approved measurement or policy. No live campaign budget was authorized.

| Subsystem or arm | Current choice | Measurement | Promotion action |
| --- | --- | --- | --- |
| Stable/off; Stable/on; Preview/on requirement review | Explicit setting/environment switch; Stable prompt text preserved | Not run | Choose simplest measured winner; use existing promotion process |
| Vector memory and learned-skill injection | Existing controls retained; skill auto-activation requires approval | On/off not run | No efficacy claim |
| Token governor digests | Existing configurable behavior retained | On/off not run | No efficacy claim |
| Learning reviewer and security watch | Off/opt-in | Interactive/RPC on/off not run | Keep off until measured benefit |
| Repo/LSP/package/build/git specialists; graph | Existing controls retained | Each on/off not run | No measured keep/cut decision |
| Full vs Lean+tool_search | Full default; Lean selectable | OpenAI and Claude A/B not run | No default promotion |
| Auto sandbox | Capable backend only, truthful unsupported status | Approval-count/success/wall-time A/B not run | No autonomy success claim |
| Context VM | Off | Dedicated sessions with at least two compactions not run | No default change |
| JEV | Off | Not run | Remains off |

## Validation boundaries

Offline checks use Rust 1.83, exact dependency pins, `DAVINCI_OFFLINE=1` and `PI_OFFLINE=1`. New behavior has fixture regressions and independent code review. Final integrated commands/results are recorded here after all review fixes are merged; worktree-only results do not certify the combined tree.

Local combined-tree checkpoint at `d3ac5e68e2125f3db2c2fd0824bb24b50e13926d`; source/native CI at `4622966215f62b7bd1f8072863dce1030f0c2826`, and final parser checks at `d0d88dd2`:

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | Passed |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed, 3m10s |
| Fixture CLI build (`--features test-fixtures`) | Passed, 1m05s |
| Agent named-file / boundary integration suites | 18 passed, including subscribed-runtime denial and shared-bus host rebinding |
| Agent library after provenance fix | 1,314 passed, including completion snapshot host-boundary assertions |
| CLI output-schema integration suite | 10 passed; named bytes reach the first provider request |
| Python benchmark harness | Final parser fix: 142 passed, 7 existing skips, 2.365s at `d0d88dd2` |
| Python AST / Node checks | 10 / 37 passed on the same unchanged check inputs at `f269335c` |
| Full workspace Rust run | Completed with exit 101: 5,218 passed, 6 failed, 40 ignored across 134 targets. All task-related regressions pass; the six cloud environment failures below remain recorded. |
| Native CI | `b01e21ab` exact-source push CI: 40 successful jobs / one failed macOS job. Kernel/crash evidence identified dyld root-directory denial. [Exact-source push CI `46229662`](https://github.com/J12003LPZ/davinci/actions/runs/36701485168) completed green: all 41 jobs, plus [push workflow lint](https://github.com/J12003LPZ/davinci/actions/runs/36701485101). Seatbelt 10/0/0 and Windows installer parser passed. Final docs/parser commit CI is separate. |

The Ubuntu foreground capture failure led to a deterministic regression for a pre-existing late stdin-close race. The integrated fix preserves queued authoritative exit receipts; its independent review verified the error path remains closed when terminal status is absent. Eighteen supervisor fixtures and the original foreground test passed in the isolated writer checkout; the combined run and subsequent source/native CI passed its regression checks.

Independent review closed the completion/runtime, hook approval, rewind alias/conflict/ownership, sandbox capability, background attempt accounting and evaluation integrity findings. Python closure included 19 adversarial reproduction cases and four synthetic installer scenarios. Reviews name immutable commits and scopes; Copilot's quota refusal is excluded. Subsequent changes need their own affected checks.

Cloud limitations are preserved as failures or skips: the Python process runner cannot establish host ownership from this container's PID namespace; native Windows and macOS execution need CI; an existing LSP orphan fixture sees unreaped zombie descendants owned by container PID 1. No test is disabled or timeout raised to hide these facts. No public hidden assertion was used to tune behavior.

Five unchanged Rust evaluator fixtures also request a bare `python` executable after clearing their environment. This cloud image provides `python3` in the default executable search path; its `python` installation exists only in the custom parent PATH. A temporary conventional alias could not be created (`Permission denied`). The fixtures failed locally with `failed to spawn python`; their source and the evaluator's strict environment allow-list were preserved. Both Ubuntu and Windows evaluator shards passed green source CI for `46229662`.

The repository includes all source and reproducible tests. Transient detailed logs and critic reproductions live in `/workspace/readiness-evidence/` in this cloud workspace. CI links provide durable platform results. Synthetic fixture successes establish gate behavior, never product success rates.

The native startup fix grants `file-read-data` only for the literal `/` directory, with no descendant, write or network grant. It addresses the observed macOS dyld CacheFinder denial; root directory entries can be enumerated. Eight profile fixtures and strict Agent library lint passed after integration, and independent source review found no scoped issue. [Native source-CI verification](https://github.com/J12003LPZ/davinci/actions/runs/36701485168/job/109841707224) includes the unchanged positive and negative enforcement assertions. The Windows installer parser check only parses the installer; it does not build or install a release.

The final audit found and closed a benchmark observation gap: reason-only `completion_reminder` events now count requirement and hook reminders, including subsequent coding requests and mutation generations. Two fixture regressions failed before the fix; independent closure passed five existing/new tests and five additional parser probes. No reminder text is added to saved sessions or benchmark activity records.

Engineering coverage: 59 requirements verified against the recorded code/fixture/native scopes, 33 awaiting external acceptance evidence, five conditional tasks not triggered, and two constrained by unavailable macOS ownership capabilities. All eight global constraints are tracked. These statuses do not certify live product acceptance. The actual 41-job push CI proof was accepted by the release identity validator in a read-only check; no tag, build/install preflight, installation or release was performed.

Validation not performed: paid/model dev or holdout campaigns, independent live grading, actual release/tag/install, and local native Windows/macOS execution. A variant tournament required by repository process was not run; independent code critics reviewed each implementation unit and the final audit found and closed the parser omission. Production readiness remains blocked by the explicitly listed acceptance and capability gaps.
