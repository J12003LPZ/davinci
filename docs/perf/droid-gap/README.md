# DaVinci ↔ Droid gap planning bundle

Created 2026-10-31 and revised the same day (v2) after a review against DaVinci `main` `e67d80a94119dcf4bee8bf873892b3f66b3d3638`, the Droid runtime reference, Factory's 2026-10-29 article, the current OpenAI docs, and the machine's current state. The v1 bundle is not carried into this repository.

## Files

- `docs/superpowers/plans/2026-10-31-droid-gap.md` is plan v2, landed at the path its D6 decision names. It measures the DaVinci–Droid gap first (Stage 0), then runs only the workstreams whose gap was measured. Code changes go through the plans that already own them.
- `docs/superpowers/specs/2026-10-31-droid-openai-runtime-parity-design.md` is the design spec. Its amendment header names the parts v2 supersedes.
- `docs/perf/droid-gap/2026-10-31-droid-parity-crosswalk.csv` is a reference ledger, not a gate: 78 article claims, 26 Droid runtime contracts and 3 OpenAI API items. Its `plan_tasks` column still uses v1 task IDs; the plan's "v1 task disposition" table maps them to v2.

## Using this bundle

1. Run the offline Stage 0 steps P0.1–P0.4, plus S1 and S2. They need no approval. The mock re-run needs no proxy stop: it runs on a copy of the mock at port 18317 (see "Mock re-run" below).
2. Approve the live gap study P0.5 separately: 3 smoke runs and 51 study runs, on the pins and model named in the request. The whole program is capped at 120 live runs unless re-approved.
3. Run Stage 1 workstreams only for the gaps that P0.5 reports.

## Limits

- Droid on this machine is now 1.1.0 (SHA-256 `7f23da2d…aa775`), not the studied 1.0.0. Its OpenAI traffic goes through a local CLIProxyAPI, which is part of the system under test.
- Factory's production numbers are vendor-reported. The reference's Script savings come from a scripted mock, not a provider bill.
- The planning bundle itself changed no code and made no provider request. This branch additionally carries the two offline correctness fixes the plan authorizes without a trigger (S1, S2).

## Mock re-run (Task 11)

The P0.1 mock suite, the P0.2 Droid columns, the P0.3 Droid and DaVinci self-test rows and the W5 time to first provider request were run on 2026-10-31 against a local mock, without stopping CLIProxyAPI. Method: the mock copy listened on `127.0.0.1:18317` (never 8317); Droid ran with a per-process `--settings` file that defines only the alias `custom:mock-probe-0` (the `Codex-via-ChatGPT` alias was never used); every Droid and DaVinci process ran with `HTTP_PROXY=HTTPS_PROXY=http://127.0.0.1:9` and `NO_PROXY=127.0.0.1,localhost`; `~\.factory\settings.json` and the CLIProxyAPI config were not written. Results: `droid-1.1.0-identity.md` section 7 (facts changed, same or still not re-checked), `first-request-anatomy.md` section 1b, `startup-memory.md` section 5b. The self-test rows and the one DaVinci caveat (usage unknown on a non-Codex provider) are in the Task 11 report.
