# Loose Screws — repo guide

6DOF browser shooter with procedurally generated, destructible worlds. Rust core + WASM + TypeScript/WebGPU client.

- Docs index: `docs/README.md`; building, running, testing, adding weapons/rule sets: `docs/dev-guide.md`
- Design: `docs/design/game-design-doc.md` (exported from the living doc; source of truth for game rules)
- Plan and scope: `docs/implementation-plan.md` (read the relevant module spec before working on a crate)
- Current milestone: `docs/milestone-2-plan.md` (M2 arsenal and physics: scope, design decisions for gaps in the design doc, waves, acceptance criteria)
- Cross-crate contracts (terrain trait/edits, axes, units, RNG streams): `docs/architecture/contracts.md`; types live in `ls_math::terrain`. Changing them is a cross-crate change: update the doc and all users together
- Architecture: `docs/architecture/overview.md` (crates, data flow), `netcode.md`, `determinism.md` (rules, golden hashes, re-blessing)
- Client/server glue (`ClientSession`, `Room`, wire conventions, lag compensation): `docs/architecture/game-glue.md`; server internals, browser client lifecycle, bots: `docs/architecture/server.md`
- Player manual and hosting guide (all server/bot flags for hosts): `userDocs/`

## Layout
- `crates/` Rust workspace crates (`ls-math`, `ls-worldgen`, `ls-sim`, `ls-net`, `ls-game` shared client/server glue, `ls-server`, `ls-wasm`, `ls-bots`)
- `data/` weapon and rule set definitions (RON) — gameplay numbers live here, not in code
- `web/` browser client (TypeScript strict, Vite, WebGPU, npm)
- `tools/` CI and helper scripts
- `docs/` developer docs; `userDocs/` player-facing manuals and hosting guides — keep them separate
- `.claude/memory/` Claude's persistent project memory (`MEMORY.md` index + one file per fact). The harness memory dir is a junction to it; never store memory or project notes in temp or user folders

## Maintaining this file
Update CLAUDE.md in the same change whenever layout, commands, conventions or rules change. Keep it short: pointers and rules, not documentation (that belongs in `docs/`).

## Commands
- `cargo test --workspace` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo fmt --all`
- WASM: `cargo build -p ls-wasm --target wasm32-unknown-unknown --release` then `wasm-bindgen` (CLI 0.2.126; crate pinned to `=0.2.126`, `js-sys =0.3.103`, `wasm-bindgen-test =0.3.76` — bump all together)
- Unit tests on wasm32 (Node): set `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`, then `cargo test --release -p ls-math -p ls-net -p ls-sim -p ls-worldgen --target wasm32-unknown-unknown` and `cargo test --release -p ls-game --lib --target wasm32-unknown-unknown` (the ls-sim replay hash and ls-worldgen golden hashes must match native). Test modules opt in with `#[cfg(target_arch = "wasm32")] use wasm_bindgen_test::wasm_bindgen_test as test;`
- Bot end-to-end tests are ignored in debug builds: `cargo test -p ls-bots --release` (part of CI). The full match-setup sweep (8 seeds x 25 slider mixes) runs only in release: `cargo test -p ls-game --release --lib setup`
- Worldgen timings: `cargo run -p ls-worldgen --release --example worldgen_bench`
- Determinism cross-check (WASM golden hashes vs native fixture, Node): `node tools/determinism-check.mjs` (needs the WASM package from `cd web && npm run build:wasm`; part of CI)
- After an intentional generator change: bump `ls_worldgen::GENERATOR_VERSION`, then `LS_BLESS_GOLDEN=1 cargo test -p ls-worldgen --release --test golden`. After a sim behavior, tuning or `data/` change: update `REPLAY_GOLDEN_HASH` in `crates/ls-sim/src/tests.rs`
- Web: `cd web && npm install && npm run typecheck && npm test && npm run build` (`npm run dev` serves on :5173). Unit tests are `src/**/*.test.ts` run by `node --test` (Node strips types: use `.ts` import extensions and erasable-only syntax, no enums/parameter properties). `npm run build:wasm` resolves a relative `CARGO_TARGET_DIR` against the repo root (use `target/<area>`, not `../target/...`)
- Play locally: `cd web && npm run build`, then `cargo run -p ls-server --release -- [--code LS..] [--ruleset data/rulesets/paint_the_void.ron] [--web-root web/dist]` and open http://localhost:7777 (other flags: `--bind <addr>` (default `0.0.0.0:7777`), `--max-players`, `--stats-interval`, `--intermission`, `--exit-when-empty`, `--wt-bind <addr>` (WebTransport UDP, default = `--bind`), `--no-webtransport`; `--help`). The browser picks WebTransport when available, else WebSocket; `?transport=ws|wt` forces one. Bots: `cargo run -p ls-bots --release -- --count 7 [--duration 600] [--transport ws|wt]` (exit 0 OK, 1 protocol error/desync, 2 never joined, 3 lost connection)
- End-to-end: `pwsh tools/e2e.ps1` or `tools/e2e.sh` (release server + 7 bots for 90 s with log checks; `-SkipBuild`/`--skip-build`; other options go to `tools/e2e.mjs`, e.g. `--count 8 --duration 330` for a full match, `--transport wt` for WebTransport bots, `node tools/e2e.mjs --help`; binaries from `$CARGO_TARGET_DIR/release`; logs in `target/e2e-logs`). Not part of CI
- Full check: `pwsh tools/ci.ps1` (Windows) or `tools/ci.sh`
- When several agents build at once, set `CARGO_TARGET_DIR=target/<your-area>` to avoid build-lock stalls

## Determinism rules (ls-math, ls-worldgen, ls-sim)
- Only IEEE-exact float ops directly; transcendental functions go through `ls_math::det` (libm). No `mul_add`, no fast-math.
- No HashMap iteration in outputs (use BTreeMap / sorted Vec), no wall-clock time, no threads, no OS RNG.
- Keep golden-hash tests passing; changing generator output requires bumping the generator version.

## Conventions
- Rust 2021, stable toolchain; workspace `rust-version = "1.85"` (also clippy's MSRV); only `ls-server` and `ls-bots` declare 1.88 (`wtransport`). `thiserror` for library errors, `anyhow` only in binaries.
- Units: 1 u = one ship length; time in ticks at 60 Hz.
- Tests next to code; every module spec in the plan has acceptance criteria — meet them.
- Don't commit; the orchestrator commits after each wave.
