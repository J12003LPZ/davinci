# Codemode execution record

Plan: [2026-10-04-codemode.md](2026-10-04-codemode.md)

## Baseline and scope

- Main baseline: `518ce4489275246594978cbbab87b4dae2b40a0b`, reconciled by merge `dabd61ea` on the existing `feat/codemode-sandbox` branch.
- Sole delivery PR: #92. Work uses the disposable local checkout and native Windows fixture execution. CI errors are nonblocking under the user's continuation instruction.
- PR #91 is merged. Its removal of TypeSafe/Jev is preserved.
- Rust remains 1.83.0 with current exact Rust dependency pins. Core defaults, subscription authentication, selected model/effort/tier, and installed binaries remain unchanged
- Only fixture-based execution is authorized. No live provider/subscription probes, merge, release, credentials, or installed-binary changes
- No nested AGENTS.md was found outside the vendored tree; root AGENTS.md applies

## Execution status

| Task | Status | Evidence / gate |
| --- | --- | --- |
| C00 baseline and fixtures | Implemented and tested | 111 focused tests and successful historical native-platform runs |
| C01 dependency admission | Partial | Exact lock recovered; core bundle, trusted manifest and component notices implemented; new adapter platform admission pending |
| C02 bounded host protocol | Partial | Worker collection cap, framing, fresh VM and Rust supervisor implemented; complete lifecycle/protocol gates pending |
| C03 structured MCP | Partial | Typed retention, internal accessor, live schema lookup and guarded child seam implemented; complete delivery acceptance pending |
| C04 guarded child dispatch | Partial | Broker reuses real admission/dispatch/hooks and sole journal; concurrency and all authority gates pending |
| C05 safe script projection | Partial | Trusted delivery hook separates safety from Governor; bounded projection implemented; complete artifact/rehydration gates pending |
| C06 opt-in read-only slice | Implemented; acceptance partial | CLI/user-only settings, SDK host injection and real QuickJS two-child execution passed on native Windows; full surface/recovery/platform gates pending |
| C07 controlled execution | Partial internal dispatch | Guarded mutation and denial fixtures passed; operator-controlled mode remains unavailable pending C08 |
| C08 recovery | Partial groundwork | Broker closes admission on durable unresolved effects; parent lifecycle/recovery gates pending; controlled tool remains unavailable |
| C09 supported surfaces | Partial | CLI, user-only settings and SDK injection implemented; remaining surface gates pending |
| C10 accounting and evidence | Partial groundwork | Durable nested publication and protected status/lineage facts implemented; expanded progress/accounting pending |
| C11 comparative fixtures | Not started | C10 required; live campaign remains unrun |
| C12 platform/release gates | Not started | All earlier gates and independent branch review required |

## Preflight interfaces and rulings

- C01 -> C02: the upstream package buffers output before resolving and exposes no configurable collector byte budget in its public options. Admission must characterize this before production integration; final output truncation alone cannot prove a bounded host
- C02 -> C04: `CodeModeRunContext` is host-owned and must never be deserialized from model/IPC input. The broker identity and wire correlations remain separate
- C03 -> C05: MCP structured data must use an internal host-owned channel; `ToolResult.details` is model/UI-visible and includes reserved host bookkeeping
- C04 -> C07/C08: reuse journal-before-effect ordering and current permission rechecks. No parent lock may be held while awaiting children; no script replay is permitted
- C05 -> C10: reserve host-authored failure/recovery facts before output truncation and the final Governor pass
- Historical ruling: early work used GitHub-only execution. The continuation now uses local fixture execution and treats CI errors as nonblocking; see the current baseline above.
- Ruling: retain this execution record next to the approved plan to preserve progress across remote commits. No alternate roadmap or product scope is introduced

## Actual verification

Historical runs 37223771440 and 37223776122 passed at 4716c00. Current native Windows checks: 47 Node package/adapter/framing/host tests; zero known vulnerabilities from npm audit; 13 Rust contract/policy/projection tests; 18 shared Codemode dispatcher regressions; three structured MCP accessor/delivery tests; nine journal-publication tests; three real supervisor tests (constant, two real guarded children, deadline during child wait); and the Governor lossless regression. Library and coding-agent binary Clippy passed. Trusted bundle v7 contains 36 assets, and all 19 repository-owned staged asset hashes match the manifest. Complete feature and final platform acceptance remain pending.

## Current continuation (supersedes historical baseline and workflow)

- Main 518ce4489275246594978cbbab87b4dae2b40a0b was merged into the existing feat/codemode-sandbox branch. PR #91 removal of TypeSafe/Jev is preserved; PR #92 remains the sole delivery PR.
- Current execution uses a disposable local checkout and native Windows fixture tests. The user directs CI errors to be non-blocking. Local safety checks remain required. No live subscription execution is authorized.
- Work and review remain solo under the user's instruction, superseding skill recommendations for delegated review.
- The required collection adapter checks guest string length before conversion, then enforces a shared UTF-8 text/console/return budget of 1,048,576 bytes. Exhaustion is terminal even when guest code catches it; subsequent capability calls are blocked. Images are rejected before collection. Tests failed against the published worker before passing against the adapter. Explicit read-only opt-in now exposes guarded capabilities; controlled operator mode remains unavailable.
- Packaging refinement announced before implementation: retain the core engine and required notices, omit unused extension binaries, and preserve the original complete platform inventories as admission evidence.
- Rust admission checks trusted manifest identities, streaming hashes, complete inventories, path traversal, workspace overrides, symlinks and Windows reparse points. Tests first failed for the missing module, then all ten passed.
- The build-time packager emits a complete 36-file core inventory outside the runtime bundle. The Rust executable embeds the trusted manifest. Pi, engine, WASI libc components, allocator notices, LLVM and compiler-rt notices are retained with pinned provenance. Runtime never installs npm packages.
- Script serialization builds validated snapshots using captured intrinsics. Sparse arrays, unsupported values, unsafe numbers, accessor properties and cycles fail before dispatch. Proxy getters and guest prototype serializers cannot alter validated arguments. The collector cap is enforced before worker messages retain output.
- The guarded broker validates the host-created workspace/run/parent binding, requires the existing operation journal, freezes capabilities, rejects aliases/duplicates/orchestrators, and enforces call/result/metadata budgets. Child origins use a distinct journal caller and idempotency scope. A projection failure records a failed child with the actual operation reference.
- The shared result owner runs authoritative script hooks on both text and structured channels without model presentation. Main preserves normal Governor delivery; script delivery invalidates native engineering state and skips Governor. Legacy text-only hooks drop independent structured content. Unsupported MCP content returns an explicit incomplete-data error rather than being described as complete text.
- Read-only Codemode is registered through the real dispatcher, exposed by explicit SDK opt-in or --codemode read-only with user-controlled absolute paths. Off ignores unused paths and performs no host probe. Controlled CLI mode is explicitly unavailable until recovery gates pass. C06–C12 are not fully accepted. No final-head Linux/macOS adapter run, live subscription benchmark, installed-binary change, merge or release is claimed.

## C00 first CI observations

- Head `c64e9e15a6f7656f3130846584909a680a62b4cf`: Rust formatting identified two fixture-only line-wrap differences; applied exactly as rustfmt reported
- The added unrestricted `cargo fetch --locked` step fails before tests on an existing optional dependency, `toml_parser 1.1.3+spec-1.1.0`, whose edition-2024 manifest Cargo 1.83 cannot parse. The product uses pinned `toml_edit 0.22.27`; the incompatible parser belongs to another already-locked graph
- Ruling: prepare only the actual tested package/target graph with `cargo test --no-run --locked`, then run the planned `--offline --locked` tests. Do not change Rust pins or lockfile to make an unrelated all-features/all-targets fetch succeed
- Package characterization job is gated on successful C00 baselines across the three native operating systems. Proposed Node identity is 24.21.0, listed as the current maintained LTS on https://nodejs.org/en/about/previous-releases at inspection. No package has been admitted yet

## C00 verified baseline / C03 test-first

- Linux passed 110 focused tests on PR head `817f39ff182a0866806f349242926196cb8f713e` (PR test-merge `6291326`): batch 4, tool ledger 20, MCP 38, new Codemode baseline 3, operation dispatch 11, Governor 34. [Job evidence](https://github.com/J12003LPZ/davinci/actions/runs/37221623922/job/111493006804)
- Format and full-workspace clippy passed at that head. Windows/macOS validation remains ongoing, not assumed
- Added an executable counted-effect/delayed/lost-response MCP transport fixture to finish the reusable C00 corpus; it invokes the actual MCP client and checks that a lost response does not automatically repeat the effect
- C03 may proceed independently of C01/C02, as the approved plan explicitly permits. Added test-first retention cases against the existing decoder/client; production changes follow only after the expected loss-of-structured-data failures are observed

## C03 production wire retention

- Red evidence: [structured MCP regression job](https://github.com/J12003LPZ/davinci/actions/runs/37222884787/job/111496669159) at head `ff326c1283197ede2328110434abe24602261713`: 9 tests ran, 5 failed for missing structured/output-schema fields and 4 passed. These are expected task C03 failures, not a passing suite
- Implemented optional typed `structured_content` / `output_schema` retention at the wire source. `CallToolResult::text()` is unchanged; private `_meta` and forged bookkeeping remain unmodeled
- Repository-wide MCP type-reference search found no other Rust struct literals requiring new fields. Full CI remains the construction/compatibility check
- Internal full-result accessor tests are committed first and intentionally await the new accessor; C03 is not complete until those tests and supervised-delivery characterization pass
- Ruling disclosed before applying: focused C00 is repeated on Linux to avoid three duplicate full Rust compiles on every test-first edit. All original required CI jobs remain unchanged; package admission and final native Windows/macOS/Linux gates remain required. This changes validation scheduling, not supported platforms or feature scope

## C03 accessor implementation

- Wire retention is green: all 9 structured MCP integration tests passed at `2378530d15d1c5cfae76502d87db04369f69b804`. [Observed green run](https://github.com/J12003LPZ/davinci/actions/runs/37223219406/job/111497605416)
- The test-first internal accessor failed exactly because `McpRegistry::call_full` did not exist. [Observed missing-implementation gate](https://github.com/J12003LPZ/davinci/actions/runs/37223216118/job/111497581837)
- Implemented the crate-private full-result seam and made ordinary `call` project it through the unchanged text/error/details contract. RPC tool errors stay errors with bounded caller projection later; broken transports still drop the server. No server field is copied into host bookkeeping
- Internal and supervised-adapter tests, unchanged direct/batch/Governor behavior, and full CI are pending on this implementation commit. This component does not expose Codemode or bypass the dispatch path
- C01 CI now records every installed package asset hash and available license text in addition to the exact lock, runtime, advisory report and sandbox smoke fixtures. An evidence report is not a trusted/admitted runtime manifest

## Read-only vertical slice continuation

- Native Windows real QuickJS fixture returned filtered IDs after two actual reads through Rust admission. One parent and two child journal operations were recorded. Fixture replies made zero provider requests.
- A separately owned host watchdog cancels the script token and kills its host while a cooperative Rust child is waiting; its native fixture passed. The script token is a child of the parent cancellation token. Noncooperative adapter cleanup still requires acceptance evidence.
- Owner settings alone supply mode/nodePath/hostPath; project settings cannot change this configuration. Four CLI/configuration tests passed.
- Mutation fixtures confirmed normal permissions, zero effects on denial, no duplicate replay, and retained durable references for pre-start denials. Controlled mode remains unexposed.
- Bounded Rust serialization now counts bytes without allocating an unbounded JSON buffer. Empty source and option-header directives fail before host launch.

## Read-only safety continuation

- Native Windows real QuickJS execution passed with two durable custom child entries and one provider-visible parent result. The first child-publication assertion failed (zero child entries); nested journal publication now includes Codemode children while preserving existing batch behavior. All nine operation_publication integration tests passed.
- Tool-wide denies now suppress broker search, describe and execution. The initial authorized snapshot remains a ceiling after later permission grants. Regression failed before the discovery fix and passed afterward.
- Presentation hooks cannot turn failed Codemode into success or remove host-authored status and operation lineage; protected facts omit child text and error messages so redactions remain effective. Regression failed before the protection and passed afterward.
- Cached child delivery without exact structured data returns explicit incomplete-data/storage-unavailable status and never reruns the adapter automatically. Regression failed before the change and passed afterward.
- All 17 Codemode baseline regressions passed; the Governor lossless regression including Codemode passed; agent and coding-agent library Clippy passed after these changes. Controlled mode, complete artifact access, bounded concurrent read admission, the crash matrix, expanded accounting and final platform acceptance remain pending.

- Completed parent replay executes the host only once; changed script payload collides. Revoking the parent tool blocks historical result disclosure before journal replay. The revocation assertion failed before the pre-replay check and passed afterward. Scoped child-source access on historical parent replay still requires its acceptance gate.
- Current remote main is e890609bb147030aa1495d86ed3aedfe78d07578 (PR93 microphone removal); reconciled by b2a2ac4e on this same branch. Packaging commit 73302afa preserves exact license bytes and LF runtime scripts; local Git hooks ran using Git Bash after the default Bash resolver selected unavailable WSL. No hook was disabled.

- Local milestones: 73302afa packages the bounded adapter and trusted assets; c9ca0b6c wires read-only execution and guarded journal delivery; b2a2ac4e reconciles current main while preserving PR91 and PR93 removals. After reconciliation, formatting, all 18 dispatcher regressions, and agent/coding-agent library plus CLI Clippy passed. Ten asset-admission and four surface tests passed; all three native supervisor tests passed with the byte-stable v7 bundle. Final-head Linux/macOS and complete C00-C12 acceptance are still pending.

## Historical result authorization binding

- Scoped source revocation initially failed: an already completed parent disclosed its cached result after `read(path=input.txt)` was denied. Cached parent delivery now requires the original permission/boundary/workspace/capability/contract digest to match current Rust authority. Failure or missing binding denies disclosure without redispatch. Only the digest is persisted in the existing operation journal.
- This is a conservative cache-access refinement: any authority change blocks cached data, including unrelated policy changes. It grants no capability, and restoring the identical policy permits retrieval without restarting the script. The refinement was disclosed before implementation.
- Presentation hooks cannot replace or erase this private host binding. All 18 existing focused dispatcher tests passed; a separate disk-reopen regression then passed with a fresh agent and execution owner, checking scoped denial and restored-policy retrieval with exactly one host invocation and three durable operations. This is an offline journal reopen, not the complete process-crash matrix.
- Agent library Clippy with warnings denied and workspace formatting passed. Artifact retrieval authorization, bounded concurrent read admission, controlled-mode exposure and final native platform acceptance remain pending.

## Queued admission cancellation

- Added deterministic production-broker checks for the 65th request and caught terminal errors: later requests cannot create operations or renew the 64-child allowance. Added a queued scoped-revocation check with a blocked pre-hook; both children are denied before effects.
- A cancellation regression failed because the private mutex lane waited for an active callback to return. The lane now uses the existing `WorkerSlotCapacity` cancellation-aware admission primitive with a ceiling of one, without holding parent/agent/journal locks across child waits. The regression passes before releasing the blocked callback; cancelling the script does not cancel its parent.
- All 22 focused agent dispatcher tests and all nine operation-publication integration tests passed. Agent library Clippy passed with warnings denied. Read execution is still serial; this does not close the bounded concurrent-read or noncooperative active-adapter cleanup gates.

## Persisted root deadline containment

- The new root-deadline regression failed because an expired root could launch the host with the full 60-second allowance. Rust now fails before launch on expired, halted or unavailable root-budget evidence and clamps the host wall allowance to the original persisted deadline. Child discovery, admission and queue waits recheck the same budget. Broker construction rejects a substituted root binding.
- The native supervisor recomputes the remaining allowance after asset/catalog/process setup and before starting its watchdog, so setup cannot renew the root deadline. A native infinite-loop fixture with a one-second root allowance and no requested script timeout passed, preserving parent cancellation isolation.
- Pre-launch errors now carry the typed Codemode outcome and actual parent reference. The cancellation/status lineage assertion failed before this change and passed afterward, making these facts available to the existing presentation-hook protection.
- All 23 focused agent tests and all four native Windows supervisor tests passed against the v7 admitted bundle. No provider request or live subscription benchmark was used. Full crash recovery, artifacts, concurrent read scheduling, controlled mode and final Linux/macOS acceptance remain open.

## MCP mutation recovery fixture

- The actual MCP registry now declares `McpRead` for locally trusted read-only capabilities and `McpMutation` for everything else, preserving conservative unknown/untrusted hint handling and normal permissions. Previously the generic `Other` declaration excluded all MCP mutations from the internal controlled broker. This classification refinement was disclosed before changing the shared registry; controlled operator mode remains unavailable.
- The real client/transport fixture applies one counted effect, loses its response, and traverses the production operation dispatcher. The broker reports `RECOVERY_REQUIRED` with the actual child reference, closes admission even after the error is caught, and never repeats the effect. A disk reopen retains the same unresolved-effect state and one operation.
- All 17 MCP library tests passed, including direct text/error behavior, hint trust and supervised parsing; the strengthened disk-reopen fixture passed separately. Agent library Clippy passed. This validates response-loss recovery, not the complete killed-host/process-crash matrix required for C07/C08.
