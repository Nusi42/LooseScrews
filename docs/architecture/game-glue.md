# Game glue (`ls-game`)

`crates/ls-game` is the pure-logic layer shared by `ls-server`, `ls-wasm` and `ls-bots`. It
joins `ls-worldgen` (terrain), `ls-sim` (rules and physics) and `ls-net` (wire format) and does
no I/O: hosts own sockets, clocks and threads, feed packets in and send what comes out.
The end-to-end netcode picture (tick model, deltas, edit replication, transports) is in
[netcode.md](netcode.md); how the crates fit together in [overview.md](overview.md).

```
ls-math <- ls-worldgen --+
        <- ls-sim -------+--> ls-game --> ls-server (feature "server": Room)
        <- ls-net -------+            --> ls-wasm   (default-features = false)
                                      --> ls-bots
```

| Module | Contents |
| --- | --- |
| `terrain` | `GameTerrain`: `ChunkStore` + edit log + dirty chunks; `TerrainQuery` + `SimTerrain` |
| `setup` | `MatchConfig`, `match_setup` (spawn and pickup points), `build_match` (world + terrain) |
| `convert` | `NetConv` (ships, status, quantization), inputs, projectiles, events |
| `client` | `InputSequencer`, `Predictor`, `InterpolationBuffer`, `ClientClock`, `ClientSession` |
| `server` | `Room` (feature `server`, on by default), `ShipHistory`, lag compensation |

Everything builds for `wasm32-unknown-unknown` (with and without the `server` feature) and the
unit tests run there under Node (`cargo test --release -p ls-game --lib --target
wasm32-unknown-unknown` with the `wasm-bindgen-test-runner` runner). The workspace dependency is
declared with `default-features = false`; `ls-server` enables `features = ["server"]`.

## GameTerrain

- Wraps `ls_worldgen::ChunkStore`; `TerrainQuery::sdf` is `ChunkStore::eval`, `bounds` the
  generator bounds (outside is solid).
- `SimTerrain::apply_edit` logs **every** edit it is given, including ones the chunk store
  rejects, so the log index is the edit's sequence number on every peer (`Welcome::edit_log`,
  `Snapshot::edit_base`). `apply_sequence(base, edits)` applies only the unseen tail and fails
  with `GameError::EditGap` if edits were missed.
- `SimTerrain::paint_coverage` maps `ls_worldgen::PaintCoverage` to teams 0/1 and caches it
  until the next edit. It generates every chunk on first use (about 7 s in debug builds at the default sliders, well
  under a second in release), which is why `Room::new` pregenerates and why tests that stay
  below tick 30 (the first score update) turn that off.
- The store sits in a `RefCell` because `paint_coverage(&self)` must generate chunks. The type
  is `Send`, not `Sync`: keep it on the thread that steps the world.
- Dirty chunks accumulate until `take_dirty_chunks()`; `mesh_chunk(c)` runs surface nets.
- `state_hash()` = edit log hash + SDF on a 4 u lattice + paint cells around every edit. It
  needs no full generation and is the desync check between peers.

## Match setup

`match_setup(code, rules, pristine_terrain)`:

1. Room box shrunk by 4 u. Team 0 spawns in the low-x 30 % slab, team 1 in the high-x slab.
   The pristine level is first scanned on a 4 u lattice (`scan_open_space`, points with at
   least 1 u clearance).
2. Per team, up to 6 spawn points. Step 1 (only if the scan saw at least 6 points with 3 u
   clearance in the slab): draw a point in the slab (`Pcg32::from_seed(seed,
   RNG_STREAM_SPAWNS)`), snap it to open space with 3 u clearance (`find_open_point`, radius
   6), keep it if still in the slab and 6 u from the team's other spawns; 64 draws per point.
   Step 2 (step 1 skipped or found nothing, on a copy of the RNG stream): pick scanned points
   with 3, 2, 1.5, then 1 u clearance in the slab, then in the team's half of the level, then
   on a 2 u lattice of that half. Step-2 points are strictly on the team's side of the
   midplane. Spawns face the level centre horizontally (no roll).
3. Up to 12 pickup points anywhere in the box (`RNG_STREAM_PICKUP_POINTS`), 2 u clearance,
   14 u apart, 10 u from every spawn; random draws only if at least 1 % of the lattice has
   2 u clearance, otherwise scanned points with 2, 1.5, then 1 u clearance.

Roomy levels keep exactly the setups of the plain random search; tight ones (Enclosure 75 and
up, where the SDF underestimates distance and almost nothing has 3 u clearance) get spawns and
pickups with less clearance (at least 1 u, twice the ship radius). `setup::tests` sweeps all 25
Enclosure x Density combinations (0/25/50/75/100): one seed in debug builds, 8 with
`cargo test -p ls-game --release --lib setup` (and in CI's wasm32 run).

The world's `WorldConfig::seed` is `mix64(code.seed ^ MATCH_SEED_SALT)`. Setup reads only the
pristine level, so a client computes the same spawn points from the share code (the predictor
uses them, see below). `build_match(&MatchConfig)` returns `(World, GameTerrain)` at tick 0.

## Wire conventions

These are the rules `ls-server`, `ls-wasm` and `ls-bots` rely on; `Room` and `ClientSession`
implement them.

| Item | Convention |
| --- | --- |
| Ids | Wire ship id = the client's slot `0..max_players` (lowest free), not `ls_sim::PlayerId` |
| Ticks | Input for tick `k` is applied by the server step that produces tick `k`; `Snapshot::tick` is the server tick after that step |
| `ack_input` | Newest *real* (not repeated) input tick of that client applied so far; `< tick` means the server ran out of input |
| Missing input | The server repeats the client's last command; late inputs (tick already simulated) are dropped |
| Quantization | `quant_config_for(bounds, weapons)`: level bounds + 2 u, ships ±64 u/s, projectiles ±max(256, fastest weapon) |
| Ship flags | `ALIVE`, `BOOSTING`, team bits, weapon bits = primary weapon id (0-15) |
| `ShipStatus` | Own ship only; each byte `ceil(value / max * 255)` of hull, energy, boost, scrap |
| Inputs | Thrust/roll to 1/127, look to 1/8192 rad; button bits identical to `ls_sim::Buttons`; `weapon` = `InputCmd::weapon_select` |
| Projectiles | `id` = sim id truncated to u16, `kind` = weapon id, `owner` = wire id or 255 |
| Scores | `[team 0 cells, team 1 cells, surface cells]` (raw counts; percent = cells / surface) |
| Edits | Each snapshot carries up to 64 edits starting at the client's acknowledged edit count (`edit_end` of its newest acked snapshot); resent until acked |
| Events | `Damaged` -> `Hit` (pos = impact point), `Killed` -> `Kill` (255 = none), `Respawned`, `MatchEnded`; batched per snapshot, reliable |
| Ping | Send right after the input for tick `c`; `Pong::server_tick - c` measures input slack |
| Match end | After the timer runs out (`match_ticks_left == 0`) the server stops stepping the world: ships freeze at the end tick, snapshots continue |
| Match restart | A host that replaces a finished match sends every client `Disconnect { reason: ls_game::MATCH_RESTART_REASON }`; clients rejoin (a new `Welcome`) instead of reporting an error |

## Client core

`ClientSession::from_welcome(welcome, weapons, tuning)` regenerates the level from the share code,
replays the edit log, parses and resolves the rule set, and computes the spawn poses and pickup
points (`pickup_points()`, which bots use). Per frame the host calls `frame(due_ticks, controls)`
and sends the returned messages; every received packet goes to `handle_packet`.

`frame` is `plan_ticks(due)` (advances the clock, returns how many ticks to run: `due` plus up to
4 while catching up, 64 before sync, or one fewer when skipping) followed by `run_ticks(n, due,
controls)`. Hosts that spread a per-frame delta over the ticks (the browser's mouse look,
`ls_wasm::controls::LookAccumulator`) must call the two halves and spread over `n`, not `due`;
otherwise a catch-up applies the look up to 5 times and a skip drops it. Before sync a long
catch-up sends only the newest 8 `Input` messages (older ones would be late anyway).

- **Input** (`InputSequencer`): consecutive ticks starting at `Welcome::server_tick + 4`; each
  command is quantized exactly as the server will decode it, and every `Input` message carries
  the newest three.
- **Sync**: until a snapshot shows the server had our input in time (`ClientClock::is_synced`),
  commands are replaced by idle ones. Look input is a per-tick delta, so a command the server
  replaced by a repeat would leave a permanent orientation difference.
- **Clock** (`ClientClock`): starvation (`ack_input < tick`) makes the client run ahead by the
  missing ticks (at most 30 per report; reports about inputs sent before that catch-up are
  ignored). A client whose next input tick is not past the snapshot tick at all (slow level
  generation in WASM) jumps to `tick + 1 + 1 + 3 + 8` in one go, whatever the distance; such
  reports only raise the target, so a backlog of snapshots handled at once is fine. Ping slack
  below 1 tick runs ahead, a minimum above 4 ticks over four pongs skips one tick; pongs for
  pings sent before a starvation catch-up are ignored. The server time estimate follows the
  newest snapshot tick and local time.
- **Prediction** (`Predictor`): `ls_sim::simulate_movement` for each input on the local
  terrain, keeping the state after every tick. A snapshot's own-ship state for tick `T` is
  compared with the prediction for `T` within quantization tolerances (position 1.5 steps,
  velocity 1.5 steps, orientation 0.012 rad, boost 1.5 status steps):
  - match: keep the full-precision prediction (replacing it would only add quantization noise);
  - mismatch: take the server's value for each component that is out of tolerance, keep the
    predicted value for the others, replay inputs `T+1..` (reconciliation), and move the jump
    of the current state into a render offset that keeps 80 % per tick;
  - respawn, alive change or error above 4 u: snap. A snapped state that sits within tolerance
    of a spawn pose at rest takes the exact spawn pose, so prediction restarts bit-identical
    to the server.
  - match end: the session learns the end tick from any running snapshot (`tick +
    match_ticks_left`) and calls `Predictor::freeze_after(end)`, so ticks after it do not move
    the ship, exactly like the server (no corrections on the post-match screen).
- **Interpolation** (`InterpolationBuffer`): remote ships at `server time - 6 ticks` (100 ms),
  lerp position/velocity, nlerp orientation, no interpolation across jumps above 8 u, at most
  6 ticks of extrapolation.
- **Terrain**: snapshot edits are applied in log order before anything else (also from stale
  snapshots); meshes of dirty chunks are rebuilt by the host.

## Browser core extras (`ls-wasm`)

- **Shot prediction** (`crates/ls-wasm/src/shots.rs`, `ShotPredictor`): every predicted tick
  runs the simulation's firing rules on the predicted ship (weapon selection, bans, fire
  interval, energy / scrap / count cost, overdrive hull) and draws the shot at once with the
  simulation's muzzle position, direction and speed. Own projectiles in a snapshot are matched
  to predictions (same weapon, direction, position within 1.5 ticks of travel); the matched
  server projectile takes over and the difference becomes a fading render offset. A prediction
  the server never shows is dropped 4 ticks after it was fired; shots are hidden past the first
  wall or their lifetime. The server does not send the secondary selection: the predictor
  knows the loadout secondary plus any weapon kind seen on an own projectile.
- **Stats for overlays**: flat arrays for the F3 debug overlay (ping, corrections, snaps, shot
  counters, chunk counts) and the scoreboard / end-of-match screen (per-ship team, paint per
  team, match info). The browser passes its clock with `setClockMs` before handling packets so
  pings are timed when they arrive.

## Server core

`Room::new(MatchConfig, RoomConfig)` builds the match. The host:

1. calls `join(&Hello)` / `join_packet(bytes)` for a new connection (queues `Welcome`, or
   returns the `Reject` to send; the `Welcome` is encoded before the player is added, so an
   unencodable one rejects with `RejectReason::Other` instead of leaving a ghost ship),
   `leave(id)` when it closes (this also drops packets still queued for that id, which the
   next client may get);
2. passes packets to `handle_packet(id, bytes)` (inputs, pings, disconnect);
3. calls `step()` at 60 Hz; every `snapshot_interval` ticks (2 = 30 Hz) it queues a snapshot per
   client, delta-encoded against the client's newest acknowledged snapshot
   (`SnapshotHistory::choose_baseline`), plus pending events;
4. sends `take_outgoing()` (`Outgoing { client, channel, bytes }`; a host may send a too-large
   unreliable packet reliably, as `ls_net::send_message` does).

Team assignment: the preferred team unless it would put that team two or more players ahead,
otherwise the smaller team (ties to team 0). Inputs are queued per client by tick (duplicates
counted and ignored, far-future inputs beyond 120 ticks dropped). `RoomStats` counts missing,
late, duplicate and dropped inputs, compensated hits, snapshot bytes and messages that could
not be encoded (`encode_errors`, a bug if not zero). Missing inputs count only from a client's
first applied input on (before that it is still setting up and has predicted nothing).

### Lag compensation

- `ShipHistory` keeps every ship's position for the last 17 ticks (the 15-tick / 250 ms
  rewind limit plus margin).
- For each command the room estimates what the client saw (`Room::view_tick`): remote ships
  are rendered at the client's server-time estimate minus the interpolation delay, and that
  estimate runs on from the acknowledged snapshot with local time, on average half a snapshot
  interval, so `view = ack - 6 + snapshot_interval / 2` and `rewind = command tick - view`,
  clamped to `0..=15`. Each command of an `Input` message (the redundant older copies too) gets
  its own rewind. The remaining error is below a tick on average; an exact value would need the
  client's render tick in `InputMsg` (a protocol change). Every projectile remembers the rewind
  of the input that fired it.
- Before each step `plan_compensation` sweeps each projectile's segment for the coming tick
  against targets rewound by that amount. If the rewound target is hit (before terrain) and the
  simulation would not hit anything at least as early, the target is displaced for this one
  step so that after its own movement it sits exactly at the rewound position. `World::step`
  then resolves the hit with its normal rules (damage, friendly fire, knockback, kill credit,
  events) and the room moves the target back.
- Guards: no displacement when the target fires or scavenges this tick, when a pickup is in
  reach of the rewound position, or when its movement would differ there (terrain contact).
  Projectiles are not compensated on the tick they are fired. Splash from other explosions in
  the same tick sees the displaced position, and a kill leaves its wreck where the shooter saw
  the target.
- `lag_compensated_raycast` offers the same rewind for hitscan validation.

`ls-sim` has no hook for per-projectile target positions; a `World::step` variant that accepts
rewound target positions for projectile tests would remove the displacement trick.

## Tests

| Test | Shows |
| --- | --- |
| `tests/netplay.rs::prediction_converges_snapshots_decode_and_terrain_replicates` | Room + 4 clients over `MemoryTransport` with 2-5 tick latency (one with 10 % loss, reordering and duplication, one joining late): snapshots decode (deltas after the first), craters and paint replicate so every client's `state_hash` equals the server's, the prediction for the newest snapshot tick is within 0.02 u of the server's exact state, no corrections or snaps during the last 1.5 s for the perfect links, remote ships interpolated near the server's past positions |
| `tests/netplay.rs::lag_compensation_hits_target_that_moved_after_the_shot` | 5-tick latency each way; the target starts climbing as the shot is fired. Without compensation the shot misses on the server, with it the rewound target takes a direct hit |
| `tests/netplay.rs::slow_setup_client_syncs_within_a_few_round_trips` | A client that builds its session 240 ticks after the `Welcome` (with the snapshot backlog) synchronizes within 3 round trips |
| `server::tests` | join/leave/team balance, full room, late/duplicate/missing inputs (repeat last), no missing inputs before the first input, 30 Hz snapshots with deltas after acks, 250 ms history, `leave` drops queued packets before the id is reused, per-command rewind, unencodable `Welcome` rejects without a ghost ship |
| `client::*::tests` | input redundancy, prediction without corrections against a delayed server, misprediction correction and smoothing, replay of inputs sent before the first snapshot, frozen prediction after the match end, interpolation, clock (including catching up from far behind) |
| `ls-wasm` `game::tests::online_client_under_latency_applies_look_exactly_once` | The browser core over a 4-7 tick jittery link with a delay spike (clock catch-up) and uneven frames: synced, converged, every mouse delta sent exactly once, `Disconnect` reason reported |
| `ls-bots` `tests/room_play.rs` | A full room of 8 bots plays a (60 s) match to its end and 5 s beyond: no desyncs or protocol errors, terrain identical to the server's, no corrections after the match end |
| `setup`, `terrain`, `convert` tests | deterministic setup in open space, steppable world, edits/dirty chunks/gaps/coverage, round trips |

The whole crate tests in about 40 s in debug builds (dominated by level generation) and about
20 s in release (where the match setup sweep runs all 8 seeds).
