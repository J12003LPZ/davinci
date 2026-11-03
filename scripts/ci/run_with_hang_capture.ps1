# Run a command with a deadline. If it is still running at the deadline, print
# all-thread stacks and handle information for every process in its tree
# (text only, via cdb), then kill the tree and fail.
#
# Usage (call with & from pwsh; -File mis-parses the command's own flags such as -p):
#   & scripts/ci/run_with_hang_capture.ps1 -DeadlineSeconds 600 -- cargo test ...
#
# Deliberately never prints the environment block or memory contents (no !peb,
# !env or .dump), so CI logs cannot gain credentials.

param(
    [Parameter(Mandatory = $true)][int]$DeadlineSeconds,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$CommandLine
)

$ErrorActionPreference = 'Stop'
$CdbTimeoutSeconds = 90
$HangExitCode = 124

if ($CommandLine -and $CommandLine[0] -eq '--') { $CommandLine = $CommandLine[1..($CommandLine.Count - 1)] }
if (-not $CommandLine) { throw 'no command given' }

function Get-ProcessTree([int]$RootId) {
    $all = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name)
    $found = [System.Collections.Generic.List[object]]::new()
    $queue = [System.Collections.Generic.Queue[int]]::new()
    $seen = [System.Collections.Generic.HashSet[int]]::new()
    $queue.Enqueue($RootId); [void]$seen.Add($RootId)
    while ($queue.Count -gt 0) {
        $id = $queue.Dequeue()
        $self = $all | Where-Object ProcessId -eq $id
        if ($self) { $found.Add($self) }
        foreach ($child in ($all | Where-Object ParentProcessId -eq $id)) {
            if ($seen.Add([int]$child.ProcessId)) { $queue.Enqueue([int]$child.ProcessId) }
        }
    }
    return , $found
}

function Stop-ProcessTree([int]$RootId) {
    # Two passes: the second catches anything spawned while the first ran.
    foreach ($pass in 1, 2) {
        foreach ($p in (Get-ProcessTree $RootId | Sort-Object ProcessId -Descending)) {
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

function Get-SymbolPath {
    $repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
    # Local PDBs only: no symbol server, so a capture works offline and fast.
    return (Join-Path $targetDir 'debug\deps')
}

function Capture-Process($Cdb, $Proc, [string]$SymbolPath) {
    Write-Host "::group::HANG CAPTURE: $($Proc.Name) pid $($Proc.ProcessId)"
    Write-Host "===== HANG CAPTURE: $($Proc.Name) (pid $($Proc.ProcessId)) ====="
    $out = [System.IO.Path]::GetTempFileName()
    try {
        $commands = '.lines -e; ~*kn 1024; !handle 0 f; qd'
        $cdbProc = Start-Process -FilePath $Cdb -PassThru -NoNewWindow `
            -ArgumentList @('-p', $Proc.ProcessId, '-y', "`"$SymbolPath`"", '-c', "`"$commands`"") `
            -RedirectStandardOutput $out -RedirectStandardError "$out.err"
        if (-not $cdbProc.WaitForExit($CdbTimeoutSeconds * 1000)) {
            Stop-Process -Id $cdbProc.Id -Force -ErrorAction SilentlyContinue
            Write-Host "cdb exceeded $CdbTimeoutSeconds seconds for pid $($Proc.ProcessId); killed it."
        }
        Get-Content -LiteralPath $out -ErrorAction SilentlyContinue | Write-Host
        Get-Content -LiteralPath "$out.err" -ErrorAction SilentlyContinue | Write-Host
    }
    catch {
        Write-Host "cdb capture failed for pid $($Proc.ProcessId): $($_.Exception.Message)"
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

Write-Host "Deadline of $DeadlineSeconds seconds passed; capturing process tree of pid $($process.Id) ($($exe.Source))."
$tree = Get-ProcessTree $process.Id
Write-Host ("Process tree: " + (($tree | ForEach-Object { "$($_.Name)($($_.ProcessId))" }) -join ', '))

$cdb = Find-Cdb
if (-not $cdb) {
    Write-Host 'cdb.exe was not found on PATH or under Windows Kits\10\Debuggers\x64; no stacks or handles can be captured.'
}
else {
    Write-Host "Using $cdb with symbol path $(Get-SymbolPath)"
    foreach ($p in $tree) { Capture-Process $cdb $p (Get-SymbolPath) }
}

Stop-ProcessTree $process.Id
Write-Host "HANG: '$($CommandLine[0])' did not finish within the $DeadlineSeconds-second deadline. This is a hang, not a test failure."
exit $HangExitCode
