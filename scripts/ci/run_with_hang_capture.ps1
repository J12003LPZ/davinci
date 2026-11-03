# Run a command with a deadline. If it is still running at the deadline, print
# all-thread stacks and open file handles (text only, via cdb) for every process
# whose executable lives in the cargo target directory, then kill them and the
# command, and fail with exit code 124.
#
# Usage (call with & from pwsh; -File mis-parses the command's own flags such as -p):
#   & scripts/ci/run_with_hang_capture.ps1 -DeadlineSeconds 600 -- cargo test ...
#
# Processes are identified by executable path, never by parent id, so pid reuse
# and orphans cannot widen the set. cdb prints thread stacks (~*kn) and file
# handle names (!handle 0 f File) only. It never runs !peb, !env or .dump, so
# CI logs cannot gain the environment block, memory contents or other object names.

param(
    [Parameter(Mandatory = $true)][int]$DeadlineSeconds,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$CommandLine
)

$ErrorActionPreference = 'Stop'
$CdbTimeoutSeconds = 90
$MaxOutputLines = 5000
$HangExitCode = 124

if ($CommandLine -and $CommandLine[0] -eq '--') { $CommandLine = $CommandLine[1..($CommandLine.Count - 1)] }
if (-not $CommandLine) { throw 'no command given' }

function Get-TargetDir {
    $repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    $dir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
    return [System.IO.Path]::GetFullPath($dir).TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
}

# Local PDBs only: no symbol server, so a capture works offline and fast.
function Get-SymbolPath { Join-Path (Get-TargetDir) 'debug\deps' }

function Test-InTargetDir($ExecutablePath) {
    [bool]$ExecutablePath -and $ExecutablePath.StartsWith((Get-TargetDir), [System.StringComparison]::OrdinalIgnoreCase)
}

# Current processes running an executable from the target directory.
function Get-HungProcesses {
    @(Get-CimInstance Win32_Process | Where-Object { $_.ProcessId -ne $PID -and (Test-InTargetDir $_.ExecutablePath) })
}

function Stop-HungProcesses {
    foreach ($p in Get-HungProcesses) {
        $now = Get-CimInstance Win32_Process -Filter "ProcessId = $($p.ProcessId)" -ErrorAction SilentlyContinue
        if ($now -and (Test-InTargetDir $now.ExecutablePath)) {
            Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue
        }
    }
}

function Find-Cdb {
    $onPath = Get-Command cdb.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($onPath) { return $onPath.Source }
    $kit = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Debuggers\x64\cdb.exe'
    if (Test-Path -LiteralPath $kit) { return $kit }
    return $null
}

function Capture-Process($Cdb, $Proc) {
    Write-Host "::group::HANG CAPTURE: $($Proc.Name) pid $($Proc.ProcessId)"
    $out = [System.IO.Path]::GetTempFileName()
    try {
        $now = Get-CimInstance Win32_Process -Filter "ProcessId = $($Proc.ProcessId)" -ErrorAction SilentlyContinue
        if (-not ($now -and (Test-InTargetDir $now.ExecutablePath))) {
            Write-Host "pid $($Proc.ProcessId) is no longer a target-directory process; skipped."
            return
        }
        $commands = '.lines -e; ~*kn 1024; !handle 0 f File; q'
        $cdbProc = Start-Process -FilePath $Cdb -PassThru -NoNewWindow `
            -ArgumentList @('-pv', '-p', $Proc.ProcessId, '-y', "`"$(Get-SymbolPath)`"", '-c', "`"$commands`"") `
            -RedirectStandardOutput $out -RedirectStandardError "$out.err"
        if (-not $cdbProc.WaitForExit($CdbTimeoutSeconds * 1000)) {
            Write-Host "cdb exceeded $CdbTimeoutSeconds seconds for pid $($Proc.ProcessId)."
            try { Stop-Process -Id $cdbProc.Id -Force -ErrorAction Stop }
            catch { Write-Host "could not stop cdb: $($_.Exception.Message)" }
        }
        $lines = @(Get-Content -LiteralPath $out -TotalCount ($MaxOutputLines + 1) -ErrorAction SilentlyContinue)
        $lines | Select-Object -First $MaxOutputLines | Write-Host
        if ($lines.Count -gt $MaxOutputLines) { Write-Host "Output cut after $MaxOutputLines lines." }
    }
    catch {
        Write-Host "capture failed for pid $($Proc.ProcessId): $($_.Exception.Message)"
    }
    finally {
        Remove-Item -LiteralPath $out, "$out.err" -Force -ErrorAction SilentlyContinue
        Write-Host '::endgroup::'
    }
}

$exe = Get-Command $CommandLine[0] -CommandType Application -ErrorAction Stop | Select-Object -First 1
$psi = [System.Diagnostics.ProcessStartInfo]::new($exe.Source)
foreach ($arg in $CommandLine[1..($CommandLine.Count)] | Where-Object { $null -ne $_ }) { $psi.ArgumentList.Add($arg) }
$psi.UseShellExecute = $false   # no redirection: stdout and stderr go straight to the log

$process = [System.Diagnostics.Process]::Start($psi)
$null = $process.Handle          # keep the handle so ExitCode stays readable
if ($process.WaitForExit($DeadlineSeconds * 1000)) {
    exit $process.ExitCode
}

Write-Host "Deadline of $DeadlineSeconds seconds passed for pid $($process.Id) ($($exe.Source))."
try {
    $hung = Get-HungProcesses
    if ($hung) { Write-Host ("Processes in $(Get-TargetDir): " + (($hung | ForEach-Object { "$($_.Name)($($_.ProcessId))" }) -join ', ')) }
    else { Write-Host "No process matched the target directory $(Get-TargetDir)." }
    $cdb = Find-Cdb
    if (-not $cdb) { Write-Host 'cdb.exe was not found on PATH or under Windows Kits\10\Debuggers\x64; nothing to capture.' }
    else { foreach ($p in $hung) { Capture-Process $cdb $p } }
}
catch {
    Write-Host "capture failed: $($_.Exception.Message)"
}
finally {
    try { Stop-HungProcesses } catch { Write-Host "kill failed: $($_.Exception.Message)" }
    try { $process.Kill() } catch { Write-Host "root kill: $($_.Exception.Message)" }
    try { Stop-HungProcesses } catch { Write-Host "kill failed: $($_.Exception.Message)" }
    [void]$process.WaitForExit(15000)
    Write-Host "HANG: '$($CommandLine[0])' did not finish within the $DeadlineSeconds-second deadline. This is a hang, not a test failure."
    exit $HangExitCode
}
