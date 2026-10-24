# Green source and installation identity

For an exported checkout without Git, `scripts/release_identity.py snapshot`
and `record-local` record a source file manifest, unchanged before/after build
content, actual binary hash and launch path, platform, features and configuration
digest. These local records have no source commit and are never release eligible.
Store them outside the source tree so the evidence does not change its own
identity. `test-fixtures` builds are also rejected by official campaign admission.
See the [October 2 execution record](openai-harness-implementation.md) for local
verification and the outstanding live/release gates.

Production installation uses `scripts/install.sh` or
`pwsh scripts/install-davinci.ps1`. Both require a clean checkout at the exact
`v<workspace version>` tag and a completed successful full CI run for that
commit, including every workspace shard, quality checks and workflow lint.
Evidence must come from push CI: GitHub's default PR checkout tests a synthetic
merge, so its `head_sha` alone does not identify the tested source tree
([GitHub event semantics](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#pull_request)).
The product version starts at 1.0.71. No release tag is created by this PR.

The manually invoked **Tag green release** workflow verifies CI before creating
an annotated version tag. An existing tag is never moved. It creates no deployed
site and publishes no executable. A reviewer invokes it after the desired
commit has passed all checks.

The workflow is split so that candidate code never runs with a write token:

- Dispatch it from the default branch; any other ref skips the `verify` job.
  The verifier (`scripts/release_identity.py`) is always the default-branch
  commit of the dispatch (`github.sha`), never the candidate.
- `verify` has read-only permissions. It validates the inputs (a full lowercase
  40-hex commit, a strict semver version), then `verify-commit` reads the
  candidate as Git objects only: the commit must be an ancestor of the
  verifier commit, `Cargo.toml` must be a regular blob whose
  `[workspace.package].version` equals the requested version, and the CI proof
  must be a push run on the default branch for exactly that commit. No candidate
  file is checked out or executed.
- `tag` runs in the `release` environment with `contents: write`. It checks out
  the verifier commit again, repeats the verification, and creates the annotated
  tag and ref through the GitHub REST API. An existing `v<version>` tag is
  accepted only if it is an annotated tag of the verified commit; a lost race
  is re-read under the same rule, and a ref is never force-updated.
- Checkouts use `persist-credentials: false`.
- Manual step for the repository owner: add a protection rule (required
  reviewers) to the `release` environment in the repository settings. Without
  it the `tag` job runs unattended after `verify`.
- Tags created with `GITHUB_TOKEN` do not trigger tag-push workflows. Anything
  that must react to a release tag needs a different trigger.

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
