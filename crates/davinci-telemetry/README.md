# davinci-telemetry

`davinci-telemetry` provides structured tracing, event logging, performance metrics, and telemetry export.

---

## Key Capabilities

- **Structured Events (`events.rs`)**:
  - Traces turn lifecycle, tool invocation durations, token generation speeds, and error rates.
- **Provider & Codex Telemetry**:
  - Formats anonymized performance statistics compatible with enterprise telemetry collectors.
  - Respects telemetry opt-outs and offline operation flags (`PI_TELEMETRY=0`, `PI_OFFLINE=1`).

---

## Testing

```bash
cargo test -p davinci-telemetry
```
