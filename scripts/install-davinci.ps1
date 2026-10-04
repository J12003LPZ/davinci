# Build this workspace and install it as `davinci`.
#
# The product executable is installed from a tagged source commit with completed green CI.
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

    New-Item -ItemType Directory -Path $stage | Out-Null
    $built = Join-Path $stage 'davinci.exe'
    Copy-Item -LiteralPath (Join-Path $repo 'target\release\davinci.exe') -Destination $built
    python (Join-Path $repo 'scripts\release_identity.py') record --proof $proof --binary $built --output "$built.identity.json"
    if ($LASTEXITCODE -ne 0) { throw "could not verify built binary before installation" }
    New-Item -ItemType Directory -Force -Path $binDir | Out-Null

    # A running davinci holds its own image open, so replace rather than overwrite.
    if (Test-Path $target) { Remove-Item $target -Force }
    Copy-Item -LiteralPath $built -Destination $target
    Copy-Item -LiteralPath "$built.identity.json" -Destination "$target.identity.json" -Force

    & $target --version
} finally {
    Pop-Location
    Remove-Item -LiteralPath $proof -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}
