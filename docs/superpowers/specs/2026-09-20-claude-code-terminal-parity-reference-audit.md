# Claude Code terminal parity: reference and coverage audit

Status: reference research and design coverage only. No UI implementation or runtime verification is claimed.

This document supplements [the whole-terminal design specification](https://github.com/J12003LPZ/davinci/blob/J12003LPZ/terminal-ui-rebuild-20260920/docs/superpowers/specs/2026-09-20-claude-code-terminal-parity-design.md) already present on `J12003LPZ/terminal-ui-rebuild-20260920`. It preserves that specification's **whole-terminal 1:1 priority**, primary fullscreen comparison profile, separate classic-renderer profile, and optional graph. It does not replace the reference with a new dashboard or reopen the approved graph direction.

DaVinci source inventory: `4ffdda411d54ba874ce1184ee8762820504c7536`. Existing specification commit inspected before adding this audit: `953fbc7ca254c429ee7c27645ca1f8179d2cb72e`. That existing file is left unchanged.

## 1. Fixed reference

Anthropic's release endpoint identified **Claude Code v2.1.278**, published September 19, 2026, as the latest published release at lookup [R1]. Pin that version for actual terminal comparisons. Live documentation may change; record any mismatch between documentation and the pinned executable instead of silently mixing versions.

The official repository README links its terminal demo, documentation, and plugins [R2]. Its license notice reserves rights [R3]. The repository is reference material for an independent DaVinci implementation, not permission to redistribute Anthropic's binaries or implementation. Keep DaVinci's truthful identity and provider data as specified in the main design.

## 2. Image evidence: found versus actually inspected

| ID | Surface | Source and observed evidence | Version and limitation |
| --- | --- | --- | --- |
| I1 | Welcome and model selection | Original Trigger.dev screenshot: full image visually inspected. Split welcome panel, `/model` echo, compact numbered options, selection pointer, separate current-model mark, inline effort and footer hints. | Visible v2.1.77. Historical evidence, not current-version pixel proof. |
| I2 | Settings Config tab | Original Medium article image-search preview: tabs, search box, compact label/value rows and highlighted focus. | Version not visible. Full-resolution fetch failed. A red annotation rectangle was added by the author, not the terminal UI. |
| I3 | Agent view | Official documentation links light and dark captures; explanatory text was read. | Alt text names v2.1.140. Both image fetches returned HTTP 403; the screenshot pixels were not inspected. |
| I4 | Terminal demonstration | Official README's `demo.gif` link located. | The GIF did not render or download in this environment. It is not inspected visual evidence. |
| I5 | Permissions, login, command suggestions, resume, rewind, detailed task activity | Official behavior documentation located and inspected. | No pinned-version screenshot for these surfaces was obtained in this pass. Direct capture is still required. |

### I1: welcome and model picker

![Historical Claude Code v2.1.77 welcome and model-picker reference, captured by Trigger.dev](https://trigger.dev/blog/10-claude-code-tips-you-did-not-know/claude-code-tip-5.png)

Attribution: Trigger.dev [R4]. This is a Claude Code reference image, not a DaVinci build. Its model names, pricing, account details, working directory, and version are not fixture values to hard-code into DaVinci.

### I2: settings

[Open the original settings screenshot](https://miro.medium.com/v2/resize:fit:1400/1*20xG1zAPbyEJV-BKSA-Zig.png). Attribution: the original Medium article author [R5]. Only the image-search preview was inspected. Treat the preview as structural corroboration, not a measurable current baseline; exclude the red annotation.

### I3 and I4: official image sources

The [official agent-view guide][R6] contains the light/dark screenshot links. The [official repository README][R2] embeds the demo. Failed image requests must remain marked uninspected even when accompanying text is readable.

No third-party or user screenshot binary is committed. The linked image remains hosted by its publisher. Capture gaps are recorded explicitly instead of filling them with generated lookalikes and labeling them reference screenshots.

## 3. What the references change in the rebuild

These observations refine the existing whole-terminal specification. They do not establish that the current application already behaves this way.

### Settings are a reference dialog, not a custom dashboard

Use the observed Status / Config / Usage tab structure as the reference to verify in v2.1.278. The current DaVinci fullscreen settings ledger must not survive behind a different color palette. Match tab placement, search, focus, density, value alignment, explanations, and contextual footer after capture. The official settings guide establishes `/config` and configuration scopes [R8].

Retain every real DaVinci setting and alias. A `/config` alias should reach the same implementation as `/settings`, not a second settings system. Keep typed values and stable setting keys separate from display strings. Search must not change which setting is saved. Status and Usage show actual provider data; absent quota data is unavailable, not a fabricated Claude subscription meter.

### Model selection is not exempt

The user said to redo model selection too. The historical reference uses compact, numbered choices with distinct focus and active-model indications [R4]. Verify its target-version form rather than retaining DaVinci's current rounded picker by default. Provider catalogs can differ, but typography, spacing, selection behavior, and confirmation should follow the measured reference.

The official model guide covers real selection and effort behavior [R9]. Persist provider/model identity, never a colored display label or a filtered row index. An unsupported reasoning level is not a selectable decoration. Preserve cancellation and failed-switch states so the UI never claims a model change the runtime rejected.

### Main conversation and input come before graph polish

Capture the ordinary transcript, welcome variants, input, streaming calls, expanded output, inline code/diff, interruptions, and slash suggestions first. Rebuild the shared shell and components, not only `graph_run.rs`. The earlier illustrative custom dashboard is not the reference.

The official interactive guide documents transcript expansion, model access, background activity and task shortcuts [R7]. There is a specific key conflict to resolve: the supplied DaVinci screenshots advertise Ctrl+T for microphone activation, while Claude Code documents Ctrl+T for task visibility. The parity keymap must not invoke both; retain voice through a separately configured and visible binding. Record the effective keymap in captures.

Preserve the shared Editor, drafts, Unicode, paste and cursor state. A matching-looking prompt is not sufficient if live graph updates consume its keys or redirect its messages. Graph selection inspects an agent; an explicit reply action changes the target. Keep any extra targeting control out of the reference main view unless needed for that action.

### Permissions and secondary screens must also match

The reference permission and checkpoint documentation supplies behavior for approvals and recovery [R10], [R11]. Preserve precise command/path/scope context, deliberate confirmation, safe cancellation and real authentication. Matching a screen must never grant new permissions, copy another product's credentials, or hide a failed operation.

The command guide defines additional entry points to include in the capture ledger [R13]. Help, MCP, sessions, plans, context, memory, budgets, export, voice and errors must not remain old-style screens. DaVinci-only content uses the same measured components without claiming a nonexistent Claude Code counterpart.

### Workflow progress and independent sessions are different

The official workflow guide describes task progress and drill-down [R12]. The agent-view guide addresses independently running sessions and input/reply behavior [R6]. Preserve that distinction: a worker node is not automatically a separate conversation. The graph remains optional, while real commands, branch/worktree information, waiting reasons and results remain inspectable.

The approved graph should reserve usable input, never fake concurrent execution. Different coding tasks need real isolation or explicit safe scheduling; view dismissal is not cancellation. Task completion, verification and whole-run success need separate status.

## 4. Exact DaVinci coverage inventory

The inventory below is transcribed from the inspected `Screen` and `Overlay` declarations and the view-module exports [R14], [R15]. It turns the whole-UI requirement into an explicit checklist, rather than assuming the graph, settings and model picker are the only surfaces. Coverage target: 28 screens, 5 overlays, and 49 view modules. These are inventory counts, not completed implementation counts.

| Screen | Existing surface | Required destination |
| --- | --- | --- |
| `Agent` | Main conversation | Reference welcome, transcript, composer, streaming, hints |
| `Plan` | Plan sheet | Reference plan presentation and approval flow |
| `Grafo` | Dependency study | Optional graph-family extension |
| `Memoria` | Recall view | Reference list/search/detail components with real recall data |
| `Mensura` | Token governor view | Shared Usage/detail treatment |
| `Models` | Model catalog | Reference model selection |
| `Settings` | Settings | Status / Config / Usage dialog |
| `Thinking` | Reasoning selection | Reference effort/choice treatment with real capabilities |
| `Login` | Provider credentials | Reference login hierarchy, actual DaVinci authentication |
| `Keys` | Keymap/help | Reference help and shortcut presentation |
| `Resume` | Session list | Reference resume picker |
| `Tree` | Session branches | Shared session/tree treatment, preserve branching |
| `Compact` | Compaction | Reference compact/status treatment |
| `Export` | Export ledger | Shared dialog/status components |
| `GraphRun` | Running graph | Optional graph/activity with usable input |
| `Vectors` | Memory index | Shared readable status/list components |
| `Governor` | Governor ledger | Shared Usage/detail treatment |
| `Securitas` | Security scan | Shared report and actionable findings |
| `Trust` | Workspace trust | Reference trust/approval flow |
| `Officina` | Reload/workshop | Shared extension status/list components |
| `Recovery` | Interrupted run | Reference interruption/recovery hierarchy |
| `Diff` | Change review | Reference code/diff review |
| `Mcp` | MCP management | Reference server/tool management treatment |
| `Permissions` | Permission rules/modes | Reference permissions interface; preserve policy semantics |
| `Workflows` | Workflow management | Reference workflow activity with DaVinci run data |
| `TaskBoard` | Checklist and live tasks | Reference task list/drill-down |
| `Agents` | Agents and task control | Reference agent activity; distinguish independent sessions |
| `ContextInspector` | Context memory | Reference context/detail treatment |

| Overlay | Required destination |
| --- | --- |
| `Instrumenta` | Reference command palette and search |
| `Sessions` | Reference compact session picker |
| `Cogitator` | Reference quick model picker |
| `SecretInput` | Consistent masked input; never transcript-backed |
| `Ask` | Reference choice/question dialog |

### All view modules assigned

| Component family | Modules under `crates/davinci-tui/src/davinci/views/` |
| --- | --- |
| Shell and shared surfaces | `chrome`, `sheet`, `semantic`, `startup`, `studio` |
| Conversation and results | `transcript`, `markdown`, `highlight`, `opera`, `diff`, `compact`, `export`, `recovery`, `rewind` |
| Commands and configuration | `completion`, `instrumenta`, `settings`, `cogitator`, `thinking`, `keys` |
| Security, identity, questions | `approval_modal`, `ask`, `decision_modal`, `login`, `permissions`, `secret_input`, `trust`, `securitas` |
| Sessions and execution | `agents`, `codex`, `disegno`, `memoria`, `officina`, `resume`, `task_board`, `tree`, `workflows` |
| Graph | `grafo`, `graph_canvas`, `graph_inspector`, `graph_layout`, `graph_nav`, `graph_run` |
| Integrations and telemetry | `budget`, `context_inspector`, `governor`, `mcp`, `mensura`, `vectors` |

## 5. Remaining reference captures

The main specification remains authoritative for viewport sizes and its fullscreen/classic comparison profiles. For every equivalent surface, record the reference version, OS, terminal, font, font size, cell geometry, theme, renderer, relevant settings, exact input sequence, expected state and capture hash. Variable branding/provider data are the only approved normalization categories alongside the dynamic fields named in the main spec.

The missing capture set includes first-run/login/trust, returning welcome, idle and streaming conversation, short and multiline input, slash and file completion, collapsed and expanded tool output, code/diff, settings tabs and search, model/effort changes, permission prompts, questions, resume/history/rewind, task and agent details, context/usage/help, error states, and tiny-terminal behavior.

Do not infer RGB values from the terminal emulator's background or treat a cropped historical example as the current fullscreen layout. Compare actual terminal cells/styles and event sequences, not only screenshots. A surface with documentation but no current capture remains unverified for 1:1 visual parity.

## 6. Execution and validation status

This pass is reference research and specification coverage, not implementation. No Rust product code was changed by this audit. Application compilation, unit/integration tests, event-replay evaluations and Windows terminal comparison have not been run.

The authenticated GitHub connector is available. The remote desktop connector reported no connected device. The container lacks Cargo, and its attempted GitHub clone failed DNS resolution. No user-PC checkout or running process was changed. These are local execution/capture limitations, not a claim that GitHub writes are unavailable.

The accompanying documentation check verifies exact inventory coverage, duplicate entries and source-reference definitions. It does not run the application, measure visual similarity, or establish that all linked images can be fetched. Written-spec review is still required before implementation planning under the selected architectural workflow.

## Sources

[R1]: https://github.com/anthropics/claude-code/releases/tag/v2.1.278
[R2]: https://github.com/anthropics/claude-code/blob/main/README.md
[R3]: https://github.com/anthropics/claude-code/blob/main/LICENSE.md
[R4]: https://trigger.dev/blog/10-claude-code-tips-you-did-not-know
[R5]: https://henriquesd.medium.com/claude-code-a-practical-guide-to-automating-your-development-workflow-675714db08ed
[R6]: https://code.claude.com/docs/en/agent-view
[R7]: https://code.claude.com/docs/en/interactive-mode
[R8]: https://code.claude.com/docs/en/settings
[R9]: https://code.claude.com/docs/en/model-config
[R10]: https://code.claude.com/docs/en/permissions
[R11]: https://code.claude.com/docs/en/checkpointing
[R12]: https://code.claude.com/docs/en/workflows
[R13]: https://code.claude.com/docs/en/commands
[R14]: https://github.com/J12003LPZ/davinci/blob/4ffdda411d54ba874ce1184ee8762820504c7536/crates/davinci-tui/src/davinci/model.rs
[R15]: https://github.com/J12003LPZ/davinci/blob/4ffdda411d54ba874ce1184ee8762820504c7536/crates/davinci-tui/src/davinci/views/mod.rs
