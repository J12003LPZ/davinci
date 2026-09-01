# Build this workspace and install it as `davinci`.
#
# The binary this repo produces is named `pi`, but on a machine that also has
# the TypeScript pi installed from npm or pnpm, those shims usually sit ahead
# of ~/.cargo/bin on PATH and win the name. Installing under `davinci` gives
# the Rust build a name of its own that nothing else claims.
#
# Usage:  pwsh scripts/install-davinci.ps1

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$binDir = Join-Path $HOME '.cargo\bin'
$target = Join-Path $binDir 'davinci.exe'

Push-Location $repo
try {
    cargo build --release -p pi-coding-agent
    if ($LASTEXITCODE -ne 0) { throw "build failed" }

    $built = Join-Path $repo 'target\release\pi.exe'

    # A running davinci holds its own image open, so replace rather than
    # overwrite: the delete succeeds once the old process has exited.
    if (Test-Path $target) { Remove-Item $target -Force }
    Copy-Item $built $target

    & $target --version
} finally {
    Pop-Location
}
