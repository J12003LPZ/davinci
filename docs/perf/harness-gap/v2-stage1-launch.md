# Large-task v2, Stage 1: launch specification

Date: 2026-10-15. Status: **specification only. Not approved, not run.**
Nothing here has spent subscription allowance.

Stage 1 is a diagnostic DaVinci-versus-Codex CLI comparison on the four
**public** v2 tasks ([large-task-contracts.md](large-task-contracts.md)). Its
fixtures, hidden tests and references are in this repository, so its results
say nothing about generalization; only the Stage 2 protected holdout can.

## 1. Pinned inputs

| Input | Pin | How it is checked at launch |
|---|---|---|
| DaVinci source | `main` @ `8119a1744c6f60d4b1fa63a7febef10b43a65dd0` | checkpoint sidecar `source_sha`/`source_tree`; requires green full CI for this exact commit (see blocker) |
| Benchmark harness | `claude/blissful-einstein-lufuzs` @ `ed85b77a4c564d557538a9fc8197b1dda7487992` (last commit touching `scripts/bench`) | runner refuses a dirty checkout; the campaign records `runner_source` |
| Fixture manifest | `scripts/bench/large_manifest_v2.json`, SHA-256 `49e83faeb7b3091db5ea2952dbc8d180f5acb1b17c44abfd8080ce91e52309e0` | per-task public/reference/hidden hashes rechecked by `bench.py` before and during the campaign |
| Tasks | `m1-config-parser-v2`, `m2-public-api-migration-v2`, `m3-state-persistence-v2`, `m4-cli-feature-v2` | `--task-set large --large-manifest …v2.json` |
| Model | `gpt-6-luna`, both arms | `BENCH_MODEL=gpt-6-luna`; a DaVinci coding-model mismatch stops the run |
| Reasoning effort | `high`, both arms | `BENCH_EFFORT=high` (the harness default is `medium`) |
| Service tier | `default` | `BENCH_SERVICE_TIER` unset; recorded in every row |
| DaVinci settings | harness defaults, no `--settings` file: compaction on (reserve 16,384, keep 20,000), transport `auto`, `effortPolicy: fixed`, `toolSurface: full`, `autoVerify: true`, `promptProfile: stable`; requirement review at its product default (on) | recorded as `effective_settings` and `configuration_sha256` |
| Codex CLI | `0.162.1`, the version recorded on 2026-10-12 (`docs/diagnostics/2026-10-efficiency-diagnosis.md`) | `codex --version` before launch; any other version stops the launch for re-approval. Codex runs with `--ignore-user-config` |
| Schedule | 4 tasks × 3 repetitions × 2 harnesses = **24 runs**; `--order counterbalanced --order-seed 0`; `--variant v2-stage1` | the full schedule is written to `campaign.json` before the first run |
| Time limits | `BENCH_TIMEOUT=900` s per run (default); `--grading-timeout 120` | recorded in the campaign |
| Output | a new directory `C:\bench\v2-stage1-<yyyymmdd>`, never reused | runner rejects an existing `BENCH_RUNS` |

v1 and v2 stay separate: v2 runs go only into the new Stage 1 directory, never
into a directory holding v1 rows. Task IDs differ (`-v2`), and the classifier
groups results by model and task. No v1 number is ever pooled with, or used as
a baseline for, a v2 number.

## 2. Launch blockers (open)

1. **No green CI for the DaVinci commit.** On `main` @ `8119a17` CI run
   38327165232 has 40 of 42 jobs green. `workspace-test (windows-latest,
   davinci-coding-agent)` was cancelled after about 55 minutes, which fails
   `workspace-tests`. The checkpoint builder requires every job for that exact
   commit to be green, so no checkpoint can be built until that shard passes,
   by a re-run or a fix on `main`. Either is the owner's decision. Pinning a
   different commit would change this specification.
2. **Harness CI.** The `bench-v2-validation` job must be green on the harness
   commit on Linux and Windows.
3. **Windows validation on the benchmark machine.** `large_v2_validation.py`
   and the v2 tests must pass there (section 4 of the Windows workflow below).
4. **Explicit approval** of this specification.

## 3. Launch sequence (after approval; do not run before)

PowerShell on the benchmark machine. `$Work` is the analysis clone from the
Windows workflow below, checked out at the harness pin.

```powershell
$Work = Join-Path $env:TEMP "davinci-analysis"
$Src  = Join-Path $env:TEMP "davinci-source-8119a17"
$Ckpt = Join-Path $env:TEMP "davinci-checkpoint-8119a17"
git -C $Work checkout --detach ed85b77a4c564d557538a9fc8197b1dda7487992
git clone --quiet https://github.com/J12003LPZ/davinci $Src
git -C $Src checkout --detach 8119a1744c6f60d4b1fa63a7febef10b43a65dd0
python "$Work\scripts\bench\runner.py" --repo $Src --output $Ckpt      # fails until blocker 1 is cleared
python "$Work\scripts\bench\make_large_tasks_v2.py"                     # regenerates tasks; manifest bytes unchanged
git -C $Work status --porcelain                                          # must print nothing
codex --version                                                          # must be 0.162.1
$env:BENCH_RUNS   = "C:\bench\v2-stage1-$(Get-Date -Format yyyyMMdd)"
$env:BENCH_MODEL  = "gpt-6-luna"
$env:BENCH_EFFORT = "high"
$env:BENCH_DAVINCI = "<the executable copied into $Ckpt>"
python "$Work\scripts\bench\bench.py" validate --task-set large --large-manifest "$Work\scripts\bench\large_manifest_v2.json"
python "$Work\scripts\bench\bench.py" run --harness davinci codex --task-set large `
  --large-manifest "$Work\scripts\bench\large_manifest_v2.json" --reps 3 `
  --order counterbalanced --order-seed 0 --variant v2-stage1
python "$Work\scripts\bench\bench.py" report
python "$Work\scripts\bench\bench.py" gate
```

Record an account usage snapshot before and after; it covers the whole
account, so the difference is not attributed to the campaign alone.

## 4. Stopping rules (fixed before execution)

- The harness's own stops end the campaign: terminal provider error,
  credentials, quota or subscription exhaustion, cleanup or change-inventory
  failure, forbidden artifact, grading failure, binary/manifest/catalog change
  during the campaign, or a coding-model mismatch.
- After any stop: no reruns, no stitching rows into this or a new campaign. A
  stopped campaign is reported as incomplete with its rows as recorded, and is
  not a paired result. A fresh attempt is a new campaign directory and needs
  approval again.
- No changes to the machine, the checkouts, settings or credentials during the
  campaign; no other model jobs on the account.
- Hard ceiling: 24 runs. Nothing is added after seeing results.

## 5. Metrics (fixed before execution)

All per harness, over all 12 scheduled runs; failed and stopped attempts stay
in denominators; missing usage stays unknown and is never counted as zero.

| Metric | Definition |
|---|---|
| Verified success (primary) | runs where `pass` is true: exit 0, hidden grader pass, cleanup confirmed, no unrelated changes, no transaction leak, no forbidden artifact |
| Hidden requirements passed | per run `tests_passed / (tests_passed + tests_failed)`; mean and per task |
| False completion | runs with exit 0 and a final answer whose hidden grader failed |
| Failure basis | from `large_task_failures.py`; every v2 test is specified, so each failure counts |
| Uncached input per verified success | Σ(`input_tokens − cached_tokens`) over all runs ÷ verified successes; undefined at zero successes |
| Total input and output per verified success | Σ`input_tokens` and Σ`output_tokens` ÷ verified successes |
| Requests per verified success | Σ`requests` ÷ verified successes |
| Wall time | median and p90 of `wall_s`, all runs; successful runs separately |
| Unrelated edits | runs with a non-empty `unrelated` list |

Interpretation: four task families with three repetitions give direction only,
with whole-task cluster bootstrap intervals from `bench.py report`. No
superiority claim, no generalization claim, no promotion decision.

## 6. Windows evidence workflow (offline, runs nothing live)

Copy and paste into PowerShell. It touches only `%TEMP%`: a fresh clone, a
private virtual environment and an output folder. It does not touch your active
DaVinci checkout, and it only reads `C:\bench`.

```powershell
# 1. Fresh clone of the analysis branch into %TEMP%
$Branch = "claude/blissful-einstein-lufuzs"
$Work   = Join-Path $env:TEMP "davinci-analysis"
$Out    = Join-Path $env:TEMP "davinci-evidence"
if (Test-Path $Work) { Remove-Item -Recurse -Force $Work }
git clone --quiet --branch $Branch --single-branch https://github.com/J12003LPZ/davinci $Work
New-Item -ItemType Directory -Force $Out | Out-Null
git -C $Work log -1 --format=%H | Out-File -Encoding utf8 (Join-Path $Out "analysis-commit.txt")

# 2. Isolated Python environment with the pinned test tooling
python -m venv (Join-Path $Out "venv")
$Py = Join-Path $Out "venv\Scripts\python.exe"
& $Py -m pip install --disable-pip-version-check -r "$Work\scripts\bench\v2-validation-requirements.txt"

# 3. Validate v2 on Windows (offline; runs the hidden tests locally)
& $Py "$Work\scripts\bench\large_v2_validation.py" | Out-File -Encoding utf8 (Join-Path $Out "v2-validation.json")
"v2 validator exit code: $LASTEXITCODE"
$env:BENCH_REQUIRE_BEHAVIORAL = "1"
& $Py -m unittest discover -s "$Work\scripts\bench\tests" -p "test_large_fixtures_v2.py"
Remove-Item Env:BENCH_REQUIRE_BEHAVIORAL

# 4. Locate campaigns with large-task runs (read-only)
& $Py "$Work\scripts\bench\large_task_failures.py" --locate C:\bench | Out-File -Encoding utf8 (Join-Path $Out "locate.jsonl")
$found = Get-Content (Join-Path $Out "locate.jsonl") | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json }
$found | Format-Table path, models, large_runs, large_passes, grader_logs, original_baseline_candidate -AutoSize

# 5. Classify the GPT-5.6 baseline and the GPT-6 campaigns separately
$base = @($found | Where-Object { $_.original_baseline_candidate } | ForEach-Object { $_.path })
$gpt6 = @($found | Where-Object { -not $_.original_baseline_candidate -and $_.models -contains "gpt-6-luna" } | ForEach-Object { $_.path })
if ($base.Count) { & $Py "$Work\scripts\bench\large_task_failures.py" @base --out (Join-Path $Out "gpt-5.6") } else { "No GPT-5.6 baseline found under C:\bench" }
if ($gpt6.Count) { & $Py "$Work\scripts\bench\large_task_failures.py" @gpt6 --out (Join-Path $Out "gpt-6") } else { "No GPT-6 campaigns found under C:\bench" }
explorer $Out
```

Return these two sanitized summaries:

1. `%TEMP%\davinci-evidence\gpt-5.6\large-task-summary.json`, the original
   GPT-5.6 Luna baseline;
2. `%TEMP%\davinci-evidence\gpt-6\large-task-summary.json`, the GPT-6 Luna
   campaigns.

Also report the validator exit code printed in step 3; `v2-validation.json`
holds only test names and may be shared too. Do not share `locate.jsonl` if
your folder names are private, and never share `.stdout.jsonl`,
`.grader.json`, wire dumps or anything under `_agent`. If step 5 prints "No
GPT-5.6 baseline found", the baseline is not under `C:\bench`: rerun step 4
with the folder that holds it in place of `C:\bench`.
