# Startup and peak memory: DaVinci vs Droid 1.1.0 (W5)

Offline, $0. No provider request was made. Only `--version` was run, one binary at a time, with no
cargo build or test running during the timing. Report-only: no product code changed.

## 1. Environment

| Item | Value |
|---|---|
| Date | 2026-10-31 (measured about 16:40, UTC-04:00) |
| OS | Windows 11 Pro, 10.0.26300 |
| CPU | AMD Ryzen 7 9800X3D 8-Core Processor |
| RAM | 61.4 GB |
| PowerShell | 7.6.9 (`pwsh`) |
| DaVinci binary | `C:\Users\sergi\.build\davinci-release\release\davinci.exe`, profile `release`, 65,749,504 B |
| DaVinci SHA-256 | `93276DF83A376AD3106E172EA2BE8F0FF235F5F20ABB0AC11A5A724376E1F83A` |
| DaVinci mtime | 2026-10-31 14:51:32 (local) |
| DaVinci `--version` | `1.1.7` |
| Droid binary | `C:\Users\sergi\bin\droid.exe`, 327,162,080 B |
| Droid SHA-256 | `7F23DA2D6D271EDD45F198B946F51A0EA7A779D7C48C7DCE3DDA2C81C52AA775` |
| Droid `--version` | `1.1.0` |
| Repo HEAD when measured | `2940cbc76f891899c0280ccc1637f2cf6b9d745a` (branch `J12003LPZ/droid-gap-stage0`) |

**The source commit of the DaVinci binary is not recorded.** The release binary was built earlier in
the run, before this measurement, and was not rebuilt. Its mtime and SHA-256 identify it, but the exact
commit it was built from is unknown. It must not be read as equal to the HEAD above, which is only the
checkout the measurement ran from.

## 2. Method

- Per binary: 1 discarded warm-up run, then 10 recorded runs. DaVinci (warm-up and 10 runs) ran first,
  then Droid (warm-up and 10 runs), in one script invocation.
- **Wall time:** `System.Diagnostics.Stopwatch` from just before `Process.Start` until `WaitForExit`,
  with stdout and stderr redirected and drained. It includes process creation and teardown. Milliseconds.
- **Peak RSS:** `Process.PeakWorkingSet64`. The retained `Process` object was polled in a tight loop
  (`Refresh()` then read) while the process ran, as the brief's fallback allows, and the maximum seen
  was kept. A post-exit read was attempted for every run and gave no usable value, so every sample is
  `sampled-while-running`. A sampled peak can miss a short spike, so these values are lower bounds on
  the true peak. The polling loop also occupies one core while the process runs.
- Peak working set counts all resident pages of that one process, shared pages included. Child
  processes were not examined.
- No cargo build or test ran during timing. No binary was installed or replaced.

### Script text (repeatable)

```powershell
$ErrorActionPreference = 'Stop'
$bins = [ordered]@{
  DaVinci = 'C:\Users\sergi\.build\davinci-release\release\davinci.exe'
  Droid   = 'C:\Users\sergi\bin\droid.exe'
}
function Invoke-Once($exe) {
  $psi = [System.Diagnostics.ProcessStartInfo]::new($exe, '--version')
  $psi.RedirectStandardOutput = $true
  $psi.RedirectStandardError = $true
  $psi.UseShellExecute = $false
  $psi.CreateNoWindow = $true
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $p = [System.Diagnostics.Process]::Start($psi)
  $peak = 0L
  $outTask = $p.StandardOutput.ReadToEndAsync()
  $errTask = $p.StandardError.ReadToEndAsync()
  while (-not $p.HasExited) {          # fallback sampler while the process runs
    try { $p.Refresh(); if ($p.PeakWorkingSet64 -gt $peak) { $peak = $p.PeakWorkingSet64 } } catch {}
  }
  $p.WaitForExit()
  $sw.Stop()
  $exitCode = $p.ExitCode
  $source = 'sampled-while-running'
  try {                                 # primary: retained Process object after exit
    $p.Refresh()
    $after = $p.PeakWorkingSet64
    if ($after -gt 0) { $peak = [Math]::Max($peak, $after); $source = 'PeakWorkingSet64-after-exit' }
  } catch {}
  [pscustomobject]@{
    WallMs = [math]::Round($sw.Elapsed.TotalMilliseconds, 2)
    PeakMiB = [math]::Round($peak / 1MB, 2)
    PeakSource = $source
    Exit = $exitCode
    Out = ($outTask.Result.Trim())
  }
}
$result = @{}
foreach ($name in $bins.Keys) {
  $null = Invoke-Once $bins[$name]       # one discarded warm-up
  $result[$name] = 1..10 | ForEach-Object { Invoke-Once $bins[$name] }
}
$result.Keys | ForEach-Object { $name = $_; $result[$name] | ForEach-Object { [pscustomobject]@{ Bin = $name; Wall = $_.WallMs; PeakMiB = $_.PeakMiB; Src = $_.PeakSource; Exit = $_.Exit; Out = $_.Out } } } | Format-Table -AutoSize
foreach ($name in $result.Keys) {
  $w = @($result[$name].WallMs | Sort-Object); $m = @($result[$name].PeakMiB | Sort-Object)
  $p90 = $w[[math]::Ceiling(0.9 * $w.Count) - 1]
  $med = { param($a) ($a[4] + $a[5]) / 2 }
  "{0}: wall median={1} p90={2} min={3} max={4} | peakMiB median={5} min={6} max={7}" -f $name, (& $med $w), $p90, $w[0], $w[-1], (& $med $m), $m[0], $m[-1]
}
```

Run as `pwsh -NoProfile -File measure.ps1`.

## 3. `--version` wall time (ms)

Raw samples, in run order (10 recorded, warm-up discarded):

| Run | DaVinci | Droid 1.1.0 |
|---|---:|---:|
| 1 | 18.13 | 83.99 |
| 2 | 17.51 | 83.25 |
| 3 | 17.37 | 90.85 |
| 4 | 17.68 | 80.98 |
| 5 | 17.60 | 82.22 |
| 6 | 17.10 | 106.50 |
| 7 | 17.68 | 82.65 |
| 8 | 17.41 | 82.91 |
| 9 | 17.65 | 81.58 |
| 10 | 16.55 | 82.41 |

| Statistic | DaVinci | Droid 1.1.0 |
|---|---:|---:|
| median | 17.56 | 82.78 |
| p90 | 17.68 | 90.85 |
| min | 16.55 | 80.98 |
| max | 18.13 | 106.50 |

p90 is the 9th of 10 sorted samples.

## 4. Peak RSS at `--version` (MiB, `PeakWorkingSet64`, sampled while running)

| Run | DaVinci | Droid 1.1.0 |
|---|---:|---:|
| 1 | 9.16 | 105.06 |
| 2 | 8.76 | 105.37 |
| 3 | 9.16 | 105.52 |
| 4 | 8.34 | 104.99 |
| 5 | 9.17 | 105.38 |
| 6 | 9.16 | 105.19 |
| 7 | 8.26 | 105.21 |
| 8 | 9.17 | 105.30 |
| 9 | 9.12 | 105.14 |
| 10 | 8.63 | 105.07 |

| Statistic | DaVinci | Droid 1.1.0 |
|---|---:|---:|
| median | 9.14 | 105.20 |
| min | 8.26 | 104.99 |
| max | 9.17 | 105.52 |

## 5. Comparison and the 25% verdict

Ratio is DaVinci / Droid on the medians; below 1 means DaVinci is lower.

| Row | DaVinci median | Droid median | Ratio | Difference | Gap above 25% against DaVinci? |
|---|---:|---:|---:|---:|---|
| `--version` wall time | 17.56 ms | 82.78 ms | 0.212 | DaVinci 78.8% lower | No. DaVinci is faster. |
| Peak RSS at `--version` | 9.14 MiB | 105.20 MiB | 0.087 | DaVinci 91.3% lower | No. DaVinci is lower. |
| Time to first provider request | not measured | not measured | n/a | n/a | not measured |

No measured row shows DaVinci worse than Droid by more than 25%, so nothing is filed under the
resource-lifecycle plan and `docs/superpowers/plans/2026-10-21-resource-lifecycle.md` is unchanged.
The plan's note that Droid 1.0.0 was about 218 MB is a different version and a different point in the
process (a session, not `--version`); it is not comparable to the 105 MiB above.

## 6. Not measured

- **Time to first provider request, both harnesses.** The Droid side needs a mock, and
  `mock-responses-server.js` is hard-wired to port 8317, where the live CLIProxyAPI runs (Task 3;
  ledger ruling R3). It was not run, the proxy was not stopped, and `run-live-script-experiments.ps1`
  was not run. No existing Droid stub that binds another port was found. DaVinci's existing offline stub
  (`docs/perf/resource-lifecycle.md`, offline CLI invocation) is an in-process stub, not a server a
  request can be timed against, and a one-sided number would not be comparable. The row is
  `not measured`, not zero, and no live request was made.
- **Peak RSS from a post-exit read.** The method fell back to sampling (section 2), so the RSS values
  are sampled lower bounds.
- **Child processes** of either binary, and cold-cache versus warm-cache behavior (OS caches were not
  reset).

## 7. What this does and does not prove

- Process start for `--version` is not task latency. It shows how heavy each runtime is to start (a
  native Rust binary against a Bun standalone runtime, per the identity doc), not how fast either
  finishes a coding task or issues its first request.
- RSS at `--version` is not RSS mid-session. Neither process had loaded a session, a model catalog,
  tools or a conversation. Mid-session memory needs a live or mocked session and is not measured here.
- The DaVinci binary's source commit is unrecorded (section 1), so this describes that binary, not a
  named commit. Ten runs on one machine, DaVinci before Droid in one script, give a descriptive
  comparison, not a statistical test. The differences (about 4.7x in wall time, 11.5x in memory) are far
  larger than the run-to-run spread, but the sampling caveat on RSS applies.
