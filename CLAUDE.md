# Loose Screws — repo guide

6DOF browser shooter with procedurally generated, destructible worlds. Rust core + WASM + TypeScript/WebGPU client.

- Docs index: `docs/README.md`; building, running, testing, adding weapons/rule sets: `docs/dev-guide.md`
- Design: `docs/design/game-design-doc.md` (exported from the living doc; source of truth for game rules)
- Plan and scope: `docs/implementation-plan.md` (read the relevant module spec before working on a crate)
- Current milestone: `docs/milestone-2-plan.md` (M2 arsenal and physics: scope, design decisions for gaps in the design doc, waves, acceptance criteria)
- Cross-crate contracts (terrain trait/edits, surface states, rock strength, mesh layers, axes, units, RNG streams): `docs/architecture/contracts.md`; types live in `ls_math::terrain`. Changing them is a cross-crate change: update the doc and all users together
- Architecture: `docs/architecture/overview.md` (crates, data flow), `netcode.md`, `determinism.md` (rules, golden hashes, re-blessing)
- Client/server glue (`ClientSession`, `Room`, wire conventions, lag compensation): `docs/architecture/game-glue.md`; server internals, browser client lifecycle, bots: `docs/architecture/server.md`
- Arsenal framework (weapon/gadget modules and hooks, ship types, entities, replication, data hash, rule set v2 and codes): `docs/architecture/arsenal-framework.md`. One module file per weapon (`crates/ls-sim/src/weapons/<family>/<id>.rs`) and gadget (`crates/ls-sim/src/gadgets/<id>.rs`), registered in `weapons/registry.rs` / `gadgets/mod.rs`; content work stays inside its module and data file
- Player manual and hosting guide (all server/bot flags for hosts): `userDocs/`; the weapon, ship and gadget tables in `userDocs/reference/` are generated from `data/` (never edit them by hand)

## Where you work
- The master repo `<master>` holds `main` and only receives pushes and `git merge --ff-only WIN1`. Never edit, build, test, npm or commit there (the guards refuse). The maintainer's session runs in it and drives the clones by absolute path (`cd <clone> && ...` per command)
- Work folders (clones, `origin` = the master): `<work root>/loosescrews.win1` (branch `WIN1`, the lead: integration, full CI, memory, CodeGraph index); `loosescrews.win2`+ (`WIN2`+, one per parallel work package: on a clean tree `git fetch && git reset --hard origin/main`, commit, `git push --force-with-lease origin WINn`). New one: `node tools/work-folders.mjs create <n>` from a work folder. Decision: `docs/adr/0001-master-repo-and-work-folders.md`; how-to: `docs/dev-guide.md` "Work folders"
- Hooks: pre-commit = per-file checks on staged files, pre-push = fast CI subset (`node tools/hooks/pre-push.mjs --list`); installed by `npm ci` in `web/`. Claude's post-edit hook runs the edited file's clone's checks and reports findings (exit 2)
- Read `.claude/memory/next-steps.md` first (project memory, first line of `MEMORY.md`): state of every work package, open questions, next steps

## If you are the lead
- Start: read `.claude/memory/next-steps.md`; `git -C <win1> fetch`; check the `WINn` branches (`node tools/work-folders.mjs status`)
- Integrate in win1: merge `origin/WINn` into `WIN1`, run `pwsh tools/ci.ps1` there; only when green: `git push origin WIN1`, then `git -C <master> merge --ff-only WIN1`. `main` is always green
- Update `.claude/memory/next-steps.md` after every finished step and before the maintainer closes a session, commit it on WIN1 and fast-forward `main` so the master's copy is current
- CodeGraph's index lives in `loosescrews.win1/.codegraph` (the master has none): pass `projectPath: <work root>/loosescrews.win1` to `codegraph_explore`, or run `codegraph explore` there
- Agents belong to the session that started them; a new session can't see or stop an older session's agents
- "Can I close this session?": report running background tasks, uncommitted/unpushed work per winN and `main` vs `WIN1` (`node tools/work-folders.mjs status`)

## Gotchas
- Never kill processes you didn't start
- Cargo runs with `-j 4` (`.cargo/config.toml`; shared machine)
- Python on Windows writes CRLF: open files with `newline=''`; write scripts to files, not heredocs with backslashes (the shell eats `\\`)
- `git stash -u` in the master can trip on locked empty folders; don't stash there (nothing should be uncommitted there anyway)
- Tests, bots, e2e and smoke bind `127.0.0.1` (port 0 where possible); `tools/check-binds.mjs` fails on wildcard binds outside its allowlist (Windows firewall prompts)

## Layout
- `crates/` Rust workspace crates (`ls-math`, `ls-worldgen`, `ls-sim`, `ls-net`, `ls-game` shared client/server glue, `ls-server`, `ls-wasm`, `ls-bots`)
- `data/` weapon, ship, gadget and rule set definitions (RON) — gameplay numbers live here, not in code; all files are embedded and hashed (`ls_sim::data_hash`, checked in the handshake)
- `web/` browser client (TypeScript strict, Vite, WebGPU, npm)
- `tools/` CI and helper scripts; work-folder guards `tools/lib/work-folder.mjs`, `tools/work-folders.mjs` (create/status/guard-master), Git hooks `tools/hooks/` + `.githooks/`, Claude hook `tools/post-edit.mjs`, `tools/check-binds.mjs`; their tests: `node --test "tools/test/*.test.mjs"`
- `docs/` developer docs; `userDocs/` player-facing manuals and hosting guides — keep them separate
- `.claude/memory/` Claude's persistent project memory (`MEMORY.md` index + one file per fact). Edit it in win1 (reaches the master via `main`); the harness memory dir is a junction to `loosescrews.win1\.claude\memory`; never store memory or project notes in temp or user folders

## Maintaining this file
Update CLAUDE.md in the same change whenever layout, commands, conventions or rules change. Keep it short: pointers and rules, not documentation (that belongs in `docs/`).

## Commands
- `cargo test --workspace` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo fmt --all`
- WASM: `cargo build -p ls-wasm --target wasm32-unknown-unknown --release` then `wasm-bindgen` (CLI 0.2.126; crate pinned to `=0.2.126`, `js-sys =0.3.103`, `wasm-bindgen-test =0.3.76` — bump all together)
- Unit tests on wasm32 (Node): set `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`, then `cargo test --release -p ls-math -p ls-net -p ls-sim -p ls-worldgen --target wasm32-unknown-unknown` and `cargo test --release -p ls-game --lib --target wasm32-unknown-unknown` (the ls-sim replay hash and ls-worldgen golden hashes must match native). Test modules opt in with `#[cfg(target_arch = "wasm32")] use wasm_bindgen_test::wasm_bindgen_test as test;`
- Bot end-to-end tests (room play, the navigation sweep over 10 worlds plus a 5-minute soak-world match, the nav grid cache (`tests/nav_cache.rs`: one build per world per process, CPU per bot at least 5x lower), `tests/arsenal_match.rs`: 8 bots, 240 s of sandbox_all_weapons, at least 15 distinct combat weapons incl. a mine and the Tectonic Charge) are ignored in debug builds: `cargo test -p ls-bots --release` (part of CI). The full match-setup sweep (8 seeds x 25 slider mixes) runs only in release: `cargo test -p ls-game --release --lib setup`
- Worldgen timings: `cargo run -p ls-worldgen --release --example worldgen_bench`
- Player reference pages: after any change in `data/weapons`, `data/ships`, `data/gadgets` or the default rule set run `node tools/gen-weapon-docs.mjs` (rewrites `userDocs/reference/*.md`); CI runs `node tools/gen-weapon-docs.mjs --check` and fails while they are stale
- Determinism cross-check (WASM golden hashes vs native fixture, Node): `node tools/determinism-check.mjs` (needs the WASM package from `cd web && npm run build:wasm`; part of CI)
- After an intentional generator change: bump `ls_worldgen::GENERATOR_VERSION`, then `LS_BLESS_GOLDEN=1 cargo test -p ls-worldgen --release --test golden`. After a sim behavior, tuning or `data/` change: update `REPLAY_GOLDEN_HASH` in `crates/ls-sim/src/tests.rs` (a behavior change without a `data/` change also bumps `ls_sim::SIM_BEHAVIOR_VERSION`, mixed into the handshake data hash)
- Web: `cd web && npm install && npm run typecheck && npm test && npm run build` (`npm run dev` serves on 127.0.0.1:41873 or the next free port, `LS_DEV_PORT` overrides; `?editor=rules` opens the rule set editor). After a data change that touches weapon names, slots, costs, levels or the default rule set: `LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode` regenerates `web/src/rules/catalog.ts`. Unit tests are `src/**/*.test.ts` run by `node --test` (Node strips types: use `.ts` import extensions and erasable-only syntax, no enums/parameter properties). `npm run build:wasm` resolves a relative `CARGO_TARGET_DIR` against the repo root (use `target/<area>`, not `../target/...`)
- Play locally: `cd web && npm run build`, then `cargo run -p ls-server --release -- [--code LS..] [--ruleset <file or LR.. code>] [--ban <weapon>]... [--balance-list <file>] [--web-root web/dist]` and open http://localhost:7777 (other flags: `--bind <addr>` (default `0.0.0.0:7777`), `--max-players`, `--stats-interval`, `--intermission`, `--exit-when-empty`, `--wt-bind <addr>` (WebTransport UDP, default = `--bind`), `--no-webtransport`; `--help`). The browser picks WebTransport when available, else WebSocket; `?transport=ws|wt` forces one. Bots: `cargo run -p ls-bots --release -- --count 7 [--duration 600] [--transport ws|wt]` (exit 0 OK, 1 protocol error/desync, 2 never joined, 3 lost connection)
- End-to-end: `pwsh tools/e2e.ps1` or `tools/e2e.sh` (release server + 7 bots for 90 s with log checks; `-SkipBuild`/`--skip-build`; other options go to `tools/e2e.mjs`, e.g. `--count 8 --duration 330` for a full match, `--transport wt` for WebTransport bots, `--ruleset data/rulesets/sandbox_all_weapons.ron` for every weapon as a pickup, `--ruleset tools/e2e-rulesets/sandbox_debris.ron --min-debris 64` for debris at cap, `--max-full-per-join 1`, `node tools/e2e.mjs --help`; binaries from `$CARGO_TARGET_DIR/release`; logs in `target/e2e-logs`). Not part of CI
- Browser smoke test: `pwsh tools/browser-smoke.ps1` or `tools/browser-smoke.sh` (`-SkipBuild`/`--skip-build`; headless Chrome/Edge with WebGPU via puppeteer-core in `tools/browser-smoke`: practice render, online flight firing weapons, wave C mechanics on a second server with `tools/browser-smoke/mechanics.ron`, rule set editor; screenshots and logs in `target/browser-smoke`; `node tools/browser-smoke/smoke.mjs --help`). Optional step of tools/ci.*: skipped without Chrome/Edge or with `LS_SKIP_BROWSER_SMOKE=1`
- Full check: `pwsh tools/ci.ps1` (Windows) or `tools/ci.sh`. Stop any Vite dev server first (on Windows it locks `web/node_modules` and CI's `npm ci` fails)
- Parallel packages use separate work folders; several cargo runs in one folder set `CARGO_TARGET_DIR=target/<your-area>` to avoid build-lock stalls

## Determinism rules (ls-math, ls-worldgen, ls-sim)
- Only IEEE-exact float ops directly; transcendental functions go through `ls_math::det` (libm). No `mul_add`, no fast-math.
- No HashMap iteration in outputs (use BTreeMap / sorted Vec), no wall-clock time, no threads, no OS RNG.
- Keep golden-hash tests passing; changing generator output requires bumping the generator version.

## Conventions
- Rust 2021, stable toolchain; workspace `rust-version = "1.85"` (also clippy's MSRV); only `ls-server` and `ls-bots` declare 1.88 (`wtransport`). `thiserror` for library errors, `anyhow` only in binaries.
- Units: 1 u = one ship length; time in ticks at 60 Hz.
- Tests next to code; every module spec in the plan has acceptance criteria — meet them.
- Don't commit; the orchestrator commits after each wave.
