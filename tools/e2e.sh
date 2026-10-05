#!/usr/bin/env bash
# End-to-end run: builds release ls-server, ls-bots, ls-directory (--directory local) and the web client, then
# runs tools/e2e.mjs (server on a free port with a fixed share code and
# data/rulesets/paint_the_void.ron, 7 bots for 90 s, log checks, clean shutdown).
# Usage: tools/e2e.sh [--skip-build] [e2e.mjs options, e.g. --duration 120 --count 7]
#        (honours CARGO_TARGET_DIR; `node tools/e2e.mjs --help` lists the options)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
node "$ROOT/tools/work-folders.mjs" guard "tools/e2e.sh"

step() { printf '\n==> %s\n' "$*"; }

build=1
if [ "${1:-}" = "--skip-build" ]; then
  build=0
  shift
fi

target="${CARGO_TARGET_DIR:-target}"
case "$target" in
  /* | [A-Za-z]:*) ;;
  *) target="$ROOT/$target" ;;
esac
exe=""
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) exe=".exe" ;; esac

if [ "$build" = 1 ]; then
  step "cargo build --release -p ls-server -p ls-bots -p ls-directory"
  cargo build --release -p ls-server -p ls-bots -p ls-directory

  step "web: build (ls-wasm package + vite)"
  cd "$ROOT/web"
  if [ ! -d node_modules ]; then npm ci; fi
  npm run build
  cd "$ROOT"
fi

step "end-to-end: ls-server + ls-bots"
node "$ROOT/tools/e2e.mjs" \
  --server-bin "$target/release/ls-server$exe" \
  --bots-bin "$target/release/ls-bots$exe" \
  "$@"
