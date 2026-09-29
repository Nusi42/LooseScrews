# Loose Screws — repo guide

6DOF browser shooter with procedurally generated, destructible worlds. Rust core + WASM + TypeScript/WebGPU client.

- Design: `docs/design/game-design-doc.md` (exported from the living doc; source of truth for game rules)
- Plan and scope: `docs/implementation-plan.md` (read the relevant module spec before working on a crate)

## Layout
- `crates/` Rust workspace crates (`ls-math`, `ls-worldgen`, `ls-sim`, `ls-net`, `ls-server`, `ls-wasm`, `ls-bots`)
- `data/` weapon and rule set definitions (RON) — gameplay numbers live here, not in code
- `web/` browser client (TypeScript strict, Vite, WebGPU, npm)
- `tools/` CI and helper scripts
- `docs/` developer docs; `userDocs/` player-facing manuals and hosting guides — keep them separate
- `.claude/memory/` Claude's persistent project memory (`MEMORY.md` index + one file per fact). The harness memory dir is a junction to it; never store memory or project notes in temp or user folders

## Maintaining this file
Update CLAUDE.md in the same change whenever layout, commands, conventions or rules change. Keep it short: pointers and rules, not documentation (that belongs in `docs/`).

## Commands
- `cargo test --workspace` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo fmt --all`
- WASM: `cargo build -p ls-wasm --target wasm32-unknown-unknown --release` then `wasm-bindgen` (CLI 0.2.126; crate pinned to `=0.2.126`)
- Web: `cd web && npm install && npm run build && npm run typecheck`
- Full check: `tools/ci.ps1` (Windows) or `tools/ci.sh`
- When several agents build at once, set `CARGO_TARGET_DIR=target/<your-area>` to avoid build-lock stalls

## Determinism rules (ls-math, ls-worldgen, ls-sim)
- Only IEEE-exact float ops directly; transcendental functions go through `ls_math::det` (libm). No `mul_add`, no fast-math.
- No HashMap iteration in outputs (use BTreeMap / sorted Vec), no wall-clock time, no threads, no OS RNG.
- Keep golden-hash tests passing; changing generator output requires bumping the generator version.

## Conventions
- Rust 2021, stable toolchain; `thiserror` for library errors, `anyhow` only in binaries.
- Units: 1 u = one ship length; time in ticks at 60 Hz.
- Tests next to code; every module spec in the plan has acceptance criteria — meet them.
- Don't commit; the orchestrator commits after each wave.
