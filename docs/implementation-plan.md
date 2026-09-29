# Loose Screws — Implementation Plan

Status: Milestone 1 (vertical slice) done — waves 1 to 5 committed (last: `b39e445`), see [section 5.1](#51-milestone-1-status); the current milestone is [M2, arsenal and physics](milestone-2-plan.md), which also closed most of the M1 gaps listed there · Source design: [design/game-design-doc.md](design/game-design-doc.md) · Developer docs: [README.md](README.md)

Milestone 1 proves the three riskiest parts of the design before anything else is built:

1. **Terrain:** voxel SDF chunks meshed fast enough to regenerate after explosions within a frame budget.
2. **Determinism:** the same share code builds a bit-identical world natively (server) and in WASM (browser).
3. **Netcode:** 8 players flying 6DOF in tunnels with an authoritative 60 Hz server, client prediction and interpolation.

Everything else in the design (full arsenal, ship types, comedy director, other modes, Tauri, lobby) follows in later milestones listed at the end.

## 1. Milestone 1 scope

| Area | In scope | Out of scope (later) |
| --- | --- | --- |
| Ship | One ship type (Tinker): hull, energy, boost, scrap, overdrive gamble, per-axis thrust cap, horizon assist | Skitter, Hauler, Tug; gadgets |
| Weapons | Scrap Laser (energy), Rivet Gun (kinetic, fires scrap), Bolt Rocket (ordnance, craters), Paint Thrower (special, needed for Paint the Void) | Rest of arsenal, Tectonic Charge, specials |
| World | Seeded SDF generator with Enclosure and Density active (other five sliders encoded but inert), 32³ chunks, surface-nets mesher then dual contouring, sphere-subtract terrain edits, paint layer | Gravity wells, palette, instability collapses, debris physics |
| Share codes | seed + all 7 sliders + generator version + checksum: 26 Crockford base32 symbols, printed as `LSxx-xxxx-…` (34 chars) | Rule set codes |
| Match rules | Data-driven weapon definitions and one rule set format (availability, loadout, pickups, friendly fire), loaded from `data/` | Rule set editor UI, live balance list |
| Mode | Paint the Void, 2 teams, up to 8 players, one server process | All other modes |
| Networking | Authoritative server, 60 Hz tick, 30 Hz snapshots, quantized ship state (~18 B), input commands, prediction + reconciliation, 100 ms interpolation, lag-compensated hits; WebSocket transport first, WebTransport second | WebRTC fallback, Tauri UDP, lobby service, regions |
| Client | Browser, TypeScript + Vite, thin WebGPU renderer, pointer-lock 6DOF controls, HUD (hull, energy, boost, scrap, ammo, team paint %), share-code entry | WebGL2 fallback (stretch), audio, cosmetics |
| Tooling | Headless bots for load tests, determinism cross-check native vs WASM, CI script | Deployment |

## 2. Repository layout

```
Cargo.toml              Rust workspace (members pre-declared)
crates/
  ls-math/              Deterministic math: Vec3, Quat, fixed tick time, SplitMix64/PCG RNG, libm wrappers
  ls-worldgen/          Sliders, share codes, SDF generator, chunk store, meshers, terrain edits, paint layer
  ls-sim/               Game simulation: ship, weapons, projectiles, collisions, rule sets, Paint the Void
  ls-net/               Wire protocol, quantization, snapshots and deltas, transport trait
  ls-game/              Shared glue (no I/O): terrain adapter, match setup, sim/net conversions, client prediction and interpolation, server Room with lag compensation
  ls-server/            Native authoritative server binary (tokio), WebSocket + WebTransport
  ls-wasm/              wasm-bindgen bindings for the browser: worldgen, meshing, prediction
  ls-bots/              Headless bot clients for load and soak tests
data/
  weapons/*.ron         One file per weapon, all stats data-driven
  rulesets/*.ron        Default rule set per mode
web/                    Browser client (TypeScript, Vite, WebGPU)
tools/                  Scripts: determinism check, CI, dev server helpers
docs/                   Developer documentation (design, architecture, plans, ADRs)
userDocs/               Player-facing manuals and hosting guides
```

## 3. Key technical decisions

| Decision | Choice | Why |
| --- | --- | --- |
| Language | Rust only for core, server and WASM | Decided in the design doc |
| Floating point | `f32` with only IEEE-exact ops (`+ - * / sqrt`) in deterministic code; transcendental functions via the pure-Rust `libm` crate; no `mul_add`, no fast-math | Bit-identical results on x86_64 and wasm32 |
| RNG | SplitMix64 for seeding, PCG32 streams per subsystem | Portable and reproducible |
| Noise | Own integer-hashed gradient noise in `ls-worldgen` | No dependency whose output could change between versions |
| Mesher | Surface nets first (fast, simple), then dual contouring with QEF for sharp mine edges | Get a working pipeline early, then sharpen |
| Terrain edits | Parametric ops (`SubtractSphere { center, radius }`, `Paint { center, radius, team }`), applied in tick order, logged | ~16 B per edit on the wire; late joiners replay the log |
| Data format | RON for weapons and rule sets, `serde` everywhere | Readable, diff-friendly, supports enums |
| Server runtime | tokio; sim runs on a fixed-step thread, I/O on async tasks | Keeps the tick independent of network jitter |
| Transport | `Transport` trait; WebSocket (`tokio-tungstenite`) first, then WebTransport (`wtransport`) with a self-signed cert and `serverCertificateHashes` | WebSocket unblocks integration; WebTransport is the target |
| Wire encoding | Hand-written bit packing in `ls-net` (no generic serializer on the hot path) | Hits the 18 B ship-state budget |
| WASM toolchain | `wasm-bindgen` CLI 0.2.126 (installed), crate pinned to the same version; no wasm-pack | Matches the local toolchain |
| Web client | TypeScript strict, Vite, plain WebGPU with a small custom renderer | Design recommends WebGPU; no engine lock-in |
| Package manager | npm | pnpm is not installed |

### Determinism rules (enforced by review and tests)

- Deterministic crates (`ls-math`, `ls-worldgen`, `ls-sim`) never call `f32::sin/cos/tan/exp/ln/powf/atan2` directly; they use `ls_math::det::*` (backed by `libm`).
- No `HashMap` iteration order in simulation or generation output; use `BTreeMap` or sorted vectors.
- No wall-clock time, threads, or platform RNG inside deterministic code; time is a tick counter.
- Golden-hash tests: a fixed list of share codes produces SDF and mesh hashes that must match between native tests and the WASM build run under Node.

## 4. Module specifications and acceptance criteria

### 4.1 ls-math
- `Vec3`, `Quat` (normalize, mul, rotate, from axis-angle, slerp), `Aabb`, fixed-step `Tick(u64)` with `DT = 1/60`.
- `det` module: `sin, cos, atan2, sqrt, exp, powf, floor` via `libm`.
- `rng`: `SplitMix64`, `Pcg32` with `stream(u64)`.
- **Done when:** unit tests pass on native and wasm32 (`cargo test`, plus build for `wasm32-unknown-unknown`).

### 4.2 ls-worldgen
- `Sliders { enclosure, density, verticality, gravity_chaos, palette, absurdity, instability }` as `u8` 0–100.
- `ShareCode`: encode/decode seed (u64) + 7 sliders (7 bits each) + generator version (u8) + 8-bit checksum → 26 Crockford base32 symbols (130 bits including 1 pad bit), printed with an `LS` prefix and dashes (34 characters). Input ignores case, dashes and spaces. Round-trip and corruption tests.
- `Generator::sample(p) -> f32` SDF: Enclosure blends open sky (sparse floating islands) → caverns → dense mine shafts; Density adds pillars and floating rocks. Level bounds are a finite box (default 256 × 128 × 256 u), sealed at the edges.
- `ChunkStore`: 32³ cells per chunk, lazily generated, edit log applied on top; `apply(TerrainEdit)` returns dirty chunk coordinates.
- Meshers: `surface_nets(chunk) -> Mesh` (positions, normals, indices, per-vertex paint team) and `dual_contouring(chunk)`; both seam-free across chunk borders.
- Paint layer: per-surface-cell team id; coverage stats per team.
- Queries for physics: `sdf(p)`, `normal(p)`, `raycast(origin, dir, max) -> Option<Hit>`, `sphere_overlap`.
- **Done when:** golden hashes match native vs WASM; meshing a 32³ chunk takes < 2 ms native release (benchmark printed by a test or bench); remeshing after one 3 u crater touches ≤ 8 chunks.

### 4.3 ls-sim
- `ShipState` (pos, vel, orientation, angular vel, hull, energy, boost, scrap, weapon states) and `InputCmd` (tick, thrust axes −1..1, look deltas, roll, boost, fire primary/secondary, weapon select, scavenge).
- Flight model: thrust capped per axis in ship space (cruise 12 u/s, boost 16.5 u/s per axis → 28.5 u/s 3-axis diagonal), damping, horizon assist option.
- Resources: energy regenerates only while not boosting; boost refills from hard braking and near-miss wall scrapes; hull only from scavenging scrap (hold R near debris/scrap pickups); overdrive burns 2 hull per 1 energy when energy is empty.
- Weapons loaded from `data/weapons/*.ron`: speed, lifetime, damage, fire interval, cost (energy / scrap / count), splash, knockback, terrain effect (crater radius, paint radius), shield interaction field (unused in M1 but present).
- Projectiles never inherit shooter velocity; swept collision against terrain and ships.
- `RuleSet` loaded from `data/rulesets/*.ron`: per-weapon availability (LoadoutAndPickup / PickupOnly / LoadoutOnly / Banned), starting loadout, pickup spawn weights and respawn times, friendly fire percent, allowed ship types, absurdity.
- `World::step(inputs) -> Events` is deterministic given the same inputs.
- Paint the Void: two teams, match timer (default 5 min), score = team share of painted surface cells, respawns.
- **Done when:** unit tests cover the speed model numbers from the design doc, resource rules, each M1 weapon, rule-set enforcement (banned weapon cannot be fired or picked up), and a replay test (same inputs → same state hash).

### 4.4 ls-net
- Messages: `Hello/Welcome` (share code, rule set, player id, team, edit log), `Input` (redundant last 3 commands), `Snapshot` (tick, acked input, ships, projectiles, new edits, scores), `Event` (hits, kills), `Ping/Pong`.
- Quantized ship state ≤ 18 B: position 3 × 16 bit within level bounds, smallest-three quaternion in 32 bits, velocity 3 × 16 bit, flags 16 bit.
- Snapshot delta against the last acked snapshot.
- `Transport` trait with send-unreliable, send-reliable, recv; implementations live in `ls-server` and the web client.
- **Done when:** round-trip property tests, a size test proving the 18 B budget, and a bandwidth estimate test: 32 ships x 18 B x 30 Hz of ship data ≤ 17 KiB/s, whole snapshot including header ≤ 18 KiB/s, before delta compression.

### 4.5 ls-server
- CLI: `ls-server --bind 0.0.0.0:7777 --code <share code> --ruleset data/rulesets/paint_the_void.ron --max-players 8`.
- Fixed 60 Hz sim thread, 30 Hz snapshots, per-client input buffers, lag compensation by rewinding ship positions up to 250 ms for hit tests.
- WebSocket endpoint first; WebTransport endpoint second (self-signed cert generated at startup, hash printed and served at `GET /cert-hash`).
- Serves the built web client statically in dev mode for one-command local play.
- **Done when:** 8 bots play a full Paint the Void match without desync warnings; server tick stays under 4 ms p99 on a dev machine.

### 4.6 ls-wasm and web client
- `ls-wasm` exposes: decode share code, generate + mesh chunks (returns typed arrays), apply edits, client-side prediction step for the local ship.
- Web client: WebGPU init, chunk mesh upload and culling, simple lit shading with team paint colors, projectiles and ship meshes, 6DOF camera with pointer lock, controls from the design doc, HUD, share-code entry screen, connect to server, prediction + reconciliation, 100 ms interpolation for remote ships.
- **Done when:** a player can open `http://localhost:7777`, enter a share code or accept the server's, fly, shoot, crater terrain, paint, and see team scores update with 7 bots.

### 4.7 ls-bots
- Headless clients reusing `ls-net` and `ls-sim` prediction; simple behaviors (wander through open space, aim at nearest enemy, paint walls).
- CLI: `ls-bots --server ws://127.0.0.1:7777/ws --count 7`.

## 5. Work breakdown and orchestration

Work runs as orchestrated waves of parallel work packages. Packages in the same wave own disjoint directories, so they can work in one tree without conflicts; each uses its own `CARGO_TARGET_DIR` to avoid build-lock stalls. The orchestrator commits after each wave once builds and tests pass.

| Wave | Work package | Owner directories | Depends on |
| --- | --- | --- | --- |
| 1 | Scaffold: workspace, crate skeletons, `ls-math` complete, `data/` schemas, `web/` Vite skeleton, `tools/ci` script, `.gitignore` | root, `crates/ls-math`, `web/`, `tools/`, `data/` | — |
| 2a | World generation, share codes, meshers, edits, paint, golden hashes | `crates/ls-worldgen` | 1 |
| 2b | Simulation, weapons, rule sets, Paint the Void | `crates/ls-sim`, `data/` | 1 (terrain via a `TerrainQuery` trait) |
| 2c | Protocol, quantization, deltas | `crates/ls-net` | 1 |
| 2d | WebGPU renderer, camera, input, HUD shell (against a test mesh) | `web/` | 1 |
| 2e | Review and verify wave 2 (build, tests, clippy, determinism rules) | read-only, then fixes | 2a–2d |
| 3a | Server: tick loop, rooms, WebSocket transport, lag compensation | `crates/ls-server` | 2 |
| 3b | WASM bindings + client integration: terrain from WASM, prediction, interpolation, connect | `crates/ls-wasm`, `web/` | 2 |
| 3c | Bots | `crates/ls-bots` | 2 |
| 3d | End-to-end test: server + 7 bots + determinism cross-check (Node runs WASM golden hashes) | `tools/` | 3a–3c |
| 4 | WebTransport transport, dual contouring upgrade, performance pass | server, worldgen, web | 3 |
| 5 | Documentation: architecture and dev guide in `docs/`; player manual and hosting guide in `userDocs/` | `docs/`, `userDocs/` | 3 |

### 5.1 Milestone 1 status

Updated in wave 5 (docs). Verified with release builds in this tree: the wasm32
unit tests, the native worldgen golden/timing, sim replay and match setup tests, `tools/determinism-check.mjs` (6/6 share codes match), `cargo test -p
ls-bots --release`, the `ls-server` WebSocket and WebTransport tests, and `tools/e2e.mjs`
(4 bots for 30 s: all 22 checks pass; 8 bots over WebTransport for a full match and restart on an
idle machine: all 23 checks pass, worst window p99 241 us).

| Wave | Status |
| --- | --- |
| 1 Scaffold | Done (commit `936b75c`) |
| 2a-2e Worldgen, sim, net, renderer, review | Done (commit `936b75c`) |
| 3a-3d Server, WASM client, bots, e2e | Done (commit `f26e093`) |
| 4 WebTransport | Done: server endpoint (`crates/ls-server/src/wt.rs`, UDP, self-signed cert, `/cert-hash`), browser transport with WebSocket fallback, bots `--transport wt`, e2e `--transport wt` |
| 4 Dual contouring | Mesher implemented, seam-free and golden-hashed, but gameplay still meshes with surface nets: dual contouring costs 4-13 ms per busy chunk against 0.5-0.8 ms, too slow for the per-frame remesh budget |
| 4 Performance pass | Done: `ls_worldgen::RegionSampler` (faster chunk generation, same bits, golden hashes unchanged) and the `worldgen_bench` example. Server tick: p50 about 0.1 ms, p99 typically 0.2-0.5 ms with 8 bots |
| 5 Documentation | `docs/`: index, architecture overview, netcode, determinism, dev guide. `userDocs/`: player manual and hosting guide. Reviewed against the code and `--help` output |

Acceptance criteria (section 4):

| Module | Criteria | State |
| --- | --- | --- |
| 4.1 ls-math | Tests on native and wasm32 | Met |
| 4.2 ls-worldgen | Golden hashes native = WASM; < 2 ms surface nets per chunk; crater touches ≤ 8 chunks | Met (0.5-0.8 ms per busy chunk; `crater_dirties_at_most_eight_chunks`) |
| 4.3 ls-sim | Speed model, resources, each weapon, rule-set enforcement, replay hash | Met |
| 4.4 ls-net | Round trips, 18 B ship state, bandwidth estimate | Met (`bandwidth_32_ships_at_30_hz`) |
| 4.5 ls-server | 8 bots play a full match without desync warnings; p99 < 4 ms | Met: `tools/e2e.ps1 --transport wt --count 8 --duration 330` on an idle machine: full match and restart, 0 desyncs, 0 protocol errors, 0 skipped ticks, worst window p99 241 us (max 297 us). An earlier run while the machine was compiling showed isolated spikes up to 7.5 ms p99 |
| 4.6 ls-wasm + web | Player opens `http://localhost:7777`, flies, shoots, craters, paints, sees scores with 7 bots | Implemented; the client core is tested headless against a live server and the WebTransport connection was checked in headless Chrome, but no person has played it in a browser yet |
| 4.7 ls-bots | CLI and behaviours | Met |

Gaps left at the end of M1 (kept as written then; M2 closed several, see the notes in
brackets and [milestone-2-plan.md](milestone-2-plan.md)):

- **Browser acceptance is manual.** No automated browser run covers 4.6 (WebGPU rendering,
  pointer lock, WebTransport in a real browser); a headless-browser smoke test would close it.
  [M2: closed by the browser smoke test, `tools/browser-smoke`, part of `tools/ci.*`.]
- **Dual contouring is not used in play** (see wave 4 above): either make it fast enough for
  the remesh budget or keep surface nets for M1 and move sharp edges to a later milestone.
- **Soak test.** Passed manually: 3 bots for 60 minutes (12 matches, 0 protocol errors, 0 desyncs, 0 disconnects, max reconciliation error 0.134 u, server p99 about 0.06 ms). It is not part of `tools/ci.*`.
- **No hosted CI.** `tools/ci.ps1` / `tools/ci.sh` run locally only.
- **Handshake does not check game data.** Weapons and ship tuning are compiled into every
  binary and are not compared in `Hello`/`Welcome`; a client built from different `data/` would
  desync silently. A data hash in the handshake (or bumping `PROTOCOL_VERSION` on data
  changes) would catch it. [M2: closed; `Hello` carries `ls_sim::data_hash`, mismatches are
  rejected with `DataMismatch`.]
- **Lag compensation estimates the shooter's view tick** from the acked snapshot (error below
  one tick on average); an exact value needs the client's render tick in `InputMsg`, a protocol
  change (see [architecture/game-glue.md](architecture/game-glue.md#lag-compensation)).
- **Pickups, scrap piles and wrecks are not drawn** in the browser (they work in the simulation;
  see `userDocs/player-manual.md`). Remote pilots show as "Player N", not by name. [M2: closed;
  generic entities are drawn from their data-driven visuals and name tags show player names.]

## 6. Testing strategy

Commands and details: [dev-guide.md](dev-guide.md#4-tests) and [architecture/determinism.md](architecture/determinism.md).

- `cargo test --workspace` for all Rust crates, `cargo test -p ls-bots --release` for the bot end-to-end tests; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --all -- --check`.
- Unit tests of `ls-math`, `ls-net`, `ls-sim`, `ls-worldgen` and `ls-game` on wasm32 under Node (`wasm-bindgen-test-runner`), including the worldgen golden hashes and the sim replay hash.
- `cargo build -p ls-wasm --target wasm32-unknown-unknown --release` + `wasm-bindgen` + `tools/determinism-check.mjs` comparing the package's golden hashes with the native fixture.
- `npm run typecheck`, `npm test` and `npm run build` in `web/`.
- Replay tests: recorded input streams must produce identical state hashes.
- End-to-end: `tools/e2e.ps1` / `tools/e2e.sh` (server + 7 bots for 90 s with log checks; a full match with `--count 8 --duration 330`). Soak test: server + 7 bots for 10 minutes (`ls-bots --duration 600`), checking tick time and desync counters. Both are manual, not part of CI.
- Since M2: the browser smoke test (`tools/browser-smoke.*`, headless Chrome or Edge with WebGPU against real servers) and the staleness check of the generated player reference (`node tools/gen-weapon-docs.mjs --check`).
- `tools/ci.ps1` and `tools/ci.sh` run everything above except the end-to-end and soak runs, in order.

## 7. Risks

| Risk | Mitigation |
| --- | --- |
| Float drift between wasm32 and x86_64 | Determinism rules, `libm`, golden hashes in CI from day one |
| Remesh cost after big explosions | Surface nets first, per-frame remesh budget, dirty-chunk queue, benchmarks |
| WebTransport setup friction (certs, browser flags) | WebSocket first behind the `Transport` trait; cert hash endpoint |
| 6DOF prediction errors in tight tunnels | Same `ls-sim` code on client (WASM) and server; reconciliation with smoothing |
| Scope creep from the full design | Strict M1 table above; everything else goes to later milestones |

## 8. Later milestones

| Milestone | Content |
| --- | --- |
| M2 Arsenal and physics | All weapons from the design doc, Tectonic Charge, debris physics, surface states, ship types, gadgets (Shield, Cloak, Decoy), rule set editor |
| M3 Modes and lobby | Team Brawl, The Squeeze, Instagib, Heist, Hot Reactor, Last Color Standing, duels; lobby service, server list, invites, party codes |
| M4 The level has moods | Comedy director and events, remaining sliders (verticality, gravity chaos, palette, instability), monochrome pigment, The Architect |
| M5 Platforms and ops | Tauri app with UDP, WebRTC fallback, WebGL2 fallback, regional deployment with Agones or Nomad, bandwidth optimization |
| M6 Coop and business | Deep Dig, Tethered, Escort the Intern; cosmetics store, creator economy |
