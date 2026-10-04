# Developer guide

How to build, run, test and extend Loose Screws locally. For how the pieces fit together see
[architecture/overview.md](architecture/overview.md). Commands are given for bash (Git Bash on
Windows works); PowerShell differences are noted where they matter.

## Work folders

Decision record: [adr/0001-master-repo-and-work-folders.md](adr/0001-master-repo-and-work-folders.md).
`<master>` is the **master repository**. It holds `main` and only
receives pushes and `git merge --ff-only WIN1`. Nothing is built, tested, installed, run or
committed there. All work happens in clones outside OneDrive, with `origin` pointing to the
master:

| Folder | Branch | Use |
| --- | --- | --- |
| `<work root>/loosescrews.win1` | `WIN1` | the lead: integration, full CI, memory, CodeGraph index |
| `loosescrews.win2`, `loosescrews.win3`, … | `WIN2`, `WIN3`, … | one per parallel work package |

Setting up a fresh work folder (this is what `create` does, plus the build):

```sh
node tools/work-folders.mjs create 3      # from a work folder: clone the master into loosescrews.win3 (branch WIN3),
                                          # write .claude/settings.local.json, npm ci in web/ (installs the Git hooks)
cd <work root>/loosescrews.win3/web && npm run build   # WASM package (wasm-bindgen 0.2.126) + client
```

By hand: `git clone <master> loosescrews.winN`, `git checkout -b WINN`,
`cd web && npm ci && npm run build`. After that, pre-push passes in the new folder.

Day to day:

```sh
git fetch origin && git reset --hard origin/main      # a package folder starts from main (clean tree first)
git push --force-with-lease origin WIN3               # package done (pre-push runs the fast CI subset)
# lead, in win1: merge origin/WIN3 into WIN1, run pwsh tools/ci.ps1; when green:
git push origin WIN1 && git -C <master> merge --ff-only WIN1
node tools/work-folders.mjs status                    # uncommitted / unpushed per winN, main vs WIN1
```

`main` only ever moves to a `WIN1` commit whose full `tools/ci.*` passed.

**Hooks.** `npm ci` / `npm install` in `web/` runs `prepare`, which sets
`core.hooksPath=.githooks`:

- **pre-commit** runs the per-file checks on the staged files (`tools/lib/run-checks.mjs`):
  rustfmt, the web `tsc`, wildcard binds, LF line endings, and the generated reference docs
  when `data/` changed. It reads the working-tree copy of each file, so stage the whole file
  you fixed.
- **pre-push** runs the fast CI subset; `node tools/hooks/pre-push.mjs --list` prints the steps.
- **Claude Code** runs the same per-file checks after every edit (`.claude/settings.json` →
  `tools/post-edit.mjs`). It finds the clone from the edited file, not from the session folder
  (a session may run in the master), and skips files of the master checkout. Findings go back
  to the agent: exit 2, at most 20 lines.

No check looks at `target/`, `node_modules/`, `dist/` or the WASM `pkg/`.

**Guards.** The following refuse to run inside any OneDrive folder, with a message that names
the work folders:

- `tools/ci.*`, `tools/e2e.*`, `tools/browser-smoke.*`;
- `npm run dev` / `build` / `typecheck` (through `build-wasm.mjs`);
- `npm ci`'s `prepare`;
- both Git hooks.

In the master, `node <win1>/tools/work-folders.mjs guard-master` writes the gitignored
`.cargo/config.local.toml`. The committed `.cargo/config.toml` includes it, and its
`rustc-wrapper` (`.cargo/refuse-build.cmd`) makes every cargo build there fail. Machine-specific
cargo settings go into a work folder's own `config.local.toml`. Cargo runs with `jobs = 4`
(shared machine).

**Ports and binds.** The Vite dev server prefers 127.0.0.1:41873, takes any free port when that
one is busy, and `LS_DEV_PORT` forces a port. Tests, bots, e2e and the smoke test bind
`127.0.0.1`, port 0 where possible, so Windows does not ask to allow every new test binary in
the firewall. `node tools/check-binds.mjs` (in CI and pre-push) fails on a new `0.0.0.0` /
`[::]` bind; real exceptions go into its allowlist, with a reason.

**The cloud-sync reparse-point fix (done once).** Git for Windows cannot check out
into OneDrive cloud-file reparse points (`fsutil reparsepoint query crates` → `0x9000e01a`;
`git merge --ff-only` fails with "unable to unlink"). The fix:

1. Stop everything that uses the master.
2. Move every folder except `.git` to `<work root>\colent.master-backup`.
3. Run `git checkout HEAD -- .` in the master, and check that `fsutil` now reports error 4390
   (not a reparse point).

If it ever comes back, repeat these steps. Never `reset --hard` and never write files by hand in
the master.

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
cargo run -p ls-server --release -- --code LS08-0000-0000-0000-8006-8001-8F9G
```

Open <http://localhost:7777>. The server serves `web/dist` (default `--web-root` when that
directory exists), the WebSocket at `/ws`, and WebTransport on UDP 7777 with the certificate
hash at `/cert-hash`. Useful flags (`ls-server --help` lists all):

| Flag | Default | Meaning |
| --- | --- | --- |
| `--bind` | `0.0.0.0:7777` | HTTP + WebSocket address |
| `--code` | random world | Share code of the level |
| `--ruleset` | built-in Paint the Void | Rule set file (e.g. `data/rulesets/sandbox_all_weapons.ron`) or an `LR...` rule set code |
| `--ban <weapon>` | none | Bans a weapon on top of the rule set (repeatable) |
| `--balance-list <file>` | none | Server-wide bans and tuning on top of the rule set, re-read before every match ([section 9](#balance-lists)) |
| `--web-root` | `web/dist` if it exists | Built web client to serve |
| `--max-players` | 8 | 1..=32 |
| `--wt-bind` / `--no-webtransport` | same port as `--bind` over UDP | WebTransport endpoint |
| `--stats-interval` | 10 | Seconds between `tick stats` log lines |
| `--intermission` | 10 | Seconds the result shows before everyone rejoins the next match |
| `--exit-when-empty` | off | Exit 2 s after the last player leaves (scripts) |

Logging follows `RUST_LOG` (default `info`), e.g. `RUST_LOG=ls_server=debug`.

At match start the server logs `rules: <name> <LR code> (banned weapons: ...)`, the
effective rules after the balance list and bans.

In the browser, `?transport=ws` or `?transport=wt` forces a transport; F3 toggles the debug
overlay, Tab shows the scoreboard, B opens the ship bay (1-4 pick the type for the next
life), V switches the selected primary's weapon mode (Paint Thrower colour), G uses the
gadget. The start screen also offers offline practice on any share code (no server needed)
and links the rule set editor (`?editor=rules`, [section 9](#rule-set-codes-and-the-editor)).

To use another port, pass `--bind 127.0.0.1:<port>` and open that port; keep one port per
server process (two servers on one port fail to bind).

### Vite dev server (hot reload)

```sh
cd web && npm run dev      # http://127.0.0.1:41873 (next free port if taken; LS_DEV_PORT=<port> forces one)
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

### Directory service (server list)

```sh
cargo run -p ls-directory --release -- --bind 127.0.0.1:7780
curl http://127.0.0.1:7780/servers
```

Optional: game servers register with heartbeats and the browser lists them. API, flags and the
client library: [architecture/directory.md](architecture/directory.md). Tests:
`cargo test -p ls-directory` (HTTP tests on `127.0.0.1:0`).

## 4. Tests

| What | Command | Notes |
| --- | --- | --- |
| All Rust tests | `cargo test --workspace` | Debug; `ls-game` alone takes about 40 s (level generation) |
| Bot end-to-end tests | `cargo test -p ls-bots --release` | Ignored in debug builds (too slow); part of CI. Includes `tests/nav_sweep.rs` (10 worlds plus a 5-minute soak-world match) and `tests/arsenal_match.rs` (8 bots, 240 s of `sandbox_all_weapons`: at least 15 distinct combat weapons incl. a mine and the Tectonic Charge, 0 desyncs, reconciliation error under 1 u); one file: `cargo test -p ls-bots --release --test arsenal_match -- --nocapture` |
| Weapon and gadget scenario tests | `cargo test -p ls-sim --lib weapons` / `cargo test -p ls-sim --lib gadgets` | One test module per weapon and gadget module (bonus, malus, lasting effect), plus the registry checks in `weapons::tests`; world-level framework tests in `tests_arsenal` |
| Rule set codes and the web catalog | `cargo test -p ls-sim --lib rulecode` | Includes `web_catalog_is_current`; regenerate with `LS_BLESS_RULES_CATALOG=1` ([section 9](#rule-set-codes-and-the-editor)) |
| Snapshot bandwidth | `cargo test -p ls-net --lib busy_fight_with_projectiles_fits_the_budget -- --nocapture` | Prints mean and worst delta size per seed and baseline age ([netcode](architecture/netcode.md#snapshots-and-deltas)); `cargo test -p ls-game --lib busy_fight_fits_the_bandwidth_budget` is the ship-physics variant |
| Match setup sweep | `cargo test -p ls-game --release --lib setup` | 8 seeds x 25 slider mixes (1 seed in debug) |
| One crate / one test | `cargo test -p ls-sim replay_golden_hash` | Standard cargo filters |
| Worldgen golden hashes | `cargo test -p ls-worldgen --release --test golden` | See [determinism](architecture/determinism.md) |
| Unit tests on wasm32 | see [determinism.md](architecture/determinism.md#the-wasm-cross-check) | Needs `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`; do not pass `-q` |
| WASM vs native hashes | `node tools/determinism-check.mjs` | Needs the package from `cd web && npm run build:wasm` (or `--pkg <dir>`) |
| Lints and format | `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all` | CI runs `cargo fmt --all -- --check` |
| Generated player reference | `node tools/gen-weapon-docs.mjs --check` | Fails when `userDocs/reference/*.md` differ from `data/`; regenerate without `--check` ([section 5](#generated-player-reference)) |
| Web | `cd web && npm run typecheck && npm test` | `src/**/*.test.ts` via `node --test`; Node strips types, so use `.ts` import extensions and erasable-only syntax (no enums, no parameter properties) |
| Browser smoke test | `pwsh tools/browser-smoke.ps1` or `tools/browser-smoke.sh` (`-SkipBuild` / `--skip-build`) | Headless Chrome or Edge with WebGPU (puppeteer-core in `tools/browser-smoke`): practice render, online flight firing weapons, the wave C mechanics against a second server with `tools/browser-smoke/mechanics.ron` (ship bay, charge meter, Leech Arc, V paint colour, Eyeball view, Decoy cooldown), the rule set editor and a server accepting its code. Screenshots and logs in `target/browser-smoke`; `node tools/browser-smoke/smoke.mjs --help`. Skips (exit 0) without Chrome or Edge |

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

Steps, in order: `cargo fmt --check`; `node tools/gen-weapon-docs.mjs --check` (the generated
player reference pages, [below](#generated-player-reference)); `node tools/check-binds.mjs` and
the tools tests (`node --test "tools/test/*.test.mjs"`: hooks, guards, settings); clippy with `-D warnings`;
`cargo test --workspace`;
the strict `ls-server` timing test alone (`--ignored --test-threads=1`; the parallel run only
checks a loose bound); `cargo test -p ls-bots --release`; the wasm32 unit tests (skipped with a message if
`wasm-bindgen-test-runner` or `node` is missing); `cargo build -p ls-wasm` for wasm32; in
`web/`: `npm ci`, `typecheck`, `test`, `build`; `tools/determinism-check.mjs`; finally the
browser smoke test (skipped without Chrome or Edge, or with `LS_SKIP_BROWSER_SMOKE=1`; a
browser without a WebGPU adapter fails it, so set the variable on such machines). Stop any
`npm run dev` (Vite) first: on Windows it locks `web/node_modules` and `npm ci` fails. Run it
before handing in a change. It honours `CARGO_TARGET_DIR`. Note that it rebuilds
`web/src/wasm/pkg` and `web/dist`.

### Generated player reference

`userDocs/reference/{weapons,ships,gadgets}.md` are generated from `data/weapons`,
`data/ships`, `data/gadgets` and the default rule set by `tools/gen-weapon-docs.mjs` (its own
RON reader, no build needed). Never edit them by hand:

```sh
node tools/gen-weapon-docs.mjs           # rewrite the three pages after a data change
node tools/gen-weapon-docs.mjs --check   # CI: exit 1 if a page is missing or stale
```

Descriptions a table cell needs (the notes under each weapon family, parameter units) live in
the generator; what a weapon does in prose belongs in the
[player manual](../userDocs/player-manual.md#weapons), which links to the tables.

## 6. End-to-end run

```sh
pwsh tools/e2e.ps1                 # or: tools/e2e.sh
pwsh tools/e2e.ps1 -SkipBuild      # or: tools/e2e.sh --skip-build
pwsh tools/e2e.ps1 --count 8 --duration 330        # a full 5-minute match plus the restart
pwsh tools/e2e.ps1 --transport wt                   # bots over WebTransport
pwsh tools/e2e.ps1 --ruleset data/rulesets/sandbox_all_weapons.ron --count 8 --duration 330   # every weapon as a pickup
pwsh tools/e2e.ps1 --ruleset tools/e2e-rulesets/sandbox_debris.ron --count 8 --duration 330 --min-debris 64
node tools/e2e.mjs --help
```

The wrappers build the release binaries and the web client, then `tools/e2e.mjs` starts the
server on a free port with a fixed share code, checks that it serves `web/dist`, runs 7 bots for
90 s and checks the bot summary and the server log: exit codes, every bot joined, no desyncs or
protocol errors, no input starvation warnings, tick p99 under 4 ms in every stats window, no
skipped ticks, craters and paint growing, the match timer running, a clean shutdown.
`--count 8 --duration 330` covers a full match including the restart.
`--min-debris 64` requires the networked debris to reach its cap in some stats window
(`tools/e2e-rulesets/sandbox_debris.ron`, the sandbox with up to 8 Tectonic Charges on the
map, gets there in a full match; the plain sandbox usually peaks at 24-48);
`--max-full-per-join 1` checks the bots got at most one full snapshot per join (M1 issue B).
Tick p99 is wall-clock time on the sim thread: run the stress runs on a quiet machine (other
builds or a busy sync client show up as slow receive/send phases in the `tick stats` line). `--transport wt` runs
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
  tick time in µs, the longest time per tick phase (`recv_max_us`, `step_max_us`,
  `send_max_us`: a slow window with high receive/send times means the thread was descheduled,
  not that the game work grew), `over_budget` and `skipped` ticks, input counters
  (`missing_inputs`, `late_inputs`, `duplicate_inputs`, `dropped_inputs`),
  `compensated_hits`, `snapshot_kib_s`, edits, craters, `debris` / `debris_peak` (networked
  pieces, cap 64), paint and surface cells, and `desync_warnings`. The budget is p99 < 4 ms;
  local release runs measure a few hundred µs (a 4-bot sandbox run: worst window p99 173 µs).
  `final stats` is logged at shutdown.
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

Every weapon of the design doc already exists as three pieces: a data file
(`data/weapons/<id>.ron`, schema `ls_sim::WeaponDef`), a behaviour module
(`crates/ls-sim/src/weapons/<family>/<id>.rs`, trait `WeaponBehavior`) and an entry in
`crates/ls-sim/src/weapons/registry.rs`. The full reference (every hook, the context helpers,
the per-tick order, entities, replication, what each of the 27 weapons does) is
[architecture/arsenal-framework.md](architecture/arsenal-framework.md); this section is the
checklist. The walkthrough adds an example ordnance secondary, the Rivet Mortar, and was
checked end to end against a copy of `ls-sim`.

### 8.1 Data

`data/weapons/nail_mortar.ron`:

```ron
// Rivet Mortar: slow lobbed shell that bursts into nails on impact.
// Behaviour: crates/ls-sim/src/weapons/ordnance/nail_mortar.rs. Schema: `ls_sim::WeaponDef`.
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
    params: { "nails": 5.0, "nail_speed": 40.0, "nail_damage": 4.0 },   // read with def.param(..)
)
```

Optional fields: `max_level` and `damage_per_level` (upgrade pickups), `projectiles` and
`spread_deg` (the generic multi-projectile pattern), `charge: Some((max_secs: 2.0, min_secs:
0.0, auto_release: false, damage_mult: 3.0))` (hold to charge; the framework handles holding,
paying on release and the HUD charge meter). `WeaponDef::validate` rejects out-of-range values.
Every number the module needs goes into the file: common ones as typed fields, anything else
in `params`, read with `def.param("nails", 5.0)`. Tuning never needs a code change.

### 8.2 Behaviour module

`crates/ls-sim/src/weapons/ordnance/nail_mortar.rs`:

```rust
//! Rivet Mortar: ordnance secondary. Bonus: bursts into nails on impact.
//! Malus: slow. Lasting effect: a 3 u crater.
//!
//! Numbers: `data/weapons/nail_mortar.ron` (`nails`, `nail_speed`, `nail_damage`).

use ls_math::terrain::RayHit;
use ls_math::{Quat, Vec3};

use crate::behavior::Ctx;
use crate::weapons::{sub_projectile, HitResponse, WeaponBehavior};
use crate::world::Projectile;

/// Behaviour of the Rivet Mortar.
#[derive(Debug)]
pub struct NailMortar;

impl WeaponBehavior for NailMortar {
    fn id(&self) -> &'static str {
        "nail_mortar"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        if p.generation > 0 {
            return HitResponse::Stop; // a nail: embeds, nothing else
        }
        // The default impact: Impact event, splash, the data's crater.
        ctx.detonate(p, hit.point, None, true);
        let def = ctx.def(p.weapon);
        let (n, speed, damage) = (
            def.param("nails", 5.0) as u32,
            def.param("nail_speed", 40.0),
            def.param("nail_damage", 4.0),
        );
        for i in 0..n {
            let turn = Quat::from_axis_angle(hit.normal, i as f32 * 6.283_185 / n as f32);
            let dir = (hit.normal + turn.rotate(Vec3::X) * 0.5).normalize_or_zero();
            let mut nail = sub_projectile(ctx, p, hit.point + hit.normal * 0.3, dir * speed, 30);
            nail.damage = damage;
            ctx.spawn_projectile(nail);
        }
        HitResponse::Stop
    }
}
```

List it in `weapons/ordnance/mod.rs` (`pub mod nail_mortar;`).

**Hooks** (override only what the weapon needs; every default is the generic straight
projectile of M1):

| Hook | Runs when | Typical use |
| --- | --- | --- |
| `fire(ctx, shot)` | a paid shot (charge weapons: on release) | patterns, instant weapons (no projectile, effect queued directly), setting `proj_flags` |
| `charge_start` / `charge_hold` | the trigger of a `charge` weapon goes down / stays down | custom release rules, overcharge |
| `update(ctx, p)` | every tick before the projectile moves | homing, guidance (`ctx.input(p.owner)`), trails |
| `on_ship_hit` / `on_entity_hit` / `on_terrain_hit` | the swept move hits something; return a `HitResponse` (`Stop`, `Continue`, `Pierce { exit }`, `Bounce`, `Stick`) | damage, piercing, bouncing, sticking |
| `on_fuse(ctx, p)` / `on_expire(ctx, p)` | `fuse_tick` reached / lifetime over | mines, timed charges, splitting |
| `entity_tick` / `entity_expired` | every tick / at expiry for entities the weapon spawned | markers, dust, ghosts |
| `on_mode(ctx, ship, w)` | V (`Buttons::WEAPON_MODE`) goes down while `w` is the selected primary | weapon modes (Paint Thrower colours) |

**Commands.** Hooks read the world through `ctx` (`ship`, `ships_within`, `nearest_ship`,
`target_in_cone`, `raycast`, `thickness`, `surface_states_near`, `material_at`,
`projectiles()`, `trail`, `def`, `input`, `ctx.view` for the tables and the level's
`instability`) and change it only by queuing commands, applied right after the hook returns:
`damage` / `damage_proportional`, `splash`, `shockwave`, `detonate` (the default impact),
`crater` (scaled by rock strength) / `crater_exact`, `edit`, `grow`, `paint` / `paint_color`,
`scorch`, `channel`, `spawn_projectile` (with `weapons::projectile_for` / `sub_projectile`),
`spawn_entity` / `spawn_debris`, `impulse_ship` / `impulse_entity` / `impulse_projectile`,
`move_pickup`, `remove_projectile`, `drain_energy`, `blind`, `swap`, `rewind`,
`disable_weapons`, `adjust_hull`, `set_weapon_state`, `cue` (a `CueKind` for client effects),
`emit`. At most `behavior::MAX_COMMANDS_PER_HOOK` (256) per hook call, and every terrain edit
is networked and remeshed, so keep edit rates low (`weapons::debris_crowded` for debris).

**Rules:** deterministic code only ([determinism.md](architecture/determinism.md)): randomness
from `ctx.rng(key)` (a private stream per key and tick, usually keyed by the projectile id),
`ls_math::det` for transcendentals, no `HashMap` iteration. Keep state in the generic fields:
per projectile `charge`, `generation`, `bounces`, `flags`, `arm_tick`, `fuse_tick`, `target`,
`vars[4]`, `path`; per entity `Entity::vars` / `state`; per ship and weapon
`ShipState::weapon_state` (`ctx.set_weapon_state`). No shared struct changes for a new weapon.

### 8.3 Registry

**Append** an entry to `registry::MODULES` and raise the array length
(`[WeaponModule; 27]` to `28`):

```rust
    WeaponModule {
        id: "nail_mortar",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::nail_mortar::NailMortar,
        file: "weapons/nail_mortar.ron",
        data: include_str!("../../../../data/weapons/nail_mortar.ron"),
    },
```

Never reorder the list: rule set codes index it (codes from an older build still decode, with
the new weapon banned), and number keys, the wheel and X follow it. A data file with no
registry entry fails `data::tests::embedded_files_match_disk`.

### 8.4 Rule sets

Every weapon must appear in `data/rulesets/paint_the_void.ron` (the default lists all of
them; an unlisted weapon is `Banned`, and `every_weapon_fires_with_its_fallback` then reports
it "never fired") and in `data/rulesets/sandbox_all_weapons.ron` as `PickupOnly` with a
`pickups` entry (the arsenal e2e run and `tests/arsenal_match.rs` use it). Then regenerate the
browser rule set editor's catalog: `LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode`
(rewrites `web/src/rules/catalog.ts`; PowerShell: `$env:LS_BLESS_RULES_CATALOG=1`, and remove
it afterwards).

A new weapon changes every rule set code, the default included (for the Rivet Mortar the default
`LR6V-69CM-G006-58R` became `LR6W-6C74-G002-WTW`), because codes carry the weapon count and a
fingerprint of the default rules. Codes made by older builds still decode, with the new weapon
banned. Update the codes quoted in docs and examples (the catalog's `default` entry has the
new one).

### 8.5 Tests and hashes

Put scenario tests for the bonus, the malus and the lasting effect next to the module. The
helpers in `crate::weapons::kinetic::rivet_gun::testkit` build a world in a closed test room
(`world`, `room`, `room_sized`, `slab`, `duel`, `equip`, `place`, `run`, `fire1` / `fire2`)
and read the events (`damage_to`, `hits_on`, `craters`, `fired`, `impacts`, `debris`):

```rust
#[cfg(test)]
mod tests {
    use crate::weapons::kinetic::rivet_gun::testkit::*;
    use crate::*;
    use ls_math::Vec3;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Bonus: the shell bursts into nails on impact. Lasting effect: its crater.
    #[test]
    fn bursts_into_nails_and_craters() {
        let mut w = world();
        let mut t = room(); // a closed box, walls 30 u from the centre
        let (a, _) = duel(&mut w);
        equip(&mut w, a, "nail_mortar");
        place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
        let ev = run(&mut w, &mut t, &fire2(a), 30);
        assert_eq!(craters(&ev).len(), 1);
        assert_eq!(w.projectiles().iter().filter(|p| p.generation == 1).count(), 5);
    }
}
```

`cargo test -p ls-sim --lib` then names what else to update. For the Rivet Mortar it was:

| Failing test | Update |
| --- | --- |
| `weapons::tests::every_weapon_has_a_module_and_a_data_file`, `weapon::tests::builtin_weapons_match_design_doc` | the weapon count (27) |
| `data::tests::hash_is_stable_and_sensitive` | the embedded file count (`27 + 3 + 4 + 2`) |
| `ruleset::tests::default_rule_set_loads_and_resolves`, `sandbox_offers_every_weapon_gadget_and_ship_type_as_pickups` | the rule set entries (8.4) |
| `rulecode::tests::web_catalog_is_current` | bless the catalog (8.4) |
| `tests::replay_golden_hash` | copy the printed hash into `REPLAY_GOLDEN_HASH` ([determinism](architecture/determinism.md#re-blessing-after-an-intentional-change)) |
| `tests::replay_same_inputs_same_state_hash` | the scripted replay draws weapon selections from the table length, so its match plays out differently; if it no longer covers a kill, a respawn or a pickup, adjust `scripted_inputs` or the replay seed so it does, then re-bless |

Regenerate the player reference pages (`node tools/gen-weapon-docs.mjs`, [section
5](#generated-player-reference)); a weapon with new `params` also wants a line in the
generator's parameter notes if its table cell needs one, and a paragraph in the player manual.

The data hash changes with any data file, so rebuild everything that embeds data together
(`cargo build --release -p ls-server -p ls-bots`, `cd web && npm run build`): a stale client is
rejected at the handshake with `DataMismatch`, not desynced. Run the wasm32 unit tests too
([determinism.md](architecture/determinism.md#the-wasm-cross-check)).

### 8.6 Client and bots

- **Drawing:** projectiles and pickups use the weapon's `visual`; `SHAPE_STYLES` in
  `web/src/game/scene.ts` maps each `Shape` to a mesh. A new shape means a variant in the
  `shapes!` list in `crates/ls-sim/src/visual.rs` (append; the index is on the wire) and a
  style in `SHAPE_STYLES`. Weapon names, slots and the weapon HUD come from the data.
- **Shot prediction:** the browser draws its own shots at once with
  `crates/ls-wasm/src/shots.rs`. `ShotModel::for_weapon` maps weapon ids to a model; the
  default `Pattern` is the generic `fire_pattern`. A weapon whose `fire` differs (instant, fans
  that rotate per shot, random cones, dropped behind the ship) needs its own entry, or
  `ShotModel::None` to draw nothing and wait for the server; otherwise the client draws a
  stray bolt.
- **Effects:** a new `CueKind` (in `crates/ls-sim/src/behavior.rs`, replicated as
  `NetEvent::Cue`) gets its banner, flash or HUD reaction in `web/src/game/mechanics.ts` and
  `web/src/ui/effects.ts`; the charge meter works from the data's `charge` without client code.
- **Bots:** `crates/ls-bots/src/arsenal.rs` gives each weapon id a `Role` and primaries a
  distance band; an unknown id is `Role::Other` (fired like a laser or a rocket). Special
  mechanics (guidance, mines, charge) need brain code in `crates/ls-bots/src/brain.rs`.

Things to know:

- `WeaponId`s are indices in **id-sorted order** (not registry order), so a new id shifts the
  ids of every weapon sorted after it. Ids travel on the wire (projectile `kind`, ship flags,
  inputs); the handshake's data hash keeps builds from different data apart.
- Ship flags carry the selected primary in 5 bits: ids 0-31, so at most 32 weapons before the
  wire format changes. Rule set codes carry the weapon count in 6 bits (at most 63).
- The fastest weapon sets the projectile velocity quantization range (`quant_config_for`: at
  least 320 u/s).

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
   lines `match ready ... ruleset=Lasers Only` and `rules: Lasers Only LR4V-... (banned weapons:
   ...)` confirm it; an invalid file stops the server with the reason (for example `invalid
   rule set: friendly_fire_percent 5 must be 0 (off) or 10..=100`). `RuleSet::resolve` checks
   that every weapon and gadget exists, loadout items are allowed in the loadout and fit their
   slot, pickups are pickup-able, and the numbers are in range.
3. Play it with bots and the browser as usual; `tools/e2e.mjs --ruleset <file>` runs the e2e
   checks on it (use `--min-craters 0` if the rule set has no crater weapons).

The embedded rule sets are `data/rulesets/paint_the_void.ron` (`RuleSet::paint_the_void()`,
the default and the practice mode's rules) and `data/rulesets/sandbox_all_weapons.ron` (every
weapon and gadget as a pickup, every ship type). Test and tool rule sets live next to their
tools (`tools/e2e-rulesets/sandbox_debris.ron`, `tools/browser-smoke/mechanics.ron`). A new
**mode** is code: a `ModeRules` variant in `crates/ls-sim/src/ruleset.rs`, its logic next to
`crates/ls-sim/src/mode.rs`, and scoring and HUD support in `ls-game`, `ls-wasm` and `web/`.

### Rule set codes and the editor

A rule set travels as a short code: `RuleSet::to_code()` prints `LR` plus Crockford base32 in
groups of four (CRC-16 at the end), `RuleSet::from_code` reads it back exactly, and
`ls-server --ruleset <code>` accepts one wherever it takes a file (an existing file of that name
wins). Codes of version 3 store only the differences from the mode's default rules, so the
default Paint the Void rules are `LR6V-69CM-G006-58R` and a ban adds about three symbols; a
rule set far from the default falls back to the full version 2 encoding when that is shorter.
Values the format cannot carry exactly (tuning in twentieths, shield fractions in hundredths,
weights like 0.3) are rejected when encoding, so a code always decodes to the rule set it was
made from. Format: `crates/ls-sim/src/rulecode.rs`; details in
[arsenal-framework.md](architecture/arsenal-framework.md#rule-set-v2-and-codes). To get the code
of a rule set file, start the server with it: the `rules:` log line prints it.

The browser rule set editor (`web/src/rules/`) opens with `?editor=rules` (for example
<http://localhost:7777/?editor=rules>, or `npm run dev` and the printed dev URL plus `?editor=rules`);
the start screen links to it. It edits every setting, shows the problems the server would
report, and gives the `LR` code, an invite link (`?world=<share code>&rules=<LR code>`) and the
host's `ls-server --code ... --ruleset ...` command line. In M2 only the editor reads invite
codes (`?editor=rules&world=...&rules=...` opens it on them); the game joins the server's own
world and rules. It reimplements the encoder, decoder and validation in TypeScript
(`code.ts`, `validate.ts`, `model.ts`, Node-tested in `rules.test.ts`) against a catalog
generated from the data: after any change to weapon ids, names, slots, costs, levels, gadgets
or the default rule set, regenerate it with

```sh
LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode   # rewrites web/src/rules/catalog.ts
```

(`rulecode::tests::web_catalog_is_current` fails until you do). The browser smoke test
([section 12](#12-browser-smoke-test)) drives the editor and checks that `ls-server` accepts its
code.

### Balance lists

Hosts can ban or weaken weapons on every match without editing rule sets:
`ls-server --ban <weapon>` (repeatable) and `--balance-list <file>` (schema
`ls_sim::ruleset::BalanceList`):

```ron
BalanceList(
    bans: ["tectonic_charge"],
    gadget_bans: [],
    // Multiplies the rule set's multipliers for these weapons (rounded to twentieths).
    tuning: { "railspike": (damage: 0.8, fire_rate: 1.0, ammo: 1.0) },
    // Replaces the rule set's shield setting for these weapons.
    shield_overrides: { "heavy_rocket": Wears(1.5) },
)
```

Effective rules = rule set, then balance list, then `--ban`s, validated at startup. The list is
re-read before every match (if it no longer loads, the server warns and keeps the previous
rules), so a host can change it while the server runs. Example:
`cargo run -p ls-server --release -- --ruleset LR6V-69CM-G006-58R --ban seeker --balance-list balance.ron`
logs `rules: Paint the Void LR6V-... (banned weapons: seeker, tectonic_charge)`; the `Welcome`
carries these effective rules, so clients see the bans. Host-facing details:
[`userDocs/hosting-guide.md`](../userDocs/hosting-guide.md).

## 10. Adding a gadget

A gadget is the same three pieces as a weapon: `data/gadgets/<id>.ron` (`ls_sim::GadgetDef`),
a module `crates/ls-sim/src/gadgets/<id>.rs` (`GadgetBehavior`) and an entry in
`gadgets::MODULES` (`crates/ls-sim/src/gadgets/mod.rs`). Example, an instant shove:

```ron
// Booster: an instant shove along the aim. Press G.
// Behaviour: crates/ls-sim/src/gadgets/booster.rs. Schema: `ls_sim::GadgetDef`.
GadgetDef(
    id: "booster",
    name: "Booster",
    activation: Instant,          // Toggle (on/off, drains while on) | Instant (use, then cooldown)
    energy_cost: 20.0,            // paid on activation
    cooldown_secs: 8.0,
    visual: (shape: Gadget, color: (255, 200, 80), scale: 1.0),
    params: { "push": 25.0 },
)
```

Other fields: `drain_per_sec` and `moving_drain_mult` (toggle gadgets; the Cloak's 3x while
moving), `duration_secs` (the Decoy's lifetime).

```rust
use crate::behavior::Ctx;
use crate::gadgets::{GadgetBehavior, GadgetUse};

/// Behaviour of the Booster.
#[derive(Debug)]
pub struct Booster;

impl GadgetBehavior for Booster {
    fn id(&self) -> &'static str {
        "booster"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_activate(&self, ctx: &mut Ctx, g: &GadgetUse) {
        let push = g.def.param("push", 25.0);
        ctx.impulse_ship(g.ship.player, g.ship.orientation.forward() * push);
    }
}
```

The framework handles the G key (`Buttons::GADGET`, press edge): toggle gadgets switch on
(paying `energy_cost`) and off, drain while on and switch off at zero energy; instant gadgets
call `on_activate` and start the cooldown. Hooks: `on_activate` / `on_deactivate`, `tick`
(every tick while on), `on_damage` (lower an `IncomingHit` while on: the Shield),
`visible_to` (snapshot relevance: the Cloak), `entity_tick` / `entity_expired` (entities the
gadget spawned: the Decoy). The world sends gadget cues (`ls_sim::gadget_cue`: on, off by key,
out of energy, instant use) that the HUD uses for the cooldown line.

Checklist: `pub mod booster;` in `gadgets/mod.rs`; append to `gadgets::MODULES` and raise its
length (`[GadgetModule; 3]`; append only, rule set codes index it, at most 15 gadgets in the
code's 4-bit count); add the gadget to `data/rulesets/sandbox_all_weapons.ron` (`gadgets` and
`gadget_pickups`) and, if it should be playable by default, to `paint_the_void.ron`; bless the
web catalog; update the embedded file count in `data::tests::hash_is_stable_and_sensitive`;
re-bless `REPLAY_GOLDEN_HASH`; regenerate the player reference (`node tools/gen-weapon-docs.mjs`);
add scenario tests (`gadgets/test_util.rs` has the shared helpers). `GadgetId`s are indices in id-sorted order, like weapon ids. Client and bots: the HUD
gadget line, cooldown and pickups work from the data; gadget-specific visuals key on the id
(`gadgetKey` in `web/src/game/mechanics.ts`: the shield bubble, the cloak shimmer), and bots
use only the gadgets `GadgetKind::from_id` in `crates/ls-bots/src/brain.rs` knows (others:
never).

## 11. Ship types

The four types are data (`data/ships/{skitter,tinker,hauler,tug}.ron`, schema
`ls_sim::ShipTuning`, embedded by `tuning::BUILTIN_SHIPS`): speeds and accelerations, hull,
energy, boost and scrap, regeneration, `damage_mult`, `secondary_slots`, hitbox `radius`, turn
rates, scrap passing (`can_pass_scrap`, `scrap_pass_rate`: the Tug) and the paint effects
(`paint_bounce_restitution`, `paint_sticky_speed`). Retuning a type is a data change:
`tuning::tests::ship_types_match_design_doc` pins the design-doc ratios, the replay hash
changes, and `userDocs/reference/ships.md` must be regenerated (`node tools/gen-weapon-docs.mjs`).

The set of types is code, not data: `ShipType` (`crates/ls-sim/src/tuning.rs`) is an enum with a
stable index (Skitter 0, Tinker 1, Hauler 2, Tug 3) used on the wire (`Hello::ship_type`,
`Message::SelectShip`, `NetPlayer`) and as a 4-bit mask in rule set codes. A fifth type needs
a new rule set code version, `ShipType::ALL` and `index`, a `BUILTIN_SHIPS` entry, and the
client lists (`SHIP_TYPE_NAMES` in `web/src/game/views.ts`, the ship cards in
`web/src/ui/startScreen.ts` and `web/src/ui/shipSelect.ts`, `web/src/rules/types.ts`).

Choice flow: the start screen's pick goes into `Hello::ship_type` for the first life; in game B
opens the ship bay and 1-4 pick the type for the next respawn (`Message::SelectShip`,
`ClientSession::select_ship`, `World::set_ship_type`). The rule set's `allowed_ship_types` is
enforced for both (the room falls back to the Tinker or the first allowed type).

## 12. Browser smoke test

`tools/browser-smoke` drives a locally installed Chrome or Edge (puppeteer-core, headless, real
WebGPU) against real servers. It is the last step of `tools/ci.*` and checks what no Node test
can: WGSL compiles and all 3 render pipelines validate on a hardware adapter, frames are not
blank, and the game works end to end.

```sh
pwsh tools/browser-smoke.ps1                 # or: tools/browser-smoke.sh; builds ls-server + web first
pwsh tools/browser-smoke.ps1 -SkipBuild      # or: tools/browser-smoke.sh --skip-build
node tools/browser-smoke/smoke.mjs --server-bin target/<area>/release/ls-server --out-dir target/<area>/browser-smoke
node tools/browser-smoke/smoke.mjs --help
```

The wrappers take the server from `$CARGO_TARGET_DIR/release` and install puppeteer-core in
`tools/browser-smoke` on first use; extra arguments go to `smoke.mjs`. Phases (60 checks):

1. **Practice:** every WGSL module and render pipeline is created under an error scope; the
   level renders for `--practice-secs`.
2. **Online:** a server on a free port; the page flies with synthetic keys and mouse, fires the
   Scrap Laser, the Paint Thrower (cycling colours with V), the Bolt Rocket and the Shield, and
   needs server-confirmed shots.
3. **Mechanics:** a second server with `tools/browser-smoke/mechanics.ron` (a loadout of the
   Fusion Charger, Leech Arc, Paint Thrower, Eyeball and Decoy; no Hauler): the ship bay (B,
   the banned Hauler card, picking the Skitter as "(next)"), the charge meter, no predicted
   bolt for the Leech Arc, V switching the paint colour as confirmed by the server, the Eyeball
   view and the Decoy cooldown.
4. **Editor:** `?editor=rules`, a change, the new code, and another `ls-server --ruleset <code>`
   that must start with the same `rules_code`.

Every phase fails on console or page errors, WebGPU validation or uncaptured errors, a lost
device, error screens, and server errors or panics. Screenshots, the browser console and the
server logs go to `target/browser-smoke` (`--out-dir`). Exit codes: 0 passed or skipped (no
browser found; `--require-browser` makes that a failure), 1 failed, 2 usage error. A software
(fallback) WebGPU adapter fails unless `--allow-fallback`; on machines without a GPU set
`LS_SKIP_BROWSER_SMOKE=1` for CI. `--mode headful` shows the window, `--browser <path>` (or
`LS_BROWSER`) picks the browser.

To cover a new mechanic: add its loadout to `mechanics.ron` if it needs one, then a step in
`mechanics()` in `smoke.mjs` that presses the keys and asserts on the DOM (HUD elements have
stable classes such as `.hud-charge` or `.ship-card`) with `check(name, ok, detail)`.
All servers it starts are stopped at the end; it never uses port 7777.

## 13. Where things are documented

- Rules for contributors and agents: [`CLAUDE.md`](../CLAUDE.md) (keep it in sync when commands
  or layout change).
- Game rules: [design/game-design-doc.md](design/game-design-doc.md).
- Scope, module specs and status: [implementation-plan.md](implementation-plan.md) (M1) and
  [milestone-2-plan.md](milestone-2-plan.md) (M2: design decisions, waves, acceptance status).
- Architecture: [architecture/](architecture/) (see the [index](README.md)); the arsenal
  (weapons, gadgets, ship types, rule sets) in
  [architecture/arsenal-framework.md](architecture/arsenal-framework.md).
- Game data conventions: [`data/README.md`](../data/README.md).
- Player-facing manuals and hosting guides belong in [`userDocs/`](../userDocs/), not here.
