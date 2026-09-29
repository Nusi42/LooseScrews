# Loose Screws — Implementation Plan

Status: Milestone 1 (vertical slice) in progress — waves 1 and 2 done · Source design: [design/game-design-doc.md](design/game-design-doc.md)

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
- CLI: `ls-bots --server ws://127.0.0.1:7777 --count 7`.

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

## 6. Testing strategy

- `cargo test --workspace` for all Rust crates; `cargo clippy --workspace -- -D warnings`.
- `cargo build -p ls-wasm --target wasm32-unknown-unknown --release` + `wasm-bindgen` + Node script comparing golden hashes with native.
- `npm run typecheck`, `npm test` and `npm run build` in `web/`.
- Replay tests: recorded input streams must produce identical state hashes.
- Soak test: server + 7 bots for 10 minutes, checks tick time and desync counters.
- `tools/ci.ps1` and `tools/ci.sh` run all of the above in order.

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
