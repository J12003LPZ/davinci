# davinci-server

`davinci-server` provides a background daemon and RPC server that hosts persistent Davinci agent sessions.

---

## Key Capabilities

- **Server Daemon (`server.rs`)**:
  - Binds to Unix domain sockets or TCP ports.
  - Decodes incoming requests using `davinci-protocol`.
  - Dispatches turns to `davinci-agent` and streams execution events, tool runs, and tokens back to connected clients.
- **Session Multiplexing**:
  - Isolates concurrent client sessions while sharing model provider connections and cached catalogs.

---

## Testing

```bash
cargo test -p davinci-server
```
