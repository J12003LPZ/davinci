<#
.SYNOPSIS
  Prepare the pinned design runtime and switch /design on, in one step.

.DESCRIPTION
  /design never installs anything itself. This is the explicit setup it needs:

    1. checks Node 24.19.0 (the version the runtime is pinned to);
    2. installs the companion's locked dependencies and builds it;
    3. installs the Playwright browser builds the pinned Playwright expects;
    4. copies everything into a new runtime directory outside the repository,
       with SHA-256 inventories of Node, packages, browser and fonts;
    5. records the runtime in DaVinci's global settings and turns on
       /config -> Design artifacts.

  Run it again after updating DaVinci: each run makes a new runtime directory
  and points the settings at it. Older ones can be deleted.

.EXAMPLE
  pwsh -NoProfile -File .\scripts\setup-design-runtime.ps1
#>
[CmdletBinding()]
param(
    # Where runtimes go; each run makes a new directory inside it.
    [string]$RuntimeRoot = (Join-Path $env:LOCALAPPDATA 'DaVinci\design-runtime'),
    # Prepare the runtime without changing DaVinci's settings.
    [switch]$NoSettings
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$ui = Join-Path $repo 'crates\davinci-coding-agent\design-ui'
$required = 'v24.19.0'

function Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }

Step 'Checking Node'
$nodeCommand = Get-Command node -ErrorAction SilentlyContinue
if (-not $nodeCommand) {
    throw "Node $required is required. Install it with: winget install OpenJS.NodeJS --version 24.19.0"
}
$node = (Resolve-Path $nodeCommand.Source).Path
$version = (& $node --version).Trim()
if ($version -ne $required) {
    throw "Node $required is required, found $version at $node. Install it with: winget install OpenJS.NodeJS --version 24.19.0"
}

Push-Location $ui
try {
    Step 'Installing the companion dependencies (npm ci)'
    & npm ci --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }

    Step 'Building the companion'
    & npm run build
    if ($LASTEXITCODE -ne 0) { throw 'companion build failed' }

    Step 'Installing the pinned Playwright browsers'
    # Keep browsers other projects installed: Playwright otherwise deletes
    # every build it does not see referenced.
    $env:PLAYWRIGHT_SKIP_BROWSER_GC = '1'
    & $node (Join-Path $ui 'node_modules\@playwright\test\cli.js') install chromium chromium-headless-shell
    if ($LASTEXITCODE -ne 0) { throw 'Playwright browser install failed' }

    $commit = (& git -C $repo rev-parse --short HEAD 2>$null)
    if (-not $commit) { $commit = 'local' }
    $runtime = Join-Path $RuntimeRoot ("{0}-{1}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'), $commit.Trim())
    New-Item -ItemType Directory -Force -Path $RuntimeRoot | Out-Null

    Step "Pinning the runtime in $runtime"
    & $node (Join-Path $ui 'scripts\install-runtime.cjs') $runtime
    if ($LASTEXITCODE -ne 0) { throw "runtime install failed; inspect $runtime" }
} finally {
    Pop-Location
}

if (-not $NoSettings) {
    # Match davinci_session::default_agent_dir: explicit DaVinci/legacy Pi
    # override, then the existing directory, then the new DaVinci default.
    $homeDir = if ($env:USERPROFILE) { $env:USERPROFILE } else { $HOME }
    $override = if ($env:DAVINCI_CODING_AGENT_DIR) { $env:DAVINCI_CODING_AGENT_DIR } else { $env:PI_CODING_AGENT_DIR }
    if ($override) {
        $agentDir = if ($override -eq '~') { $homeDir } elseif ($override.StartsWith('~/')) { Join-Path $homeDir $override.Substring(2) } else { $override }
    } elseif (Test-Path (Join-Path $homeDir '.davinci\agent')) {
        $agentDir = Join-Path $homeDir '.davinci\agent'
    } elseif (Test-Path (Join-Path $homeDir '.pi\agent')) {
        $agentDir = Join-Path $homeDir '.pi\agent'
    } else {
        $agentDir = Join-Path $homeDir '.davinci\agent'
    }
    $settingsPath = Join-Path $agentDir 'settings.json'
    Step "Turning on /design in $settingsPath"
    New-Item -ItemType Directory -Force -Path $agentDir | Out-Null
    if (Test-Path $settingsPath) {
        $backup = "$settingsPath.before-design-setup.$([guid]::NewGuid().ToString('N'))"
        Copy-Item -LiteralPath $settingsPath -Destination $backup
        Step "Settings backup: $backup"
    }
    $settings = if (Test-Path $settingsPath) {
        Get-Content -Raw -LiteralPath $settingsPath | ConvertFrom-Json -AsHashtable
    } else { @{} }
    if ($null -eq $settings) { $settings = @{} }
    $settings['designEnabled'] = $true
    $settings['designRuntime'] = $runtime
    $settings['designNode'] = $node
    $temp = "$settingsPath.tmp"
    $settings | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $temp -Encoding utf8NoBOM
    Move-Item -Force -LiteralPath $temp -Destination $settingsPath
}

Write-Host ''
Write-Host "Design runtime ready: $runtime" -ForegroundColor Green
Write-Host "Node: $node"
if (-not $NoSettings) {
    Write-Host 'Restart DaVinci, pick an openai-codex model with thinking low/medium/high/xhigh, then: /design new "a pricing page" --kind landing'
}
