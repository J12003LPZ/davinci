<#
.SYNOPSIS
Offline Windows evidence for the large-task v2 Stage 1 benchmark.

.DESCRIPTION
Runs no model and spends no subscription allowance. In order, and stopping at
the first failure:

  1. checks that this clone is clean, that scripts/bench matches the harness
     pin and that the v2 manifest has the pinned SHA-256;
  2. fingerprints every file under each -BenchRoot (path, size, write time);
  3. creates a private virtual environment and installs the pinned test tools;
  4. runs the v2 behavioral validator and the v1/v2 fixture and classifier
     regression tests;
  5. locates large-task campaigns under -BenchRoot (read-only) and classifies
     the GPT-5.6 baseline and the GPT-6 campaigns into separate folders;
  6. fingerprints -BenchRoot again and fails if anything changed.

It writes only to -OutDir and -VenvDir, which must not exist yet. Exit code 0
means every step passed; anything else means STOP, with the reason printed.
Works in Windows PowerShell 5.1 and PowerShell 7.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\collect-v2-evidence.ps1 `
  -OutDir $env:TEMP\v2-evidence-1\evidence -VenvDir $env:TEMP\v2-evidence-1\venv
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$VenvDir,
    [string[]]$BenchRoot = @('C:\bench'),
    [string]$Python = 'python',
    [string]$HarnessPin = 'ed85b77a4c564d557538a9fc8197b1dda7487992',
    [string]$ManifestSha256 = '49e83faeb7b3091db5ea2952dbc8d180f5acb1b17c44abfd8080ce91e52309e0',
    # Test hook: run every precondition, then stop before anything is created.
    [switch]$PreflightOnly
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
trap {
    Write-Host "STOP: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}

$Repo = Split-Path -Parent $PSScriptRoot
$Bench = Join-Path $Repo 'scripts\bench'

function Get-FullPath([string]$Path) {
    [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
}

function Test-Inside([string]$Child, [string]$Parent) {
    $c = (Get-FullPath $Child) + '\'
    $p = (Get-FullPath $Parent) + '\'
    $c.StartsWith($p, [StringComparison]::OrdinalIgnoreCase)
}

function Invoke-Git([string[]]$Arguments) {
    $global:LASTEXITCODE = 0
    $output = & git -C $Repo @Arguments
    if ($LASTEXITCODE -ne 0) { throw "git $($Arguments -join ' ') failed with exit code $LASTEXITCODE" }
    $output
}

# Start-Process passes one command line; quote each argument. A trailing
# backslash would escape the closing quote, so it is dropped.
function Invoke-Checked([string]$Name, [string]$FilePath, [string[]]$Arguments, [string]$StdoutFile) {
    Write-Host "==> $Name"
    $quoted = @($Arguments | ForEach-Object { '"' + ($_ -replace '\\+$', '') + '"' })
    $options = @{ FilePath = $FilePath; ArgumentList = $quoted; WorkingDirectory = $Repo
                  NoNewWindow = $true; Wait = $true; PassThru = $true }
    if ($StdoutFile) { $options.RedirectStandardOutput = $StdoutFile }
    $process = Start-Process @options
    if ($process.ExitCode -ne 0) { throw "$Name failed with exit code $($process.ExitCode)" }
}

function Get-TreeFingerprint([string]$Root) {
    $full = Get-FullPath $Root
    $denied = @()
    $files = @(Get-ChildItem -LiteralPath $full -Recurse -File -Force -ErrorAction SilentlyContinue -ErrorVariable +denied)
    $lines = $files | ForEach-Object {
        '{0}|{1}|{2}' -f $_.FullName.Substring($full.Length), $_.Length, $_.LastWriteTimeUtc.Ticks
    } | Sort-Object -CaseSensitive
    $bytes = [Text.Encoding]::UTF8.GetBytes(($lines -join "`n"))
    $sha = [Security.Cryptography.SHA256]::Create()
    [ordered]@{
        root           = $full
        files          = $files.Count
        bytes          = [int64]($files | Measure-Object Length -Sum).Sum
        unreadable     = $denied.Count
        listing_sha256 = (($sha.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') }) -join '')
    }
}

function Write-Utf8([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding $false))
}

# 1. Preconditions. Nothing is created before all of them pass.
Write-Host '==> Preconditions'
if (Invoke-Git @('status', '--porcelain')) { throw "the clone at $Repo has uncommitted changes" }
$head = (Invoke-Git @('rev-parse', 'HEAD')).Trim()
$global:LASTEXITCODE = 0
& git -C $Repo merge-base --is-ancestor $HarnessPin HEAD
if ($LASTEXITCODE -ne 0) { throw "harness pin $HarnessPin is not an ancestor of $head" }
& git -C $Repo diff --quiet $HarnessPin HEAD -- scripts/bench
if ($LASTEXITCODE -ne 0) { throw "scripts/bench differs from harness pin $HarnessPin; this is not the pinned harness" }
$manifest = Join-Path $Bench 'large_manifest_v2.json'
$actual = (Get-FileHash -LiteralPath $manifest -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $ManifestSha256) {
    throw "large_manifest_v2.json SHA-256 is $actual, expected $ManifestSha256 (line endings converted on checkout?)"
}
foreach ($target in @($OutDir, $VenvDir)) {
    if (Test-Path -LiteralPath $target) { throw "$target already exists; choose a new, unique folder" }
    if (Test-Inside $target $Repo) { throw "$target is inside the clone; choose a folder outside it" }
}
if ((Get-FullPath $OutDir) -eq (Get-FullPath $VenvDir)) { throw '-OutDir and -VenvDir must differ' }
foreach ($root in $BenchRoot) {
    if (-not (Test-Path -LiteralPath $root -PathType Container)) { throw "bench root $root does not exist; pass -BenchRoot <folder with the campaigns>" }
    foreach ($target in @($OutDir, $VenvDir)) {
        if (Test-Inside $target $root) { throw "$target is inside bench root $root, which must stay untouched" }
    }
}
if (-not (Get-Command $Python -CommandType Application -ErrorAction SilentlyContinue)) {
    throw "'$Python' is missing or older than 3.10 (install Python 3.10+ or pass -Python)"
}
$global:LASTEXITCODE = 0
$pythonVersion = (& $Python -c 'import sys; print(sys.version.split()[0]); sys.exit(sys.version_info < (3, 10))') 2>$null
if ($LASTEXITCODE -ne 0 -or -not $pythonVersion) { throw "'$Python' is missing or older than 3.10 (install Python 3.10+ or pass -Python)" }
Write-Host "    clone $head, harness pin matches, manifest $actual, Python $pythonVersion"
if ($PreflightOnly) { Write-Host 'PREFLIGHT OK'; exit 0 }

# 2. Fingerprint the campaign folders before reading them.
New-Item -ItemType Directory -Path $OutDir | Out-Null
Write-Host '==> Fingerprinting bench roots (before)'
$before = @($BenchRoot | ForEach-Object { Get-TreeFingerprint $_ })

# 3. Private environment with the pinned test tooling.
Invoke-Checked 'Create virtual environment' $Python @('-m', 'venv', $VenvDir)
$Py = Join-Path $VenvDir 'Scripts\python.exe'
Invoke-Checked 'Install pinned test tooling' $Py @('-m', 'pip', 'install', '--disable-pip-version-check',
    '--no-input', '-r', (Join-Path $Bench 'v2-validation-requirements.txt'))
Invoke-Checked 'Record installed packages' $Py @('-m', 'pip', 'freeze', '--disable-pip-version-check') (Join-Path $OutDir 'pip-freeze.txt')

# 4. Offline validation and regression tests. BENCH_REQUIRE_BEHAVIORAL makes
# the behavioral test fail, not skip, when pytest is unavailable.
Invoke-Checked 'v2 behavioral validation' $Py @((Join-Path $Bench 'large_v2_validation.py')) (Join-Path $OutDir 'v2-validation.json')
$env:BENCH_REQUIRE_BEHAVIORAL = '1'
$tests = @('test_large_fixtures_v2.py', 'test_large_fixtures.py', 'test_large_task_failures.py')
foreach ($pattern in $tests) {
    Invoke-Checked "Regression tests $pattern" $Py @('-m', 'unittest', 'discover', '-s', (Join-Path $Bench 'tests'), '-p', $pattern)
}
Remove-Item Env:BENCH_REQUIRE_BEHAVIORAL

# 5. Locate (read-only) and classify. Baseline and GPT-6 never share an output.
$classifier = Join-Path $Bench 'large_task_failures.py'
$locateFile = Join-Path $OutDir 'locate.jsonl'
Invoke-Checked 'Locate large-task campaigns (read-only)' $Py (@($classifier, '--locate') + $BenchRoot) $locateFile
$found = @(Get-Content -LiteralPath $locateFile | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })
$groups = [ordered]@{ 'gpt-5.6' = @(); 'gpt-6' = @() }
$skipped = @()
foreach ($campaign in $found) {
    $models = @($campaign.models)
    $isBaseline = [bool]$campaign.original_baseline_candidate
    $isGpt6 = $models -contains 'gpt-6-luna'
    if ($isBaseline -and $isGpt6) { $skipped += [ordered]@{ path = $campaign.path; reason = 'mixes GPT-5.6 and GPT-6 rows' } }
    elseif ($isBaseline) { $groups['gpt-5.6'] += $campaign.path }
    elseif ($isGpt6) { $groups['gpt-6'] += $campaign.path }
    else { $skipped += [ordered]@{ path = $campaign.path; reason = "models: $($models -join ', ')" } }
}
$found | Format-Table path, models, large_runs, large_passes, grader_logs, original_baseline_candidate -AutoSize | Out-Host
$returnFiles = @((Join-Path $OutDir 'v2-validation.json'), (Join-Path $OutDir 'evidence-manifest.json'))
foreach ($name in $groups.Keys) {
    if ($groups[$name].Count -eq 0) { Write-Host "    no $name campaigns found under $($BenchRoot -join ', ')"; continue }
    $target = Join-Path $OutDir $name
    Invoke-Checked "Classify $name ($($groups[$name].Count) campaign folders)" $Py (@($classifier) + $groups[$name] + @('--out', $target)) (Join-Path $OutDir "$name-stdout.json")
    $returnFiles += Join-Path $target 'large-task-summary.json'
}
foreach ($entry in $skipped) { Write-Host "    not classified: $($entry.path) ($($entry.reason))" }

# 6. Prove the campaign folders were not modified.
Write-Host '==> Fingerprinting bench roots (after)'
$after = @($BenchRoot | ForEach-Object { Get-TreeFingerprint $_ })
$unchanged = ($before | ConvertTo-Json -Compress) -eq ($after | ConvertTo-Json -Compress)

$evidence = [ordered]@{
    schema                = 'v2-stage1-windows-evidence/1'
    created_utc           = (Get-Date).ToUniversalTime().ToString('o')
    analysis_commit       = $head
    harness_pin           = $HarnessPin
    harness_matches_pin   = $true
    manifest_sha256       = $actual
    python                = $pythonVersion
    powershell            = $PSVersionTable.PSVersion.ToString()
    validator_exit_code   = 0
    regression_tests      = $tests
    campaigns_classified  = [ordered]@{ 'gpt-5.6' = $groups['gpt-5.6'].Count; 'gpt-6' = $groups['gpt-6'].Count }
    campaigns_skipped     = $skipped.Count
    bench_before          = $before
    bench_after           = $after
    bench_unchanged       = $unchanged
}
Write-Utf8 (Join-Path $OutDir 'evidence-manifest.json') ($evidence | ConvertTo-Json -Depth 6)
if (-not $unchanged) { throw "files under $($BenchRoot -join ', ') changed during the run; see evidence-manifest.json (another job running?)" }

Write-Host ''
Write-Host 'ALL CHECKS PASSED. Return these sanitized files:' -ForegroundColor Green
$returnFiles | ForEach-Object { Write-Host "  $_" }
Write-Host 'Do not share locate.jsonl, *-stdout.json, large-task-runs.*, or anything from C:\bench.'
exit 0
