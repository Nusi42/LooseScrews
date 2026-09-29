# Developer guide

How to build, run, test and extend Loose Screws locally. For how the pieces fit together see
[architecture/overview.md](architecture/overview.md). Commands are given for bash (Git Bash on
Windows works); PowerShell differences are noted where they matter.

## 1. Toolchain

| Tool | Version | Notes |
| --- | --- | --- |
| Rust | stable via `rustup` | `rust-toolchain.toml` selects stable and installs `rustfmt`, `clippy` and the `wasm32-unknown-unknown` target on first use. Minimum versions: 1.85 for the workspace (also clippy's MSRV), 1.88 for `ls-server` and `ls-bots` (`wtransport`). Tested with 1.97.1 |
| wasm-bindgen CLI | exactly **0.2.126** | `cargo install wasm-bindgen-cli --version 0.2.126 --locked`; installs `wasm-bindgen` and `wasm-bindgen-test-runner`. The crates are pinned to match (`wasm-bindgen =0.2.126`, `js-sys =0.3.103`, `wasm-bindgen-test =0.3.76` in the root `Cargo.toml`); bump all four together |
| Node.js + npm | Node with built-in TypeScript type stripping (22.18+ or 23.6+); tested with Node 26.7, npm 11.19 | The web unit tests run `.ts` files directly with `node --test`. npm, not pnpm |
| PowerShell 7 | optional | For `tools/*.ps1` on Windows; the `.sh` twins do the same in bash |
| Browser | WebGPU (current Chrome or Edge) | Firefox/Safari only where WebGPU is enabled. WebTransport is used when available, WebSocket otherwise |

No C toolchain beyond what Rust needs: `wtransport` uses the `ring` backend, so there is no
cmake or NASM step.

## 2. Building

```sh
cargo build --workspace                        # native crates (debug)
cargo build --release -p ls-server -p ls-bots  # the binaries you will run
cd web && npm install && npm run build         # WASM package + typecheck + Vite bundle into web/dist
```

`npm run build` runs `npm run build:wasm` (`web/scripts/build-wasm.mjs`: `cargo build -p ls-wasm
--target wasm32-unknown-unknown --release`, then `wasm-bindgen --target web` into
`web/src/wasm/pkg`, which is gitignored), `npm run typecheck` and `vite build`.
`node scripts/build-wasm.mjs --debug` builds the debug profile: faster to compile, but level
generation becomes very slow in the browser.

**Several builds at once.** Cargo holds a lock on its target directory, and a running
`ls-server.exe` locks its own binary on Windows. When several people or agents build in one
checkout, give each their own target directory:

```sh
export CARGO_TARGET_DIR=target/<your-area>       # PowerShell: $env:CARGO_TARGET_DIR = 'target/<your-area>'
```

Everything honours it: `build-wasm.mjs` resolves a relative path against the repo root (use
`target/<area>`, not `../target/...`), and the CI and e2e scripts take binaries from
`$CARGO_TARGET_DIR/release`.

## 3. Running locally

### Server + browser client

```sh
cd web && npm run build && cd ..
cargo run -p ls-server --release -- --code LS04-0000-0000-0000-8006-8001-8F9W
```

Open <http://localhost:7777>. The server serves `web/dist` (default `--web-root` when that
directory exists), the WebSocket at `/ws`, and WebTransport on UDP 7777 with the certificate
hash at `/cert-hash`. Useful flags (`ls-server --help` lists all):

| Flag | Default | Meaning |
| --- | --- | --- |
| `--bind` | `0.0.0.0:7777` | HTTP + WebSocket address |
| `--code` | random world | Share code of the level |
| `--ruleset` | built-in Paint the Void | Rule set file, e.g. `data/rulesets/paint_the_void.ron` |
| `--max-players` | 8 | 1..=32 |
| `--wt-bind` / `--no-webtransport` | same port as `--bind` over UDP | WebTransport endpoint |
| `--stats-interval` | 10 | Seconds between `tick stats` log lines |
| `--intermission` | 10 | Seconds the result shows before everyone rejoins the next match |
| `--exit-when-empty` | off | Exit 2 s after the last player leaves (scripts) |

Logging follows `RUST_LOG` (default `info`), e.g. `RUST_LOG=ls_server=debug`.

In the browser, `?transport=ws` or `?transport=wt` forces a transport; F3 toggles the debug
overlay, Tab shows the scoreboard. The start screen also offers offline practice on any share
code (no server needed).

To use another port, pass `--bind 127.0.0.1:<port>` and open that port; keep one port per
server process (two servers on one port fail to bind).

### Vite dev server (hot reload)

```sh
cd web && npm run dev      # http://localhost:5173
```

`npm run dev` builds the WASM package first. Vite proxies `/ws` and `/cert-hash` to
`127.0.0.1:7777` (`web/vite.config.ts`), so "Play online" needs an `ls-server` on port 7777;
practice works without one. Rust changes need `npm run build:wasm` and a page reload.

### Bots

```sh
cargo run -p ls-bots --release -- --count 7                       # ws://127.0.0.1:7777/ws, until Ctrl+C
cargo run -p ls-bots --release -- --count 7 --duration 600        # 10-minute soak
cargo run -p ls-bots --release -- --server ws://127.0.0.1:7791/ws --transport wt --count 2 --duration 30
```

With `--transport wt` the bots fetch `/cert-hash` from the `--server` host and port and connect
over WebTransport. Each bot prints a summary (snapshots, reconciliation, combat, protocol errors,
desyncs). Exit codes: 0 OK, 1 protocol error or desync, 2 a bot never joined, 3 a bot lost its
connection (`--allow-disconnects` ignores that). Other flags: `--stagger-ms`, `--seed`,
`--name-prefix`, `--no-reconnect`, `--verbose`.

## 4. Tests

| What | Command | Notes |
| --- | --- | --- |
| All Rust tests | `cargo test --workspace` | Debug; `ls-game` alone takes about 40 s (level generation) |
| Bot end-to-end tests | `cargo test -p ls-bots --release` | Ignored in debug builds (too slow); part of CI |
| Match setup sweep | `cargo test -p ls-game --release --lib setup` | 8 seeds x 25 slider mixes (1 seed in debug) |
| One crate / one test | `cargo test -p ls-sim replay_golden_hash` | Standard cargo filters |
| Worldgen golden hashes | `cargo test -p ls-worldgen --release --test golden` | See [determinism](architecture/determinism.md) |
| Unit tests on wasm32 | see [determinism.md](architecture/determinism.md#the-wasm-cross-check) | Needs `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`; do not pass `-q` |
| WASM vs native hashes | `node tools/determinism-check.mjs` | Needs the package from `cd web && npm run build:wasm` (or `--pkg <dir>`) |
| Lints and format | `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all` | CI runs `cargo fmt --all -- --check` |
| Web | `cd web && npm run typecheck && npm test` | `src/**/*.test.ts` via `node --test`; Node strips types, so use `.ts` import extensions and erasable-only syntax (no enums, no parameter properties) |

Rust tests sit next to the code (`#[cfg(test)] mod tests`) or in each crate's `tests/`.
A test module that should also run on wasm32 opts in with
`#[cfg(target_arch = "wasm32")] use wasm_bindgen_test::wasm_bindgen_test as test;`.
Every module spec in the [implementation plan](implementation-plan.md) has acceptance criteria;
new work should come with tests that show them.

## 5. CI

There is no hosted CI yet; the full check is a local script that stops at the first failure:

```sh
pwsh tools/ci.ps1        # Windows
tools/ci.sh              # bash
```

Steps, in order: `cargo fmt --check`; clippy with `-D warnings`; `cargo test --workspace`;
`cargo test -p ls-bots --release`; the wasm32 unit tests (skipped with a message if
`wasm-bindgen-test-runner` or `node` is missing); `cargo build -p ls-wasm` for wasm32; in
`web/`: `npm ci`, `typecheck`, `test`, `build`; finally `tools/determinism-check.mjs`. Run it
before handing in a change. It honours `CARGO_TARGET_DIR`. Note that it rebuilds
`web/src/wasm/pkg` and `web/dist`.

## 6. End-to-end run

```sh
pwsh tools/e2e.ps1                 # or: tools/e2e.sh
pwsh tools/e2e.ps1 -SkipBuild      # or: tools/e2e.sh --skip-build
pwsh tools/e2e.ps1 --count 8 --duration 330        # a full 5-minute match plus the restart
pwsh tools/e2e.ps1 --transport wt                   # bots over WebTransport
node tools/e2e.mjs --help
```

The wrappers build the release binaries and the web client, then `tools/e2e.mjs` starts the
server on a free port with a fixed share code, checks that it serves `web/dist`, runs 7 bots for
90 s and checks the bot summary and the server log: exit codes, every bot joined, no desyncs or
protocol errors, no input starvation warnings, tick p99 under 4 ms in every stats window, no
skipped ticks, craters and paint growing, the match timer running, a clean shutdown.
`--count 8 --duration 330` covers a full match including the restart. `--transport wt` runs
the bots over WebTransport and also checks `/cert-hash` and that every bot joined, and every
server session ran, over WebTransport. Logs go to `target/e2e-logs` (`--log-dir`). To reuse
binaries from your own target directory:

```sh
node tools/e2e.mjs --server-bin target/<area>/release/ls-server --bots-bin target/<area>/release/ls-bots \
    --count 4 --duration 30 --log-dir target/<area>/e2e-logs
```

(add `.exe` on Windows). The e2e run is not part of `tools/ci.*`.

## 7. Profiling tips

- **Server tick time:** the `tick stats` log line (every `--stats-interval` s) has p50/p99/max
  tick time in µs, `over_budget` and `skipped` ticks, input counters (`missing_inputs`,
  `late_inputs`, `duplicate_inputs`), `snapshot_kib_s`, edits, craters, paint cells, and
  `desync_warnings`. The M1 budget is p99 < 4 ms; a local release run with 4 bots
  measured a p99 of about 0.1 ms. `final stats` is logged at shutdown.
- **Always measure release builds.** Debug builds are far slower at level generation (a full
  level takes about 7 s in debug at the default sliders and well under a second in release).
- **World generation and meshing:**

  ```sh
  cargo run -p ls-worldgen --release --example worldgen_bench      # generation, remesh after craters, eval cost, dual contouring
  cargo run -p ls-worldgen --release --example mesh_bench [CODE]   # meshing timings for one share code
  cargo test -p ls-worldgen --release --test timing -- --nocapture # asserts < 2 ms per 32³ chunk (surface nets)
  ```

- **Browser:** F3 shows FPS, ping, transport, prediction corrections and snaps, shot prediction
  counters, chunk counts and skipped sends. Chunk meshing is budgeted per frame (4 ms / 8 chunks
  in play, `web/src/game/chunks.ts`). Use the browser's Performance panel for frame breakdowns;
  a release WASM build is required for meaningful numbers.
- **Network:** `ls_net::MemoryTransport::pair_with(LinkConditions { .. })` reproduces latency,
  jitter, loss, reordering and duplication in tests (see `crates/ls-game/tests/netplay.rs`).
  Bot summaries show reconciliation counts and maximum error per bot.

## 8. Adding a weapon

Every weapon of the design doc already exists: a data file (`data/weapons/<id>.ron`, schema
`ls_sim::WeaponDef`), a behaviour module (`crates/ls-sim/src/weapons/<family>/<id>.rs`,
`WeaponBehavior`) and an entry in `crates/ls-sim/src/weapons/registry.rs`. How modules work,
their hooks and the per-tick order are in
[architecture/arsenal-framework.md](architecture/arsenal-framework.md). To add a new one
(example: an ordnance secondary):

1. Create `data/weapons/nail_mortar.ron`:

   ```ron
   // Rivet Mortar: slow lobbed nails that crater on impact.
   WeaponDef(
       id: "nail_mortar",            // snake_case, unique; rule sets refer to it
       name: "Rivet Mortar",
       family: Ordnance,             // Energy | Kinetic | Ordnance | Special
       slot: Secondary,              // Primary | Secondary
       speed: 25.0,                  // u/s, never inherits the shooter's velocity
       lifetime_secs: 2.5,
       damage: 20.0,                 // direct hit at level 1
       fire_interval_secs: 1.0,
       cost: Count(1),               // Energy(f32) | Scrap(f32) | Count(u32)
       max_ammo: 4,                  // required for Count weapons (>= the count)
       projectile_radius: 0.2,
       splash: Some((radius: 3.0, damage: 15.0)),   // optional `hits_owner: true`
       knockback: 4.0,               // u/s on a direct hit (scales with damage)
       terrain: (crater_radius: 1.5),                // radius = design-doc diameter / 2; also paint_radius
       shield: Wears(1.0),           // Blocked | Wears(f >= 0) | PassThrough(0..=1)
       visual: (shape: Grenade, color: (200, 160, 90), scale: 1.0),
       params: { "nails": 5.0 },     // free numbers for the module (`def.param("nails", 5.0)`)
   )
   ```

   Optional fields: `max_level` and `damage_per_level` (upgrades), `projectiles` and
   `spread_deg` (generic multi-projectile pattern), `charge: Some((max_secs: 2.0, min_secs:
   0.0, auto_release: false, damage_mult: 3.0))`. `WeaponDef::validate` rejects out-of-range
   values.
2. Add the module file `crates/ls-sim/src/weapons/ordnance/nail_mortar.rs` (copy a `TODO(wave-B)`
   module), list it in `weapons/ordnance/mod.rs` and **append** an entry to
   `registry::MODULES` (rule set codes index that list; never reorder it). Override the hooks
   the weapon needs; the defaults are the generic straight-projectile weapon.
3. Make it available in a rule set: `weapons` (`LoadoutAndPickup`, `PickupOnly`, `LoadoutOnly`;
   unlisted means `Banned`), and `starting_loadout` and/or `pickups` (count weapons take an
   `ammo` per pickup; `max_on_map` caps copies on the map).
4. Client presentation: projectiles and pickups are drawn from the weapon's `visual` (see the
   `VisualTable` in the arsenal doc and `SHAPE_STYLES` in `web/src/game/scene.ts`); number keys,
   the wheel and X follow the registry order automatically, and the weapon panel takes names
   from the data. No client code is needed unless the weapon needs a new `Shape`.
5. Bots pick weapons by cost (`crates/ls-bots/src/brain.rs`); teach them special mechanics.
6. Tests and hashes: `weapons::tests::every_weapon_has_a_module_and_a_data_file` and
   `builtin_weapons_match_design_doc` assert the weapon count; add scenario tests; update
   `REPLAY_GOLDEN_HASH` (see [determinism.md](architecture/determinism.md#re-blessing-after-an-intentional-change)).
   The data hash changes too, so rebuild everything that embeds data together:
   `cargo build --release -p ls-server -p ls-bots` and `cd web && npm run build`.

Things to know:

- `WeaponId`s are indices in **id-sorted order**, so a new id shifts the ids of every weapon
  sorted after it. Ids travel on the wire (projectile `kind`, ship flags, inputs); the
  handshake compares the data hash, so a client built from other data files is rejected with
  a clear reason instead of desyncing.
- Ship flags carry the selected primary in 5 bits (ids 0-31); the table holds at most 255.
- The fastest weapon sets the projectile velocity quantization range
  (`quant_config_for`: at least 320 u/s).

## 9. Adding a rule set

A rule set for an existing mode is pure data (schema `ls_sim::RuleSet`, v2): the server loads
the file at startup, validates it against the weapon and gadget tables, and sends its RON text
to every client in the `Welcome`, so browsers and bots need no rebuild. Every setting of the
design doc's match rules is a field; see `data/rulesets/paint_the_void.ron` for all of them.

1. Copy `data/rulesets/paint_the_void.ron` to a new file and edit it:

   ```ron
   // Lasers Only: Paint the Void without rockets or rivets, 2-minute rounds.
   RuleSet(
       name: "Lasers Only",
       mode: PaintTheVoid((teams: 2, match_secs: 120.0, respawn_secs: 2.0)),
       weapons: {                                // unlisted weapons are Banned
           "scrap_laser": LoadoutAndPickup,
           "paint_thrower": LoadoutOnly,
       },
       gadgets: { "shield": LoadoutOnly },       // unlisted gadgets are Banned
       starting_loadout: (
           primaries: ["scrap_laser", "paint_thrower"],
           secondaries: [],                      // (id, count) pairs for secondaries
           gadget: Some("shield"),
       ),
       pickups: {},                              // weapon pickups: (weight, respawn_secs, ammo, max_on_map)
       upgrade_pickups: {
           "scrap_laser": (weight: 1.0, respawn_secs: 20.0),   // laser levels 1-6
       },
       scrap_pickup: Some((weight: 1.0, respawn_secs: 15.0, amount: 30.0)),
       shield_overrides: { "scrap_laser": PassThrough(0.5) },
       tuning: { "scrap_laser": (damage: 1.25, fire_rate: 1.0, ammo: 1.0) },
       friendly_fire_percent: 0,                 // 0 = off, otherwise 10..=100
       allowed_ship_types: [Skitter, Tinker],
       absurdity: 0,                             // 0..=100, used from M4
   )
   ```

2. Run it: `cargo run -p ls-server --release -- --ruleset data/rulesets/lasers_only.ron`. The log
   line `match ready ... ruleset=Lasers Only` confirms it; an invalid file stops the server with
   the reason (for example `invalid rule set: friendly_fire_percent 5 must be 0 (off) or
   10..=100`). `RuleSet::resolve` checks that every weapon and gadget exists, loadout items are
   allowed in the loadout and fit their slot, pickups are pickup-able, and the numbers are in
   range.
3. Play it with bots and the browser as usual; `tools/e2e.mjs --ruleset <file>` runs the e2e
   checks on it (use `--min-craters 0` if the rule set has no crater weapons).

Rule sets also travel as short codes: `RuleSet::to_code()` gives `LR...` (Crockford base32,
CRC-16) and `RuleSet::from_code` reads it back exactly (format in
`crates/ls-sim/src/rulecode.rs`; values the format cannot carry, such as a weight of 0.3, are
rejected when encoding).

Only `data/rulesets/paint_the_void.ron` is embedded (`RuleSet::paint_the_void()`, the default
and the practice mode's rules). A new **mode** is code: a `ModeRules` variant in
`crates/ls-sim/src/ruleset.rs`, its logic next to `crates/ls-sim/src/mode.rs`, and scoring and
HUD support in `ls-game`, `ls-wasm` and `web/`.

## 10. Where things are documented

- Rules for contributors and agents: [`CLAUDE.md`](../CLAUDE.md) (keep it in sync when commands
  or layout change).
- Game rules: [design/game-design-doc.md](design/game-design-doc.md).
- Scope, module specs and status: [implementation-plan.md](implementation-plan.md).
- Architecture: [architecture/](architecture/) (see the [index](README.md)).
- Player-facing manuals and hosting guides belong in [`userDocs/`](../userDocs/), not here.
