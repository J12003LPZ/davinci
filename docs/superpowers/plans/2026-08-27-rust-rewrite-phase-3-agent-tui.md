# Phase 3: pi-ai

## Overview

Port the pi-ai stream protocol and faux provider. `StreamFn` never throws: failures become a terminal assistant message with `stopReason: error|aborted`.

## Must-have

- Message/content unions: text, thinking, image, toolCall
- Events: `start`, `text_*`, `thinking_*`, `toolcall_*`, `done`, `error`
- Faux provider with scripted steps (the unit-test authority)
- `validate_tool_arguments` against JSON Schema-shaped parameters
- At least OpenAI-compatible and Anthropic-shaped request builders (offline; no live network in default tests)
