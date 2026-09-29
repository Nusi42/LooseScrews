# End-to-end run: builds release ls-server + ls-bots and the web client, then
# runs tools/e2e.mjs (server on a free port with a fixed share code and
# data/rulesets/paint_the_void.ron, 7 bots for 90 s, log checks, clean shutdown).
# Usage: pwsh tools/e2e.ps1 [-SkipBuild] [e2e.mjs options, e.g. --duration 120 --count 7]
#        (honours CARGO_TARGET_DIR; `node tools/e2e.mjs --help` lists the options)
param(
    [switch]$SkipBuild,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$E2eArgs = @()
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Root = Resolve-Path (Join-Path $PSScriptRoot '..')
Push-Location $Root

function Step([string]$Name) { Write-Host "`n==> $Name" -ForegroundColor Cyan }

function Invoke-Checked([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "'$Exe $($Arguments -join ' ')' failed with exit code $LASTEXITCODE" }
}

try {
    $Target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    if (-not [System.IO.Path]::IsPathRooted($Target)) { $Target = Join-Path $Root $Target }
    $Exe = if ($IsWindows -or $env:OS -eq 'Windows_NT') { '.exe' } else { '' }

    if (-not $SkipBuild) {
        Step 'cargo build --release -p ls-server -p ls-bots'
        Invoke-Checked cargo @('build', '--release', '-p', 'ls-server', '-p', 'ls-bots')

        Step 'web: build (ls-wasm package + vite)'
        Push-Location (Join-Path $Root 'web')
        try {
            if (-not (Test-Path 'node_modules')) { Invoke-Checked npm @('ci') }
            Invoke-Checked npm @('run', 'build')
        } finally { Pop-Location }
    }

    Step 'end-to-end: ls-server + ls-bots'
    $NodeArgs = @(
        (Join-Path $Root 'tools/e2e.mjs'),
        '--server-bin', (Join-Path $Target "release/ls-server$Exe"),
        '--bots-bin', (Join-Path $Target "release/ls-bots$Exe")
    ) + $E2eArgs
    & node @NodeArgs
    if ($LASTEXITCODE -ne 0) { throw "end-to-end checks failed (exit code $LASTEXITCODE)" }
} finally {
    Pop-Location
}
