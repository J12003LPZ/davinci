# davinci-core (Legacy Archive)

> [!WARNING]
> **Legacy / Uncompiled**: `davinci-core` is an early prototype crate from Phase 0/1 of the migration. It is **not** an active Cargo workspace member and is not compiled into the `davinci` binary.
> 
> The active protocol, framing, and serialization types live in [`davinci-protocol`](../davinci-protocol/).

---

## Historical Context

During the initial transition from TypeScript to Rust, `davinci-core` served as the first prototype for length-prefixed framing and CBOR serialization. Its production responsibilities have been entirely assumed by `davinci-protocol`.

This directory is preserved strictly for historical reference. Do not add dependencies to this crate.
