# Browser smoke test: builds the web client and the release ls-server, then runs
# tools/browser-smoke/smoke.mjs (headless Chrome/Edge with WebGPU via puppeteer-core:
# practice render, online flight with weapons fired, rule set editor; screenshots and
# logs in target/browser-smoke). Skips with a message when no Chrome/Edge is installed.
# Usage: pwsh tools/browser-smoke.ps1 [-SkipBuild] [smoke.mjs options, e.g. --mode headful]
#        (honours CARGO_TARGET_DIR; `node tools/browser-smoke/smoke.mjs --help` lists the options)
param(
    [switch]$SkipBuild,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$SmokeArgs = @()
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Root = Resolve-Path (Join-Path $PSScriptRoot '..')
node (Join-Path $Root 'tools/work-folders.mjs') guard 'tools/browser-smoke.ps1'
if ($LASTEXITCODE -ne 0) { exit 1 }
Push-Location $Root

function Step([string]$Name) { Write-Host "`n==> $Name" -ForegroundColor Cyan }

function Invoke-Checked([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "'$Exe $($Arguments -join ' ')' failed with exit code $LASTEXITCODE" }
}

try {
    $Smoke = Join-Path $Root 'tools/browser-smoke/smoke.mjs'
    # --browser given on the command line wins over the search.
    $FindArgs = @($Smoke, '--find-browser')
    $i = [Array]::IndexOf($SmokeArgs, '--browser')
    if ($i -ge 0 -and $i + 1 -lt $SmokeArgs.Count) { $FindArgs += @('--browser', $SmokeArgs[$i + 1]) }
    $Browser = & node @FindArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'browser smoke test skipped: no Chrome or Edge found (set LS_BROWSER or pass --browser <path>)' -ForegroundColor Yellow
        return
    }
    Write-Host "browser: $Browser"

    $Target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    if (-not [System.IO.Path]::IsPathRooted($Target)) { $Target = Join-Path $Root $Target }
    $Exe = if ($IsWindows -or $env:OS -eq 'Windows_NT') { '.exe' } else { '' }

    if (-not $SkipBuild) {
        Step 'cargo build --release -p ls-server'
        Invoke-Checked cargo @('build', '--release', '-p', 'ls-server')

        Step 'web: build (ls-wasm package + vite)'
        Push-Location (Join-Path $Root 'web')
        try {
            if (-not (Test-Path 'node_modules')) { Invoke-Checked npm @('ci') }
            Invoke-Checked npm @('run', 'build')
        } finally { Pop-Location }
    }

    Push-Location (Join-Path $Root 'tools/browser-smoke')
    try {
        if (-not (Test-Path 'node_modules/puppeteer-core')) {
            Step 'browser-smoke: npm ci (puppeteer-core)'
            Invoke-Checked npm @('ci')
        }
    } finally { Pop-Location }

    Step 'browser smoke test'
    $NodeArgs = @($Smoke, '--server-bin', (Join-Path $Target "release/ls-server$Exe")) + $SmokeArgs
    & node @NodeArgs
    if ($LASTEXITCODE -ne 0) { throw "browser smoke test failed (exit code $LASTEXITCODE)" }
} finally {
    Pop-Location
}
