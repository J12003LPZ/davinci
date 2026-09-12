# Claude Code differential benchmarks

The competitor benchmark is a matched experiment, not an architecture claim.
Each run uses the same immutable fixture snapshot, user request, timeout, and
independent verification commands for DaVinci and the external harness. Raw
transcripts, changed paths, verification results, and the fixture hash belong
in the artifact directory for the run.

## Comparison classes

- `harness-controlled`: the underlying model and relevant public controls are
  known and matched closely enough for a harness comparison.
- `product-system`: the products or underlying models differ. Results describe
  the observed systems and must not be presented as same-model harness proof.
- `architecture-only`: no matched product execution occurred. This class may
  document design differences but cannot support a suite superiority claim.

The report always records the class and competitor version/control status. A
missing version or unavailable binary is a configuration/infrastructure issue,
not a passing competitor result.

## Suite claim gate

An official suite claim requires at least 150 shared scenarios, three
repetitions per harness, a DaVinci pass-rate advantage of at least 3 percentage
points, a confidence interval whose lower bound is above zero (or a separately
approved non-inferiority route), no runtime-boundary failures, no higher
unrelated-edit rate, and a DaVinci median wall time no more than 15% above the
competitor. The typed gate in `davinci-evals::competitor::report` rejects
missing, invalid, or insufficient evidence.

The public-pattern challenge corpus is stored at
`crates/davinci-evals/fixtures/competitor/claude-public-challenges.json` and
contains at least 20 cases in each of five categories. It is a challenge
fixture corpus, not live Claude Code evidence; live results require the actual
binary, configured credentials, repeated matched runs, and persisted artifacts.

## Trusted workflow

`.github/workflows/competitor-differential.yml` is manual-only. It probes the
actual competitor version before building release binaries, runs the matched
suite, generates normalized aggregate output, and uploads raw evidence with
30-day retention. A missing or unidentifiable competitor version fails closed;
it cannot be represented as a competitor pass. Text artifacts are scrubbed for
configured API keys and authorization values before upload. The workflow is
intended for a trusted runner because the competitor binary is an explicit
operator-supplied input.
