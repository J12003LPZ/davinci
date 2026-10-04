# Optional Codemode host admission

This directory currently contains dependency-characterization fixtures only. It is not a production host and the CLI does not load it.

Proposed exact packages: `@earendil-works/pi-codemode@1.0.2`, `quickjs-wasi@3.6.2`; maintained test runtime: Node 24.21.0. The full package-lock, asset manifest, licensing and native-platform evidence remain admission gates. CI resolves the package lock and uses `npm ci --ignore-scripts`; normal DaVinci startup never installs anything.

The upstream API exposes a 16 Mi-character output collector. A fixture records this gap against DaVinci's strict 1 MiB UTF-8 collection ceiling. A bounded adapter or reviewed upstream fix is required before production capabilities may be connected. Small final output alone is not admission evidence.
