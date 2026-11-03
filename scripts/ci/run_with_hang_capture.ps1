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

# Never captured and never killed, whatever the walk returns.
$ProtectedProcessNames = @(
    'smss', 'csrss', 'wininit', 'winlogon', 'services', 'lsass', 'LsaIso', 'svchost',
    'fontdrvhost', 'dwm', 'MsMpEng', 'MpDefenderCoreService', 'Runner.Listener',
    'Runner.Worker', 'System', 'Registry'
)
# A real cargo test tree is a handful of processes; more means the walk is wrong.
$MaxTreeSize = 32

function Get-ProcessRecords {
    @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, CreationDate)
}

# Windows keeps ParentProcessId after the parent exits and reuses pids, so a record
# only counts as a child if it was created at or after its parent.
function Test-IsChildOf($Child, $Parent) {
    $Child.ParentProcessId -eq $Parent.ProcessId -and $Child.CreationDate -ge $Parent.CreationDate
}

# Ids of $SelfId and its real ancestors (Runner.Worker, Runner.Listener, ...).
function Get-ProtectedIds([int]$SelfId, $All) {
    $ids = [System.Collections.Generic.HashSet[int]]::new()
    $cur = $All | Where-Object ProcessId -eq $SelfId | Select-Object -First 1
    while ($cur -and $ids.Add([int]$cur.ProcessId)) {
        $child = $cur
        $cur = $All | Where-Object { $_.ProcessId -eq $child.ParentProcessId -and (Test-IsChildOf $child $_) } | Select-Object -First 1
    }
    return , $ids
}

# Returns Members (root first, each with Depth) and Excluded (Process, Reason).
# Excluded processes are not entered, so their descendants are not reached either.
function Get-ProcessTree([int]$RootId, $All, $ProtectedIds) {
    $members = [System.Collections.Generic.List[object]]::new()
    $excluded = [System.Collections.Generic.List[object]]::new()
    $root = $All | Where-Object ProcessId -eq $RootId | Select-Object -First 1
    if ($root) {
        $members.Add(($root | Select-Object *, @{ n = 'Depth'; e = { 0 } }))
        $queue = [System.Collections.Generic.Queue[object]]::new()
        $seen = [System.Collections.Generic.HashSet[int]]::new()
        $queue.Enqueue($members[0]); [void]$seen.Add($RootId)
        while ($queue.Count -gt 0) {
            $parent = $queue.Dequeue()
            foreach ($child in ($All | Where-Object { Test-IsChildOf $_ $parent })) {
                if (-not $seen.Add([int]$child.ProcessId)) { continue }
                $reason = $null
                if ($ProtectedIds -and $ProtectedIds.Contains([int]$child.ProcessId)) { $reason = 'this script or one of its ancestors' }
                elseif ($ProtectedProcessNames -contains ($child.Name -replace '\.exe$', '')) { $reason = 'protected process name' }
                if ($reason) { $excluded.Add([pscustomobject]@{ Process = $child; Reason = $reason }); continue }
                $node = $child | Select-Object *, @{ n = 'Depth'; e = { $parent.Depth + 1 } }
                $members.Add($node)
                $queue.Enqueue($node)
            }
        }
    }
    return [pscustomobject]@{ Members = $members; Excluded = $excluded }
}

# Kills only processes that are still the ones recorded (same pid and creation time),
# children before parents.
function Stop-ProcessList($Processes) {
    foreach ($p in ($Processes | Sort-Object Depth -Descending)) {
        $now = Get-CimInstance Win32_Process -Filter "ProcessId = $($p.ProcessId)" -ErrorAction SilentlyContinue
        if ($now -and $now.CreationDate -eq $p.CreationDate) {
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
$records = Get-ProcessRecords
$protected = Get-ProtectedIds $PID $records
$walk = Get-ProcessTree $process.Id $records $protected
foreach ($e in $walk.Excluded) {
    Write-Host "Excluded $($e.Process.Name)($($e.Process.ProcessId)): $($e.Reason)"
}
$toKill = [System.Collections.Generic.List[object]]::new()
if ($walk.Members.Count -gt $MaxTreeSize) {
    Write-Host "Process walk returned $($walk.Members.Count) processes (limit $MaxTreeSize); treating it as broken. Nothing captured; killing only pid $($process.Id)."
    $toKill.Add([pscustomobject]@{ ProcessId = $process.Id; CreationDate = $walk.Members[0].CreationDate; Depth = 0 })
}
else {
    $tree = $walk.Members
    $toKill.AddRange($tree)
    Write-Host ("Process tree: " + (($tree | ForEach-Object { "$($_.Name)($($_.ProcessId))" }) -join ', '))

    $cdb = Find-Cdb
    if (-not $cdb) {
        Write-Host 'cdb.exe was not found on PATH or under Windows Kits\10\Debuggers\x64; no stacks or handles can be captured.'
    }
    else {
        Write-Host "Using $cdb with symbol path $(Get-SymbolPath)"
        foreach ($p in $tree) { Capture-Process $cdb $p (Get-SymbolPath) }
    }

    # Second snapshot catches children spawned during capture. Once the root has
    # died, orphans can no longer be found through it, so do not walk again.
    if (-not $process.HasExited) {
        $after = Get-ProcessTree $process.Id (Get-ProcessRecords) $protected
        foreach ($m in $after.Members) {
            if (-not ($toKill | Where-Object { $_.ProcessId -eq $m.ProcessId -and $_.CreationDate -eq $m.CreationDate })) { $toKill.Add($m) }
        }
    }
}

Stop-ProcessList $toKill
Write-Host "HANG: '$($CommandLine[0])' did not finish within the $DeadlineSeconds-second deadline. This is a hang, not a test failure."
exit $HangExitCode
