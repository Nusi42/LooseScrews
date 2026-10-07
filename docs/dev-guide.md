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
cd <work root>/loosescrews.win3/web && npm run build   # WASM package (wasm-bindgen 0.2.129) + client
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
  Each step's output is also kept in `target/pre-push/<n>-<step>.log` (overwritten by the next
  push); a failed step names its log, so a failure is readable even when the pushing terminal
  cut the output short.
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
| wasm-bindgen CLI | exactly **0.2.129** | `cargo install wasm-bindgen-cli --version 0.2.129 --locked`; installs `wasm-bindgen` and `wasm-bindgen-test-runner`. The crates are pinned to match (`wasm-bindgen =0.2.129`, `js-sys =0.3.106`, `wasm-bindgen-test =0.3.79` in the root `Cargo.toml`); bump all four together |
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
cargo run -p ls-server --release -- --code LS0G-0000-0000-0000-1001-6RFX-250V
```

Open <http://localhost:7777>. The server serves `web/dist` (default `--web-root` when that
directory exists), the WebSocket at `/ws`, and WebTransport on UDP 7777 with the certificate
hash at `/cert-hash`. Useful flags (`ls-server --help` lists all):

| Flag | Default | Meaning |
| --- | --- | --- |
| `--bind` | `0.0.0.0:7777` | HTTP + WebSocket address |
| `--code` | random world | Share code of the level |
| `--ruleset` | default rotation | Rule set file (e.g. `data/rulesets/sandbox_all_weapons.ron`), built-in id (`sandbox_all_weapons`) or an `LR...` rule set code, played every match |
| `--playlist <file>` | default rotation | Match rotation (rule set, world, player range per entry; [server.md](architecture/server.md#rooms-lobby-and-rotation-roomsrs-lobbyrs-playlistrs-m3)) |
| `--warmup` / `--countdown` | 30 / 10 | Lobby: longest warm-up (weapons off) and countdown in seconds; `--warmup 0 --countdown 0` turns the lobby off |
| `--private-rooms` | 4 | Most private rooms (duels, parties) next to the public room |
| `--ban <weapon>` | none | Bans a weapon on top of the rule set (repeatable) |
| `--balance-list <file>` | none | Server-wide bans and tuning on top of the rule set, re-read before every match ([section 9](#balance-lists)) |
| `--web-root` | `web/dist` if it exists | Built web client to serve |
| `--max-players` | 8 | 1..=32 |
| `--wt-bind` / `--no-webtransport` | same port as `--bind` over UDP | WebTransport endpoint |
| `--stats-interval` | 10 | Seconds between `tick stats` log lines |
| `--intermission` | 10 | Seconds the result shows before everyone moves to the next match (same connection) |
| `--exit-when-empty` | off | Exit 2 s after the last player leaves (scripts) |

Logging follows `RUST_LOG` (default `info`), e.g. `RUST_LOG=ls_server=debug`.

At match start the server logs `rules: <name> <LR code> (banned weapons: ...)`, the
effective rules after the balance list and bans.

In the browser, `?transport=ws` or `?transport=wt` forces a transport; F3 toggles the debug
overlay, Tab shows the scoreboard, B opens the ship bay (1-4 pick the type for the next
life), V switches the selected primary's weapon mode (Paint Thrower colour), G uses the
gadget. The start screen also offers offline practice on any share code (no server needed),
links the rule set editor (`?editor=rules`, [section 9](#rule-set-codes-and-the-editor)) and
opens the world workshop ("Make a world", `web/src/ui/workshop.ts`; `?workshop` or
`?workshop=<code>` opens it at once): seven sliders, a seed and the level archetype with a
live preview (the start backdrop regenerated on the new code), Save (copy the code), Fly it
(practice) and Host it (an invite link with `code=`, Belt Up proposed on belt worlds). Codes
are made and read by ls-wasm (`makeShareCode`, `checkShareCode`, `levelArchetypes`), never in
TypeScript; `SliderPanel` is the slider block The Architect's UI reuses.

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
cargo run -p ls-bots --release -- --directory http://127.0.0.1:7780 --count 4 --accept-duels   # server from a directory; accept duels
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

A local server list with two servers (then open http://127.0.0.1:7777/?servers):

```sh
cargo run -p ls-server --release -- --bind 127.0.0.1:7777 --directory http://127.0.0.1:7780 --server-name One
cargo run -p ls-server --release -- --bind 127.0.0.1:7778 --directory http://127.0.0.1:7780 --server-name Two
```

Invites, parties and duels go through the game server's JSON API (`/api/...`,
[architecture/server.md](architecture/server.md#server-list-invites-parties-and-duels-apirs-partyrs-duelrs-directoryrs-m3-b7));
`cargo test -p ls-server --test social` runs them against real servers and a real directory
(two listed servers, a party invite, a local and a relayed duel).

## 4. Tests

| What | Command | Notes |
| --- | --- | --- |
| All Rust tests | `cargo test --workspace` | Debug; `ls-game` alone takes about 40 s (level generation) |
| Bot end-to-end tests | `cargo test -p ls-bots --release` | Ignored in debug builds (too slow); part of CI. Includes `tests/nav_sweep.rs` (10 worlds plus a 5-minute soak-world match) `tests/nav_cache.rs` (8 bots of one world start one grid build; navigation CPU per bot at least 5x lower than a build per bot) and `tests/arsenal_match.rs` (8 bots, 240 s of `sandbox_all_weapons`: at least 15 distinct combat weapons incl. a mine and the Tectonic Charge, 0 desyncs, reconciliation error under 1 u), `tests/mode_matches.rs` (every mode's built-in rule set played to a result by 8 bots playing its objective, about 60 s), `tests/race_match.rs` (Belt Up: 8 racing bots, 3 heats, every bot finishes 2 of 3; the time trials on 8 belt seeds that gate the radiation tuning; a few seconds), `tests/mood_matches.rs` (all 18 comedy events forced over an Absurdity 100 rotation with 8 bots, reconciliation near wells and ducks at most 0.1 u, about 90 s; The Architect with 6 bots) and `tests/social.rs` (lobby Ready, `--directory` join, duel acceptance against a real server); one file: `cargo test -p ls-bots --release --test arsenal_match -- --nocapture` |
| Weapon and gadget scenario tests | `cargo test -p ls-sim --lib weapons` / `cargo test -p ls-sim --lib gadgets` | One test module per weapon and gadget module (bonus, malus, lasting effect), plus the registry checks in `weapons::tests`; world-level framework tests in `tests_arsenal` |
| Mode scenario tests | `cargo test -p ls-sim --lib modes` | Framework tests in `modes::tests`; one test module per implemented mode (Hot Reactor: `modes::hot_reactor::tests`, incl. a 60 s replay golden hash checked on wasm32 too). Headless 8-bot match: `cargo test -p ls-game --test hot_reactor_match` (short-fuse variant; the full default rule set match runs only with `--release`) |
| Rule set codes and the web catalog | `cargo test -p ls-sim --lib rulecode` | Includes `web_catalog_is_current`; regenerate with `LS_BLESS_RULES_CATALOG=1` ([section 9](#rule-set-codes-and-the-editor)) |
| Snapshot bandwidth | `cargo test -p ls-net --lib busy_fight_with_projectiles_fits_the_budget -- --nocapture` | Prints mean and worst delta size per seed and baseline age ([netcode](architecture/netcode.md#snapshots-and-deltas)); `cargo test -p ls-game --lib busy_fight_fits_the_bandwidth_budget` is the ship-physics variant |
| Match setup sweep | `cargo test -p ls-game --release --lib setup` | 8 seeds x 25 slider mixes (1 seed in debug) |
| ls-game in release | `cargo test -p ls-game --release` | Part of CI: the setup sweep, the Hot Reactor 8-bot match with the default rule set, The Squeeze's 8-seed shell remesh benchmark (`tests/squeeze.rs`, about 5 minutes), the match-start regression (`tests/match_start.rs`: no tick of a fresh `build_match` world or `Room` over 50 ms, every built-in rule set) |
| Visitor event replay hashes (M4 B2) | `cargo test -p ls-sim --lib visitor_replays` | Tax drone, Gossip wall, Pizza delivery, Customer survey and Mimic pickups forced in a 23 s replay each (`ls_sim::events::visitor_replays`) against their `event/` lines of `crates/ls-sim/tests/fixtures/mood_replays.txt`, native and wasm32. Re-bless: `LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib visitor_replays`. Netplay and generated-level drone tests: `cargo test -p ls-game --release --test visitor_netplay` |
| World physics replay hashes (M4) | `cargo test -p ls-sim --lib world_replays` | Gravity, cave-in, pigment and radiation replays (`ls_sim::world_replays`) against the `world/` lines of `crates/ls-sim/tests/fixtures/mood_replays.txt`, native and wasm32. Re-bless: `LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib world_replays` |
| Comedy event replay hashes (M4) | `cargo test -p ls-sim --lib event_replays` | One forced replay per spectacle event (`ls_sim::event_replays`, B3) and per other built event without its own replay (`ls_sim::events::replays`, B1) against their `event/` lines of the same fixture, native and wasm32. Re-bless: `LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib event_replays` (both harnesses' tests match the filter) |
| Event netplay and remesh (M4 B3) | `cargo test -p ls-game --test spectacle_netplay`; `cargo test -p ls-game --release --test cat_paw_remesh` | Duck parade, Hat rain, Cat paw, Disco inferno and Tired announcer with 4 in-process clients (no desync, server terrain and hats on every client, duck pushes predicted within 0.1 u); Cat paw grooves against the client remesh budget on 8 levels |
| Per-mode replay hashes | `cargo test -p ls-sim --lib golden` | One 40 s scripted 8-ship replay per built-in rule set (`ls_sim::golden`) against `crates/ls-sim/tests/fixtures/mode_replays.txt`, native and wasm32; `node tools/determinism-check.mjs` compares the WASM build too. Re-bless: `LS_BLESS_MODE_REPLAYS=1 cargo test -p ls-sim --lib golden` ([determinism](architecture/determinism.md#per-mode-replay-hashes)) |
| One crate / one test | `cargo test -p ls-sim replay_golden_hash` | Standard cargo filters |
| Worldgen golden hashes | `cargo test -p ls-worldgen --release --test golden` | See [determinism](architecture/determinism.md) |
| Unit tests on wasm32 | see [determinism.md](architecture/determinism.md#the-wasm-cross-check) | Needs `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`; do not pass `-q` |
| WASM vs native hashes | `node tools/determinism-check.mjs` | Worldgen golden hashes, the per-mode replay hashes and the mood replays (`moodReplayHashes()` against `crates/ls-sim/tests/fixtures/mood_replays.txt`: world physics, every event, The Architect, Belt Up); needs the package from `cd web && npm run build:wasm` (or `--pkg <dir>`) |
| Lints and format | `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all` | CI runs `cargo fmt --all -- --check` |
| Generated player reference | `node tools/gen-weapon-docs.mjs --check` | Fails when `userDocs/reference/*.md` differ from `data/`; regenerate without `--check` ([section 5](#generated-player-reference)) |
| Web | `cd web && npm run typecheck && npm test` | `src/**/*.test.ts` via `node --test`; Node strips types, so use `.ts` import extensions and erasable-only syntax (no enums, no parameter properties) |
| Browser smoke test | `pwsh tools/browser-smoke.ps1` or `tools/browser-smoke.sh` (`-SkipBuild` / `--skip-build`) | Headless Chrome or Edge with WebGPU (puppeteer-core in `tools/browser-smoke`): practice render, online flight firing weapons, the wave C mechanics against a second server with `tools/browser-smoke/mechanics.ron` (ship bay, charge meter, Leech Arc, V paint colour, Eyeball view, Decoy cooldown), lobby and match transitions, the server list, invite links, a party and a duel ([section 12](#12-browser-smoke-test)), the rule set editor and a server accepting its code. Screenshots and logs in `target/browser-smoke`; `node tools/browser-smoke/smoke.mjs --help`. Skips (exit 0) without Chrome or Edge |

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
checks a loose bound); `cargo test -p ls-bots --release`; `cargo test -p ls-game --release`
(the match-setup sweep, the Hot Reactor match, The Squeeze's remesh benchmark); the wasm32 unit tests (skipped with a message if
`wasm-bindgen-test-runner` or `node` is missing); `cargo build -p ls-wasm` for wasm32; in
`web/`: `npm ci`, `typecheck`, `test`, `build`; `tools/determinism-check.mjs` (worldgen and
per-mode replay hashes); finally the browser smoke test (it also builds `ls-directory`) (skipped without Chrome or Edge, or with `LS_SKIP_BROWSER_SMOKE=1`; a
browser without a WebGPU adapter fails it, so set the variable on such machines). Stop any
`npm run dev` (Vite) first: on Windows it locks `web/node_modules` and `npm ci` fails. Run it
before handing in a change. It honours `CARGO_TARGET_DIR`. Note that it rebuilds
`web/src/wasm/pkg` and `web/dist`.

### Generated player reference

`userDocs/reference/{weapons,ships,gadgets,modes}.md` are generated from `data/weapons`,
`data/ships`, `data/gadgets` and `data/rulesets` (plus the mode list of the generated
`web/src/rules/catalog.ts` for the team counts each mode allows) by `tools/gen-weapon-docs.mjs`
(its own RON reader, no build needed). Never edit them by hand:

```sh
node tools/gen-weapon-docs.mjs           # rewrite the four pages after a data change
node tools/gen-weapon-docs.mjs --check   # CI: exit 1 if a page is missing or stale
```

Descriptions a table cell needs (the notes under each weapon family, parameter units) live in
the generator, and so do the mode texts of `modes.md` (`MODES`, and `MODE_FIELDS`: one meaning
per mode setting; `tools/test/gen-weapon-docs.test.mjs` fails when a new `ModeRules` field has
none); what a weapon or a mode does in prose belongs in the
[player manual](../userDocs/player-manual.md#weapons), which links to the tables. The
pre-commit and post-edit checks run `--check` after edits in those `data/` folders, the
catalog or the generator.

## 6. End-to-end run

```sh
pwsh tools/e2e.ps1                 # or: tools/e2e.sh
pwsh tools/e2e.ps1 -SkipBuild      # or: tools/e2e.sh --skip-build
pwsh tools/e2e.ps1 --count 8 --duration 330        # a full 5-minute match plus the restart
pwsh tools/e2e.ps1 --transport wt                   # bots over WebTransport
pwsh tools/e2e.ps1 --ruleset data/rulesets/sandbox_all_weapons.ron --count 8 --duration 330   # every weapon as a pickup
pwsh tools/e2e.ps1 --ruleset tools/e2e-rulesets/sandbox_debris.ron --count 8 --duration 330 --min-debris 64
pwsh tools/e2e.ps1 --count 8 --playlist quick --directory local   # every mode in rotation, ~11 min (M3)
pwsh tools/e2e.ps1 --count 8 --playlist default --duration 3600 --private-rooms duel:2,team_brawl:4   # the M3 soak
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

**Comedy events (M4 C6).** `--event-schedule <file|spec>` passes ls-server's testing flag of
the same name: comedy events forced in every match room, `<seconds> <event id|next>` items (one
per line in a file, or comma-separated; `next` walks every event in turn across matches). The
server logs `event schedule: events forced in this match` per match room and a `comedy event`
line per phase change (Announce, Active, Ending, Over); the e2e checks that every public match
room got its events, that the first forced event of every match was announced and that announced
events went active, and lists the events seen. The quick playlist uses `5 next, 6 next` unless
`--event-schedule off`.

**Rotation (M3).** `--playlist quick` writes a playlist of every mode (Team Brawl, The
Squeeze, Paint the Void, Hot Reactor, Heist, Last Color Standing) plus Instagib Brawl (M4:
then Team Brawl on a world at Absurdity 100 for 110 s, The Architect with 15 s rounds for 2+
players and Belt Up with 20 s heats, left out above 8 players) into
`<log-dir>/playlist/`, each built-in rule set shortened to `--match-secs` (45; The Squeeze's
phases to 3 s + 5 x (3 s warning + 12 s close), Last Color Standing's stalemate to half the match), on random
worlds, and runs the server with the lobby on (`--warmup 10 --countdown 2 --intermission 3`).
`--playlist default` plays the server's built-in rotation at full length, `--playlist <file>`
any playlist file. The bot run time defaults to every entry once (lobby and build time
included). Per entry it checks that a match reached its result (printed: mode, winner, time
left) and tick p99 per entry; for the transitions that every bot joined exactly once (no
reconnect), no disconnects, the bots' in-place transitions equal the players the server moved
into each next room (warm-up and match rooms), nobody was refused by a next room, and at most
one full snapshot per join or transition. `--directory local` starts an `ls-directory` on a
free port (`--directory <url>` uses a running one) and registers the server: the list must
show it with its name, players, protocol and data hash (as `/api/info`), its mode and world
must follow the rotation, and the entry must go when the server shuts down. With
`--bots-directory auto` the bots find the server through the directory when `ls-bots` has a
`--directory` flag (C2), else they join by address.

`--private-rooms duel:2,team_brawl:4` opens private rooms next to the public one
(`POST /api/rooms` with a built-in rule set id or `LR` code) and runs a bots process per room
(`ls-bots --room R-...`, log `bots-<room>.log`); each room must keep its bots on one connection
without protocol errors or desyncs, at most one full snapshot per join or transition, reach a
match result and stay under the tick budget. With `--playlist` the run also sends Welcome
probes (`--welcome-probes on|off`): right after every match end and warm-up swap of the public
room, while the next room is being built, a one-bot `ls-bots` joins for a second; its `joined`
line carries `welcome_ms` (Hello sent to Welcome received), which must stay within
`--welcome-budget-ms` (200). The server then gets one player slot more than `--count`.
Bots log `slow bot step` (one packet or loop turn of 50 ms or more, with the bot's mode) to
tell a stalled bot from a slow server when the server reports input starvation; the server's
`tick stats` line counts `full_snapshots` per window.

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
  cargo test -p ls-worldgen --release --test mesh_quality -- --nocapture # drawn rock vs collision surface, normals, occlusion, mine-corner error
  cargo test -p ls-worldgen --release --test thin_tunnels -- --nocapture # thin bores, thin rock blades, bore-mouth folds
  cargo run -p ls-worldgen --release --example mesh_timing [PASSES]   # per-chunk meshing time over five whole levels (p50/p95/max)
  cargo test -p ls-worldgen --release --lib bake_timing -- --ignored --nocapture # 35 u crater bake vs the per-corner bake
  ```

- **Browser:** F3 shows FPS, ping, transport, prediction corrections and snaps, shot prediction
  counters, chunk counts and skipped sends. Chunk meshing is budgeted per frame (4 ms / 8 chunks
  in play, `web/src/game/chunks.ts`; a chunk starts only if the running average chunk time
  still fits, so only a single chunk can overrun it). Use the browser's Performance panel for frame breakdowns;
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
| `weapons::tests::every_weapon_has_a_module_and_a_data_file`, `weapon::tests::builtin_weapons_match_design_doc` | the weapon count (28 since the Paperweight) |
| `data::tests::hash_is_stable_and_sensitive` | nothing (it counts `weapons::MODULES`) |
| `ruleset::tests::default_rule_set_loads_and_resolves`, `sandbox_offers_every_weapon_gadget_and_ship_type_as_pickups` | the rule set entries (8.4) |
| `rulecode::tests::web_catalog_is_current` | bless the catalog (8.4) |
| `tests::replay_golden_hash` | copy the printed hash into `REPLAY_GOLDEN_HASH` ([determinism](architecture/determinism.md#re-blessing-after-an-intentional-change)) |
| `tests::replay_same_inputs_same_state_hash` | nothing: scripted inputs select weapons by the M3 ids (`golden::scripted_select`), so the match plays out as before; but every `WeaponId` sorted after the new id shifts, so every replay hash (`REPLAY_GOLDEN_HASH`, the module goldens, `mode_replays.txt`, the `world/` lines of `mood_replays.txt`) changes and is re-blessed |

Regenerate the player reference pages (`node tools/gen-weapon-docs.mjs`, [section
5](#generated-player-reference)); a weapon with new `params` also wants a line in the
generator's parameter notes if its table cell needs one, and a paragraph in the player manual.

The data hash changes with any data file, so rebuild everything that embeds data together
(`cargo build --release -p ls-server -p ls-bots`, `cd web && npm run build`): a stale client is
rejected at the handshake with `DataMismatch`, not desynced. Run the wasm32 unit tests too
([determinism.md](architecture/determinism.md#the-wasm-cross-check)).

### 8.6 Client and bots

- **Drawing:** projectiles and pickups use the weapon's `visual`; `SHAPE_STYLES` in
  `web/src/game/scene.ts` maps each `Shape` to a mesh, and `VFX_SHAPES` in
  `web/src/game/vfx.ts` to its particle effects (glowing core instead of the mesh, trail,
  muzzle flash, impact or explosion radius at visual scale 1). A new shape means a variant in
  the `shapes!` list in `crates/ls-sim/src/visual.rs` (append; the index is on the wire), a
  style in `SHAPE_STYLES` and an entry in `VFX_SHAPES` (without one it looks like an energy
  bolt). A new weapon with an existing shape needs no client code: the visual colour and scale
  from its data tint and size the effects. Weapon names, slots and the weapon HUD come from the
  data.
- **Models:** hulls and props are built in TypeScript with the modelling kit in
  `web/src/renderer/models/kit.ts` (loft, lathe, bevelled box, sphere, mirror, smoothing
  groups); every triangle carries a material part (`PART_*`: hull, livery, trim, metal,
  canopy, engine, nav lights, glow, scorch; `PART_PLAIN` keeps the instance colour) that the
  object shader (`models/shading.ts`) turns into paint, glass, glow and blinking lights. Hulls
  live in `models/ships.ts` (3-4 closed pieces each, which `models/wrecks.ts` throws apart for
  the wreck), props in `models/props.ts`, mesh kinds in `models/index.ts`. A new mesh kind is
  one entry in `MESH_BUILDERS` and `LEGACY_MESH_BUILDERS`. `models/models.test.ts` checks the
  budgets (600-2000 triangles per hull, inside the hitbox sphere x 1.05), winding and parts.
  Look at them with `?view=ships` (ship bay; `&show=wrecks|props`, `&scene=flight`,
  `&focus=0..3`, `&boost=1`, `&models=legacy` for the old primitives) and take before/after
  shots plus the object-cost bench with `node tools/browser-smoke/models.mjs` (after
  `cd web && npm run build`; images in `target/visual-gallery/vq4`).
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
       absurdity: 0,                             // 0..=100: cap of the comedy events (0 = none)
       // events: { "duck_parade": Banned },      // comedy events this rule set never rolls
       // pigment_pickup: Some((respawn_secs: 30, players_per_pickup: 4, min_on_map: 2)),
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
**mode** is code: a `ModeRules` variant with its settings (`mode_settings!` in
`crates/ls-sim/src/ruleset.rs`; the field list is part of the rule set code format, append
only), a `ModeKind`, and a module `crates/ls-sim/src/modes/<id>.rs` implementing `ModeModule`,
registered in `modes/mod.rs` (see
[arsenal-framework.md](architecture/arsenal-framework.md#mode-framework-m3)); its default rule
set is `data/rulesets/<file>.ron` (`ModeKind::ruleset_file`, listed in `BUILTIN_RULESETS`).
A mutator is a module in `crates/ls-sim/src/mutators/`. Then regenerate the editor catalog
(`LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode`).

### Rule set codes and the editor

A rule set travels as a short code: `RuleSet::to_code()` prints `LR` plus Crockford base32 in
groups of four (CRC-16 at the end), `RuleSet::from_code` reads it back exactly, and
`ls-server --ruleset <code>` accepts one wherever it takes a file (an existing file of that name
wins). Codes (format 5, every mode, comedy event bans and pigment pickups) store only the
differences from the mode's default rules (`RuleSet::default_for`), so the default Paint the
Void rules are `LRAW-740M-8S00-01JD-4` and a ban adds about three symbols (an event ban one or
two); a rule set far from the default falls back to the full encoding when that is shorter.
Codes of the M3 format 4 (`LR8...`) and the M2 formats 2 and 3 (Paint the Void, `LR6...`) still
decode, against the defaults of their time (Absurdity 20, no pigment, no event bans).
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
`cargo run -p ls-server --release -- --ruleset LRAW-740M-8S00-01JD-4 --ban seeker --balance-list balance.ron`
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

### 11.1 Comedy events (M4)

The comedy director (`crates/ls-sim/src/director/`, numbers in `data/events/director.ron`) is
the last stage of `World::step`: it runs at the effective Absurdity
`min(MatchSetup::absurdity, RuleSet::absurdity)` (0 = off, nothing hashed), drifts the mood,
rolls every 20 s, announces an event for 3 s, runs it and keeps the gaps. Each of the 18
events is a module `crates/ls-sim/src/events/<id>.rs` (`EventModule`) with a data file
`data/events/<id>.ron` (`EventDef`: name, `enabled`, `weight`, `active_secs`, `ending_secs`,
`moods`, `targeted`, `params`), registered in `events::MODULES` in `EventKind` order (append
only). Wave A ships announce-only stubs; to build one, replace its stub body:

- lifecycle hooks `eligible`, `on_announce` (target, `params` bytes), `on_start`, `on_tick`,
  `on_end` get an `EventCtx` (the hook `Ctx` for damage, entities, edits and cues, the
  `EventState` with scratch values and spawned entity ids, `EventCtx::rng`);
- effect hooks (`move_effects`, `hit_radius_scale`, `projectile_flags`, `crater_scale`,
  `scavenge_scale`, `well_scale`, `reveal`, `allows_pickup`) are pure functions of the
  replicated `MoodView`, the match seed and the tick, so the client predictor computes the
  same (`director::event_effects`, `director::move_effects`, `apply_move_effects_before` /
  `_after`); the world already applies them. `level_up` turns the level's up (Bored level),
  `hit_plane` makes projectile hits ignore depth (Wrong genre), `MoveEffects::facing` turns or
  constrains the orientation after moving (`Facing::Toward`, `Facing::InPlane`). Values an
  event picks at its announce go into the 8 `params` bytes (`events::params` packs directions
  and positions) so the client decodes the same; a client predicting past its snapshot uses
  `MoodView::advanced(now)` (phase changes are deterministic);
- tests: `World::schedule_event(tick, kind)` forces an event whatever the Absurdity, bans,
  vetoes and eligibility; `director/tests.rs` has the cadence and statistics tests
  (`cargo test --release -p ls-sim --lib event_statistics -- --nocapture` prints the mean
  events per match at Absurdity 0, 20, 50 and 100). A built event sets `implemented()`, gets
  scenario tests (B1's: `events/world_events_tests.rs`) and a replay golden: `events::replays`
  plays every built event forced in an 8-ship scripted match; its hash is an `event/<id>` line
  in `crates/ls-sim/tests/fixtures/mood_replays.txt` (re-bless:
  `LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib events::replays`), checked on native and
  wasm32. `crates/ls-game/tests/world_events_netplay.rs` runs B1's events with 4 clients.

Rule sets ban events by id (`events: { "duck_parade": Banned }`, rule set code format 5) and
cap the Absurdity; modes and mutators veto with `allows_event`, and `ModeModule::leader` picks
the target of leader events. Warm-up rooms run at Absurdity 0.

## 12. Browser smoke test

`tools/browser-smoke` drives a locally installed Chrome or Edge (puppeteer-core, headless, real
WebGPU) against real servers. It is the last step of `tools/ci.*` and checks what no Node test
can: WGSL compiles and every render pipeline (the scene's terrain, instanced and translucent
ones and the post chain's bloom down, bloom up and composite) validates on a hardware adapter, frames are not
blank, and the game works end to end.

```sh
pwsh tools/browser-smoke.ps1                 # or: tools/browser-smoke.sh; builds ls-server + web first
pwsh tools/browser-smoke.ps1 -SkipBuild      # or: tools/browser-smoke.sh --skip-build
node tools/browser-smoke/smoke.mjs --server-bin target/<area>/release/ls-server --out-dir target/<area>/browser-smoke
node tools/browser-smoke/smoke.mjs --help
```

The wrappers take the server from `$CARGO_TARGET_DIR/release` and install puppeteer-core in
`tools/browser-smoke` on first use; extra arguments go to `smoke.mjs`. Phases (about 160 checks):

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
4. **Lobby:** a third server with the lobby on (`tools/browser-smoke/lobby.ron`): warm-up card,
   ready, countdown, match, results, the next match's warm-up and match on one page and one
   connection.
5. **Social** (`social.mjs`): a local `ls-directory` and two servers (A: Paint the Void, B:
   Team Brawl), players in separate browser contexts (own identities): a party invite link from
   the "Party & duels" panel puts a friend in the leader's room and team; an invite link with a
   world and a rule set code opens a private custom match with that world; B's server list
   (`?servers`) shows both servers with mode, players, region and ping and the leader in A's
   roster; a duel challenged from that list is relayed through the directory, accepted in the
   leader's toast, and both play in a private two-player room on B with the challenger's seed,
   without the challenge being offered again or the pages reloading. Needs `ls-directory` next
   to the server binary (`--directory-bin`).
6. **Editor:** `?editor=rules`, a change, the new code, and another `ls-server --ruleset <code>`
   that must start with the same `rules_code`.
7. **Audio** (M4 A6): the practice page and `?audio=demo` create no AudioContext before a user
   gesture (`audio-probe.mjs` records each one with `navigator.userActivation`); the sound check
   page lists every registered sound, starts on a click, keeps at most 32 voices under a
   64-sound burst and speaks an announcer line.
8. **Workshop** (`workshop.mjs`, M4 A8): "Make a world" on the start screen, the preview flies
   the workshop's world (`body[data-preview-code]`), a new seed and two sliders change the code
   and the preview regenerates on it, Save copies a code that decodes to that seed and those
   sliders (ls-wasm under Node from `web/src/wasm/pkg`, and the workshop reopened on
   `?workshop=<code>`), Fly it starts practice on that world (the F3 overlay shows the code).
   Once more as an asteroid belt when the build generates belts (generator version 4); without
   one the belt pass is noted and skipped.
9. **Moods** (`moods.mjs`, M4 C6): practice on a cave world with wells at Absurdity 0, the
   first event from `?event=<id>@4`, the rest through `window.__lsEvents.schedule`; each of the
   18 events gets a three-frame strip: `event-<id>-1-tell.png` (1.5 s into the warning),
   `-2-peak.png` (a moment of the active phase per event, reached with
   `window.__lsEvents.warp`, then 0.9 s of live frames, firing for the shot-based events) and
   `-3-after.png` (after its end), with `moods-strip.json`. Two practice pages, so the events fit
   Paint the Void's 5-minute practice match. Checks each event announces, runs and ends.
10. **The Architect** (`architect.mjs`, M4 C4): a server with `data/rulesets/the_architect.ron`
    and two pages (each brought to the front while it is checked: only the front page gets
    animation frames); the first pilot becomes the architect (`.arch-hud[data-role]`) and starts
    in the map view, the second a runner; key 4 and a click place a Gravity+ stroke, which the
    runner's page shows telegraphed and then acting (`data-telegraphs`, `data-acting`), the
    architect's `data-budget` drops by its cost, V switches to free flight.
11. **Race** (`race.mjs`, M4 C6): a server with the lobby on and Belt Up in 2 heats of at most
   40 s on a belt world; two pages ready up and race, flown by a script autopilot (mouse look
   toward the next gate over the fake pointer lock; one in the belt plane, one on the open line
   above it): start lights hold the racers, gates passed, heat results with places, the next
   heat, the match results screen.

`--only <phase,...>` runs a subset (`practice, online, mechanics, lobby, fit, social, editor,
audio, workshop, moods, architect, race`). Scripts in the page can use two debug handles:
`window.__lsEvents` in practice (`ids`, `schedule(id, delaySecs)`, `warp(secs)`, `mood()`;
`web/src/game/eventParam.ts`) and `window.__lsGame` in every session (`playerId()`,
`modeState()`, `pose()`, `belt()`, `architectStroke(kind, x, y, z, radius)`;
`web/src/game/debugHooks.ts`).

Every phase fails on console or page errors, WebGPU validation or uncaptured errors, a lost
device, error screens, and server errors or panics. Screenshots, the browser console and the
server logs go to `target/browser-smoke` (`--out-dir`). Exit codes: 0 passed or skipped (no
browser found; `--require-browser` makes that a failure), 1 failed, 2 usage error. A software
(fallback) WebGPU adapter fails unless `--allow-fallback`; on machines without a GPU set
`LS_SKIP_BROWSER_SMOKE=1` for CI. The browser always runs headless (there is no automatic
headful fallback) and the page gets a fake Pointer Lock (`fake-pointer-lock.mjs`), so a run
never pops up a window or captures the desktop mouse. `--mode headful` shows the window (only
for a person at the machine), `--browser <path>` (or `LS_BROWSER`) picks the browser.

To cover a new mechanic: add its loadout to `mechanics.ron` if it needs one, then a step in
`mechanics()` in `smoke.mjs` that presses the keys and asserts on the DOM (HUD elements have
stable classes such as `.hud-charge` or `.ship-card`) with `check(name, ok, detail)`.
All servers it starts are stopped at the end; it never uses port 7777.

### Visual gallery and GPU budget

`tools/browser-smoke/gallery.mjs` (visual-quality-plan.md VQ8) renders the fixed scenes of
`web/src/gallery/scenes.ts` (cave close-up, cave tunnel, open sky, open sky toward the sun with
light shafts, open space with the planet, caverns opening to the sky, mine shaft, a rock
material border, paint edges, surface states, the four hulls, a fight frame with an explosion,
`light-volley` and `light-storm`: a volley of glowing shots down a mine shaft and 64 dynamic
lights in a cave tunnel, VQ6; a ship blowing up and the 20 k particle budget scene, VQ5; VQ1: a
Tinker and a Hauler in front of a carved 1.2 u tunnel with the cockpit frame and fit ring, and
the Skitter, Tug and Hauler cockpits and the Tinker's under the open sky; VQP: pillars fading
before the hidden ceiling (`?roomfade=off` cuts them sharply), the Fusion Charger charge-up
from outside and from the cockpit, cloaked ships (`?cloak=flat` for the old translucent
hull); M3 C1's mode scenes `squeeze-warn`, `squeeze-close` (the shell
put on the practice terrain with `Game.practiceShell`), `heist-carrier`, `heist-dig`, `reactor`,
`last-color` and `teams4`, built by `web/src/gallery/modeScenes.ts`; M4 A7's Palette steps
`palette-{0,40,65,85,100}` on caves (with `-mine` and `-sky` variants), `palette-100-paint`,
`palette-100-pigment` and `palette-85-fight`, where `--query palette=<0..100>` renders any
scene at another Palette step; pass page options with `--query`) at 1920x1080 and saves one PNG per scene, plus a
`manifest.json` with the share code, the camera pose and the per-pass GPU times, under
`target/visual-gallery/<label>/`. `target/visual-gallery/index.html` shows every label side by
side per scene, so a package takes a `before` run on its base and an `after` run on its branch:

```sh
cd web && npm run build && cd ..
node tools/browser-smoke/gallery.mjs --label before        # on the base commit
node tools/browser-smoke/gallery.mjs --label after         # on the branch
node tools/browser-smoke/gallery.mjs --index-only --compare before,after
node tools/browser-smoke/gallery.mjs --help
```

The page is the normal client with `?gallery=<scene id>` (`web/src/gallery/gallery.ts`): offline
practice on a share code built from a fixed seed and sliders, no dummies, a camera pose picked
from the spawn point by CPU ray casts against the chunk meshes (so poses follow generator
changes), synthetic ships, shots and effect cues drawn through `game/scene.ts`, and a fixed
shader time. Particles are the sum of what happened before the frame, so scenes with content
replay a short synthetic history through the normal effect code with a seeded RNG
(`web/src/gallery/vfxScenes.ts`, `simulateVfx`); the PASS line reports the live particle count.
It needs no game server (a static file server on 127.0.0.1 serves `web/dist`).

GPU timing: with the optional `timestamp-query` feature every render pass writes timestamps
(`web/src/renderer/gpuTimer.ts`; labels `terrain`, `objects`, `post`, later `particles`,
`lights`, `sky`, `ink`), averaged over 60 frames. `Renderer.detailedTiming` (on in the gallery and while
the F3 overlay shows) splits the scene into a terrain and an objects pass; otherwise the scene is
one pass timed as `scene`. The F3 overlay shows the line `gpu <total> ms (worst ..) · <pass>
<ms> ...`. The gallery launches the browser with `--enable-webgpu-developer-features`
(unquantised timestamps) and fails a scene whose pass exceeds its budget share
(`PASS_BUDGETS_MS`, 1080p "high" on the reference mid GPU) by more than 25 %, or whose frame
exceeds 8 ms by more than 25 %. `--budget-scale 1.75` checks the integrated-GPU "medium"
target (14 ms), `--power low` asks for the integrated GPU (`?gpu=low-power`; Chrome needs
`--use-webgpu-power-preference=force-low-power`, which the script passes; Chrome on Windows still
hands WebGPU only its GPU process's adapter, so when that yields none the script looks up the
integrated GPU's LUID in `chrome://gpu` and relaunches with `--use-adapter-luid`, or pin one with
`--adapter-luid <high>,<low>`; it fails when no such GPU is available to WebGPU), `--no-budget` records without failing, `--quality high|medium|low` renders another preset
(default high, `?quality=`; no automatic pick in the gallery), `--grade <preset>` previews a
colour grade (`GRADE_PRESETS` in `web/src/renderer/post.ts`, `?grade=`; it replaces the world's
grade), `--no-atmosphere` renders without the per-world sky, fog and grade (`?atmosphere=off`,
the pre-VQ7 look, for before shots of scenes added later), `--lights off` renders the same
frames without dynamic lights (`?lights=0`; compare the pass times with a `--lights on` run for
the lights' shading cost), `--vfx off` draws the pre-VQ5 mesh effects instead of particles
(`?vfx=off`, also in the game), `--query <k=v&...>` adds page parameters (`--query
view=legacy`: the pre-VQ1 camera, 75° vertical FOV without cockpit frame or fit ring, for before
shots of the FOV change; the fit scenes print the ring's state and the CPU time of its probes;
`--query shadows=off`: the pre-VQ9 sun that shines through rock, for before shots and the sun
occlusion's cost; `--query shadowbench=full` or `chunk` redraws the whole sun shadow map or
one crater's two chunks every frame, so the `shadow` pass shows the update cost).
Without
`timestamp-query` or on a software adapter the budget is not checked. Measured on a much faster
GPU than the reference, the check only catches gross regressions; read the absolute numbers on
the reference class.

First-impression pages (FI): the scenes `start-screen`, `start-flythrough`, `lobby-flythrough`
and `ship-bay` (`page` in `scenes.ts`) render the real start screen, the lobby card and the
in-game ship bay over the live backdrop (`web/src/attract/galleryPages.ts`): the flythrough
steps at a fixed 60 Hz from a fixed seed up to the scene's time and then holds the frame; the
hull views' GPU time is reported as the pass `bay`. The world workshop pages `workshop`,
`workshop-tall` (Verticality 90) and `workshop-belt` (an asteroid belt with the host box open;
it fails until the build generates belts) show the workshop over the preview of its own world
(`workshop` in `scenes.ts`: seed, sliders, archetype); `--query seed=<n>` tries other seeds.
To curate the backdrop's worlds
(`BACKDROP_WORLDS` in `web/src/attract/backdrop.ts`) try others with `--scenes start-flythrough
--query world=<seed>_<enclosure>_<density>&t=<seconds>`.

### Start-screen timing

`node tools/browser-smoke/startup.mjs [--web-root dir] [--label name] [--runs 5] [--query k=v]`
loads the start page headless several times and prints the median time until the start card
shows, time to interactive (the first moment followed by 5 s without a long task), total
blocking time, the longest task, the backdrop's first frame and frame rate, and the page's
`ls:*` performance measures (renderer creation, backdrop world); `startup.json` and a
screenshot go to `target/startup/<label>/`. Run it against a before build (`--web-root` of a
copy of its `web/dist`) for comparisons; `--query backdrop=off` measures without the flythrough.

### Sound check and sound previews

`?audio=demo` (for example `http://localhost:7777/?audio=demo`) is the sound check page: every
registered sound with a play button (loops toggle), played centred or from a direction and
distance, the volume sliders, a 64-sound burst against the voice cap, and the three announcer
voice packs saying any of their lines with the in-game caption. Offline, without a browser:

```sh
node web/scripts/audio-preview.ts [--only <id-prefix>] [--out <dir>]   # WAV + spectrogram PNG per sound, levels and render times
```

writes `target/audio-preview/<id>.wav`, `<id>.png` and `index.md`. Adding a sound and the mix
conventions: [architecture/audio.md](architecture/audio.md#adding-sounds). The gallery scenes
`announcer-tired`, `announcer-overexcited` and `announcer-passive` show the caption over the
fight frame.

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
