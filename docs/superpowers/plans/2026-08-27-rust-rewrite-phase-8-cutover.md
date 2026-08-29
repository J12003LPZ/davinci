# Phase 8: Final Cutover and Production Sign-off

## Overview
Phase 8 defines the formal cutover checklist:
1. `cargo test --workspace` 100% pass.
2. `cargo clippy --workspace --all-targets -- -D warnings` zero errors/warnings.
3. `cargo fmt --check` cleanly formatted.
4. Parity fixtures green.
