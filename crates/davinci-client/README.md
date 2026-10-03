# davinci-client

`davinci-client` is the client library for interacting with background Davinci agent daemons over local sockets or network TCP connections.

---

## Key Capabilities

- **Transport Support**:
  - Unix domain socket transport (on supported platforms).
  - TCP socket transport with TLS support.
- **Client Interface**:
  - High-level async/blocking APIs for initiating agent sessions, sending user prompts, streaming assistant responses, and issuing slash actions.
  - Automatically handles handshake, protocol negotiation, and CBOR framing.

---

## Testing

```bash
cargo test -p davinci-client
```
