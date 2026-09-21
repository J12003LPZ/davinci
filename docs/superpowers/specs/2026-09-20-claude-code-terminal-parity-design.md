# DaVinci: whole-terminal Claude Code parity

Status: written specification awaiting review. Product implementation has not started.

Branch: `J12003LPZ/terminal-ui-rebuild-20260920`

Inspected DaVinci base: `4ffdda411d54ba874ce1184ee8762820504c7536`

## 1. Primary requirement and precedence

Rebuild **the entire DaVinci terminal UI as a 1:1 visual and interaction match to Claude Code**. This is not a theme refresh, a Claude-inspired redesign, or a graph-first project. The user's latest clarification makes whole-terminal parity the main acceptance requirement. The previously approved readable, optional graph remains an additional DaVinci feature.

The earlier illustrative chat layout is not a visual reference. Its invented cards, always-visible branch header, and permanent message-destination row must not replace Claude Code's actual default layout. Reference captures control placement, spacing, colors, borders, glyphs, density, selection treatment, and normal interaction behavior.

Permitted differences are DaVinci's product identity, real model/provider names and capabilities, actual account/configuration data, and its additional graph and harness features. Do not claim to be Anthropic, rename a different model to Claude, invent balances, or display nonfunctional controls. Product-specific content uses the matched components instead of a second design language. Any other intentional deviation needs an explicit entry and approval; missing work is not an exception.

Outcome: someone familiar with Claude Code should be able to use DaVinci without learning a different terminal interface. A completed graph does not satisfy this outcome while other screens retain the old UI.

## 2. Fixed reference and evidence

Reference candidate: **Claude Code v2.1.278**, verified from Anthropic's official release metadata during this design pass. Pin the version rather than chasing a moving latest release. The reference is the native terminal CLI, not Claude Desktop, the browser app, or a VS Code webview. [R1]

Use an explicitly configured fullscreen reference profile for the primary Windows Terminal comparison, with a separate classic-renderer profile for existing regular/scrollback behavior. Do not mix captures from the two profiles. This explicit choice avoids relying on the reference's setup-dependent renderer default. The official fullscreen documentation distinguishes fixed-bottom input, in-app selection, and viewport behavior from classic rendering. [R2]

Before implementing shared visual primitives, capture the actual reference application. Record version, OS, terminal version, font, font size, cell dimensions, theme, renderer, relevant configuration, and the exact input sequence. Include first-run and returning-user states, with each profile isolated from personal credentials and configuration. Record unavailable account-dependent states as unavailable, not as visual matches. Public documentation grounds behavior but is not sufficient evidence for exact geometry or color values.

Capture both programs under identical terminal conditions and deterministic fixtures. Keep screenshots and recordings outside source control; commit the manifest, textual fixtures, capture procedure, and comparison report. Font files and credentials are never redistributed. Reference captures have **not** been produced in this design pass, and visual parity has **not** been verified.

## 3. Whole-application surface contract

Every row below is required. For a Claude-equivalent surface, reproduce the captured reference rather than inventing a better-looking alternative. For a DaVinci-only surface, use those same measured primitives and preserve the feature.

| Surface | Required result | Current code owners to audit |
| --- | --- | --- |
| Startup and welcome | Match hierarchy, spacing, identity placement, onboarding and returning-session presentation; remove the existing decorative masthead. | `startup`, shared `chrome` |
| Main conversation | Match user/assistant rows, wrapping, blank lines, headings, lists, markdown, code blocks, links and transcript density. | `transcript`, `markdown`, `highlight`, `semantic` |
| Prompt editor | Match separators, cursor, placeholder, multiline growth, input modes, paste and attachment presentation, queued messages and contextual hints. | `chrome`, `model`, shared editor and input routing |
| Live execution | Match working indicators, tool-call summaries, command and file presentation, output collapsing, completion, interruption and failure states. | `opera`, `transcript`, `studio`, runtime adapters |
| Commands and completion | Match `/` suggestions, filtering, selected rows, descriptions, `@` completion, help and acceptance/dismissal behavior. Preserve DaVinci commands. | `completion`, `instrumenta`, autocomplete |
| Settings and themes | Recreate the reference settings interaction and layout, including navigation, values, descriptions and applicable search. No independently designed dashboard. | `settings`, `sheet`, `theme` |
| Models and reasoning | Recreate the full model picker and quick switcher, including focus/current distinction and effort controls. Use real providers and preserve selection identity after filtering. | `cogitator`, `thinking` |
| Permissions and questions | Match trust prompts, permission choices, explanations, question forms, selection and cancellation; never weaken authorization. | `approval_modal`, `decision_modal`, `permissions`, `trust`, `ask` |
| Login and credentials | Use the same dialog language for provider selection, progress, errors and masked input. Preserve DaVinci authentication, never send its secrets to another product. | `login`, `secret_input` |
| Sessions and history | Match resume, search, session selection, history navigation and rewind presentation; retain session-tree functionality. | `resume`, `tree`, `rewind`, session overlays |
| Changes and diffs | Match changed-file lists, additions/deletions, diff focus, scrolling and expanded views. Keep source paths, branch and worktree truthful. | `diff`, `codex`, `studio` |
| Tasks and agents | Match activity and task-list conventions, details, background controls and attention states. Graph navigation remains optional. | `task_board`, `agents`, `workflows` |
| Context and utility screens | Rebuild compaction, context, memory, usage, budgets, exports, security and recovery with the same components. No legacy-screen exclusions. | `compact`, `context_inspector`, `memoria`, `vectors`, `mensura`, `governor`, `budget`, `export`, `securitas`, `recovery` |
| Extensions and help | Restyle MCP, extensions/workshop, plans, key help, notifications and voice setup/status. Retain extension output and accessible controls. | `mcp`, `officina`, `disegno`, `keys`, voice view |
| Optional graph | Apply the approved graph design within the new terminal shell, never as a replacement for it. | `grafo`, all `graph_*` modules |

The inspected `Screen` inventory is: `Agent`, `Plan`, `Grafo`, `Memoria`, `Mensura`, `Models`, `Settings`, `Thinking`, `Login`, `Keys`, `Resume`, `Tree`, `Compact`, `Export`, `GraphRun`, `Vectors`, `Governor`, `Securitas`, `Trust`, `Officina`, `Recovery`, `Diff`, `Mcp`, `Permissions`, `Workflows`, `TaskBoard`, `Agents`, `ContextInspector`.

The inspected `Overlay` inventory is: `Instrumenta`, `Sessions`, `Cogitator`, `SecretInput`, `Ask`. Also audit nested dialogs, empty/loading/error states, and extension/voice-owned rendering that are not separate enum variants. Every current surface must map to a captured reference or a named DaVinci extension; no unmapped entries may ship. [D1, D2]

## 4. Interaction parity, not just screenshots

Build a reference action matrix from the pinned CLI and official interactive documentation. Cover ordinary typing, cursor movement, word editing, undo, history, search, multiline entry, paste bursts, attachment chips, autocomplete, model switching, permission modes, transcript inspection, scrolling, backgrounding and interruption. Commands, keyboard shortcuts and mouse events must cause the corresponding state transition, not merely show a matching hint. [R3, R4]

Respect context: an editor key must not accidentally navigate a graph, accept a permission, or cancel a different task. Preserve drafts, cursor position, attachments and editor mode when opening and closing panels. Distinguish dismissing a view, interrupting foreground work and stopping a background run. Do not make permission grants an implicit result of changing layout or mode.

For settings and models, match presentation while preserving correct DaVinci configuration scope, persistence and backend identities. Only offer supported reasoning levels. Report validation/authentication failures inline; do not silently substitute a different provider or model. Reference command and model documentation supplements, but does not replace, live captures. [R4, R5, R6]

## 5. Approved graph and multitasking extension

Graph execution does not force open a canvas or remove the prompt. The normal conversation remains the default. Offer compact background status, readable activity and an optional interactive graph. Closing the graph changes visibility only.

Use meaningful task titles, clear dependency direction, stable placement, explicit running/waiting/blocked/failed/completed states, collapsible finished work and a discoverable inspector. Show what an agent is doing, the actual command and relevant output, files, Git branch/worktree and any reason it is waiting. Keep internal IDs, token details and contracts secondary. Distinguish worker completion from run verification and final success.

Wide layouts can dock the graph; narrow layouts use a readable list or focused panel with a reserved input region. Never steal selection, focus or scroll position when live updates arrive. Click and keyboard inspection must agree.

Make submission targets explicit when directing work to another conversation or a background run, without adding an unnecessary permanent control to the reference main view. Store drafts per target. Selecting a node only inspects it; sending to an agent requires an explicit action. Multiple coding tasks use isolated worktrees when they can otherwise collide. A task without Git context says so instead of showing an invented branch.

Each task/run has its own identity, lifecycle, cancellation and event stream. A late event cannot overwrite another task's state, and switching conversations cannot cancel hidden work. Reconnection reconciles authoritative snapshots. Pausing is reported as requested until the runtime confirms it. This extends the existing runtime only where necessary to make multitasking real, not simulated.

## 6. Architecture and migration boundaries

Keep Rust, Ratatui and Crossterm and retain the existing providers, tools, storage, permissions and graph execution contracts unless a tested interaction requires a targeted change. Rebuild the presentation architecture; do not rewrite unrelated execution code for the sake of starting from zero.

Separate six concerns: reference-derived theme/geometry tokens; reusable rows, pickers, dialogs and status components; conversation/editor presentation; explicit focus and navigation state; runtime-to-view event adapters; and optional graph layout/inspection. The shell owns region allocation and focus, not execution. View functions do not invoke tools, write settings or grant permissions.

Audit `crates/davinci-tui/src/davinci/{app.rs,model.rs,runtime.rs,theme.rs,ui.rs}` and every module in `views/mod.rs`, plus host integration paths discovered during implementation planning. The old shared paper labels, textured rules, fixed-width assumptions and screen-takeover behavior are not compatibility requirements. [D1, D2]

Use stable IDs across filtered/reordered lists, Unicode cell-aware clipping, explicit tiny-terminal fallbacks and bounded visible-row rendering. Provider/tool output is untrusted text: preserve secret redaction and sanitize control sequences before display or copy. Do not turn arbitrary output into executable controls.

The new reference appearance becomes the default, including upgrades from shipped brown presets. Preserve settings/session files and unknown configuration keys for rollback. Retain explicit custom themes as opt-in choices, not as the acceptance baseline. Never change permissions, telemetry consent, project trust, credentials or execution defaults merely to match a screen.

## 7. Verification and release gates

Whole-terminal parity and graph usability are **separate mandatory gates**. Neither can compensate for failure of the other.

Reference gate: every equivalent surface has a pinned capture and action trace before it is marked matched. At matched sizes and fixtures, compare terminal cell contents, foreground/background styles, emphasis, borders and positions. Normalize only identified variable fields such as product/provider identity, paths, clocks and real counters. Do not normalize away layout, wrapping, colors or selection differences. Static regions require zero unexplained cell/style differences. Assess animation timing with recorded sequences, not one still image. Missing reference evidence remains a blocker, not a pass.

Rendering matrix: 40x12 graceful fallback; 80x24; 100x30; 120x40; 160x50; dark and light reference profiles; reduced-color and no-color fallbacks. Cover Unicode, wide and combining characters, long paths, long output, empty/error/loading states and repeated resize. Test both existing regular and fullscreen execution paths; no old-theme islands may remain.

Interaction gate: deterministic tests must prove draft/attachment preservation, correct filtered model selection, live-output collapse/expand, keyboard/mouse hit-testing, permission cancellation, focus ownership, queue handling and background-task routing. Test repeated switching during updates and stopping one task while another continues. Use failure injection for dropped/reordered events and reconnect.

Execution gate: run Rust formatting, compilation, relevant crate suites, workspace regression checks and lint in the actual build environment. New interaction behavior ships with regression tests and offline event-replay evaluations. UI validation must not require paid model calls. Do not claim compilation or passing tests from static inspection.

Manual gate: exercise native Windows Terminal, including ConPTY redraw, input, paste, scroll, copy/select, mouse, resize, interruption, reconnect and terminal restoration on exit. Also smoke-test a POSIX terminal and both renderers. Record environment and actual results. A human compares all core surfaces side by side; the graph is reviewed separately for readability. Fix discrepancies rather than relabeling them as enhancements.

Release only from the isolated task branch after the complete surface inventory passes. Do not merge into `main` or restart the user's running harness without authorization. Deliver the implementation diff, tests/evaluations, parity matrix, evidence locations, exact build/restart instructions and any remaining limitations. This specification alone is not a shipped UI.

## 8. Current evidence and next review

Completed in this design pass: branch/base verification, inventory of the existing screen/overlay/view modules, official reference release verification, documentation research and a written whole-terminal acceptance contract.

Not completed: actual reference captures, implementation, compilation, runtime tests or Windows terminal comparison. Local clone access failed because the working container could not resolve GitHub; authenticated GitHub connector access remained available for this document. No user-PC checkout or running process was changed.

Review this written specification for whole-terminal scope and the explicitly configured reference profiles. After approval, write the implementation plan with whole-terminal parity first and graph integration second; review that plan and select its execution method before product code changes.

## Sources

[R1] Anthropic, Claude Code v2.1.278 release metadata: https://github.com/anthropics/claude-code/releases/tag/v2.1.278

[R2] Anthropic, fullscreen rendering: https://code.claude.com/docs/en/fullscreen

[R3] Anthropic, interactive mode: https://code.claude.com/docs/en/interactive-mode

[R4] Anthropic, commands: https://code.claude.com/docs/en/commands

[R5] Anthropic, model configuration: https://code.claude.com/docs/en/model-config

[R6] Anthropic, settings files and precedence: https://code.claude.com/docs/en/settings

[D1] Inspected DaVinci `crates/davinci-tui/src/davinci/model.rs` at commit `4ffdda411d54ba874ce1184ee8762820504c7536`.

[D2] Inspected DaVinci `crates/davinci-tui/src/davinci/views/mod.rs` at the same commit. Initial inspection of shared rendering and graph code is recorded in the associated design conversation.
