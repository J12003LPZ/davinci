# Legacy TypeScript Packages (`packages/`)

This directory contains legacy TypeScript package stubs from the early phases of the TypeScript-to-Rust migration.

> [!NOTE]
> **Active Implementation**: The production, authoritative codebase is written in Rust and lives under [`crates/`](../crates/).
> 
> **Behavioral Reference**: The complete, authoritative reference TypeScript codebase (~1,169 files) is preserved under [`vendor/pi/`](../vendor/pi/).

---

## Directory Contents

- `agent/`: Legacy TypeScript agent wrapper stub (`@pi/agent`)
- `ai/`: Legacy TypeScript provider client stub (`@pi/ai`)
- `client/`: Legacy TypeScript protocol client stub (`@pi/client`)
- `core/`: Legacy TypeScript core types stub (`@pi/core`)
- `server/`: Legacy TypeScript daemon server stub (`@pi/server`)
- `session-sqlite/`: Legacy TypeScript SQLite session storage stub (`@pi/session-sqlite`)

These stubs are retained for backward traceability and early migration contract verification. For all active development, use the Rust workspace in [`crates/`](../crates/).
