# Optional Codemode host admission

This directory contains the pinned optional host, bounded worker adapter, framing, build-time packager, notices, and characterization fixtures. Read-only execution is wired through Rust; controlled execution remains unavailable pending recovery acceptance.

Exact packages: `@earendil-works/pi-codemode@1.0.2`, `quickjs-wasi@3.6.2`; required runtime: Node 24.21.0. The lock and embedded asset manifest are committed sources. Build preparation uses `npm ci --ignore-scripts`; normal DaVinci startup never installs anything. Final native-platform admission remains pending.

The upstream collector exceeded the strict 1 MiB UTF-8 collection ceiling. `bounded-worker.mjs` replaces that collector and enforces a shared console/output/return limit before worker messages retain output. Exhaustion closes admission even if the guest catches it. `strict-prelude.mjs` validates JSON snapshots before dispatch. Tests retain the upstream failure characterization separately.

`package-bundle.mjs NEW_DIRECTORY MANIFEST_PATH` emits a core runtime bundle and an external manifest. Runtime admission trusts the manifest embedded in Rust, requires an exact complete inventory, rejects links/reparse points and workspace-provided hosts, and rechecks hashes for each execution. Rebuilding a bundle requires rebuilding DaVinci with its matching trusted manifest. See the execution record for verified checks and open acceptance gates.
