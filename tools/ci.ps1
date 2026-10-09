# Full local CI: formatting, generated docs, lints, tests (native + wasm32), WASM release build, web build,
# WASM-vs-native determinism cross-check. The end-to-end soak run is separate: tools/e2e.ps1.
# Usage: pwsh tools/ci.ps1   (run from anywhere; honours CARGO_TARGET_DIR if set)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Root = Resolve-Path (Join-Path $PSScriptRoot '..')
node (Join-Path $Root 'tools/work-folders.mjs') guard 'tools/ci.ps1'
if ($LASTEXITCODE -ne 0) { exit 1 }
Push-Location $Root

function Step([string]$Name) { Write-Host "`n==> $Name" -ForegroundColor Cyan }

function Invoke-Checked([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "'$Exe $($Arguments -join ' ')' failed with exit code $LASTEXITCODE" }
}

try {
    Step 'cargo fmt --check'
    Invoke-Checked cargo @('fmt', '--all', '--', '--check')

    Step 'generated reference docs are current (userDocs/reference from data/)'
    Invoke-Checked node @((Join-Path $Root 'tools/gen-weapon-docs.mjs'), '--check')

    Step 'no wildcard binds outside the allowlist; tools tests (hooks, guards)'
    Invoke-Checked node @((Join-Path $Root 'tools/check-binds.mjs'))
    Invoke-Checked node @((Join-Path $Root 'tools/check-private.mjs'))
    Invoke-Checked node @('--test', 'tools/test/*.test.mjs')

    Step 'cargo clippy (workspace, all targets, -D warnings)'
    Invoke-Checked cargo @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')

    Step 'cargo test --workspace'
    Invoke-Checked cargo @('test', '--workspace')

    Step 'ls-server timing accuracy (strict, serial: ignored in the parallel run)'
    Invoke-Checked cargo @('test', '-p', 'ls-server', '--lib', 'timing::tests::wait_until_is_accurate', '--', '--ignored', '--test-threads=1')

    Step 'cargo test -p ls-bots --release (bot end-to-end tests are ignored in debug builds)'
    Invoke-Checked cargo @('test', '-p', 'ls-bots', '--release')

    Step 'cargo test -p ls-game --release (The Squeeze remesh benchmark, Hot Reactor 8-bot match, match-setup sweep)'
    Invoke-Checked cargo @('test', '-p', 'ls-game', '--release')

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

    Step 'browser smoke test (optional: Chrome or Edge with WebGPU; tools/browser-smoke.ps1)'
    if ($env:LS_SKIP_BROWSER_SMOKE) {
        Write-Host 'skipped: LS_SKIP_BROWSER_SMOKE is set'
    } else {
        & node @((Join-Path $Root 'tools/browser-smoke/smoke.mjs'), '--find-browser') | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Host 'skipped: no Chrome or Edge found (set LS_BROWSER to use another Chromium browser)'
        } else {
            Invoke-Checked cargo @('build', '--release', '-p', 'ls-server', '-p', 'ls-directory')
            # The web client was built above.
            & (Join-Path $Root 'tools/browser-smoke.ps1') -SkipBuild
        }
    }

    Write-Host "`nCI passed." -ForegroundColor Green
} finally {
    Pop-Location
}
