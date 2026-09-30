# Production harness readiness evidence

This branch implements the engineering portions of the [authoritative plan](production-harness-readiness-plan.md), based on `2e52d0f68f9cc989fc69a6114409e237e1990158`. The [requirement ledger](requirements-evidence.json) retains every checkbox and its source line. It distinguishes code verification from acceptance evidence. The product is not certified as better than Codex or Claude Code.

The work is isolated in the cloud branch `codex/production-readiness-cloud-20260930`, reviewed in [draft PR #74](https://github.com/J12003LPZ/davinci/pull/74). No release tag, installation, deployment, private campaign or paid model call was made. Copilot's quota-blocked review is not independent review evidence.

## Coverage and acceptance

| Task | Engineering result | Outstanding acceptance evidence |
| --- | --- | --- |
| 0.1 Windows failure | Two full parallel native runs for PR branch head `d6c2ab6c82f4a4a0a1446da37dedebee40e52b15` (GitHub synthetic merge checkout): each 1,385 passed, 0 failed, 17 existing ignored. The graph test passed both. [Actions evidence](https://github.com/J12003LPZ/davinci/actions/runs/36687936657). | Original baseline graph failure did not reproduce; its cause is unproven. Green PR checks do not make main green. |
| 0.2 Release identity | Push-CI proof, every required shard, immutable source/binary hashes, tag and clean-tree checks, staged install, manual green-tag workflow. [Release discipline](release-discipline.md). | No tag or release created; final source CI must pass before release. |
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
| 4.1 Seatbelt | Profile generation and filesystem/environment/network fixtures; capabilities restricted to enforcement actually available. | macOS native CI. Seatbelt does not prevent process-group escape, so deterministic whole-tree teardown is unsupported. |
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

Cloud limitations are preserved as failures or skips: the Python process runner cannot establish host ownership from this container's PID namespace; native Windows and macOS execution need CI; an existing LSP orphan fixture sees unreaped zombie descendants owned by container PID 1. No test is disabled or timeout raised to hide these facts. No public hidden assertion was used to tune behavior.

The repository includes all source and reproducible tests. Transient detailed logs and critic reproductions live in `/workspace/readiness-evidence/` in this cloud workspace. CI links provide durable platform results. Synthetic fixture successes establish gate behavior, never product success rates.
