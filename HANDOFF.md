# Provider prompt-cache parity — resumed

Tasks 7 and 8 are committed on `rust-rewrite`.

- `16d5f3e` — `feat(ai): session-affinity headers for prompt-cache routing`
- `ae6c603` — `test(ai): compaction retention none regression for cache fields`

Plan-specific verification:
- `cargo test -p pi-ai`: passed (Task 7; 100/100 at that point)
- `cargo test -p pi-ai compaction_retention_none_strips_all_cache_fields`: passed
- `cargo fmt --all -- --check`: passed
- `cargo clippy --workspace --all-targets -- -D warnings`: passed

Full `cargo test --workspace` was rerun after Task 8 and is blocked by one unrelated pre-existing dirty Davinci/TUI test failure:
`davinci_session::tests::a_trusted_project_opens_without_a_warning` in `crates/pi-coding-agent/src/davinci_session.rs` (310 other tests in that binary passed). That unrelated file was not modified or staged for this plan.

`make clippy` could not be invoked because `make` is not installed on PATH; the Makefile-equivalent cargo clippy command above passed.

Unrelated dirty files were left uncommitted.
