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
| `mode` | `mode_view_to_net` (`World::mode_view`, the replicated `ls_sim::ModeView`, to wire ids), `results_to_net` (`World::results` to `MatchResultsMsg`), `legacy_scores` (protocol 5), `squeeze_shell` (The Squeeze's walls from the replicated mode state); the mode framework itself: [modes-and-lobby.md](modes-and-lobby.md) |
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
  rejects, so the log index is the edit's sequence number on every peer (`Welcome::edit_count`,
  `Snapshot::edit_base`). `apply_sequence(base, edits)` applies only the unseen tail and fails
  with `GameError::EditGap` if edits were missed. `welcome_log(now)` is the log for a
  `Welcome`, without the expired surface-state edits that cannot matter any more; a late
  joiner builds its terrain with `replay_welcome(gen, log, edit_count)` and counts the
  left-out edits (`edit_count()`, `pruned_count()`), so its own `edit_log()` is not indexed
  by sequence number (only the server serves logs).
- `SimTerrain::paint_coverage` maps `ls_worldgen::PaintCoverage` to teams 0-3 and caches it
  until the next edit. It generates every chunk on first use (about 7 s in debug builds at the default sliders, well
  under a second in release), which is why `Room::new` pregenerates and why tests that stay
  below tick 30 (the first score update) turn that off.
- The store sits in a `RefCell` because `paint_coverage(&self)` must generate chunks. The type
  is `Send`, not `Sync`: keep it on the thread that steps the world.
- Every path applies edits verbatim (`SimTerrain::apply_edit`, `apply`, `apply_sequence`,
  `replay`). Rock strength is applied earlier, by the simulation (`ls_sim::Ctx::crater` reads
  `SimTerrain::material_at`, which `GameTerrain` implements from the material field), so the
  logged and sent edit is already scaled. `SimTerrain::surface_states_near` is implemented too
  (charged rock drains ships that scrape it, a framework step in `World::step`).
- Surface states: stored as expiry ticks by the edit log (scorch: a permanent bit).
  `surface_states_near(center, radius, tick)` / `surface_states_at(p, tick)` answer for any
  tick. `TerrainQuery::paint_colors_near` answers the paint colours under a ship (ship
  physics: bouncy, slippery, sticky paint). `advance_to(tick)` sets
  the display tick used by `mesh_chunk` (mesh `states`) and marks chunks whose states expired
  dirty; whoever owns the terrain calls it once per tick (server: optional, it does not mesh;
  client: with its display tick).
- Dirty chunks accumulate until `take_dirty_chunks()`; `mesh_chunk(c)` runs surface nets.
- Shell layer (The Squeeze, [contracts.md](contracts.md#shell-layer-the-squeeze)):
  `set_shell` (also `SimTerrain::set_shell`, called by `World::step` every tick) makes
  everything outside the safe sphere solid for `sdf` and meshes; `take_dirty_chunks()` adds the
  chunks the shell's movement since its last call changed (near open space only),
  `mesh_chunk` meshes those the shell reaches from the combined samples. Not logged, not in
  `state_hash()`.
- `state_hash()` = edit count and lasting edits (geometry, paint, the scorch part of surface
  edits) + SDF on a 4 u lattice + every painted or scorched cell (team, colour, scorch). It is
  independent of time (a late joiner's pruned log agrees with the full one), needs no full
  generation and is the desync check between peers. `state_hash_at(tick)` adds the timed
  surface states running at `tick`; the display tick is part of neither.

## Match setup

`match_setup(code, rules, pristine_terrain)`:

1. Room box shrunk by 4 u. Spawn points are placed for at most two sides (`teams.clamp(1, 2)`):
   team 0 in the low-x 30 % slab, team 1 in the high-x slab; free-for-all (`teams: 0`) gets one
   set over the whole level. Teams 2 and 3 of a 3- or 4-team match have no points of their own
   and spawn at any team's (`World::pick_spawn`).
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

4. The Heist site (core vein, extraction zones) comes from the generator
   (`heist_level` -> `MatchSetup::heist`; every rule set gets it, only Heist uses it).

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
| Quantization | `quant_config_for(bounds, weapons)`: level bounds + 2 u, ships ±64 u/s, projectiles ±max(320, fastest weapon) |
| Ship flags | `ALIVE`, `BOOSTING`, `GADGET_ACTIVE` (= `SHIELDED`), team bits, weapon bits = primary weapon id (0-31, bits 10-14) |
| `ShipStatus` | Own ship only; each byte `ceil(value / max * 255)` of hull, energy, boost, scrap |
| Inputs | Thrust/roll to 1/127, look to 1/8192 rad; button bits identical to `ls_sim::Buttons`; `weapon` = `InputCmd::weapon_select` |
| Projectiles | `id` = sim id truncated to u16 (unique among live projectiles: the sim's allocator skips live wire ids), `kind` = weapon id, `owner` = wire id or 255, `state` = `Projectile::net_state` (low byte of `proj_flags`) |
| Entities | `World::entity_views` -> `entity_to_net`: `id` = entity id & 0x7fff, `kind` = `ReplicatedKind`, `visual` = `VisualId`, `owner` = wire id or 255, debris without velocity (see [arsenal-framework.md](arsenal-framework.md)) |
| Inventory | `Message::Inventory` (reliable, own client only, whenever it changes): weapons (id, ammo clamped to u16, level, paid shots `WeaponState::shots`, low byte of `ShipState::weapon_state`), selected primary and secondary, gadget (255 = none) |
| Players | Every connected client: wire id, team, ship type index, gadget id (255 none), name (at most 32 bytes); sorted by id |
| Relevance | `Snapshot::retain_relevant(viewer, &SimRelevance)`: ships hidden by `World::ship_visible_to` (Cloak) are left out, and moving debris more than 100 u from the viewer's ship; the own ship always stays |
| Handshake | `Hello` carries `ls_sim::data_hash()` (build it with `ls_game::client_hello(name, team, ship)`) and an optional ship type; a different hash gets `Reject { reason: DataMismatch }`; optional room and party codes (`Hello::with_room` / `with_party`). `Welcome` carries the ship type, the data hash, the match epoch (`RoomConfig::epoch`) and the room code (`Room::set_room_code`) |
| Ship choice | `Message::SelectShip { ship_type }` (reliable, `ClientSession::select_ship`) sets the type for the next respawn; types the rule set does not allow are ignored |
| Scores | `Snapshot::scores` = the mode view's team scores, then `score_total` (`ls_game::mode::legacy_scores`): Paint the Void with 2 teams gives `[team 0 cells, team 1 cells, surface cells]` (raw counts; percent = cells / surface) |
| Mode state | `Snapshot::mode` = `ls_game::mode::mode_view_to_net(&world.mode_view())` (ids are wire ids; players without one are left out); once ended, `end_tick` is the last simulated tick |
| Results | `Message::MatchResults` once when the mode stops running (`World::results` through `ls_game::mode::results_to_net`: winner and team scores from the mode view, kills and deaths counted by the simulation per ship, rows best first), and after the `Welcome` to clients joining a finished match |
| Lobby | `Ready`, `PickTeam`, `Spectate` are queued as `LobbyRequest`s (`Room::take_lobby_requests`); hosts send `LobbyState` with `Room::send` / `Room::broadcast` |
| Mood (protocol 6) | `NetModeState::mood` = `ls_game::mode::mood_to_net(&view.mood, world.up(), wire)`: the director's mood, event (`EventKind` index), phase (`EventPhase` index), target wire id, active tick, phase end tick, 8 parameter bytes and the level's up |
| Architect | `ArchitectAction` (stroke, centre, radius) is queued per sender player (at most `MAX_QUEUED_ARCHITECT_ACTIONS` between two steps, the rest counted in `RoomStats::dropped_architect_actions`); `Room::step` hands the queue to `World::architect_action` before each tick in arrival order (it validates role, round, kind, radius, range, cooldown, strokes in flight and budget and plans the stroke against `GameTerrain`); accepted and refused ones are counted (`RoomStats::architect_strokes`, `rejected_architect_actions`) |
| Ship ext (protocol 6) | `NetShip::ext` = `ls_game::convert::ship_ext(ship)`: pigmented (`ShipState::pigmented`), hats worn (`ShipState::hats`, Hat rain, B3) |
| Event obstacles (B3) | Nothing on the wire: the predictor computes the running event's kinematic obstacles (Duck parade's ducks) from the replicated mood view, the match seed and its own `MatchSetup` (`Predictor::set_setup`, built from the share code like the server's; `routes` from `ls_game::setup::network_routes`) and pushes the own ship out of them after its movement, as `World::step` does (`ls_sim::obstacle::push_sphere`). Measured: worst reconciliation error 0.0065 u through duck pushes on a lossy link (0.19-0.51 u without the predictor's push; `tests/spectacle_netplay.rs`) |
| Results awards (B3) | `World::results().awards` -> `MatchResultsMsg::awards` (`ls_game::mode::results_to_net`; awards of players without a wire id are dropped); the browser reads them in `Game.results()` as `awards: [{event, player, value}]` |
| Chicken | `NetProjectile::chicken` = `proj_flags::CHICKEN` of the projectile (Rubber chickens) |
| Wells | every `ls_sim::gravity::DynamicWell` of the world as a `ReplicatedKind::Well` entity (`ls_game::convert::well_entities`, layout `ls_net::well_state`), appended to the snapshot's entities |
| Announcements | `Event::Director(Announced)` -> `Cue` `EventAnnounce` (value = `EventKind` index, target = the named player) |
| Edits | Each snapshot carries up to 64 edits starting at the client's acknowledged edit count (`edit_end` of its newest acked snapshot); resent until acked |
| Events | `Damaged` -> `Hit` (pos = impact point), `Killed` -> `Kill` (255 = none), `Respawned`, `MatchEnded`, `Cue` -> `Cue` (kind byte, source, target, pos, value); batched per snapshot, reliable |
| Ping | Send right after the input for tick `c`; `Pong::server_tick - c` measures input slack |
| Match end | Once the mode stops running (timer, or early: kill target, last ship) the server stops stepping the world: ships freeze at the end tick, snapshots continue with `match_ticks_left == 0` and the mode's `end_tick` = the last simulated tick |
| Match transition | A host that starts the next match builds a new `Room` with the next `RoomConfig::epoch` (wrapping) and `join`s every connected client again: each gets the new `Welcome` on its open connection; `ClientSession` rebuilds in place (`Incoming::MatchTransition`; bots then rebuild their brain for the new level, `BotCore::next_match`). Inputs and snapshots of another epoch are dropped (`RoomStats::stale_epoch_inputs`, `SessionStats::stale_epoch_snapshots`; the client checks a snapshot's epoch with `Message::peek_snapshot_epoch` before decoding, since an old delta snapshot's baseline is gone) |
| Match restart | (Older hosts) a host that replaces a finished match may still send `Disconnect { reason: ls_game::MATCH_RESTART_REASON }`; clients rejoin instead of reporting an error |

## Client core

`ClientSession::from_welcome_with(welcome, GameData)` (or the older `from_welcome(welcome,
weapons, tinker_tuning)`, which uses the built-in ships and gadgets) checks the data hash,
regenerates the level from the share code, replays the edit log, parses and resolves the rule
set, and computes the spawn poses and pickup points (`pickup_points()`, which bots use). The
predictor simulates the own ship type from the `Welcome` and follows the players list of each
snapshot when it changes (ship choice at a respawn). `players()`, `player_name(id)` and
`entities()` (interpolated at the render tick like remote ships, `entities_at(tick)`) expose
the new snapshot sections, and `inventory()` the own ship's inventory from the last
`Message::Inventory` (`Incoming::Inventory` when one arrives). While the newest snapshot shows
an own projectile with `GUIDED_STATE_BIT` (Eyeball), the predictor gets the command without
thrust or look, as the server applies it. Per frame the host calls `frame(due_ticks,
controls)` and sends the returned messages; every received packet goes to `handle_packet`.

Protocol 5: `mode()` is the newest snapshot's mode state, `lobby()` the newest
`LobbyState` (kept across matches), `results()` the match results once they arrived
(`Incoming::Lobby` / `Incoming::Results`); `ready`, `pick_team` and `spectate` build the lobby
requests. A `Welcome` on the open connection calls `transition(welcome)`: the session rebuilds
itself from it with the same game data (on error it stays unchanged), keeps its counters (plus
`SessionStats::transitions`), ping nonces and lobby state, and returns
`Incoming::MatchTransition { epoch }`; hosts rebuild what they derived from the old level
(meshes, bot navigation). Inputs are stamped with `epoch()`; snapshots of another epoch are
counted (`stale_epoch_snapshots`) and ignored. A snapshot whose mode is ended freezes the
prediction after the mode's `end_tick`. In The Squeeze, `tick` sets the terrain's shell for the
predicted tick from the mode state (`ls_game::mode::squeeze_shell`, exact: the schedule is a
pure function of the replicated zones and the tick), so the walls the own ship collides with
and the meshes match the server's.

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
  reports only raise the target, so a backlog of snapshots handled at once (a stalled tab) is
  fine and counts as one starvation (`ClientClock::starved`). Before sync, a negative ping
  slack (behind the server altogether; the first pong after slow level generation comes
  before the first snapshot) jumps to the same 12-tick slack; once synced it is an ordinary
  slack sample. Ping slack below 1 tick runs ahead, a minimum above 4 ticks over four pongs
  skips one tick; pongs for pings sent before a starvation catch-up are ignored. The server
  time estimate follows the newest snapshot tick and local time.
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
- **Moods, wells and up (M4, protocol 6)**: `Predictor::set_mood(view, up)` takes the
  replicated mood view (`ls_game::mode::mood_from_net`; target ids are wire ids, the ids the
  client gives its own ship) and the level's up before each reconciliation; every predicted tick
  applies the running event's movement effects (`ls_sim::director::move_effects` with the match
  seed, `apply_move_effects_before` / `_after`) around the movement, exactly as `World::step`
  does, with the view advanced to the predicted tick (`MoodView::advanced`: an event's phases
  start and end on the server's tick even when the newest snapshot is older), the level's up
  of the event (Bored level's turn is a pure function of the view, `EventEffects::up`) and the
  wells dimmed by `EventEffects::well_scale` (Gravity coffee break). `Predictor::set_wells` takes the level's well field: the static wells of the match setup
  plus the dynamic wells rebuilt from the newest snapshot's well entities; the movement runs
  `simulate_movement_env` with the clamped pull at the ship and the level's up. Well entities
  carry their exact start and end ticks: a telegraphed well starts on its tick, and a changed
  well list makes the next reconciliation replay the stored inputs from the anchored state, so a
  well the client learns of after it started pulls from its exact start as on the server. `ClientSession::mood()`,
  `well_sites()`, `belt()` expose the rest; `architect_action(...)` builds the stroke message.
  Test: `crates/ls-game/tests/mood_netplay.rs` (reconciliation error below 0.05 u through forced
  events, a turned up, a well field and dynamic wells starting and ending mid-flight) and
  `crates/ls-game/tests/world_events_netplay.rs` (B1's eight world and movement events with 4
  clients: no desync, reconciliation error below 0.1 u).
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
  knows the loadout secondary plus any weapon kind seen on an own projectile. Alternating and
  rotating patterns (Scatter Laser, Corkscrew) take their phase from the inventory's shot
  counts (`ShotPredictor::on_inventory`: server count plus the shots predicted after the
  inventory's tick; volleys predicted after it with another phase are rebuilt); nothing is
  inferred from projectiles.
- **Own Leech Arc beam** (`crates/ls-wasm/src/beam.rs`, `Game.localBeam()`): the predictor
  records its Leech Arc arcs (`ShotPredictor::last_beam`); while one fired within the fire
  interval the core aims the beam like the weapon module (nearest enemy in the 30° cone within
  12 u with line of sight, lock kept in the 60° cone, else grounded on the rock ahead or full
  range) on the ships as drawn, so the own beam shows with the click instead of a round trip
  later. The server's own `Beam` cues are not passed on (they only correct the lock).
  `ChargeStart` cues carry the charging weapon in `target` for the charge-up effect.
- **Paint Thrower colour**: from the inventory's weapon state byte (a V press guesses ahead
  until an inventory at least as new as the press arrives) and the `PaintColor` cues, so a
  client that missed the cues (a rejoin, a transition) still shows the server's colour.
- **Modes, lobby, transitions (protocol 5)**: `Game.modeState()`, `lobbyState()` and
  `results()` return JSON (`crates/ls-wasm/src/modes.rs`), `readyPacket`, `pickTeamPacket`,
  `spectatePacket` and `helloPacketWith` (room and party codes) build packets. A match
  transition rebuilds the online core in place (inventory, shot predictor, chunk streamer,
  per-match HUD state; ping state carries over); `takeMatchTransition()` reports it once and
  `takeUnloads` first returns every chunk of the previous level. The loadout HUD carries each
  weapon's shot count and state byte (`shots`, `states`).
- **M4 level features (protocol 6)**: `modeState()` JSON carries `mood` (`modes::mood_json`:
  mood and event ids, phase, target, ticks, parameters, up); `eventIds()` lists the events;
  `shipExt()` gives `[id, pigmented, hats]` per ship; `architectActionPacket(stroke, x, y, z,
  radius)` builds the stroke packet; `beltSite()` returns the belt as JSON (`level::belt_json`:
  centre, radii, gate azimuths, slots, pickup points, boulder orbits) and `boulderPoses(tick)`
  the drifting boulders at a fractional tick from the simulation's orbit functions;
  `wells()`, `up()`, `monochrome` and `doseRateAt(x, y, z)` give the well field, the level's
  up, the monochrome flag and the radiation dose rate for the HUD meter.
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
   (`SnapshotHistory::choose_baseline`), plus pending events. A client gets snapshots only
   once its first `Input` arrived: until then it is still generating its level (~0.25 s for a
   bot, longer in WASM), nothing can be acknowledged, and every snapshot would be full and
   handled in one burst later (M1 issue B: ~8 full snapshots, starvation reports and
   prediction snaps per join). So a join costs one full snapshot: after it the client gets
   no snapshot until it acknowledges one or `FULL_SNAPSHOT_RETRY_TICKS` (30) passed
   (test `one_full_snapshot_until_acknowledged_or_retry`); the inventory still goes out;
4. sends `take_outgoing()` (`Outgoing { client, channel, bytes }`; a host may send a too-large
   unreliable packet reliably, as `ls_net::send_message` does).

Team assignment (team modes): the preferred team unless it would put that team two or more
players ahead of the smallest team, otherwise the world's choice (the smallest team, ties to
the lowest index); free-for-all ignores the preference. Inputs are queued per client by tick (duplicates
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
  step so that after its own movement it sits exactly at the rewound position: the movement is
  previewed with `World::preview_movement` (M4), which moves a ship exactly as the coming step
  does (mode limits, the running event's movement effects, the gravity wells, static and
  dynamic, the level's up), so targets in a well or under a turned up land on the rewound
  position too (`server::tests::lag_compensation_previews_targets_in_gravity_wells`,
  `tests/netplay.rs::lag_compensation_hits_a_target_pulled_by_a_gravity_well`). `World::step`
  then resolves the hit with its normal rules (damage, friendly fire, knockback, kill credit,
  events) and the room moves the target back (`World::shift_ship`, which also corrects the
  newest Borrowed Time trail sample). Ships the step teleported keep their new place: after a
  Swap Harpoon swap (`CueKind::Swapped`) the two ships exchange their corrections (each now
  stands where the other was, so it takes the other's offset), and a Borrowed Time rewind
  (`CueKind::Rewound`) needs none (`restore_offsets` in `ls_game::server`).
- Guards: no displacement when the target fires or scavenges this tick, when a pickup is in
  reach of the rewound position, or when its movement would differ there (terrain contact).
  Projectiles are not compensated on the tick they are fired. Splash from other explosions in
  the same tick sees the displaced position, and a kill leaves its wreck where the shooter saw
  the target.
- Comedy events (M4): each history frame also keeps that tick's hit rules
  (`EventEffects::hit_radius_scale` of Big head day, `hit_plane` of Wrong genre), so a rewound
  target is tested with the hit sphere and depth rule of its tick, and the simulation checks
  with the coming step's (`World::next_event_effects`).
- `lag_compensated_raycast` offers the same rewind for hitscan validation.

`ls-sim` has no hook for per-projectile target positions; a `World::step` variant that accepts
rewound target positions for projectile tests would remove the displacement trick.

## Tests

| Test | Shows |
| --- | --- |
| `tests/netplay.rs::prediction_converges_snapshots_decode_and_terrain_replicates` | Room + 4 clients over `MemoryTransport` with 2-5 tick latency (one with 10 % loss, reordering and duplication, one joining late): snapshots decode (deltas after the first), craters and paint replicate so every client's `state_hash` equals the server's, the prediction for the newest snapshot tick is within 0.02 u of the server's exact state, no corrections or snaps during the last 1.5 s for the perfect links, remote ships interpolated near the server's past positions |
| `tests/netplay.rs::lag_compensation_hits_target_that_moved_after_the_shot` | 5-tick latency each way; the target starts climbing as the shot is fired. Without compensation the shot misses on the server, with it the rewound target takes a direct hit |
| `tests/netplay.rs::slow_setup_client_syncs_within_a_few_round_trips` | A client that builds its session 240 ticks after the `Welcome` synchronizes within 3 round trips |
| `tests/netplay.rs::a_join_costs_one_full_snapshot_and_starves_once` | M1 issue B regression: bots with 250-350 ms level generation on 1-tick links, joining, rejoining with reused wire ids and after a match restart (fresh room): exactly 1 full snapshot per join (at most `1 + (RTT - 1) / 2` on an 8-tick RTT), at most 1 starvation report and 1 prediction snap, and none of the three after the join settled; a few late inputs per client while joining (the catch-up) and none after the join settled |
| `tests/squeeze.rs` | The Squeeze on generated levels: the client derives the server's shell from snapshots at every tick; walls close in every phase on 8 seeds, deterministically, never with a living ship inside the shell (release); shell remeshing within the frame budget on 8 seeds, printing per-chunk cost, dirty counts and backlog (release); the default rule set runs a full 8-player match in a headless `Room` to a result (release) |
| `tests/transition.rs` | Three match transitions on the same connections (local, 3-tick and lossy reordering links): every session rebuilds in place, one full snapshot per transition on a local link (on longer links the ones sent before the first acknowledgement, as for any join), terrain equal to the server's, results and lobby state received, synced again, prediction converged; inputs and snapshots of another epoch are dropped on both ends |
| `server::tests` | epoch and room code in `Welcome`, lobby requests queued, stale-epoch inputs dropped, `send`/`broadcast`, shot counts and state bytes in the inventory, mode state in snapshots, results once at an early end and to late joiners, join/leave/team balance, full room, late/duplicate/missing inputs (repeat last), no missing inputs before the first input, no snapshots before the first input, 30 Hz snapshots with deltas after acks, 250 ms history, `leave` drops queued packets before the id is reused, per-command rewind, unencodable `Welcome` rejects without a ghost ship, inventory messages only on change, far moving debris culled and sent without velocity, guided-projectile prediction and entity interpolation in a `ClientSession` |
| `client::*::tests` | input redundancy, prediction without corrections against a delayed server, misprediction correction and smoothing, replay of inputs sent before the first snapshot, frozen prediction after the match end, interpolation, clock (including catching up from far behind, a snapshot backlog counting as one starvation) |
| `ls-wasm` `game::tests::online_client_under_latency_applies_look_exactly_once` | The browser core over a 4-7 tick jittery link with a delay spike (clock catch-up) and uneven frames: synced, converged, every mouse delta sent exactly once, `Disconnect` reason reported |
| `ls-bots` `tests/room_play.rs` | A full room of 8 bots plays a (60 s) match to its end and 5 s beyond: no desyncs or protocol errors, terrain identical to the server's, no corrections after the match end |
| `setup`, `terrain`, `convert` tests | deterministic setup in open space, steppable world, edits/dirty chunks/gaps/coverage, round trips |

The whole crate tests in about 40 s in debug builds (dominated by level generation) and about
20 s in release (where the match setup sweep runs all 8 seeds).
