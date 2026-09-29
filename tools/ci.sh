#!/usr/bin/env bash
# Full local CI: formatting, generated docs, lints, tests (native + wasm32), WASM release build, web build,
# WASM-vs-native determinism cross-check. The end-to-end soak run is separate: tools/e2e.sh.
# Usage: tools/ci.sh   (run from anywhere; honours CARGO_TARGET_DIR if set)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

step() { printf '\n==> %s\n' "$*"; }

step "cargo fmt --check"
cargo fmt --all -- --check

step "generated reference docs are current (userDocs/reference from data/)"
node "$ROOT/tools/gen-weapon-docs.mjs" --check

step "cargo clippy (workspace, all targets, -D warnings)"
cargo clippy --workspace --all-targets -- -D warnings

step "cargo test --workspace"
cargo test --workspace

step "cargo test -p ls-bots --release (bot end-to-end tests are ignored in debug builds)"
cargo test -p ls-bots --release

step "unit tests on wasm32: ls-math, ls-net, ls-sim, ls-worldgen, ls-game (Node via wasm-bindgen-test-runner)"
if command -v wasm-bindgen-test-runner >/dev/null 2>&1 && command -v node >/dev/null 2>&1; then
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
    cargo test --release -p ls-math -p ls-net -p ls-sim -p ls-worldgen --target wasm32-unknown-unknown
  # ls-game: unit tests only (its integration tests need threads and sockets).
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
    cargo test --release -p ls-game --lib --target wasm32-unknown-unknown
else
  echo "skipped: wasm-bindgen-test-runner or node not found"
fi

step "cargo build -p ls-wasm (wasm32, release)"
cargo build -p ls-wasm --target wasm32-unknown-unknown --release

step "web: install, typecheck, test, build"
cd "$ROOT/web"
if [ -f package-lock.json ]; then npm ci; else npm install; fi
npm run typecheck
npm test
npm run build

step "determinism cross-check: WASM golden hashes vs native fixture (Node)"
node "$ROOT/tools/determinism-check.mjs"

step "browser smoke test (optional: Chrome or Edge with WebGPU; tools/browser-smoke.sh)"
if [ -n "${LS_SKIP_BROWSER_SMOKE:-}" ]; then
  echo "skipped: LS_SKIP_BROWSER_SMOKE is set"
elif ! node "$ROOT/tools/browser-smoke/smoke.mjs" --find-browser >/dev/null; then
  echo "skipped: no Chrome or Edge found (set LS_BROWSER to use another Chromium browser)"
else
  cd "$ROOT"
  cargo build --release -p ls-server
  # The web client was built above.
  "$ROOT/tools/browser-smoke.sh" --skip-build
fi

printf '\nCI passed.\n'
