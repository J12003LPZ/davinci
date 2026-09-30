# Green source and installation identity

Production installation uses `scripts/install.sh` or
`pwsh scripts/install-davinci.ps1`. Both require a clean checkout at the exact
`v<workspace version>` tag and a completed successful full CI run for that
commit, including every workspace shard, quality checks and workflow lint.
The product version starts at 1.0.71. No release tag is created by this PR.

The manually invoked **Tag green release** workflow verifies CI before creating
an annotated version tag. An existing tag is never moved. It creates no deployed
site and publishes no executable. A reviewer invokes it after the desired
commit has passed all checks.

The installed executable has an adjacent `davinci.identity.json` (Unix) or
`davinci.exe.identity.json` (Windows). Schema 3 records the exact source commit,
tree, clean-source digest, version tag, CI run/URL, complete CI evidence and its
digest, binary SHA-256 and build time. Source is checked before and after the
build. The install scripts fail if authentication, CI, tags or provenance are
unavailable. Developer builds remain available through `make build`.

Benchmark checkpoints use the same green-CI preflight and schema 3. They may use
an untagged development commit, but cannot benchmark a dirty or unverified
build. Older schema 2 sidecars require rebuilding after full CI succeeds.

`davinci update --self` names the source install scripts. Plain `davinci update`
continues to update extension packages; the upstream package-manager parity
implementation remains unchanged.

Current evidence: the plan baseline's Windows coding-agent shard failed. A
successful Linux build does not establish a Windows fix, and a green draft PR
does not establish green main or a released tag. Full parallel Windows evidence
is retained by the Readiness Windows evidence workflow.
