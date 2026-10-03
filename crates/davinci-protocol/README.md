# davinci-protocol

`davinci-protocol` defines the binary wire protocol used for inter-process communication (IPC) between Davinci clients, daemons, and embedded runners.

---

## Wire Format

- **Framing (`framing.rs`)**:
  - Length-prefixed frame encoder/decoder.
  - Strict size bounds and recursion depth caps to protect against denial-of-service and malformed stream frames.
- **Serialization (`cbor.rs`)**:
  - Compact Binary Object Representation (CBOR) encoding.
  - Faster serialization and significantly reduced payload sizes compared to JSON for IPC streams.
- **Messages & RPC Contract (`protocol.rs`, `types.rs`)**:
  - Strongly-typed `Request`, `Response`, `Notification`, and `Error` envelopes.
  - Version negotiation (`PROTOCOL_VERSION`) validated on initial handshake.

---

## Testing

```bash
cargo test -p davinci-protocol
```
