# Phase 8: Gate and Sign-off

## Checklist

1. `cargo test --workspace` passes.
2. `cargo clippy --workspace --all-targets -- -D warnings` is clean.
3. `cargo fmt --check` is clean.
4. Phase 7 fixtures are green.
5. TypeScript packages still compile as the authority; Rust is the port, not the product cutover.

This gate does **not** remove the TypeScript runtime. Product cutover is a later decision.
