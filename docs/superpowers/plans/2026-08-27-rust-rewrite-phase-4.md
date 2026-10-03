# Phase 4 — `pi-ai`

## Goal

Port the LLM stream contract, message types, usage accounting, and a mock provider used by later phases.

## TypeScript sources

- `vendor/pi/packages/ai/src/types.ts`
- `vendor/pi/packages/ai/src/utils/event-stream.ts`
- `vendor/pi/packages/ai/src/compat.ts`

## Deliverables

- Message / Model / Context / Tool / Usage types matching TypeScript JSON.
- `AssistantMessageEvent` lifecycle: `start` → partials → `done` | `error`.
- `stream` / `complete` that never throw for model/request failures.
- Mock provider driven by fixtures (no live HTTP in CI).
- Tool-argument validation helper.

## Done when

`cargo test -p pi-ai` covers stream lifecycle, usage totals, and tool-call fixtures.
