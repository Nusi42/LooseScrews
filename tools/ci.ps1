# Full local CI: formatting, lints, tests (native + wasm32), WASM release build, web build,
# WASM-vs-native determinism cross-check. The end-to-end soak run is separate: tools/e2e.ps1.
# Usage: pwsh tools/ci.ps1   (run from anywhere; honours CARGO_TARGET_DIR if set)
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
    Step 'cargo fmt --check'
    Invoke-Checked cargo @('fmt', '--all', '--', '--check')

    Step 'cargo clippy (workspace, all targets, -D warnings)'
    Invoke-Checked cargo @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')

    Step 'cargo test --workspace'
    Invoke-Checked cargo @('test', '--workspace')

    Step 'cargo test -p ls-bots --release (bot end-to-end tests are ignored in debug builds)'
    Invoke-Checked cargo @('test', '-p', 'ls-bots', '--release')

    Step 'unit tests on wasm32: ls-math, ls-net, ls-sim, ls-worldgen, ls-game (Node via wasm-bindgen-test-runner)'
    if ((Get-Command wasm-bindgen-test-runner -ErrorAction SilentlyContinue) -and (Get-Command node -ErrorAction SilentlyContinue)) {
        $env:CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER = 'wasm-bindgen-test-runner'
        # --release keeps the worldgen golden and generation tests fast; the golden
        # hashes must match native.
        try {
            Invoke-Checked cargo @('test', '--release', '-p', 'ls-math', '-p', 'ls-net', '-p', 'ls-sim', '-p', 'ls-worldgen', '--target', 'wasm32-unknown-unknown')
            # ls-game: unit tests only (its integration tests need threads and sockets).
            Invoke-Checked cargo @('test', '--release', '-p', 'ls-game', '--lib', '--target', 'wasm32-unknown-unknown')
        }
        finally { Remove-Item Env:CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER }
    } else {
        Write-Host 'skipped: wasm-bindgen-test-runner or node not found'
    }

    Step 'cargo build -p ls-wasm (wasm32, release)'
    Invoke-Checked cargo @('build', '-p', 'ls-wasm', '--target', 'wasm32-unknown-unknown', '--release')

    Step 'web: install, typecheck, test, build'
    Push-Location (Join-Path $Root 'web')
    try {
        if (Test-Path 'package-lock.json') { Invoke-Checked npm @('ci') } else { Invoke-Checked npm @('install') }
        Invoke-Checked npm @('run', 'typecheck')
        Invoke-Checked npm @('test')
        Invoke-Checked npm @('run', 'build')
    } finally { Pop-Location }

    Step 'determinism cross-check: WASM golden hashes vs native fixture (Node)'
    Invoke-Checked node @((Join-Path $Root 'tools/determinism-check.mjs'))

    Write-Host "`nCI passed." -ForegroundColor Green
} finally {
    Pop-Location
}
