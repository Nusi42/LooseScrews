#!/usr/bin/env bash
# Browser smoke test: builds the web client and the release ls-server, then runs
# tools/browser-smoke/smoke.mjs (headless Chrome/Edge with WebGPU via puppeteer-core:
# practice render, online flight with weapons fired, rule set editor; screenshots and
# logs in target/browser-smoke). Skips with a message when no Chrome/Edge is installed.
# Usage: tools/browser-smoke.sh [--skip-build] [smoke.mjs options, e.g. --mode headful]
#        (honours CARGO_TARGET_DIR; `node tools/browser-smoke/smoke.mjs --help` lists the options)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

step() { printf '\n==> %s\n' "$*"; }

build=1
if [ "${1:-}" = "--skip-build" ]; then
  build=0
  shift
fi

smoke="$ROOT/tools/browser-smoke/smoke.mjs"
# --browser given on the command line wins over the search.
find_args=(--find-browser)
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  if [ "${args[$i]}" = "--browser" ] && [ $((i + 1)) -lt ${#args[@]} ]; then
    find_args+=(--browser "${args[$((i + 1))]}")
  fi
done
if ! browser="$(node "$smoke" "${find_args[@]}")"; then
  echo "browser smoke test skipped: no Chrome or Edge found (set LS_BROWSER or pass --browser <path>)"
  exit 0
fi
echo "browser: $browser"

target="${CARGO_TARGET_DIR:-target}"
case "$target" in
  /* | [A-Za-z]:*) ;;
  *) target="$ROOT/$target" ;;
esac
exe=""
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) exe=".exe" ;; esac

if [ "$build" = 1 ]; then
  step "cargo build --release -p ls-server"
  cargo build --release -p ls-server

  step "web: build (ls-wasm package + vite)"
  cd "$ROOT/web"
  if [ ! -d node_modules ]; then npm ci; fi
  npm run build
  cd "$ROOT"
fi

if [ ! -d "$ROOT/tools/browser-smoke/node_modules/puppeteer-core" ]; then
  step "browser-smoke: npm ci (puppeteer-core)"
  (cd "$ROOT/tools/browser-smoke" && npm ci)
fi

step "browser smoke test"
node "$smoke" --server-bin "$target/release/ls-server$exe" "$@"
