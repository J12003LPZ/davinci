# Phase 6: Coding Agent and TUI

## Overview

Port print-mode coding agent and the TUI differential renderer.

## TUI

`Component.render(width) -> lines`. Compare against previous lines. Full redraw on first frame, width change, or change above viewport. Differential path writes only changed lines. Lines must not exceed width (`truncate_to_width`).

## Coding agent

Print mode (`-p`), built-in tools `read`/`write`/`edit`/`bash`/`grep`/`find`/`ls`, JSONL session append, faux-provider suite harness. Interactive TUI binds agent events to the component tree. TypeScript remains the shipping interactive product until a later cutover.
