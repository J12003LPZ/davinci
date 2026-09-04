# davinci-parity

`davinci-parity` provides differential conformance and golden-fixture test suites ensuring exact behavioral parity between the Rust implementation and the reference TypeScript version (`vendor/pi`).

---

## Key Capabilities

- **Golden Fixture Verification**:
  - Compares token streams, prompt assembly outputs, compaction summaries, and session serializations against reference golden snapshots.
  - Ensures bug-for-bug behavioral compatibility where required by the wire contract.
- **Fixture Runner (`runner.rs`)**:
  - Validates edge-case tool outputs, multi-line diff representations, and permission prompts.

---

## Testing

```bash
cargo test -p davinci-parity
cargo run -p davinci-parity
```
