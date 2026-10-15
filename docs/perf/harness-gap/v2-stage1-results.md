# Large-task v2 Stage 1: recorded results and attribution

Campaign `v2-stage1-20261015-run2` (local, outside Git), 24 pinned runs: 4 public v2 tasks, 3
repetitions, DaVinci (`main@8119a17`) and Codex CLI 0.162.1, `gpt-6-luna`, effort `high`, harness
`ed85b77`, manifest SHA-256 `49e83f…`. Four public task families and one machine give direction
only. They support no superiority or generalization claim.

## Recorded results (unchanged)

These figures are the campaign's own rows. Nothing in this document, the regrade in
`v2-harness-changes.md` or the reminder analysis made a model call or produced a new live result.

| | Codex | DaVinci |
|---|---|---|
| Verified successes | 12 / 12 | 11 / 12 |
| Hidden requirements passed (mean) | 100% | 99.4% |
| Unrelated edits, leaks, stops, grading failures | 0 | 0 |
| Uncached input per verified success | 31.3k | 117.4k |
| Requests per verified success | 12.8 | 20.0 |
| Median / p90 wall time (harness report) | 56.5 s / 80.8 s | 166.3 s / 201.4 s |
| Cache ratio (cached / prompt tokens) | 0.90 | 0.62 |

## The one failure: DaVinci m1 rep 0

* Grader identity is valid: the campaign's `runner_source` is `ed85b77`, `validate_fixture_manifest`
  returns no errors, the campaign `fixture_hash` matches, and the hidden grader file in the workspace
  is byte-identical to the pinned fixture.
* The failing check is `test_public_tests_pass` (`m1.public_tests`), which runs the workspace's
  public test file. The test named in the grader output, `test_cli_prints_sorted_key_value_lines`,
  is not in the pinned fixture: DaVinci wrote it. The ASCII test in the hidden file,
  `test_cli_prints_keys_sorted`, passed.
* DaVinci ran its new test four times after the completion reminder; every run passed (7 of 7) in
  the agent's environment. The grader's environment differed (see `v2-harness-changes.md`), so the
  same file failed there.
* Attribution: two things together produced the failure. DaVinci wrote an encoding-sensitive test
  (a child process prints `café`, read back as text), and the grader ran it in a Python text mode
  different from the agent's. The stored evidence supports that pair, not an agent verification or
  completion failure. The earlier "1 false completion" count and the
  "DaVinci UTF-8 defect" hypothesis in the first Stage 1 summary are withdrawn.
* The result is preserved as recorded. It is flagged as not cleanly attributable to the agent. It
  was not repaired and not rerun. A supplementary offline regrade of a copy under the matched
  environment gives 15 of 15 (see `v2-harness-changes.md`).
* No other run is exposed: across all 24 workspaces only this run has a non-ASCII test of CLI
  output.

## Deviations from the launch specification

* The output directory is `v2-stage1-20261015-run2`, because two earlier launches failed in setup
  before any model call (`…-20261015` holds only `validate` scratch files, `…-run` only a
  `campaign.json`).
* The harness was pointed at the existing login with `PI_CODING_AGENT_DIR=~\.pi\agent`, because
  `~\.davinci\agent` had no `auth.json`. No credential was changed.
* The harness clone used `core.autocrlf=false` so the manifest bytes matched the pinned hash.
* No account usage snapshot was taken before or after.
