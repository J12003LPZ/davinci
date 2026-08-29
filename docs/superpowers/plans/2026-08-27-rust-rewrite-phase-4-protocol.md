# Phase 4: Agent Core

## Overview

Port `packages/agent` loop contracts into `crates/pi-agent`.

## Loop

Outer follow-up loop, inner tool+steering loop. `prepareNextTurn` may update model/thinking. Steering injects messages. Stream assistant. `stopReason == length` fails every tool call (truncated arguments are unsafe). Parallel tools emit `tool_execution_end` in completion order; tool-result messages stay in assistant source order. Batch `terminate` only if every result sets terminate.

## Tests

Event order, sequential vs parallel tools, length-stop fail-all, steering/follow-up, abort.
