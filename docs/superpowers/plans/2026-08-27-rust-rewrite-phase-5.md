# Phase 5 — `pi-agent`

## Goal

Port the agent loop, stateful `Agent`, and tool execution modes.

## TypeScript sources

- `vendor/pi/packages/agent/src/agent-loop.ts`
- `vendor/pi/packages/agent/src/agent.ts`
- `vendor/pi/packages/agent/src/types.ts`

## Deliverables

- `agent_loop` / `agent_loop_continue`.
- Sequential and parallel tool execution.
- Steer after turn; follow-up when the agent would stop.
- `stopReason == length` fails remaining tool calls.
- Events: `agent_start`, `turn_start`, `message_*`, `tool_execution_*`, `turn_end`, `agent_end`.

## Done when

Agent-loop tests for steer, follow-up, parallel tools, and length-truncation pass.
