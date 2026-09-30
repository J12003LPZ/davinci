# Build this workspace and install it as `davinci`.
#
# The product executable and native voice helper are installed together from
# a tagged source commit with completed green CI.
#
# Usage:  pwsh scripts/install-davinci.ps1

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$cargoRoot = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
$binDir = Join-Path $cargoRoot 'bin'
$target = Join-Path $binDir 'davinci.exe'
$proof = [System.IO.Path]::GetTempFileName()
$stage = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())

Push-Location $repo
try {
    python (Join-Path $repo 'scripts\release_identity.py') preflight --repo $repo --require-tag --output $proof
    if ($LASTEXITCODE -ne 0) { throw "installation requires an exact product version tag and green full CI" }
    cargo build --release -p davinci-coding-agent --locked --target-dir (Join-Path $repo 'target')
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
    cargo build --release -p davinci-voice --features native --bin davinci-voice-worker --locked --target-dir (Join-Path $repo 'target')
    if ($LASTEXITCODE -ne 0) { throw "voice helper build failed (CMake and a C++ compiler are required)" }

    New-Item -ItemType Directory -Path $stage | Out-Null
    foreach ($name in @('davinci.exe', 'davinci-voice-worker.exe')) {
        $built = Join-Path $stage $name
        Copy-Item -LiteralPath (Join-Path $repo "target\release\$name") -Destination $built
        python (Join-Path $repo 'scripts\release_identity.py') record --proof $proof --binary $built --output "$built.identity.json"
        if ($LASTEXITCODE -ne 0) { throw "could not verify both built binaries before installation" }
    }
    New-Item -ItemType Directory -Force -Path $binDir | Out-Null
    $noticeDir = Join-Path (Split-Path -Parent $binDir) 'share\davinci-voice'
    New-Item -ItemType Directory -Force -Path $noticeDir | Out-Null
    Copy-Item -LiteralPath (Join-Path $repo 'crates\davinci-voice\THIRD_PARTY_NOTICES.md') -Destination $noticeDir -Force
    Copy-Item -LiteralPath (Join-Path $repo 'crates\davinci-voice\licenses') -Destination $noticeDir -Recurse -Force

    # A running davinci holds its own image open, so replace rather than
    # overwrite: the delete succeeds once the old process has exited.
    foreach ($name in @('davinci-voice-worker.exe', 'davinci.exe')) {
        $installed = Join-Path $binDir $name
        if (Test-Path $installed) { Remove-Item $installed -Force }
        Copy-Item -LiteralPath (Join-Path $stage $name) -Destination $installed
        Copy-Item -LiteralPath (Join-Path $stage "$name.identity.json") -Destination "$installed.identity.json" -Force
    }

    & $target --version
} finally {
    Pop-Location
    Remove-Item -LiteralPath $proof -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}
