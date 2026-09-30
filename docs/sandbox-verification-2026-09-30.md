# Phase 4 sandbox verification, 2026-09-30

Status: partial implementation with mandatory capability fences. This is not completion of Phase 4.1 or the macOS part of Phase 4.2.

## Implemented behavior

The partial Seatbelt backend generates deny-default profiles from typed workspace/runtime/hidden-path policy, protects existing and absent workspace control roots, excludes `/System/Volumes` from a broad OS runtime grant, clears ambient secrets, and maps HOME/temp to an exclusive launch directory. Relocated mounts and unsupported resources fail closed. No dependency was added.

Seatbelt cannot mediate `setsid` or `setpgid`, and the current Unix execution plane kills a process group. A detached descendant can survive. The backend therefore reports process-tree isolation, deterministic teardown, tree timeout, and lifetime-private temp as unavailable. Session settings retain these required capabilities and cannot launch through Seatbelt. Native Auto remains disabled. The native fixture deliberately uses a reduced-capability policy to test filesystem/environment/network controls and demonstrate the detached-descendant gap; product settings never drop the requirements.

Linux Auto defaults to workspace-write with denied network only after mandatory-capability negotiation and a bounded execution probe of the trusted host bubblewrap. Startup, public permission-mode changes, mode cycling, and plan exit share activation. Explicit user policy wins, project policy only narrows, and approval decisions remain unchanged. The active boundary persists for the session. Existing project/plugin MCP launched through raw host stdio prevents an in-place Auto default: reconnecting would kill only the direct server child. Status asks for a restart directly in Auto, which starts those servers through the executor. Explicit user host MCP authority is preserved.

JavaScript and extension-command raw spawn attempts enter a monotonic host history. A live Auto activation guard checks that history before installing a default, so later reload, failed startup and dropped sessions cannot bypass it. Every persistent JS pool dispatch also checks the active boundary. A process-isolated real warmed Node runner fixture proves both truthful default refusal and refusal to dispatch through an existing pool after explicit activation.

Early startup uses the same CLI parser and trust override as final agent construction, including repeated permission/execution flags and help-time extension discovery. Invalid future Auto narrowing does not change an existing Ask/Edits session; that future boundary is unavailable.

WSL2 with usable bubblewrap is the documented Windows confinement path. Native Windows Job Objects do not claim filesystem/network isolation.

## Local verification

Environment: Linux, repository Rust 1.83, locked offline Cargo, one build job, isolated `/workspace/sandbox-target`, debug info and incremental compilation disabled. No paid/provider calls. The managed execution environment synthesizes read-only `.git` roots under `/tmp` and `/workspace`; full-suite fixtures used an authorized clean `/var/tmp` root. Logs are under `/workspace/readiness-evidence/sandbox/` in this execution workspace, not repository artifacts.

| Check | Observed result | Log |
| --- | --- | --- |
| Agent full library before final activation guard/lint refactors | 1,280 passed | `final-library-tests.log` |
| Protocol full library | 22 passed | `final-protocol-tests.log` |
| Coding full library | 1,370 passed, 5 failed, 17 ignored before fixture corrections | `final-library-tests.log` |
| Coding CLI suite before final JS transition guard/future-policy adjustment | 432 passed, 8 ignored | `final-cli-tests.log` |
| CLI production and fixture builds | Passed | `final-cli-build.log`, `fixture-cli-build.log` |
| Security CLI corrected fixtures | 3 passed | `security-cli-rerun.log` |
| Security RPC corrected fixture | 1 passed | `security-rpc-rerun.log` |
| Analyzer corrected temp-path fixture | 1 passed | `analyzer-rerun.log` |
| Existing LSP descendant check, isolated | Failed; same failure in sibling baseline worktree | `lsp-rerun.log`, sibling `rewind-process-final.log` |
| macOS test source, Linux type check with temporary Unix cfg | Passed, compile only | `native-fixture-typecheck.log` |
| Focused sandbox policy | 36 passed | `final-policy-tests.log` |
| Supervisor lifecycle | 17 passed | `final-runtime-tests.log` |
| Auto API/approval/late activation guard | 6 passed | `final-runtime-tests.log` |
| Sandbox settings and capability eligibility | 15 passed | `final-settings-tests.log` |
| Repeated CLI flags/trust | 1 passed | `final-parser-tests.log` |
| Legacy MCP transition blocker | 1 passed | `final-mcp-tests.log` |
| Warmed JS dispatch/activation/reservation race | 2 passed, including isolated real runner and two-thread race | `final-js-tests.log` |
| Formatting and strict touched-crate clippy, all targets | Passed, including temporary compile-only Unix fixture | `final-format.log`, `final-clippy.log` |

Three full-library failures came from building the CLI without `test-fixtures`. One analyzer assertion rejects the substring `fix` anywhere in argv, including the original temporary root `davinci-sandbox-fixtures`; it passed with `davinci-sandbox-tmp`. No unrelated source or test was changed to make these reruns pass. The LSP test checks descendant existence with signal zero and remains an existing environment/platform failure; this record does not call the full coding suite green.

The final focused policy, supervisor/Auto, warmed JS transition, settings, MCP provenance, CLI parsing, formatting and strict clippy results are recorded in `final-policy-tests.log`, `final-runtime-tests.log`, `final-js-tests.log`, `final-settings-tests.log`, `final-mcp-tests.log`, `final-parser-tests.log`, `final-format.log`, and `final-clippy.log`. Integration must rerun checks after resolving shared Agent/main changes.

Native Auto selection additionally checks Seatbelt's mandatory capabilities before choosing it, preserving an explicitly configured container fallback. The final small selection guard was separately checked with strict protocol/agent library clippy (`final-native-selection-clippy.log`); its macOS execution awaits native CI. The temporary Unix diagnostic source is removed and is never tracked or executed.

## Outstanding acceptance evidence

- Required native execution is complete: `cargo test -p davinci-agent --locked --test sandbox_seatbelt` passed 10 tests, zero failures/ignores, in 0.97s at source `4622966215f62b7bd1f8072863dce1030f0c2826`. The [complete macOS job](https://github.com/J12003LPZ/davinci/actions/runs/36701485168/job/109841707224) passed on macOS 26.6.2 arm64. The fixtures cover protected/hidden paths, symlink/hardlink escapes, ambient secrets, IPv4/IPv6/UDP/Unix-socket controls, parallel temp separation, group-member cleanup, detached escape, and mandatory ownership rejection. Eight startup controls cover true/shell/Rust and pipe/file stdio. No other OS version/architecture claim is made.
- Implement and prove macOS ownership of detached descendants before advertising the missing capabilities or enabling native Auto. Process-group killing and a deny-default Seatbelt profile do not establish this property. Apple's XNU handlers in `bsd/kern/kern_prot.c` (`setsid`, `setpgid`) and `bsd/kern/kern_proc.c` (`enterpgrp`) have no Seatbelt mediation; this source-path inference was reviewed locally, not executed on macOS here.
- Measure paired dev-split prompt count, success, and wall time before accepting the Auto default as an improvement. No private corpus, live budget, or performance evidence was supplied; no superiority or non-regression claim is made.

Review findings and reproduction evidence are in the parent workspace's `sandbox-cold-review.md`, `sandbox-mcp-drop.log`, and `sandbox-js-transition.log`. Parent integration owns the required CI wiring and immutable final review. No push or release is performed by this builder.

Native crash/kernel evidence at `b01e21ab` identified a `file-read-data /` denial in dyld CacheFinder before main. The independently reviewed fix `34a2ff65` grants data access only to the literal root directory, permitting root directory entry enumeration; it adds no descendant, write or network grant. Root profile tests passed 8/8, strict Agent library lint passed, and the native result above confirms startup and the original enforcement assertions. Whole-tree ownership limitations remain unchanged. Detailed native log SHA-256: `9efac372d397a329cc37fc38e672d966bff30ac42ab25e02898dd0e88d4844f8`.
