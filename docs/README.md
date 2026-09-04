# Davinci Documentation Index

Welcome to the **Davinci** technical documentation hub. This directory contains architectural specifications, implementation plans, design systems, security reviews, and historical archives for the Davinci coding agent.

---

## Directory Organization

```
docs/
├── superpowers/
│   ├── specs/        # Architectural specifications & technical design documents
│   └── plans/        # Step-by-step implementation & phase delivery plans
├── ui/               # Terminal User Interface design system, mockups & prototypes
├── security/         # Security reviews, threat models, and scan reports
└── archive/          # Historical milestone handoffs and transition notes
```

---

## 1. Technical Specifications (`docs/superpowers/specs/`)

Comprehensive specifications detailing the design contracts and algorithms:

| Specification | Focus Area |
| :--- | :--- |
| [`2026-09-01-competitive-harness-roadmap.md`](superpowers/specs/2026-09-01-competitive-harness-roadmap.md) | High-level roadmap and competitive capability targets against Claude Code and Codex CLI. |
| [`2026-09-03-codex-efficiency-design.md`](superpowers/specs/2026-09-03-codex-efficiency-design.md) | Codex provider optimizations, payload caching, and tool execution efficiencies. |
| [`2026-09-02-davinci-instruments-fidelity-design.md`](superpowers/specs/2026-09-02-davinci-instruments-fidelity-design.md) | TUI command sheet fidelity, ratatui layouts, interactive panels, and status meters. |
| [`2026-09-02-harness-throughput-design.md`](superpowers/specs/2026-09-02-harness-throughput-design.md) | Multi-lane tool scheduling, parallel execution, context pruning, and batch dispatch. |
| [`2026-09-01-native-mcp-design.md`](superpowers/specs/2026-09-01-native-mcp-design.md) | Native Model Context Protocol (MCP) client implementation over stdio and SSE transports. |
| [`2026-09-01-plan-and-subagents-design.md`](superpowers/specs/2026-09-01-plan-and-subagents-design.md) | Subagent spawning, `/plan` mode mutation freezing, and task fan-out. |
| [`2026-09-01-tools-that-compete-design.md`](superpowers/specs/2026-09-01-tools-that-compete-design.md) | Modern tool suite (`grep`, `find`, `ls`, `web_fetch`, `web_search`, `todo`, `apply_patch`). |
| [`2026-09-01-trust-and-control-design.md`](superpowers/specs/2026-09-01-trust-and-control-design.md) | Permission gates (`auto`, `ask`, `edits`, `read-only`), allow/deny rules, and interactive approval. |
| [`2026-08-31-provider-prompt-cache-parity-design.md`](superpowers/specs/2026-08-31-provider-prompt-cache-parity-design.md) | Prompt caching parity with Anthropic, OpenAI, and Bedrock. |
| [`2026-09-01-hooks-and-observability-design.md`](superpowers/specs/2026-09-01-hooks-and-observability-design.md) | Lifecycle hooks (`pre_tool`, `post_tool`, `stop`) and event streaming. |

---

## 2. Implementation Plans (`docs/superpowers/plans/`)

Phased engineering execution plans that drove the TypeScript-to-Rust migration and feature implementations:

- **Migration Phases**: [`Phase 1`](superpowers/plans/2026-08-27-rust-rewrite-phase-1.md) through [`Phase 8`](superpowers/plans/2026-08-27-rust-rewrite-phase-8-cutover.md) detailing protocol, storage, agent loop, TUI, and conformance testing.
- **Master Plan**: [`2026-08-27-rust-rewrite-program.md`](superpowers/plans/2026-08-27-rust-rewrite-program.md).
- **Feature Plans**: Provider prompt caching, native MCP, trust & control, and instruments fidelity.

---

## 3. UI Design System (`docs/ui/`)

- [`design.md`](ui/design.md): Core design tenets, color palette, typography, grid, glyph guidelines, and animation constraints.
- [`Pi TUI Instruments.dc.html`](ui/Pi TUI Instruments.dc.html): Visual canvas export containing command sheets and meters (`3a` through `6d`).
- [`Pi TUI Mockups.dc.html`](ui/Pi TUI Mockups.dc.html): Visual canvas export for transcript screens (`1a` through `2c`).
- [`davinci_tui/`](ui/davinci_tui/): Prototype Elixir/Ratatouille reference implementation for UI prototyping.

---

## 4. Security & Governance (`docs/security/`)

- [`report.md`](security/report.md): Codex Security static code analysis and threat model audit review. Covers asset boundaries, tool permissions, and credential protections.

---

## 5. Historical Archives (`docs/archive/`)

- [`HANDOFF.md`](archive/HANDOFF.md): Historical milestone handoff document from the landing of phases 1–6.
