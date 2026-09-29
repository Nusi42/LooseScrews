#!/usr/bin/env bash
# Full local CI: formatting, lints, tests (native + wasm32), WASM release build, web build.
# Usage: tools/ci.sh   (run from anywhere; honours CARGO_TARGET_DIR if set)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

step() { printf '\n==> %s\n' "$*"; }

step "cargo fmt --check"
cargo fmt --all -- --check

step "cargo clippy (workspace, all targets, -D warnings)"
cargo clippy --workspace --all-targets -- -D warnings

step "cargo test --workspace"
cargo test --workspace

step "unit tests on wasm32: ls-math, ls-net, ls-sim, ls-worldgen (Node via wasm-bindgen-test-runner)"
if command -v wasm-bindgen-test-runner >/dev/null 2>&1 && command -v node >/dev/null 2>&1; then
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
    cargo test --release -p ls-math -p ls-net -p ls-sim -p ls-worldgen --target wasm32-unknown-unknown
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

printf '\nCI passed.\n'
