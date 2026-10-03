# Native `/security-scan`: Engineering Design and Implementation Plan

**Date:** 2026-09-06  
**Status:** Design only. Implementation, execution of security scans, and changes to existing source are not authorized by this document.  
**Target:** `C:\Users\sergi\Desktop\pi-rust`  
**Source skills:** `C:\Users\sergi\Desktop\skills`  
**Requested deliverable:** One Markdown document, based on inspection of the harness and the supplied security skills.

> Implementation handoff: implement this plan only after a separate, explicit implementation request. Use test-driven development, preserve existing user changes, and verify each phase before continuing. This document is the combined design and implementation specification; do not create a parallel architecture or automatically execute the plan.

## 1. Executive recommendation

Extend the existing **native Rust security extension** into an evidence-driven AI review workflow. Do not create a competing JavaScript scanner, mechanically copy the Codex prompts, or route the command through the graph engine's coding-and-patching workflow.

The repository already has `SecurityScanController`, `SecurityArtifactStore`, fourteen `sec_*` tool names, terminal report presentation, and an integration with graph verification. However, the current scan is four substring checks; its deep scan repeats those checks. There is no public native `/security-scan` entry in the inspected command registry. The existing implementation is a useful integration scaffold, not the requested security-analysis engine. [R2, R3]

The recommended design has three separately owned layers:

1. **Native control plane:** command parsing, authorization, immutable source identity, budgets, worker lifecycle, evidence validation, persistence, and reporting. Rust owns these decisions; model output cannot authorize actions or declare completion.
2. **Harness-native methodology:** versioned, explicitly loaded security instructions adapted from the fourteen supplied skills. These guide reconnaissance, discovery, counterevidence review, and reporting without Codex APIs or missing helper files.
3. **Restricted analysis workers:** existing model/provider infrastructure behind a new scan-specific runner. Workers receive bounded source access and task packets, return structured claims, and cannot edit the target, publish findings, spawn unrestricted agents, or seal reports.

The no-argument command performs a **standard, static-first review of the selected repository snapshot**, using the already selected and authorized model. Optional analyzers supply leads, never final verdicts. Runtime verification requires a separately authorized disposable environment. A finding requires a reachable attacker-controlled path, an actual violated security boundary, and evidence that survives a counterargument pass.

### Alternatives considered

| Alternative | Benefit | Reason not selected as the primary design |
|---|---|---|
| Expand `/security-scan` into a long skill prompt | Small initial patch; uses existing `/skill:` machinery | Prompts cannot enforce scope, capability restrictions, completion gates, artifact integrity, or output schemas. Existing skill discovery also accepts arbitrary Markdown. |
| Invoke the existing graph coding workflow | Reuses planning, workers, verification, and recovery | Its implementation and patch stages are inappropriate for a read-only scan. Reuse lifecycle concepts and selected primitives, not coding authority. |
| Build an unrelated scanner plugin | Independent release cycle | Duplicates native commands, stores, permission integration, and the existing security UI. |
| **Upgrade the native security extension** | Reuses product integration while adding explicit boundaries | Requires a dedicated worker adapter and schema migration, but keeps one canonical security subsystem. |

### Non-goals

No automatic fixes, policy edits, dependency upgrades, issue creation, advisory publication, production probing, account discovery, secret validity checks against external services, or unrestricted external research. No claim of equivalence to unavailable Codex infrastructure or of a perfect scanner. Ordinary implementation bugs without a supported security impact remain separate observations, not vulnerabilities.

## 2. Current harness architecture relevant to plugins and security

### 2.1 Inspection basis and limitations

Repository inspection reported branch `main`, short HEAD `2edc963`, and 87 existing modified/untracked status entries. This is a **dirty working-tree observation**, not a claim that all inspected bytes belong to that commit. Source locations below refer to the files read on 2026-09-06; refresh them before implementation. No application, build, test suite, exploit, or analyzer was run for this design.

Read the repository's `AGENTS.md` and `CLAUDE.md` first. The active product is the Rust workspace, not the stale TypeScript `packages/*` tree. `vendor/davinci` remains reference-only. The pinned toolchain is Rust 1.83.0, edition 2021, and workspace dependencies use exact versions. Tests belong in inline `#[cfg(test)]` modules; shared fixtures may live under `crates/davinci-parity/fixtures`. [R1]

### 2.2 Existing integration points

| Concern | Observed implementation | Design consequence |
|---|---|---|
| Runtime and workspace | Thirteen active Rust crates, including `davinci-agent`, `davinci-ai`, `davinci-coding-agent`, `davinci-tui`, and `davinci-evals` | Keep domain orchestration in the native coding-agent extension; reuse provider and agent layers. |
| Native extension registry | `NATIVE_TOOLS`, `NATIVE_COMMANDS`, `command_specs()`, `native_invocable_commands()` | Register a real command and discovery metadata, not merely a prompt alias. |
| Dispatch | `ExtensionHost::execute_native_command()` and `execute_native_tool()` | Add a non-blocking start handle and shared scan state without holding host locks during analysis. |
| Interactive UI | `davinci_interactive::run_extension_command()`; `Screen::Securitas`; `views/securitas.rs` | Extend the existing security sheet and result adapter; preserve terminal styles and keyboard navigation. |
| Other input surfaces | Native command handling in `main.rs`; native discovery is also appended to RPC command lists | Use one parser and controller across TUI, print, JSON, and supported RPC invocation paths. |
| Agent workers | `SubagentRunner`, `SubagentRequest`, runtime cancellation and capability registry | Reuse bounded task execution, but add a scan-specific context and tool contract. |
| Existing worker limits | Up to eight submitted tasks, four concurrently in the general subagent implementation | Scan limits must be no greater than runtime limits and share the runtime capacity pool. |
| Tool authority | `PermissionMode`, `ToolClass`, `RuntimeCapabilityRegistry`, parent/child scoping | Intersect scan grants with parent permissions. A read-only classification does not establish offline operation or filesystem isolation. |
| Shell processes | Native graph process supervisor has abort flags, deadlines, and Windows process-tree termination | Reuse only after adding scan-specific output limits, environment controls, and verified cross-platform containment. |
| Skills | `discover_skills()` loads `SKILL.md` and other Markdown; `/skill:name` expands body text | Do not grant authority through arbitrary project skill content. Load a reviewed methodology bundle explicitly. |
| Model reasoning | `ThinkingLevel`, supported-level mapping, `StreamOptions`, thinking budgets | Resolve actual supported effort and record the effective request; do not assume `max` is supported. |
| Configuration | `Settings` has typed permissions and learning blocks, with flattened extra keys | Add a typed `securityScan` block and explicit trusted-source merge rules. |
| Context and memory | Token governor, recovery output, vector memory, automatic learning | Use run-scoped evidence retrieval; do not automatically index sensitive security evidence in general memory or learned skills. |
| Persistence | Existing scan artifacts under the system temporary directory; controller holds a single in-memory current scan | Add durable load/resume and run identity; do not call the current controller resumable. |
| Tests/evaluation | Inline Rust tests and a fixture-based eval crate | Add both deterministic contract tests and separate real-model security evaluations. Canned output is not evidence of detection ability. |

### 2.3 Concrete gaps in the present security implementation

These are **implementation observations and design gaps**, not independently validated vulnerability disclosures.

- `start()` scans for `BEGIN PRIVATE KEY`, `sk-`, `eval(`, and `shell=True`, and immediately creates both candidates and findings. No cross-file AI investigation occurs. [R2:372-425, 824-907]
- `deep_scan()` repeats file enumeration and the same rules; coverage counters accumulate rather than representing independent review coverage. [R2:505-525]
- `sec_scan_context`, `sec_scan_progress`, and `sec_scan_draft` all return the current snapshot. `sec_policy_resolve` returns scan configuration, not a resolved hierarchy of `SECURITY.md` files. [R2:569-639]
- Candidate validation accepts a disposition with optional text; it does not require a complete proof chain. `complete()` can mark the scan completed without checking candidate closure or validation quality. [R2:432-478, 537-551]
- Scope digests are derived from path names, not the analyzed content. Completed reports therefore need stronger snapshot identity before evidence can be reliably resumed or exported. [R2:372-384]
- The finding structure lacks confidence, attacker prerequisites, control semantics, multiple role-aware locations, and explicit proof gaps. Tool schemas allow arbitrary properties instead of defining strict inputs. [R2:93-125, 1135-1156]
- The store uses ordinary file writes and temporary storage. It verifies per-artifact hashes plus a combined digest; this detects accidental changes relative to a trusted manifest, not tampering by an actor able to rewrite the manifest and all hashes. [R2:172-340]
- `safe_join()` checks the canonical parent, while enumeration avoids following links. The replacement scope boundary must also validate the final file and Windows reparse cases at access time. This observation alone is not a demonstrated exploit. [R2:912-942, 992-1007]
- Graph `verify_changed_surface()` uses the deterministic findings as blockers. Do not silently replace that gate with an expensive AI scan or make existing blockers disappear during schema migration. [R2:663-777]
- The extension host already handles long-running `graph_run` outside its native mutex because holding that lock blocks status and cancellation. Security work must follow the same lifecycle principle. [R4:555-597]
- General subagents default to some network/MCP read tools. The capability registry also marks `ToolClass::Network` as read-only. Consequently, selecting only `read_only` capabilities is insufficient for a network-disabled scan. [R5:25-33; R6:70-100; R7:93-111]

## 3. Existing Codex security skill analysis

### 3.1 What was supplied

All fourteen supplied `SKILL.md` files were read. The directory inventory contained their fourteen directories and fourteen Markdown files, without the shared references, schemas, scripts, or nested reference directories expected by the skills.

The following referenced locations were checked and were absent: `Desktop/references`, `Desktop/scripts`, `Desktop/schemas`, and the nested reference directories for `security-scan`, `validation`, and `attack-path-analysis`. Missing files include contracts referred to as `core-scan.md`, `scan-artifacts.md`, `static-finding-assessment.md`, security guidance, severity guidance, validation guidance, finalization helpers, and tracking validators. Their contents have **not** been inferred or presented as inspected.

The visible methodology can be adapted. Exact compatibility with those missing contracts cannot be promised. No license or provenance grant for redistributing the supplied skill bundle was established. Implement native-authored instructions and contracts from this design; review source rights before copying or distributing original skill text.

### 3.2 Methodology, inputs, and expected tools

Each row names the actual supplied file as `<skill>/SKILL.md` relative to the supplied skills directory. [S1-S14]

| Skill | Reusable security objective and prompt logic | Expected inputs and tools | Native treatment |
|---|---|---|---|
| `security-scan` | One coherent standard review: threat model, discovery, validation, attack paths, final report; an independent audit complements coordinator investigation | Repository, scoped context, scan identity, source retrieval, progress/draft/finalization tools, shared core-scan reference | Primary standard methodology. Rust owns lifecycle and finalization. Preserve coherent review rather than launching a new worker for every tiny phase. |
| `deep-security-scan` | Independent complete standard audits, then reconcile findings and coverage | Codex deep-scan host, scan identities, durable workers, progress, handoff/lease support, optional service integrations | Deep mode is repeated independent analysis within native budgets. Remove service assumptions and the original multi-day runtime contract. |
| `security-diff-scan` | Pin compared states; review all changed files, including removed controls; close every candidate once | Git revisions/patch, changed-file inventory, candidate pagination, compact validation and attack-path writes | Dedicated diff targeting feeding the same canonical pipeline. Preserve before/after evidence and every changed-file coverage row. |
| `finding-discovery` | Find plausible source/control/sink failures across files; preserve distinct root controls and affected instances | Threat model, source navigation, scoped inventory, candidate recording, optional supplied seeds | Produce candidates only. Do not emit vulnerabilities at discovery time or substitute a nearby weakness for the supplied claim. |
| `attack-path-analysis` | Connect actor, exposure, control semantics, boundary crossing, and narrow impact; separate facts from severity policy | Candidate, threat model, deployment evidence, policy, referenced attack-path/severity guidance | Typed attack-path evidence and versioned native severity rubric; missing Codex severity rules are not fabricated. |
| `threat-model` | Identify supported actors, sensitive assets, entry points, trust boundaries, and assumptions; reuse only with matching identity/context | Source, policies, existing model, target identity, scope and user context | Snapshot-keyed threat model. Existing documents are evidence and may be incomplete or stale. |
| `define-security-policy` | Resolve applicable local policy and clarify supported security boundaries; approval before changing policy | Canonical repository root, affected scope, nested `SECURITY.md`, owner context, policy resolver/helper | Read-only resolver in the scanner. Policy authoring remains a separate explicitly requested workflow. |
| `validation` | Falsify each candidate, inspect mitigating controls, use proportionate static or dynamic proof, and preserve all dispositions | Candidate collection, source, feedback, tests/PoCs/debuggers when authorized, validation receipts | Validator contract with exact proof gaps and per-instance closure. Default static; disposable runtime verification is permission-gated. |
| `triage-finding` | Assess existing external claims against current code, one result per input, with exploitability ranking | Supplied finding/SARIF/advisory/ticket, static code, selected external intake transport | Separate static-only intake/triage contract. Do not auto-query accounts, deduplicate supplied claims, or run tests during triage. |
| `track-findings` | Track selected validated findings with duplicate checks, exact previews, explicit approval, and readback | Verified sealed scan, selected IDs, one provider/destination/identity, external connector or selected CLI | Optional post-scan integration only. Preserve disclosure and idempotency safeguards; no publishing from `/security-scan`. |
| `fix-finding` | Re-establish the original issue, enforce the narrowest complete invariant, preserve legitimate behavior, verify a minimal fix | Finding, affected code/callers/tests, optional fresh investigator/reviewer, patch and verification tools | Produce a remediation handoff only. Any patch workflow requires a new explicit user request. |
| `verify-fix` | Determine whether the original security boundary is closed, not whether a ticket was closed or a rescan is quiet | Original finding, current checkout, read-only checks; exact per-input result contract | Separate verification operation with `fixed`, `still_vulnerable`, or `inconclusive`. Do not auto-run or write verification artifacts contrary to its selected contract. |
| `propose-security-hardening` | Derive a small set of structural options from actual recurring weaknesses, with tradeoffs and migration constraints | Findings/disclosures, source identity, architecture, constraints, proposal format | Keep hardening separate from vulnerabilities. Detailed portfolios are opt-in derived documents, not mandatory scan output. |
| `vulnerability-writeup` | Skeptically reconstruct the same causal exploit path; distinguish source proof, actual runs, unexecuted PoCs, and unknown versions | Exact assessed source, finding, controls, release history if available, evidence and report-format references | Canonical concise reports derive from validated data. Optional disclosure drafting uses bounded independent review, not a mandatory subagent per finding. |

### 3.3 Runtime assumptions and conflicts to remove

| Skill group | Repository/agent/permission assumption in the source | Required adaptation |
|---|---|---|
| Standard/deep/diff | Codex scan IDs, host tools, environment variables, SDK-specific completion ownership, shared scripts | Bind every operation to a native `ScanId`, immutable snapshot, run generation, and coordinator-owned state machine. |
| Deep scan | Host-only deep workers, long multi-day budgets, special continuation and service availability | Native bounded audits; explicit budgets; sequential fallback when concurrency is unavailable; partial coverage rather than simulated completion. |
| Discovery/validation/attack paths | Different ledgers, receipts, and compact workbench protocols depending on Codex mode | One native candidate store and typed phase outputs; never require missing file conventions. Preserve each candidate's history and disposition. |
| Threat model/policy | External helpers and scope-dependent cache rules | Native root-to-leaf policy resolution and content-addressed, scope-aware caching; no policy edits. |
| Triage | Transport-specific GitHub/Linear/Jira rules and Codex project attachments | Use only a user-selected native transport with explicit identity and destination. Never silently switch transport or attempt token extraction. |
| Tracking | Specific installed apps, validation scripts, sealed Codex schema, provider-specific write actions | Defer until a native sealed-source reader and explicit connector contract exist. Tracking does not mutate the scan bundle. |
| Fixing | Codex workbench action tokens and generate/apply/verify stages | Separate later remediation request bound to source digest and exact approved patch; never introduce workbench token fiction. |
| Writeup | Exactly one drafting subagent per finding; missing source/history blocks the original production-disclosure flow | Core scan reporting does not depend on delegation or network history. State limitations; offer no unverified affected-release range. |
| Hardening | Many derived artifacts and automatic portfolio generation | Default scan emits only justified, separate hardening recommendations. A portfolio is a separate user-selected product. |

Missing references are resolved by writing **new native specifications and tests**, not by inventing the missing original files. No automatic workflow reads or executes instructions from the supplied directory at runtime after migration.

## 4. Codex-to-harness adaptation matrix

Existing native names below are verified registry entries; the behavioral changes in the last column are proposed work, not existing capability.

| Codex-specific primitive or convention | Native equivalent | Contract needed |
|---|---|---|
| Scan start/workbench identity | `SecurityScanController`, `sec_scan_start` | Preflight, source snapshot, unique run identity, authorization record; return immediately with a handle. |
| `get_codex_security_scan_context` | `sec_scan_context` | Read bounded authoritative context by scan ID; never return arbitrary other-session evidence. |
| Progress/draft update calls | Coordinator events plus `sec_scan_progress` / `sec_scan_draft` read APIs | Worker phase output is validated before state changes. Read APIs remain read-only; they are not disguised mutators. |
| Candidate recording | `sec_candidates_record` compatibility adapter | Strict typed candidate submission, content/line provenance, idempotency key, size limit, worker scope validation. |
| `list_codex_security_candidates` | `sec_candidates_list` | Stable candidate ordering and cursor pagination tied to a store version. |
| `record_codex_security_candidate_validations` | Coordinator validation reducer behind `sec_candidates_validate` | Validate a bounded batch atomically; disposition for every submitted candidate; reject stale or foreign IDs. |
| Candidate attack-path recording | `sec_candidates_attack_path` | Typed source/control/sink/impact evidence; reject arbitrary unbounded JSON. |
| `start_codex_security_deep_scan` | `SecurityOrchestrator` deep policy; `sec_deep_scan` compatibility entry | Independent complete audits, shared immutable snapshot, bounded workers, single reconciliation. No repeated substring scan. |
| Changed-file preparation/listing helpers | `ScopeResolver` and `sec_scope_files` | Before/after records, deletion/rename handling, pagination, exclusion reasons, no dropped changed files. |
| `resolve_security_guidance.py` and related scripts | `PolicyResolver` behind `sec_policy_resolve` | Native read-only policy resolution with bounded file size and non-authoritative policy text. |
| `CODEX_SECURITY_SCAN_*` environment | Host-owned `ScanContext` / `WorkerPacket` | Do not trust environment strings as authority. Pass immutable identity and restricted handles explicitly. |
| `fork_turns: "none"` | Fresh scan worker context | No parent conversation, unrelated secrets, remembered findings, or prior worker conclusions in an independent audit. |
| Codex subagent infrastructure | Scan-specific `SecurityWorkerRunner` using native agent/provider/runtime primitives | Explicit tools, permissions, reasoning, cancellation, bounded output, and validated terminal result. |
| Finalize/seal scripts and host completion | `FindingStore::finalize` through the coordinator | All candidate and coverage gates satisfied; exactly one finalization; atomic sealed manifest. |
| `validate_tracking_source.py` | Native sealed-bundle verification; `sec_tracking_validate` | Load by verified run identity; verify referenced bytes and schema; preserve immutable completed artifacts. |
| Codex artifact schemas and receipts | Versioned native schema v2, event journal, evidence references | No fabricated fields to satisfy unknown schemas; retain legacy v1 read support. |
| Native Codex apps, TAC, workbench routing | Optional separately authorized native connectors | Not required for scanning; no automatic installation, publication, remote lookup, or transport substitution. |
| `$fix-finding`, `$track-findings`, disclosure/hardening continuations | Prompt-ready handoff and explicit later user action | Recommendation is not authorization. Keep scan, remediation, verification, and disclosure identities distinct. |

## 5. `/security-scan` command UX and syntax

### 5.1 Public command contract

```text
/security-scan [path]
/security-scan --scope <path> [--scope <path> ...]
/security-scan [path] --mode <quick|standard|deep>
/security-scan [path] --focus <category>
/security-scan --changed
/security-scan --diff [<base>..<head>]
/security-scan [selection options] --format <terminal|json|sarif>

/sec-status [scan-id]
/sec-report [scan-id] [--finding <finding-id>]
/sec-abort [scan-id]
/sec-resume <scan-id>
```

Keep the existing `sec-status`, `sec-report`, and `sec-abort` aliases. Add `sec-resume` because durable interrupted reviews have real reuse value. Do not add automatic fixing, uploading, provider switching, or arbitrary-command flags.

The positional path is shorthand for one `--scope`. Reject a mixture of the positional path and `--scope` to avoid ambiguous unions. Multiple explicit `--scope` values form a union. `--changed` and `--diff` are mutually exclusive; either may be intersected with explicit paths. Repeated or unknown scalar flags are errors. Support `--` for a literal path beginning with a hyphen.

Parse quoted arguments without passing the command line through a shell. Accept spaces and Unicode. Resolve relative paths against the canonical repository root. An absolute path is accepted only after proving it is inside the same authorized root; normalize it to a repository-relative identity. Reject drive-relative paths, UNC/device escapes, alternate data stream syntax, and unresolved boundary cases. A supplied `.` means the repository root; do not preserve the current empty-normalized-path failure.

### 5.2 Defaults and selection semantics

**No arguments:** select the local Git worktree root containing the session directory, or the session directory when no Git repository exists; mode `standard`; all applicable security categories; format `terminal`. Snapshot the current worktree bytes, not merely HEAD. Display the selected root and dirty-state identity before review starts.

**Targeted paths:** review those product surfaces plus necessary supporting code within the authorized supporting-read boundary. Supporting reads do not silently expand the advertised finding scope. A finding whose root control lies outside the target is labeled supporting/out-of-scope unless the user expands the target. When the user explicitly restricts all reads to a path, do not read outside it; record the resulting proof gap.

**`--changed`:** compare the current worktree snapshot to HEAD. Include staged and unstaged changes and non-ignored untracked files; inspect deleted files at the baseline. On an unborn branch, use an empty baseline. Conflict stages are separate evidence; unresolved merges prevent a fully complete verdict.

**`--diff`:** bare `--diff` means the latest local commit compared with its first parent, or the empty baseline for a root commit. An explicit `base..head` compares the two locally resolved commits exactly. It does not implicitly compute a merge base or fetch remotes. Display full resolved object IDs; reject missing/ambiguous objects. For a pull-request-style comparison, the caller must provide the desired local base. Preserve head-side, base-side, rename, copy, and deletion identities.

**`--focus`:** one category from `auth`, `injection`, `filesystem`, `secrets`, `dependencies`, `crypto`, `logic`, `configuration`, or `ai-tools`. Focus narrows investigation priority and completion claims; it never permits ignoring a mandatory control needed to validate the selected category.

**Full repository:** enumerate the eligible corpus, map product surfaces, and prioritize high-risk review units. A budget-limited run is explicitly partial. No mode guarantees exhaustive path or vulnerability coverage.

### 5.3 Exclusions and size handling

Distinguish **hard denied** paths from **default discovery exclusions**. Hard-denied content is never read, even as supporting evidence. Discovery exclusions can be revisited only when still authorized and materially needed for a proof chain; record that supporting read.

Prune `.git`, build output, dependency caches, scanner artifacts, and known generated trees during enumeration rather than traversing them and filtering afterward. Honor trusted exclusion configuration and Git ignore semantics. Record exclusion counts and reasons. Do not treat all hidden files as irrelevant: workflow, configuration, and deployment files can matter. Do not read user-home credentials or files outside the repository.

Default proposed limits: 2 MiB per ordinary source file, 1 MiB per policy file, 50,000 inventory entries, and 512 MiB of captured source bytes. These are tunable safety defaults, not performance measurements. Oversized, binary, unreadable, symlinked, reparse-point, submodule, and unsupported-encoding files receive explicit coverage dispositions. No silent truncation, submodule fetch, Git LFS download, or recursive archive extraction.

Do not copy known credential files into the model context. A local secret detector may return redacted presence/location evidence for authorized repository files, but never perform an online secret-validity test. A secret-shaped example is a lead, not automatically a live credential exposure.

### 5.4 Progress and completion presentation

Display these factual stages: `Mapping repository`, `Identifying attack surface`, `Investigating high-risk areas`, `Validating candidates`, and `Generating report`. Include mode, target identity, worker count, actual usage when known, reviewed versus eligible units, and any blocking limitation. Do not stream internal reasoning.

The existing Securitas sheet remains the drill-down view. Separate confirmed findings, likely findings, hardening, and unresolved leads; show severity and confidence independently. Use keyboard selection and the existing theme, with a readable narrow-terminal fallback. Escape terminal control sequences and untrusted hyperlinks in every title, path, message, and excerpt.

On completion, report confirmed findings by severity, likely findings, areas actually reviewed, checks actually performed, deferred coverage, snapshot drift, and artifact locations. Say `No confirmed vulnerabilities found in the reviewed scope`, not `The repository is secure`.

### 5.5 Cancellation, resumability, and process status

A scan belongs to one session and one coordinator. A second active scan in that session returns a conflict with the current scan ID; other sessions use separate state and shared runtime capacity accounting. `Ctrl+C` or `/sec-abort` cancels the active scan, stops new work, propagates child cancellation, records partial evidence, and leaves the chat usable. A cancelled scan is not completed.

`/sec-resume` reloads a checkpoint only after checking ownership, snapshot availability, content hashes, methodology/schema version, capability policy, and remaining budget. Resume the original snapshot rather than mixing current files into old evidence. Re-ask for any expired dynamic or external authorization. Sealed scans are read-only; a new analysis creates a new scan ID. A completed worker result is reused only when its input and policy hashes match.

The interactive command does not exit the application. The proposed one-shot print invocation, such as `pi -p "/security-scan --changed --format json"`, waits for the scan and returns a command result. Exit codes: `0` completed with no policy-blocking finding; `1` completed with a blocking finding; `2` invalid invocation, failed, or incomplete required coverage; `130` cancelled. Partial/infrastructure errors take precedence over `1`, while the report still includes known findings. Default blocking policy is confirmed High/Critical; likely findings remain visible and may be configured to block by explicit trusted policy.

For structured output, stdout contains only the requested document; progress goes to stderr. The JSON event mode uses the existing transport envelope rather than mixing plain JSON and terminal prose. RPC clients must invoke the shared command adapter and receive stable status/results; merely listing the command in discovery is not sufficient.

## 6. Security methodology

### 6.1 Reconnaissance and threat model

Start with manifests, binary/package exports, routes, commands, plugin registrations, tool adapters, settings, authentication/session code, storage boundaries, and deployment configuration. Identify who can supply each input, whose authority is used, which assets matter, and which security guarantees the product actually supports.

Create a compact `RepositoryMap` containing product surfaces, source anchors, trust boundaries, sensitive operations, language/build metadata, and unresolved environment assumptions. Distinguish shipped code, examples, fixtures, build scripts, vendored code, and operator configuration. A label such as `local`, `admin`, or `plugin` does not establish trust; trace how another actor could influence the value.

Resolve root-to-leaf `SECURITY.md` guidance for each affected path. The nearest applicable policy may refine product-scope evidence, but repository policy cannot authorize tools, network, exclusions from host safety rules, or disclosure. An absence of policy is a proof gap, not proof that a boundary is unsupported. Do not suppress a reachable issue solely because malicious repository text says to ignore it.

### 6.2 Attack-surface mapping and coverage units

Represent each review unit as an entry point or privileged control with its relevant callers and data flow, rather than one arbitrary file. Record source identity, assets, actor capabilities, closest control, sink/decision, related configuration, assigned worker, and coverage disposition.

Prioritize combinations of attacker reachability, privilege difference, sensitive data, dangerous interpretation, and deployment evidence. Static analyzer matches are ranking signals only. A diff still requires a coverage row for every changed file, including changes ranked low risk. If time runs out, mark rows deferred instead of removing them from the denominator.

### 6.3 Discovery and hypothesis formation

For each candidate, require a concise claim: an identified actor controls a particular value or sequence, passes a specific entry point and control, reaches a protected operation, and may violate a stated invariant. Trace across helpers, dispatchers, persistence, encodings, and error paths. Keep the exact claim stable throughout validation.

Security-relevant categories include authentication/authorization and IDOR, tenant isolation, injection, path traversal, unsafe file use, SSRF, XSS/template injection, deserialization, cryptography/randomness, session/token handling, race conditions, business logic, subprocesses, sandbox boundaries, plugin authorization, prompt/tool injection, data exposure, supply-chain evidence, unsafe defaults, and security-sensitive error handling.

Do not infer a vulnerability from `eval`, a subprocess, missing authentication in a helper, an optional unsafe mode, or a dependency advisory alone. Determine actual inputs, callers, protections, configuration, and consequence. Conversely, a sanitizer name, authentication check, exception handler, or secure default does not automatically defeat the claim.

### 6.4 Verification and counterevidence

Inspect complete source/control/sink paths, direct and alternate callers, decoding or normalization after checks, cancellation and generation checks, state copies, and downstream reinterpretation. For authorization, inspect the check on the final resource identity, not merely a nearby login gate. For races, identify the shared state and possible interleaving; do not claim reliability from a theoretical schedule.

Prefer source-backed static validation when it establishes all material conditions. Dynamic validation can strengthen evidence when safely available, but is not mandatory for every confirmed issue. Missing services or a failed build are limitations, not proof of safety. A test of a reimplemented miniature example does not prove the real application is affected.

A separately authorized reproduction must use the exact snapshot, a disposable lab, bounded resources, a positive trigger, and an appropriate negative or legitimate-behavior control. Preserve the exact command, tool version, exit status, observed output, and limitation. Distinguish `not attempted`, `blocked`, `attempted but inconclusive`, and `reproduced`.

### 6.5 Reconciliation and reporting

The coordinator rereads decisive evidence, reconciles contradictory worker results, and determines confidence and severity from the surviving proof. It does not vote by agent count or concatenate drafts. Dedupe by root control, invariant, and exploit path, while preserving distinct affected entry points and occurrences.

Close every candidate as reportable, suppressed, not applicable, duplicate, or deferred. Report separate likely findings only when the vulnerability is plausible and its remaining material proof gap is stated. Low-evidence suspicions stay in unresolved leads. Hardening recommendations are never counted as vulnerabilities.

## 7. Agent and orchestration workflow

### 7.1 Worker modes

| Mode | Work arrangement | Completion expectation |
|---|---|---|
| Quick | One scoped worker performs reconnaissance-guided investigation and a separate counterargument pass | Bounded high-risk coverage; no external processes by default. Label unreviewed areas. |
| Standard | Coordinator builds the map; one fresh worker performs a coherent audit. The coordinator validates candidates independently, with one targeted reviewer when useful | Every selected review unit receives a disposition and every candidate is closed; uncovered units make scope completeness partial. |
| Deep | Two independent standard audits of the same immutable scope, with separate contexts, followed by coordinator reconciliation and targeted verification | Intentional overlap provides independent perspectives. Different conclusions must be resolved with evidence, not majority voting. |

For a large scope, the coordinator may subdivide investigation into bounded trust-boundary workstreams after reconnaissance. Independent auth, injection, configuration, dependency, and AI-tool tasks are justified only where those surfaces exist. Keep one owner per narrow workstream; deliberate independent validation is the exception to avoiding overlap.

General runtime concurrency limits still apply. Default scan concurrency is two, maximum four, sharing the native scheduler's capacity. One scan may not starve normal runtime cancellation or status. When concurrent execution is unavailable, run the same assignments sequentially. A separate pass in the same model context is labeled self-review, not a fresh independent agent.

### 7.2 Dedicated worker contract

Introduce `SecurityWorkerRunner` at the coding-agent composition boundary. Reuse native agent/provider APIs, cancellation, and capability registration, but do not call the ordinary subagent path blindly. The current nested-agent constructor loads profiles and models, may select edit mode for worktrees, shares MCP connections, and ensures a governor recovery tool. Those choices require a scan-specific override. [R5; R8:1450-1563]

A `WorkerPacket` contains:

- scan ID, attempt/generation ID, worker role, task ID, immutable snapshot ID, and methodology version;
- exact target and supporting-read authorization, task objective, product-scope facts, and unresolved assumptions;
- a bounded evidence index and source references, never the whole repository or unrelated chat;
- effective model and reasoning configuration, token/turn/deadline budget, explicit tool capabilities, and cancellation linkage;
- output schema version, candidate/coverage limits, and a completion contract.

An independent reviewer receives the candidate claim, pinned source references, relevant policy, and permitted scope, but not the originating worker's persuasive rationale or claimed test success. It retrieves the decisive source itself. A runtime-test reviewer receives actual recorded results with provenance and independently checks what those results establish.

Workers return a typed `WorkerResult` containing reviewed units, candidates, evidence references, counterevidence, limitations, resource observations, and terminal status. Exit success or fluent prose alone is not success. Parse, schema-check, scope-check, and provenance-check the result before accepting it.

Workers cannot recursively delegate. The coordinator owns scheduling, global deduplication, authorization requests, cancellation, persistence, and the final report. Worker text cannot override roles or widen a capability.

### 7.3 State machine and failure handling

```text
Created -> Preflight -> Snapshotting -> Mapping -> Investigating
                                              -> Validating -> Reporting -> Completed
any active stage -> Cancelling -> Cancelled
any active stage -> Failed
checkpointable active stage -> Interrupted -> resume original stage
Reporting -> Completed with coverageStatus = complete | partial
```

Lifecycle completion and coverage completeness are separate. A run can finish its budget and seal a partial report, but never claim complete coverage. It cannot finish reporting with candidates left in an accidental transient state: unfinished claims become deferred with a reason.

Coordinator state is a shared run handle, not a clone of an independently mutable `current` value. Every mutation includes the run ID, generation, expected store version, and idempotency key. Worker generations prevent late or retried outputs from rewriting newer decisions. Completing twice is idempotent only for the same final digest; completing after cancellation is rejected. Worker crash, malformed result, deadline, rate limit, and unavailable provider each have explicit outcomes and bounded retry policies.

Reserve at least 25% of the total review budget for validation and final reconciliation. Stop scheduling discovery when that reserve would be consumed. Prefer closing high-impact candidates to generating an unlimited list of unverified suspicions.

## 8. False-positive suppression and finding validation

### 8.1 Mandatory evidence gates

A confirmed vulnerability must pass all five gates:

| Gate | Required evidence | Failure behavior |
|---|---|---|
| Identity and location | Exact snapshot; valid paths/functions/line ranges; source bytes match referenced content | Reject fabricated/stale references; defer if source identity cannot be established. |
| Actor and reachability | Concrete lower-trust actor, controllable input or sequence, shipped/supported entry point, required privileges/configuration | Preserve a proof gap; do not assume trusted or untrusted solely from labels. |
| Control semantics | Full transformations and closest checks; what they actually reject or authorize; relevant alternate/failure paths | Investigate counterevidence; a control name is insufficient. |
| Boundary and impact | Specific violated confidentiality, integrity, availability, authorization, or isolation property after all relevant controls | Downgrade to hardening/non-security observation or defer when the boundary is unproven. |
| Adversarial acceptance | Explicit counterargument review, strongest realistic disconfirming evidence, calibrated validation basis, residual caveats | Suppress only with concrete defeating evidence; unresolved uncertainty is not confirmation or suppression. |

Static evidence can pass these gates. Runtime reproduction adds a stronger observation only when it tests the original path and supported preconditions. A missing runtime is not automatically disqualifying, but a material unproven runtime assumption prevents confirmation.

### 8.2 Dispositions and categories

Internal candidate dispositions are `reportable`, `suppressed`, `not_applicable`, `duplicate`, and `deferred`. Reportable security findings additionally have `classification: confirmed | likely`. Hardening recommendations use a distinct collection and no vulnerability classification. Deferred leads remain separate from likely findings when evidence is too weak.

`confirmed` means the complete claim is established by observed source and/or runtime evidence. `likely` means the claim is credible but one or more named material conditions remain unresolved. `suppressed` requires exact counterevidence. `not_applicable` requires evidence that the affected component, version, or supported surface does not apply. `duplicate` links to a canonical candidate/finding without dropping the affected occurrence. `deferred` records missing evidence, denied capabilities, exhausted budget, or failed setup.

Severity describes impact and exploit conditions. Confidence describes evidence strength. A High-severity hypothesis can have low confidence and must not become a confirmed High finding. Use `high`, `medium`, or `low` confidence labels with an evidence rationale; do not invent numerical probabilities.

The native severity rubric is versioned: Critical is reserved for demonstrated system-wide or similarly extreme impact under supported conditions; High for substantial authorization/isolation breach, sensitive disclosure, or execution; Medium/Low for narrower constrained impacts. Record why the selected level follows from the actual actor, prerequisites, asset, and blast radius. CWE is optional and evidence-based. Do not emit a CVSS vector without establishing every chosen metric and its methodology.

### 8.3 Deduplication and suppression feedback

A canonical fingerprint combines a versioned root-control identity, violated invariant, normalized sink/decision, and attack-path class. It does not use line numbers or title text alone. Keep occurrence IDs for distinct entry points, parameterizations, tenants, or files; a shared tactical fix may group them without erasing them.

False-positive feedback is data keyed by fingerprint, scope, relevant source/control hashes, policy version, reason, and expiry. Revalidate the reason after source or configuration changes. Never permanently suppress an entire CWE or all findings matching a human title. Keep suppression counts and reasons inspectable. Preserve external triage inputs individually even when they resemble the same discovery finding.

## 9. Model and reasoning configuration strategy

Select the model from the active native model configuration. Do not hard-code a provider, assume a Codex subscription, silently choose a more expensive model, or read credentials into the prompt. Model switches require an existing applicable authorization or a new explicit choice.

| Operation | Requested effort | Fallback and record |
|---|---|---|
| Quick reconnaissance/investigation | `medium`, or an explicit lower-cost user profile | Use the strongest supported level not exceeding the configured request; show effective effort. |
| Standard investigation | `high` | Respect provider-supported mapping and total budget. |
| Deep independent audit | Highest explicitly supported practical level, bounded by user configuration | `max`/`xhigh` only when the selected model exposes them; otherwise `high`. |
| Final candidate validation | `high`, or the supported deep level for complex/high-impact candidates | Do not reduce the evidence gates when budget is tight; defer instead. |
| Deterministic reporting | No model needed for canonical JSON, SARIF, or summary counts | Optional prose cannot alter finding classification or canonical fields. |

`davinci-ai::get_supported_thinking_levels()` and the configured `thinkingLevelMap` are the source of supported effort. Budget-based paths currently clamp `xhigh` and `max` to `high`; record the effective mapping, not merely the requested label. The inspected `SubagentRequest` has no explicit thinking-effort field, so the scan runner must carry one in its own packet and set the child request deliberately. [R5:128-156; R9:19-45, 118-140]

Proposed initial total budgets, for evaluation rather than promised quality: Quick 24,000 aggregate input/output tokens and 20 model turns; Standard 96,000 tokens and 60 turns; Deep 240,000 tokens and 140 turns. Shared provider quotas and user caps may lower these. Default wall-clock ceilings are 5, 20, and 45 minutes respectively, configurable before execution. These are runtime cutoffs for the future feature, not estimates of performance or this document's completion time.

Use actual provider usage where available and conservative pre-request reservations otherwise. Missing usage/cost is `null`/unknown, not zero. Record provider/model identity, effective settings, prompt/methodology version, truncation, retries, and budget termination. Do not retain or expose private chain-of-thought; concise evidence-backed decision summaries are sufficient.

## 10. Tool and analyzer integration

### 10.1 Three independent authorization dimensions

1. **Source access:** what repository snapshot bytes may be read and what redacted evidence may be included in a model request.
2. **Model transport:** which already configured provider endpoint may receive those bytes. A cloud model is network access even when all analysis tools are offline.
3. **Tool execution/egress:** which local analyzer or disposable validation process may execute, and whether it may access any network destination.

Default analyzer/tool egress is denied. Model transport is limited to the user's already authorized selected provider and repository scope; selection alone is not blanket authorization to send proprietary code. When authorization is absent or ambiguous, block the model stage before source transmission and require the host's approval flow. Under a fully offline policy, use an already configured local model; otherwise return a clear unavailable/blocked result. Never silently fall back to a cloud provider or call deterministic substring checks an AI scan.

### 10.2 Local source tools

Add scan-scoped `sec_source_read` and `sec_source_search`, and extend `sec_scope_files` for inventory paging. These tools accept logical snapshot/path identities, bounded ranges, and bounded queries. They do not accept arbitrary filesystem roots, command text, remote URLs, or output destinations. The host resolves every source reference against the immutable snapshot and authorization.

Do not give workers the general `bash`, `powershell`, `write`, `edit`, `agent`, graph coding, MCP, or unrestricted recovery/memory tools. A recovery read, when needed, is implemented through the same run-scoped evidence store and cannot reveal another session's stored output. Lifecycle and finalization tools remain coordinator-only. An audit worker may submit candidate data, not execute its suggested reproduction.

### 10.3 Optional analyzer decisions

Official project/licensing sources were consulted on 2026-09-06. The following are integration choices, not claims that the programs are installed, tested here, or universally compatible with the pinned Rust toolchain.

| Tool | Verified licensing/status basis | Decision and constraints |
|---|---|---|
| `cargo-audit` | Maintained in the RustSec tooling repository; its README identifies MIT or Apache-2.0 licensing and Cargo.lock advisory analysis. [W1] | First optional Rust dependency adapter. Analyze a snapshot lockfile using a deliberately supplied local database. Pin/test supported executable versions; no install, lockfile generation, fix command, or network database refresh during a scan. Missing/stale database is a limitation. |
| OSV-Scanner | Official repository and Apache-2.0 license; documented offline database operation and normal-mode external services. [W2, W3] | Optional multi-ecosystem dependency adapter after the native pipeline works. Require tested offline behavior and pre-provisioned data; do not run remediation or license-query modes implicitly. Record database age and actual dependency evidence. |
| Semgrep CE | Official licensing distinguishes the LGPL-2.1 engine from Semgrep-maintained rules under a separate rules license with use restrictions. [W4] | Optional local engine adapter using native-authored or explicitly license-reviewed rules. Do not bundle the default registry rules as though they were uniformly permissive/open source, and do not enable proprietary features or cloud uploads. |
| CodeQL | Official GitHub documentation distinguishes CLI usage/licensing from the separately licensed query repository and limits ordinary private-repository availability. [W5] | Not a mandatory or bundled dependency. Defer automatic execution; import user-supplied results or add an explicit licensed integration only when deployment rights and execution permissions are established. |

Bandit, gosec, Trivy, npm/pnpm audit, and specialized secret scanners are candidate future adapters, not selected dependencies in this release. Their current licenses, platform support, network behavior, and rule/database terms were not individually verified here. They remain disabled until the same review and execution contract is met. This avoids turning the first release into an unbounded scanner collection.

### 10.4 Analyzer execution contract

An adapter returns an `AnalyzerPlan` with a pinned executable identity, version-compatible argv array, explicit input files, isolated working directory, environment allowlist, network policy, deadline, output-byte limit, expected output schema, and declared side effects. The host validates it before execution; a model cannot supply arbitrary shell text or executable paths.

Resolve executables through trusted configuration, not repository-controlled PATH precedence. Clear inherited proxy/credential variables except the minimal explicitly approved environment. Do not load repository analyzer configuration that can execute hooks, fetch rules, or widen scope. No automatic packages, database downloads, Git hooks, package-manager scripts, or repository builds.

Use `std::process::Command` with separate arguments, not interpolated shell commands. Capture bounded stdout/stderr and record return status. Normalize analyzer results as untrusted signals; path, version, rule, and evidence fields are checked against the snapshot. Nonzero exit codes have tool-specific meanings and do not imply either a vulnerability or infrastructure failure without parsing the documented adapter contract.

A missing tool does not fail the entire AI review unless trusted policy required that check. An analyzer failure does not become `passed`. Dependency presence establishes an affected-component signal; product exploitability still requires feature, call-path, platform, version, and mitigation evidence.

## 11. AI-harness-specific security coverage

The scanner must review both the target application's AI boundaries and its own execution boundaries.

| Surface | Investigation objective | Required scanner-side protection |
|---|---|---|
| Repository prompt injection | Can comments, documents, instruction files, package text, or retrieved snippets influence privileged actions? | Repository text is quoted data. Only the reviewed methodology and host policy supply instructions. |
| Malicious project instructions | Can a lower-trust project change execution permissions or trigger startup hooks? | Scan workers do not auto-load project profiles, arbitrary skills, extensions, hooks, or prompt templates. |
| Tool authorization and aliases | Do direct, batch, alias, MCP, or extension routes enforce the same final authority? | Default deny on unrecognized routes; enforce scope and capabilities at dispatch, not just advertised tool lists. |
| Approval bypass | Can retries, changed parameters, stale tokens, or another worker reuse an approval? | Bind grants to scan, generation, subject, arguments, destination, and lifetime; deny rules still win. |
| Confused deputy | Does the harness apply its user credentials to attacker-controlled URLs, files, or commands? | No worker access to ambient credentials or external connectors; distinguish model transport from target egress. |
| Cross-agent trust | Can one worker forge another's completion, evidence, or permissions? | Host-assigned identities, strict result schemas, evidence readback, and coordinator-only reducers. |
| Untrusted tool output | Can analyzer output alter instructions, inject terminal escape sequences, or forge citations? | Treat output as data; bound, sanitize, and validate every field and source location. |
| Secret/context leakage | Do transcripts, embeddings, summaries, telemetry, or learned skills persist credentials or source? | Run-local private artifacts; redact before provider, renderer, logging, and memory boundaries; no automatic memory/learning ingestion. |
| Plugin isolation | Can plugin load/configuration or native/JS/MCP authority exceed the intended boundary? | Do not execute target plugins to inspect them. Native metadata is an input to policy, not proof of OS containment. |
| Filesystem authority | Do links, junctions, final-component replacements, special files, or snapshot paths escape scope? | Handle-validated reads and copies; deny symlink/reparse/device access; enforce output roots separately. |
| Subprocess authority | Are untrusted values interpreted by shells, build scripts, environment expansion, or child processes? | No arbitrary process execution in discovery; approved adapters use explicit argv and a verified sandbox where required. |
| Recovery and caching | Can stale state, shared memory, or poisoned caches change findings or reveal another project? | Cache by source/methodology/policy/tenant identity; validate evidence again; never inherit authorization from cache. |
| External actions | Could a scan publish private findings, update a ticket, or test a production endpoint? | Explicitly absent from the scan's capability set; later actions require separate destination and payload approval. |

Do not describe native read-only permissions, a temporary directory, or a Git worktree as an operating-system sandbox. An arbitrary repository build can execute arbitrary code even when its output directory is disposable. Dynamic execution against untrusted code is disabled unless a verified containment backend or an explicit separately managed lab is available.

## 12. Architecture and component boundaries

### 12.1 Proposed file organization

Keep `native_extensions/security_scan.rs` as the public facade so existing imports remain stable. Move domain implementation into submodules under `native_extensions/security_scan/`; do not leave both a competing `mod.rs` and the facade defining the same module.

```text
crates/davinci-coding-agent/src/
  native_extensions/security_scan.rs        existing facade and compatibility exports
  native_extensions/security_scan/
    command.rs                             parser and public command contract
    config.rs                              typed defaults and effective policy
    types.rs                               schema v2 and validated identifiers
    controller.rs                          run registry and admission
    orchestrator.rs                        stages, budgets, scheduling, reconciliation
    scope.rs                               selection, Git states, exclusions
    snapshot.rs                            content identity and confined source access
    recon.rs                               repository/attack-surface mapping
    policy.rs                              SECURITY.md data resolution
    skills.rs                              reviewed methodology bundle and manifest
    worker.rs                              task/result contracts and runner interface
    validation.rs                          evidence gates, dispositions, deduplication
    tools.rs                               strict worker/coordinator tool adapters
    analyzers.rs                           optional executable adapters
    store.rs                               checkpoints, evidence, atomic finalization
    report.rs                              terminal/Markdown/JSON/SARIF projections
    eval.rs                                inline contract and fixture orchestration tests
    methodology/                           new native-authored, versioned phase instructions
```

All paths in this tree are **proposed**, except the existing facade. Keep related helpers together until splitting materially improves ownership; the boundaries are required, not an obligation to create empty files.

### 12.2 Responsibilities and interfaces

| Component | Inputs and outputs | Authority/invariant |
|---|---|---|
| `SecurityScanCommand` | Argument tokens -> validated `ScanRequest` | No source read, worker spawn, or artifact creation on invalid input. |
| `SecurityScanConfig` | Built-in defaults plus trusted settings -> `EffectiveScanPolicy` | Repository content cannot raise permissions, budgets, network access, or disclosure rights. |
| `SecurityScanController` | Valid request/context -> `ScanRunHandle`; status/cancel/resume | Admission and session ownership; release locks before long-running work. |
| `ScopeResolver` / `SourceSnapshot` | Authorized root and selection -> immutable manifest and confined read handles | Exact bytes, stable before/after identity, explicit skipped/deferred records. |
| `RepositoryRecon` / `AttackSurfaceMapper` | Snapshot evidence -> product map and review units | Observed facts distinguished from assumptions; no unsupported completeness claims. |
| `SkillAdapter` | Reviewed bundle version and task type -> bounded methodology packet | No Codex runtime dependencies, auto-loaded repository instructions, or executable helpers. |
| `SecurityOrchestrator` | Run handle, policy, map, runner -> reconciled canonical state | Sole scheduler and decision owner; budgets and lifecycle cannot be overridden by model prose. |
| `SecurityWorkerRunner` | Valid `WorkerPacket` -> typed `WorkerResult` | Fresh restricted agent, explicit model/effort, no recursive delegation or arbitrary edits. |
| `FindingValidator` | Candidate, exact source evidence, counter-review -> disposition and optional finding | Mechanical gates plus evidence assessment; schema validity alone does not establish truth. |
| `ScanToolAdapter` | Host-authenticated task and strict input -> bounded evidence or candidate submission | Enforces run/scope/version on every call; not merely a prompt warning. |
| `AnalyzerAdapter` | Approved immutable execution plan -> observed raw signals and diagnostics | No shell interpretation, auto-fix, implicit network, or target modification. |
| `FindingStore` | Coordinator events and evidence -> checkpoints and sealed bundle | Single writer, atomic publication, immutable completed records. |
| `SecurityReporter` | Canonical findings/coverage -> format-specific projections | Cannot promote confidence, invent counts, or expose unredacted secrets. |

Inject runner, filesystem/snapshot access, clock, budget accounting, process executor, and artifact store interfaces so tests do not require real models or analyzers. The interfaces must return typed errors, not an undifferentiated successful string containing an error.

### 12.3 Integration changes outside the security module

- `native_extensions/mod.rs`: register the public command and strict tool metadata; preserve existing callers and command aliases; initialize typed configuration and run handles.
- `extension_host.rs`: add security start/status/cancel dispatch that never holds the host mutex over model work, filesystem enumeration, or process waits. Make privilege checks apply equally to commands and tools.
- `main.rs`: compose the scan-specific runner with native model/provider infrastructure, restricted capabilities, structured output, and shutdown cancellation. Preserve normal subagent behavior outside scan mode.
- `settings.rs`: add `securityScan`, merge only approved configuration sources, and preserve unrelated flattened settings. Project settings may narrow scope but cannot broaden privileged permissions without host approval.
- `davinci_interactive.rs`: consume scan events and adapt schema v2 to the existing security sheet, including start/resume/cancel behavior and selected finding details.
- `davinci-tui/src/davinci/model.rs` and `views/securitas.rs`: distinguish classifications and coverage states without inferring network or validation guarantees from display text.
- `davinci-agent` runtime/capability and permission integration: register the scan's strict tools and cancellation linkage; change shared APIs only when required for deterministic containment. Coordinate with already modified/untracked capability code rather than replacing it.
- `davinci-ai`: reuse existing supported-effort APIs; only add targeted fixes/tests when the new runner cannot faithfully express effective effort. No unrelated provider rewrite.
- `davinci-evals`: add schema-aware security scoring support without creating a dependency cycle back into the coding-agent binary. Run scanner-specific fixtures from the coding-agent test module using the injected runner.

### 12.4 Proposed settings contract

The following is an example of the new schema, not a statement that these keys work today:

```json
{
  "securityScan": {
    "defaultMode": "standard",
    "model": "inherit",
    "providerAccess": "existing-authorization-only",
    "toolNetwork": "deny",
    "dynamicValidation": "disabled",
    "maxConcurrency": 2,
    "maxFileBytes": 2097152,
    "maxPolicyBytes": 1048576,
    "maxInventoryEntries": 50000,
    "maxSnapshotBytes": 536870912,
    "validationReserveRatio": 0.25,
    "analyzers": [],
    "failOn": {"classifications": ["confirmed"], "minimumSeverity": "high"},
    "retentionDays": 30,
    "indexIntoMemory": false
  }
}
```

Mode budgets are the defaults in Section 9 and may be overridden in typed per-mode settings. `model: inherit` resolves the currently selected model once at admission; it does not follow later unrelated chat model changes mid-run. Invalid security settings fail admission rather than silently enabling a permissive fallback.

Resolve storage from the harness's existing agent-directory resolution, not a hard-coded `.pi` or `.davinci` guess. Store scans under `<resolved-agent-dir>/security-scans/<repo-id>/<scan-id>/`, outside the target. Retention is user-configurable; do not delete evidence during an active run or enable cleanup as part of this documentation task.

## 13. Data flow and workflow diagram

```text
User: /security-scan ...
          |
          v
   Parse + authorize -------- invalid/denied --------> typed error, no scan
          |
          v
   Snapshot + inventory ---- drift/omission --------> explicit coverage record
          |
          v
   Recon + policy-as-data + attack-surface map
          |
          v
   Coordinator / budget / cancellation / run identity
          |
          +----> Restricted audit worker A ----+
          |                                    |
          +----> Independent audit B (deep) ----+--> untrusted candidates
          |                                    |       and evidence refs
          +----> Approved optional analyzer ---+              |
                                                               v
                                                   Evidence + counterreview
                                                               |
                                +------------------------------+------------------+
                                |                              |                  |
                           confirmed/likely               suppressed/         hardening
                                |                         deferred/duplicate       |
                                +------------------------------+------------------+
                                                               |
                                                   Closure + coverage gates
                                                               |
                                                   Atomic canonical bundle
                                                               |
                                               Terminal / JSON / SARIF / drill-down
```

```text
Trust boundary:
  host policy + vetted methodology + user authorization
                          |
             constrained reads / typed calls
                          |
  repository text, SECURITY.md, comments, analyzer output, worker prose
                          |
                  untrusted evidence only

No edge from untrusted evidence to:
  target writes | permission changes | arbitrary processes | external publication
```

Model transport is a separate permitted edge from the redacted worker context to the selected authorized provider. The diagram's analyzer edge is absent unless its execution plan is permitted.

## 14. Finding schema and report formats

### 14.1 Canonical versioning and identity

Use numeric `schemaVersion: 2` for the new native documents, retaining the existing camelCase convention. Validate with typed Rust structures and an offline JSON schema fixture. Reject unknown security-critical input fields; allow explicitly namespaced non-authoritative extension metadata only in a designated `extensions` object.

`scanId` identifies the run; `snapshotId` identifies exact captured source; `candidateId` identifies one claim within that run; `findingId` identifies a canonical validated issue; `occurrenceId` identifies an affected instance. IDs are generated and validated by the coordinator, not accepted as arbitrary filesystem paths. Cross-run fingerprints are versioned and do not grant access to another run.

Do not expose absolute user paths or remote URLs with credentials in portable reports. The private manifest can retain a local root binding for resume; export projections replace it with a neutral repository label. Source locations carry `snapshotSide: worktree | base | head`, relative path, optional function, one-based inclusive lines, content hash, and role (`source`, `entrypoint`, `control`, `sink`, `configuration`, or `test`). A missing file location is explicit, never a fabricated line 1.

### 14.2 Required record contracts

| Record | Required fields and rules |
|---|---|
| `ScanManifest` | schemaVersion, scanId, lifecycle status, coverageStatus, engineKind, snapshotId, source kind and resolved revisions where available, selected scopes, exclusions, source digests, methodology/policy versions, start/end time, model/effective reasoning, authorization summary without credentials, budgets/usage, limitations, final artifact hashes. |
| `Candidate` | candidateId, task/worker provenance, original claim, actor/source/control/sink/impact hypothesis, prerequisites, source references, evidence and counterevidence refs, proof gaps, disposition history, current disposition, optional duplicate target. |
| `SecurityFinding` | findingId, candidateIds, fingerprint and version, occurrences, title, classification, severity and rationale, confidence and rationale, optional CWE, affected locations, vulnerable behavior, attacker prerequisites, causal attack path, impact, protections assessed, evidence, validation basis, reproduction status, remediation, residual caveats, provenance. |
| `Validation` | method (`static`, `runtime`, or `mixed`), five gate results with evidence refs, reviewer identity/type, actual commands and observations when present, counterevidence, exact remaining gap, disposition and timestamp. |
| `Coverage` | eligible/enumerated/read/analyzed/validated units separately, bytes actually read, exclusions/skips/deferred by reason, per-changed-file dispositions, applicable checks with actual status, supporting reads, scope completeness and stop reason. |
| `AnalyzerObservation` | tool/executable version, rule/database identity and date, parsed input scope, status, actual exit code, network observation, normalized signals, stderr limitation, evidence references. |
| `HardeningRecommendation` | separate ID, source finding/evidence refs, proposed invariant, concrete recommendation, tradeoffs, caveats; not a vulnerability severity count. |

The parser requires every finding to have a nonempty causal claim and validated evidence references. `validated: true` from an old artifact is not sufficient to synthesize a confirmed v2 finding. Import legacy findings as legacy/unassessed evidence unless revalidated under the new contract.

### 14.3 Illustrative finding object

The following is a **synthetic contract example**, not a discovered vulnerability or an executed test. Fixture hashes/IDs are explicitly symbolic; real output must contain actual verified identities.

```json
{
  "schemaVersion": 2,
  "findingId": "fixture-finding-001",
  "candidateIds": ["fixture-candidate-001"],
  "fingerprint": {"version": 1, "value": "SYMBOLIC-FIXTURE-FINGERPRINT"},
  "title": "Document lookup omits the owner authorization check",
  "classification": "likely",
  "severity": "high",
  "severityRationale": "Would disclose another user's private document if the route is exposed as claimed.",
  "confidence": "medium",
  "confidenceRationale": "The missing owner predicate is established; deployment exposure remains unresolved.",
  "cwe": "CWE-862",
  "locations": [{
    "path": "fixtures/documents/handler.rs",
    "function": "download_document",
    "startLine": 12,
    "endLine": 18,
    "role": "control",
    "snapshotSide": "worktree",
    "contentHash": "SYMBOLIC-FIXTURE-CONTENT-HASH"
  }],
  "occurrences": [{"occurrenceId": "fixture-occurrence-001", "entrypoint": "download_document"}],
  "vulnerableBehavior": "The lookup uses a caller-controlled document ID without an owner predicate.",
  "attackerPrerequisites": ["A normal authenticated account", "Reachability of the affected route"],
  "attackPath": [
    "A normal user submits another user's document ID.",
    "The handler checks login but does not bind the requested document to that user.",
    "The storage lookup returns the requested document."
  ],
  "impact": "Potential cross-user disclosure; deployment reachability is not established.",
  "protectionsAssessed": ["Login check exists; owner authorization is absent in the cited fixture path."],
  "evidenceRefs": ["fixture-evidence-source", "fixture-evidence-control"],
  "validation": {
    "method": "static",
    "reproductionStatus": "not_attempted",
    "counterevidenceRefs": [],
    "proofGaps": ["Confirm the route is enabled for ordinary users in a supported deployment."]
  },
  "remediation": "Enforce ownership at the shared document-access boundary and add cross-user and legitimate-owner tests.",
  "residualCaveats": ["No runtime reproduction or release-range claim is made."],
  "provenance": {"taskId": "fixture-audit", "reviewType": "counterargument-pass"}
}
```

A production schema expands gate results and provenance as specified in the record table. This example demonstrates honest likely-finding output, not a shortcut around required validation fields.

### 14.4 Bundle, checkpoint, and seal

```text
<scan-dir>/
  scan-manifest.json
  snapshot-manifest.json
  context.json
  candidates.json
  coverage.json
  findings.json
  report.md
  results.sarif
  events.jsonl
  evidence/                 bounded private source/test evidence
  working/                  coordinator checkpoints; not a completed report
```

Only the coordinator writes canonical state. Use restrictive directory/file permissions, collision-resistant create-new identities, temporary files followed by atomic publication, bounded journal records, and recovery from a torn final journal record. Sync durable state before publishing the final manifest. Lock by scan identity; do not rely only on a process-global singleton.

Seal findings, candidate dispositions, coverage, context, snapshot manifest, canonical report projections, and referenced evidence using a deterministic artifact index. Exclude volatile working files. The final manifest references the complete index without a self-hash cycle. A hash seal proves consistency relative to its trusted root, not authenticity against a same-user adversary who can replace that root. Document that threat model; do not claim cryptographic attestation.

A final report is generated from canonical data, not parsed back from Markdown. Completed artifacts are immutable. Subsequent triage, tracking, fixes, or hardening live in separate derived state. Retain the old `report.sarif` compatibility alias only if a real consumer requires it; canonical new output is `results.sarif`.

### 14.5 Format mappings

**Terminal/Markdown:** same classification, severity, evidence, coverage, and limitation fields as JSON. Default summary is concise; drill-down reveals the causal path and protections assessed. Render untrusted text safely and do not expose command-like links as executable actions.

**JSON:** the top-level report contains `schemaVersion`, `scan`, `findings`, `hardening`, `unresolvedLeads`, `coverage`, `checks`, and `limitations`. Suppressed and duplicate candidate histories remain available in the canonical bundle without flooding the default report.

**SARIF:** support version 2.1.0 as an interoperability projection, not the internal authority. Map canonical rule IDs, normalized artifact URIs, regions, related locations, fingerprints, and meaningful code-flow steps. Put classification/confidence/proof gaps in namespaced properties. Confirmed High/Critical may map to `error`, confirmed Medium to `warning`, and lower-impact results to `note`; likely results must be clearly identified and not presented as confirmed. Keep hardening separate by default. Mark incomplete execution honestly. Validate against the official schema and consumer-specific constraints; do not invent a numeric security score or add unverified source links. [W6]

## 15. Performance and context strategy

Enumeration, reconnaissance, and targeted retrieval precede detailed model context. Do not load the repository wholesale. Partition by product boundary and high-risk control, use compact symbol/path inventories, and retrieve exact source ranges only when needed. Source summaries retain links to original immutable evidence; decisive lines are reread during validation.

Use content-addressed snapshots with canonical, length-delimited identity records including path bytes, content hash, entry type, scope side, and relevant configuration. Preserve case-sensitive distinctions on platforms that support them; do not lowercase all roots as a universal repository identity rule. Record source metadata before/after capture and retry or defer files changed during capture. A source hash mismatch never silently updates an old proof.

Cache repository maps and source fragments by snapshot/content identity, parser/methodology version, authorized scope, policy, and relevant model configuration. Invalidate on any changed dependency. No cache hit implies a file was deeply analyzed. Persisted threat models remain data and cannot change permission policy. Cross-project or cross-user caches are disabled unless separately designed and authorized.

Page inventories and candidate collections. Default source reads are capped at 200 lines and 64 KiB; default search pages at 50 matches; worker final packets at 256 KiB. These limits are proposed and adjustable, but every truncation is explicit with a continuation mechanism. Cap evidence and child output bytes independently of token budgets. Oversized tool output is stored privately and retrieved by bounded reference, never silently discarded as a successful complete result.

Limit parallelism by global runtime permits, memory, and model quotas. Account for all independent audits, retries, and validation passes in the same scan budget. Avoid concurrent writes to canonical state and avoid loading duplicate source fragments into every worker unnecessarily. Background native execution must remain cancellable and observable throughout.

Default telemetry contains only redacted status, durations, counts, and usage when authorized. No repository source, secrets, finding bodies, local absolute paths, or raw prompts are sent through analytics. Automatic learning and vector indexing are disabled for scan content; the scan's private evidence store supplies recovery instead.

## 16. Implementation phases

Every phase below is future implementation work. Begin by refreshing repository guidance and the dirty state; do not revert, stage, or overwrite unrelated changes. Add failing inline tests before behavior changes, implement the narrowest solution, run the focused tests, and inspect the final diff. No tests in this section were executed while preparing this document.

### Phase 1 — Harness and plugin integration

**Objective:** expose one real command with correct dispatch, configuration, admission, and lifecycle boundaries before adding AI analysis.

**Changes and files:** keep the `security_scan.rs` facade; introduce `command.rs`, `config.rs`, `types.rs`, and `controller.rs`. Update `native_extensions/mod.rs`, `extension_host.rs`, `settings.rs`, and shared dispatch in `main.rs`/`davinci_interactive.rs`. Add command discovery/help and preserve existing `sec-*` aliases. Create a shared run handle and cancellation token; do not perform long work under either outer extension-host or native-host locks.

**Dependencies:** existing Rust/serde/runtime APIs; no external analyzer or new provider. Preserve exact dependency pins.

**Test-first tasks:** `security_scan_parser_accepts_quoted_scope`; `security_scan_rejects_unknown_or_conflicting_flags_before_side_effects`; `security_scan_command_is_discoverable_and_invocable`; `security_scan_status_and_abort_do_not_wait_on_worker_lock`; `security_scan_same_session_conflict_is_explicit`; `security_scan_settings_do_not_broaden_parent_permissions`.

**Verification:** `cargo test -p davinci-coding-agent security_scan`; inspect TUI, print, JSON, and RPC adapter assertions using a fake runner. Invalid arguments must create no artifact and make no provider call.

**Completion criterion:** command reaches the same controller on supported surfaces, status/cancel remain responsive with a blocked fake worker, and the command cannot accidentally become a generic model prompt or invoke a JS command of the same name.

### Phase 2 — Codex skill adaptation

**Objective:** make the methodology self-contained and executable without any Codex runtime or missing support files.

**Changes and files:** add `security_scan/skills.rs` and new native methodology documents for reconnaissance, discovery, validation, attack paths, and reporting. Add a bundle manifest with version, content hashes, supported roles, declared inputs/outputs, and migration notes. Do not copy the source skill tree into project auto-discovery. Keep optional triage/fix/tracking/disclosure contracts separate and inactive.

**Dependencies:** Phase 1 task/result schema and policy contract. Source-license review is required before redistributing original text; native-authored methodology does not depend on unknown reference contents.

**Test-first tasks:** `security_methodology_has_no_unresolved_runtime_dependencies`; `security_methodology_required_references_exist`; `security_scan_does_not_autoload_project_skills_or_profiles`; `security_scan_missing_optional_skill_does_not_enable_fallback_authority`; `security_scan_worker_prompt_separates_evidence_from_instructions`.

**Verification:** offline bundle validation plus focused coding-agent tests. Inspect each migration row against the fourteen source skills, including preserved instance coverage and explicit deferred functionality.

**Completion criterion:** every active phase has complete local instructions and typed output requirements; no active call to `get_codex_*`, `record_codex_*`, `start_codex_*`, unavailable app IDs, or missing helper scripts remains.

### Phase 3 — Repository reconnaissance and attack-surface mapping

**Objective:** produce accurate authorized source identity, policy context, and coverage before investigation.

**Changes and files:** implement `scope.rs`, `snapshot.rs`, `policy.rs`, and `recon.rs`; connect the run-specific store. Resolve current/diff/changed targets, ignored and denied paths, root policy inheritance, before/after records, supporting reads, and content digests. Use safe native Git invocations with fixed argv and no hooks, external diff drivers, text conversion, or implicit fetch.

**Dependencies:** Phase 1 authority/state; Phase 2 methodology. A disposable Git fixture is sufficient for tests; no network required.

**Test-first tasks:** `security_scope_rejects_parent_drive_unc_and_reparse_escape`; `security_scope_checks_final_file_identity`; `security_changed_includes_staged_unstaged_untracked_and_deleted`; `security_diff_preserves_base_head_and_rename_locations`; `security_snapshot_changes_when_bytes_change_at_same_path`; `security_policy_is_data_not_tool_authority`; `security_inventory_records_every_skip_and_truncation`; `security_supporting_reads_respect_explicit_read_boundary`.

**Verification:** inline tempfile/Git fixtures on Windows and Unix; contents and timestamps of target files remain unchanged. Enumeration counts distinguish eligible files, captured files, and analyzed units.

**Completion criterion:** every evidence reference resolves to authorized immutable bytes, dirty changes are not lost by relying on HEAD, and omitted/deferred scope is visible in coverage.

### Phase 4 — Core security analysis workflow

**Objective:** implement real architecture-aware AI investigation, not another rule loop.

**Changes and files:** implement `orchestrator.rs`, `worker.rs`, and strict `tools.rs`; compose the scan-specific worker runner in `main.rs`. Add `sec_source_read/search`, typed candidate submission, runtime cancellation linkage, and effective model/effort. Disable arbitrary tools, inherited MCP, project hooks/profiles, unrelated memory, and recursive agents for these workers. Keep the four old rules only as explicitly labeled local signals.

**Dependencies:** Phases 1-3; existing provider model mapping and runtime capacity. No dependency on optional analyzers or writable graph workers.

**Test-first tasks:** `security_worker_denies_unadvertised_and_aliased_tools`; `security_worker_cannot_read_other_scan_or_host_files`; `security_worker_does_not_inherit_conversation_or_credentials`; `security_scan_reasoning_records_effective_supported_level`; `security_deep_runs_independent_standard_audits_not_duplicate_grep`; `security_scan_cancel_propagates_to_every_worker`; `security_scan_budget_reserves_validation`; `security_worker_malformed_result_is_not_success`.

**Verification:** scripted fake-model interactions exercise actual tool dispatch and complete quick/standard/deep control flow. Count provider requests, tool calls, and denied effects. A canned candidate tests plumbing only; detection quality is evaluated in Phase 8.

**Completion criterion:** the workflow retrieves cross-file evidence, produces structured candidate hypotheses, respects all runtime budgets, and can complete or return honest partial results without any Codex infrastructure.

### Phase 5 — Finding verification and false-positive suppression

**Objective:** make high-confidence reportability a first-class gate.

**Changes and files:** implement `validation.rs`, candidate transitions, independent counter-review, occurrence-preserving deduplication, and strict finalization prerequisites in `controller.rs`/`store.rs`. Add static validation as the default. Define a disabled-by-default dynamic verification plan and authorize it only through the host, never worker prose.

**Dependencies:** Phase 4 source/evidence pipeline; Phase 3 immutable snapshot. Runtime reproduction depends on a separately verified lab backend, not on relaxed native permissions.

**Test-first tasks:** `security_dangerous_api_only_is_not_reportable`; `security_missing_authz_requires_real_actor_and_reachability`; `security_sanitizer_name_without_semantics_does_not_suppress`; `security_missing_runtime_is_a_gap_not_counterevidence`; `security_candidates_all_receive_dispositions`; `security_duplicate_preserves_distinct_occurrences`; `security_feedback_expires_after_control_change`; `security_completion_rejects_unclosed_or_stale_candidates`; `security_runtime_claim_requires_observed_result`.

**Verification:** paired vulnerable/secure and partial-evidence fixtures. Explicitly test a true vulnerability demonstrated statically, a secure alternate caller, a source mismatch, and a likely finding with a named unresolved precondition.

**Completion criterion:** no confirmed finding bypasses the five evidence gates; zero dangerous-API-only fixture matches are promoted; suppressions and unresolved claims remain auditable.

### Phase 6 — Reporting and terminal UX

**Objective:** make findings actionable and format-consistent without overstating results.

**Changes and files:** implement `report.rs`; finish schema v2 and stable public report envelopes; update `davinci_interactive.rs`, TUI `model.rs`, and `views/securitas.rs`. Add `/sec-report --finding`, progress events, classified result groups, format-clean stdout, and explicit incomplete/cancelled states. Implement atomic bundle publication and legacy v1 read-only loading.

**Dependencies:** Phase 5 validated records and store contracts. SARIF schema validation uses an offline fixture based on the official schema, with appropriate notices.

**Test-first tasks:** `security_report_separates_confirmed_likely_hardening_and_leads`; `security_json_stdout_has_no_terminal_prefix`; `security_sarif_uses_valid_relative_locations`; `security_report_redacts_secrets_and_terminal_controls`; `security_report_counts_match_canonical_findings`; `security_legacy_validated_flag_is_not_v2_confirmation`; `security_store_detects_tampering_and_torn_checkpoint`; `security_complete_and_cancel_race_has_one_terminal_outcome`.

**Verification:** snapshot/golden formatting tests, JSON parsing, SARIF schema checks, narrow-terminal rendering, selected-finding navigation, and interrupted-write recovery. Compare all projections against the same canonical data.

**Completion criterion:** a developer can inspect exact evidence and uncertainty in the terminal, exported JSON/SARIF remains valid, and incomplete work never appears as a clean security verdict.

### Phase 7 — Optional external analyzer integration

**Objective:** add useful corroborating signals without expanding default authority or replacing AI analysis.

**Changes and files:** implement the reviewed `cargo-audit` adapter first in `analyzers.rs`; OSV-Scanner and Semgrep CE follow only when their execution/licensing contracts are satisfied. Add executable/version capability detection, explicit local database/rules input, bounded process output, and source-grounded result normalization. Keep CodeQL and other candidates deferred unless separately selected.

**Dependencies:** Phase 5 signal intake; Phase 6 diagnostics; verified executable/database/rule rights and tested version support. Recheck upstream documentation when selecting exact supported versions.

**Test-first tasks:** `security_analyzer_never_auto_installs_or_updates`; `security_analyzer_does_not_use_repository_path_executable`; `security_analyzer_argv_is_not_shell_interpreted`; `security_analyzer_denied_network_stays_denied`; `security_analyzer_missing_or_stale_database_is_reported`; `security_analyzer_timeout_or_invalid_json_is_not_pass`; `security_advisory_signal_does_not_invent_exploitability`.

**Verification:** stub executables and canned JSON exercise side effects, argv, failure codes, malformed paths, and output caps. Real optional-tool checks run only in a deliberately provisioned offline test environment.

**Completion criterion:** the scanner works fully without analyzers; enabling one adds observed, provenance-preserved signals and never causes implicit network, target edits, or hidden process authority.

### Phase 8 — Evaluation and tuning

**Objective:** measure actual security quality and tune budgets/methodology using evidence.

**Changes and files:** add `security_scan/eval.rs` and an independent scoring module in `crates/davinci-evals/src/security_eval.rs`, exported from that crate's `lib.rs`. Keep expected-data fixtures under `crates/davinci-parity/fixtures/security-scan/` where shared files are useful; otherwise construct them in inline tests. Separate evaluation labels from worker-visible inputs.

**Dependencies:** Phases 4-6 are mandatory; Phase 7 is evaluated as an optional configuration. Real-model runs require explicit model/provider authorization and recorded cost policy.

**Test-first tasks:** `security_eval_scores_root_cause_not_title_substrings`; `security_eval_counts_deferred_positive_as_miss_for_recall`; `security_eval_reports_zero_denominator_as_undefined`; `security_eval_separates_fixture_plumbing_from_live_quality`; `security_eval_preserves_per_category_metrics`; `security_eval_records_model_and_methodology_versions`.

**Verification:** first validate the scorer on fabricated result records; then run the approved held-out model evaluation defined in Section 17. Repeat with and without analyzers, counter-review, and independent deep audit to measure contribution rather than assume it.

**Completion criterion:** publish measured precision/recall, category coverage, severity calibration, evidence validity, duplication, latency, and usage; meet the provisional release gates or keep the feature explicitly experimental with documented failures.

### Phase 9 — Cross-platform and production hardening

**Objective:** make cancellation, privacy, persistence, and compatibility reliable under hostile input and failures.

**Changes and files:** harden snapshot/store/process adapters, shutdown/resume hooks, and shared runtime integration only where necessary. Add Windows reparse/device/ACL cases, Unix symlink and process-group cases, bounded child output, strict temporary-file handling, and isolated dynamic-lab capability checks. Preserve the existing graph deterministic verification contract as a separate `engineKind` and add explicit incomplete-state handling without silently clearing blockers.

**Dependencies:** all preceding phases; supported Windows/Linux/macOS environments and an explicitly configured lab for dynamic checks. No unverified claim of a portable sandbox.

**Test-first tasks:** `security_cancel_stops_descendants_on_supported_platform`; `security_resume_rejects_changed_snapshot_or_policy`; `security_store_cannot_escape_through_existing_link`; `security_scan_excludes_evidence_from_memory_learning_and_telemetry`; `security_graph_deterministic_gate_preserves_blockers`; `security_scan_shutdown_leaves_recoverable_checkpoint`; `security_process_output_is_bounded_without_newlines`; `security_authorization_is_rechecked_after_resume`.

**Verification:** focused cross-platform tests, then repository-required checks: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`, using offline fixtures. Exercise concurrency, cancellation, crashed workers, disk errors, malformed files, and source changes during capture. Record unrelated existing failures instead of claiming they pass.

**Completion criterion:** no unauthorized side effects in adversarial tests; durable partial results and recovery behave predictably; compatibility and quality gates pass on supported platforms; unresolved platform capabilities stay disabled with clear diagnostics.

## 17. Test and evaluation framework

### 17.1 Deterministic engineering tests

All ordinary CI tests are offline and use injected providers, analyzers, clocks, and filesystem/process fixtures. Assert behavior at the actual dispatch, scope, and persistence boundaries, not only prompt contents. A test that checks whether a prompt says `do not write` is not a write-denial test.

Mandatory families include parser/dispatch parity; permission inheritance and deny precedence; source/path confinement; source drift; changed/deleted/renamed coverage; strict schema validation; worker output and resource limits; cancellation/resume; atomic completion; legacy compatibility; secret and terminal-output redaction; and complete classification separation.

Test prompt injection in `README`, `AGENTS.md`, `SECURITY.md`, comments, analyzer messages, and candidate prose. Malicious fixtures request shell execution, permission escalation, external submission, altered output paths, fake completion, and fabricated citations. Assert zero execution of prohibited effects and no forged references entering a confirmed report.

### 17.2 Security-quality corpus

Use a minimum initial held-out corpus of 40 known vulnerable cases paired with 40 secure lookalikes. Include at least four pairs in each of ten families: auth/IDOR, tenant isolation, injection/XSS, filesystem, secrets/configuration, dependency applicability, crypto/session, race/business logic, subprocess/approval, and prompt/tool/plugin boundaries. At least ten vulnerable cases span multiple files; include real control-flow and deployment-condition traps rather than only obvious signatures.

For representative cases, provide a safely executable local positive trigger and a legitimate negative control. Examples of required fixture relationships include an owner check missing versus present at the shared access boundary; a path checked before a later reinterpretation versus checked at final use; attacker-controlled shell text versus an argv-only invocation with no dangerous interpretation; a dependency whose affected feature is used versus absent; and an AI tool request that crosses an approval boundary versus a correctly denied request.

Ground-truth labels include root control, actor, source, sink, supported preconditions, expected classification/severity band, decisive evidence locations, and secure-control rationale. Keep labels and evaluator-only explanations outside the scanned source and worker context. Use repository-native implementations rather than an evaluator reimplementation of the vulnerability.

### 17.3 Metrics and release gates

| Metric | Measurement | Provisional gate, not a measured result |
|---|---|---|
| Confirmed precision | Correct confirmed findings / all confirmed findings, matched by root cause and occurrence | At least 90% overall; no confirmed High/Critical on the designated secure critical-boundary traps. |
| Recall | Known vulnerabilities detected as correctly supported findings / all eligible ground-truth vulnerabilities | At least 80% for standard/deep within their declared scopes; report confirmed-only recall separately. |
| Per-family behavior | TP/FP/FN and coverage by vulnerability family | No supported family with zero evaluated coverage; report family regressions rather than hiding them in an average. |
| Severity calibration | Agreement with independently reviewed severity bands | At least 85% within the accepted band; explain every Critical overstatement. |
| Evidence validity | Referenced path/hash/range exists and supports the stated claim | 100% mechanical validity; manual causal-evidence rubric at least 90%. |
| Duplicate rate | Redundant reported root-cause results / all reported results | At most 5%; distinct affected instances must still be preserved. |
| Disposition closure | Candidates with an explicit terminal disposition / all admitted candidates | 100%, including suppression and deferred outcomes. |
| Safety | Forbidden writes, network destinations, command execution, secret exposure, or cross-run reads | Zero permitted violations in deterministic adversarial tests. |
| Reproducibility | Results under fixed source, methodology, model configuration, and repeated runs | Report spread over at least three runs; do not promise deterministic model outputs. |
| Runtime cost | Actual wall time, p50/p95 latency, input/output/cache usage, retries, cost when known | Stay within configured caps; establish measured baselines before optimizing. |

False negatives include relevant vulnerabilities left unresolved or missed; they do not disappear because the scanner reports a proof gap. Show separate precision for likely findings. When a metric has a zero denominator, report `not defined` and the raw counts. Confidence intervals and per-run variation are more informative than a single flattering percentage.

Canned fixture outputs validate orchestration and schema handling only. Real model capability gates require actual authorized model runs against held-out source. Do not call a test suite successful evidence of vulnerability discovery merely because the expected words appeared in a canned answer; the current generic eval helper's substring assertions are insufficient for this feature. [R12:33-103]

### 17.4 Rollout and regression control

Keep the new command experimental until engineering safety gates and measured quality gates are satisfied. Compare Quick/Standard/Deep, analyzer-off/on, and counter-review-off/on on the same held-out cases. Do not assume deeper reasoning or more workers improves precision or recall; measure the marginal change and resource cost.

Record scanner/schema/methodology version, model and effective reasoning, source digest, analyzer/database/rule versions, exclusions, and budgets with each eval. A model or methodology change triggers the relevant regression suite. User false-positive feedback informs future fixtures only after redaction and authorization; it must not silently train persistent memory on proprietary findings.

## 18. Risks, limitations, and unresolved decisions

| Risk or unknown | Evidence or current limitation | Planned treatment |
|---|---|---|
| Missing original reference files | Supplied directory contains only fourteen skill entry files; required helpers/contracts were absent | Native-authored contracts and explicit migration notes; no claim of exact original compatibility. |
| Skill redistribution rights | No license/provenance grant was established for the supplied bundle | Review before copying/distributing original text; keep the native implementation independent of those files. |
| Moving dirty codebase | 87 pre-existing status entries, including runtime capability and UI work | Refresh all affected interfaces before implementation; preserve user changes and coordinate overlap. |
| Current build/test health unknown | No builds or tests were run for the documentation-only request | Establish baseline in an authorized implementation task; distinguish pre-existing failures. |
| Provider capability and authorization | Actual selected model, quotas, local-model availability, and source-disclosure grant were not inspected through credentials/settings | Resolve at scan admission using existing host configuration and approval; no invented model or automatic cloud fallback. |
| Native permissions are not OS isolation | Read-only includes network metadata; generic workers can share MCP; process helpers do not constitute a sandbox | Scan-specific capability and source boundary; dynamic untrusted code disabled without verified containment. |
| False positives and false negatives | AI reasoning and static analysis are fallible; missing deployment context can change conclusions | Explicit proof gaps, independent counter-review, measured eval gates, and truthful coverage. |
| Reporting integrity | Hashes alone do not authenticate a fully replaceable local bundle | Restrictive storage, atomic writes, trusted run binding, immutable lifecycle; document same-user tampering limitation. |
| Large/complex repositories | Budget and context may not cover every caller or service | Prioritize review units, preserve all exclusions/deferred rows, and return partial coverage without a clean verdict. |
| Analyzer compatibility and licensing | Exact supported installed versions and all optional tools were not verified | Pin/test selected adapters later; recheck official terms; no mandatory external scanner. |
| Snapshot privacy/storage | Source and proof artifacts may contain proprietary information | Private storage, redacted exports, explicit retention, no automatic vector indexing or telemetry bodies. |
| Cross-platform process termination | Existing helper has Windows tree termination; equivalent full Unix descendant containment was not established | Test platform-specific cancellation and add explicit process-group/job containment where required. |
| Legacy graph gate semantics | Current graph gate uses deterministic signals as blockers | Keep engine kind and policy separate; no covert gate weakening or unexpected AI invocation. |
| Source/release claims | Local snapshot is not proof of a released affected version or production prevalence | Report snapshot findings; release-history and disclosure campaigns are separate explicitly scoped work. |

No remaining unknown requires redesigning the core workflow. Installation-specific provider grants, precise analyzer versions, and optional containment backends are admission/integration decisions with conservative defaults, not reasons to invent capabilities. If a required safety capability is unavailable, that operation remains disabled and the report states the limitation.

## 19. Definition of done

### 19.1 This design document

The design is complete when the actual native integration points and gaps are identified; all fourteen supplied skills have a migration treatment; the command, scope, permissions, orchestration, evidence gates, formats, persistence, and resource behavior are specified; each implementation phase has concrete files, tests, and acceptance criteria; and missing source contracts or environment capabilities remain explicit. The sections above provide those contracts without implementing the feature.

### 19.2 Future feature release

Release readiness requires all of the following, supported by actual verification:

- `/security-scan` is discoverable and invokes one shared native workflow; supported terminal, print, JSON, and RPC paths are tested.
- Standard mode performs real source-grounded investigation; Deep performs independent audits rather than repeated substring checks.
- All active methodology is self-contained and contains no required Codex-only APIs, apps, environment conventions, or missing files.
- Target access, model transmission, analyzer execution, dynamic validation, and disclosure are separately enforced; no automatic source edits or external publication occur.
- Every candidate is closed; confirmed findings satisfy the five evidence gates; likely findings, hardening, and leads remain separate.
- Source identities and evidence locations are verified; changed/deleted/renamed files and deferred scope remain traceable.
- Cancellation, resume, budget exhaustion, malformed output, unavailable tools, and drift produce accurate non-success or partial states.
- Canonical JSON, terminal reports, and SARIF agree; private artifacts and redaction are verified; completed scans cannot be silently modified.
- The existing graph deterministic verification contract is preserved or changed only through an explicit tested migration.
- Offline engineering tests pass on supported platforms, and actual authorized model evaluations meet the declared quality gates or the product remains clearly experimental.

Stop adding agents, integrations, flags, or report products once these requirements are satisfied. Improving detection quality thereafter should be driven by measured misses and false positives, not by increasing prompt length or tool count.

### Source register

Repository references use paths relative to the target root. Line ranges identify the inspected working-tree observations, not immutable commit-pinned claims. Skill references use paths relative to the supplied skills directory.

| ID | Inspected source and relevant anchors |
|---|---|
| R1 | `AGENTS.md`; `CLAUDE.md`; `Cargo.toml`; `rust-toolchain.toml:1-3`; `Makefile`. Repository instructions, active workspace, exact pins, test conventions. |
| R2 | `crates/davinci-coding-agent/src/native_extensions/security_scan.rs:19-340, 348-821, 824-1041, 1088-1280`. Current types, lifecycle, scan rules, path checks, reports, schemas, inline tests. Inspection hash: `8ab33c0e361bc8bc`. |
| R3 | `crates/davinci-coding-agent/src/native_extensions/mod.rs:22-167, 175-245`. Native registrations, discovery, controller composition and worker guards. |
| R4 | `crates/davinci-coding-agent/src/extension_host.rs:460-597`. Native hooks, memory/learning integration, long-run lock avoidance and command/tool dispatch. |
| R5 | `crates/davinci-agent/src/subagent.rs:25-50, 84-156, 184-240`. Worker defaults, limits, request fields, parent permission/capability scoping. |
| R6 | `crates/davinci-agent/src/permission.rs:17-100`. Permission modes, classes, and network/read-only distinction. |
| R7 | `crates/davinci-agent/src/runtime/capabilities.rs:36-111`. Capability metadata and builtin read-only classification; this file was already untracked in the inspected checkout. |
| R8 | `crates/davinci-coding-agent/src/main.rs:1450-1563`; native command/discovery call sites found at `2542`, `2689`, `4480`, and `5671` during source search. Nested-worker construction and integration anchors. |
| R9 | `crates/davinci-ai/src/thinking.rs:19-45, 118-158`. Thinking budgets and supported-level resolution. |
| R10 | `crates/davinci-agent/src/skills.rs:42-108, 111-186`. Markdown discovery and skill/prompt expansion. |
| R11 | `crates/davinci-coding-agent/src/settings.rs:7-147, 161-171`. Typed configuration, permission settings, flattened extension keys. |
| R12 | `crates/davinci-evals/src/lib.rs:33-103`. Existing fixture evaluation and substring assertions. |
| R13 | `crates/davinci-coding-agent/src/davinci_interactive.rs:1838-1953`; `crates/davinci-tui/src/davinci/views/securitas.rs:1-150`. Native interactive dispatch and current security presentation. |
| R14 | `crates/davinci-coding-agent/src/native_extensions/graph/worker.rs:1-13, 87-260`; `graph/process.rs:20-86, 113-190` under the same native-extension directory. Worker output/lifecycle and process supervision. |
| S1-S7 | `security-scan/SKILL.md`; `deep-security-scan/SKILL.md`; `security-diff-scan/SKILL.md`; `finding-discovery/SKILL.md`; `attack-path-analysis/SKILL.md`; `threat-model/SKILL.md`; `define-security-policy/SKILL.md`. Read in full. |
| S8-S14 | `validation/SKILL.md`; `triage-finding/SKILL.md`; `track-findings/SKILL.md`; `fix-finding/SKILL.md`; `verify-fix/SKILL.md`; `propose-security-hardening/SKILL.md`; `vulnerability-writeup/SKILL.md`. Read in full. |

External references below were checked on 2026-09-06 for the limited decisions attributed to them. No private repository text was used in the external queries. Exact deployment/version compatibility must be rechecked when implementing an optional adapter.

- **W1 — RustSec cargo-audit project and license:** <https://github.com/rustsec/rustsec/tree/main/cargo-audit> and <https://github.com/rustsec/rustsec/blob/main/cargo-audit/README.md>.
- **W2 — OSV-Scanner project, offline operation, and external service behavior:** <https://github.com/google/osv-scanner>.
- **W3 — OSV-Scanner Apache-2.0 license:** <https://github.com/google/osv-scanner/blob/main/LICENSE>.
- **W4 — Semgrep product, engine, and rule licensing:** <https://docs.semgrep.dev/licensing>.
- **W5 — GitHub CodeQL CLI availability/licensing and distinction from query source:** <https://docs.github.com/en/code-security/concepts/code-scanning/codeql/codeql-cli> and <https://github.com/github/codeql>.
- **W6 — OASIS SARIF 2.1.0 specification:** <https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/sarif-v2.1.0-os.html>.

**Execution record for this task:** read-only source and skill inspection, limited public reference verification, and creation of this Markdown document only. No scan, implementation, test/build run, installation, source-skill migration, policy mutation, commit, or external publication was performed.
