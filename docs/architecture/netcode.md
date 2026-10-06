# Netcode

An authoritative server runs the simulation at 60 Hz. Clients send input commands, predict
their own ship with the same `ls-sim` code, interpolate everyone else, and rebuild the level
locally from the share code plus a replicated log of terrain edits. This page is the
end-to-end picture; the exact rules and numbers live next to the code:

- `crates/ls-net` (wire format, `Message`, snapshots, `Transport`);
- [game-glue.md](game-glue.md) (`ClientSession`, `Room`, the [wire
  conventions](game-glue.md#wire-conventions) table, lag compensation);
- [arsenal-framework.md](arsenal-framework.md#replication-ls-net-protocol-4) (what the M2
  wire items mean: entities, players, cues, inventory, ship choice, data hash);
- [server.md](server.md) (sockets, WebSocket and WebTransport mapping, backpressure);
- [modes-and-lobby.md](modes-and-lobby.md) (what the M3 mode state, lobby and transitions are
  for: modes, the lobby state machine, rooms, parties, duels).

The wire format is versioned by `ls_net::PROTOCOL_VERSION` (checked in `Hello`): 1 was M1;
2 added the M2 entities, players, cues, data hash and ship choice; 3 predictive ship deltas;
4 paint colours in paint edits, `Welcome::edit_count` (pruned edit logs) and predictive
projectile deltas; 5 (M3) the generic mode state in snapshots, match transitions on an open
connection (`Welcome` with a match epoch, carried by `Input` and `Snapshot`), the lobby and
results messages, room and party codes in `Hello` and weapon shot counts and state bytes in
`Inventory` (see [Match transitions, lobby and mode state](#match-transitions-lobby-and-mode-state-protocol-5));
6 (M4) the mood view, ship `ext` bits, the rubber-chicken projectile bit, `ArchitectAction`, new
entity kinds and cues, and the reserved race section and well entity layout (see
[Protocol 6](#protocol-6-moods-the-architect-belt-up)).
This page describes version 6.

## Tick model

| Item | Value | Where |
| --- | --- | --- |
| Simulation rate | 60 Hz, `DT = 1/60` (`ls_math::TICK_HZ`, `Tick(u64)`) | `ls-math` |
| Snapshot rate | every 2nd tick, 30 Hz (`RoomConfig::snapshot_interval = 2`) | `ls-game` server |
| Wire tick | `u32` | `ls-net` |
| Server scheduler | deadline based; bursts of up to 4 ticks to catch up, at most 250 ms backlog, the rest counted as skipped | `ls-server/src/timing.rs` |

Server step `k` (`Room::step`): take every client's input stamped `k` (or repeat its last
command), plan lag compensation, run `World::step`, record ship positions for rewinds, and
on snapshot ticks queue one snapshot per client that has sent an input (a client still
generating its level gets none; see [Snapshots and deltas](#snapshots-and-deltas)).
`Snapshot::tick` is the tick after that step.
Once the match timer runs out the room stops stepping the world but keeps sending snapshots.

The client runs its own 60 Hz clock ahead of the server: its first command is stamped
`Welcome::server_tick + 4` (`INITIAL_LEAD_TICKS = 3`, plus one), so the command for tick `k`
arrives before the server steps `k`. `ClientClock` keeps that lead right:

- a snapshot with `ack_input < tick` means the server ran out of our input (starvation), so the
  client runs ahead by the missing ticks (at most 30 per report);
- a client that is not ahead at all (for example after slow level generation in WASM) jumps
  to `tick + 13` in one go;
- a `Ping` sent right after the input for tick `c` comes back as a `Pong` with `server_tick`;
  `c - server_tick` is the input slack. Below 1 tick the client runs ahead, a minimum above 4
  ticks over four pongs skips one tick. A negative slack before the clock is synced (the
  client is behind the server altogether, typically the first pong after slow level
  generation, which arrives before the first snapshot) jumps to a slack of 12 ticks like the
  catch-up above, instead of just to the bottom of the window. Once synced, a negative slack
  (one late input) is an ordinary slack sample. Pings go out every 30 ticks.

Until a snapshot proves the server had our input in time (`ClientClock::is_synced`), the
client sends idle commands: look input is a per-tick delta, and a command the server replaced
by a repeat would leave a permanent orientation difference.

## Input and redundancy

- `InputCmd` per tick: thrust axes and roll (quantized to 1/127), look deltas (1/8192 rad),
  button bits (`ls_sim::Buttons`, wire `ls_net::buttons`: 0 boost, 1 fire primary, 2 fire
  secondary, 3 scavenge, 4 overdrive, 5 horizon assist, 6 gadget (G), 7 weapon mode (V)) and a
  weapon selection (`0` keeps the selection, `n` selects the carried weapon with id `n - 1`
  into its slot). The client quantizes each command exactly as the server will decode it, so
  both simulate the same numbers.
- G and V act on their press **edge** in the simulation. The browser core (`ls-wasm`
  `EDGE_BUTTONS`) keeps a press from a frame that ran no tick for the next tick, so a tap
  shorter than a frame still reaches the server. Charge weapons fire on the release of the
  fire button, so a held button must stay set in every command until release.
- Every `Input` message carries the newest **three** commands (`MAX_INPUT_CMDS`), newest first,
  plus `ack_snapshot`, the newest snapshot the client stored (the server's delta baseline). It
  goes on the unreliable channel; a lost datagram costs nothing as long as one of the next two
  arrives.
- The server queues commands per client by tick: duplicates are counted and ignored, commands
  for ticks already simulated are late and dropped, commands more than 120 ticks ahead are
  dropped. A missing command is replaced by the last one (`missing_inputs` counts it once the
  client has sent its first input).
- `Snapshot::ack_input` reports the newest *real* input tick applied, which drives the clock
  and tells the predictor what the server has seen.
- The browser sends inputs as droppable packets: while the socket's send buffer is backed up
  (16 KiB of WebSocket `bufferedAmount`) it skips them, relying on the redundancy.
- Before sync a frame that catches up many ticks sends only the newest 8 `Input` messages
  (`MAX_BURST_INPUTS`).

## Prediction and reconciliation (own ship)

`ls_game::client::Predictor` runs `ls_sim::simulate_movement` (the server's movement code)
on the client's copy of the terrain for every command and keeps the state after each tick.
When a snapshot brings the server's state of our ship for tick `T`, the prediction for `T` is
compared within quantization tolerances: a match keeps the full-precision prediction; a
mismatch takes the server's value for the components out of tolerance, re-simulates the stored
commands `T+1..now` (reconciliation) and turns the jump into a decaying render offset; a
respawn or a large error snaps. After the match end tick prediction freezes, exactly like the
server. Tolerances, smoothing and snap rules:
[game-glue.md, Client core](game-glue.md#client-core).

The predicted movement includes everything `simulate_movement` models: paint colours under
the ship (red rebounds, blue keeps speed, yellow slows; the client's terrain has the same paint
from the edit log), charged-rock contact, and the freeze while guiding an Eyeball (below).

Shots: the browser also predicts its own projectiles (`crates/ls-wasm/src/shots.rs`,
`ShotPredictor`) with the simulation's firing rules, draws them at once, and hands over to the
server's projectile when a snapshot shows a match (the difference fades as a render offset).
Unmatched predictions disappear after 4 ticks. Each weapon has a `ShotModel` mirroring its
module's `fire`: the generic pattern, the Scatter Laser's alternating fan and the Corkscrew's
helix (their phase from the server's shot counts in the inventory, protocol 5), the Scrap Shotgun's centre pellet
only (its cone is random), and nothing for the Leech Arc (instant; its beam is drawn locally
from the predicted arcs, `crates/ls-wasm/src/beam.rs`), Borrowed Time and the Prox Mine (the
cost is still predicted). Charge weapons are predicted on release; Borrowed Time
takes the weapons offline locally as it fires. Bots do not predict shots.

## Interpolation (other ships and projectiles)

- Remote ships are drawn at `server time - 6 ticks` (100 ms, `INTERP_DELAY_TICKS`) from the
  buffered snapshots: position and velocity lerped, orientation nlerped. Gaps larger than 8 u
  (respawns) are not interpolated; at most 6 ticks of extrapolation when snapshots run late.
- Projectiles are straight-line extrapolated from the newest snapshot to the client's
  server-time estimate (at most 8 ticks).
- Generic entities (debris, pickups, decoys, ...) are drawn at the same render tick as remote
  ships (`ClientSession::entities`): the entity set of the first buffered snapshot at or after
  it, positions lerped from the one before (debris carries no velocity on the wire, so it
  never runs ahead of the ships that push it).
- While the own ship guides a projectile (`proj_flags::CONTROLS_OWNER`, Eyeball) the server
  gives it no thrust or look; the client predicts that too (`GUIDED_STATE_BIT` in the newest
  snapshot's own projectiles), so guided flights do not mispredict.
- The server's lag compensation assumes the same 6-tick delay when it rewinds targets.

## Snapshots and deltas

Wire layout (`crates/ls-net/src/snapshot.rs`): `tick`, optional baseline (as a `u8` age),
`ack_input`, `match_ticks_left`, terrain edits (`edit_base`, count, edits), scores, ships
(a 32-bit presence mask, then each ship), projectiles (17 bytes full: id, kind, owner, state
byte, position, velocity), generic entities (debris, pickups, scrap, wrecks, decoys, ghosts,
dust: kind, visual, owner, team, position, velocity and rotation with presence bits, 4 state
bytes), the players list (names, teams, ship types, gadgets) and (protocol 5) the mode state
(below). Projectiles and entities are
**id lists**: in a full snapshot a count, then per item its id as a varint gap from the
previous one and its full state; in a delta one bit per item of the baseline's list ("present
and unchanged", so resting debris, pickups and wrecks cost 1 bit), then for the others 1 bit
"still present" plus field diffs (entities: a "position only" fast path), then the new items
as an id list (new projectiles predicted from their owner, see below). Players in a delta: 1
bit "changed", then a 32-bit presence mask and a
change bit per known player; only changed or new players are sent. Entity positions are
`ENTITY_POS_SHIFT` (3) bits coarser than ship positions (about 0.03 u), so drifting debris
moves few steps between snapshots. The entity and player sections are unchanged since protocol 4; what their fields mean
is in [arsenal-framework.md](arsenal-framework.md#replication-ls-net-protocol-4). The server
filters each client's snapshot through `ls_net::Relevance` first (a cloaked enemy is left
out; moving debris farther than 100 u from the viewer's ship too, `DEBRIS_RELEVANCE_RADIUS`)
and sends debris without velocity.

The own inventory (carried weapons, ammo, levels, selection, gadget) is not in snapshots: the
room sends `Message::Inventory` reliably to each client whenever its ship's inventory changes
(checked on snapshot ticks).

- A full ship is exactly **18 bytes** (`SHIP_CORE_BITS = 144`): position 3 x 16 bit inside the
  quantization box (level bounds + 2 u), smallest-three orientation in 32 bits, velocity
  3 x 16 bit (±64 u/s), 16 flag bits (alive, boosting, gadget active, team, selected primary
  weapon id in 5 bits).
  Flag bit 15 tells the decoder a 4-byte `ShipStatus` (hull, energy, boost, scrap, each
  `ceil(value / max * 255)`) follows; only the receiving client's own ship carries it.
- **Deltas.** The server keeps a `SnapshotHistory` per client (128 snapshots, 256 ticks) and
  encodes against the client's newest acknowledged snapshot (`choose_baseline`: age 1..=255
  ticks and still in history, else a full snapshot). Fields are compared as packed integers,
  so a delta decodes to exactly the packed snapshot the server built. Ships in the baseline
  (protocol 3) send a change bit plus residuals against integer predictions: position against
  `base.pos + base.vel x ship_motion x age`, velocity against the baseline velocity or
  `2 x displacement / (ship_motion x age) - base.vel` (1 bit picks the cheaper), orientation as
  smallest-three field differences while the dropped component stays the same (else the raw
  32 bits); flags and status keep their change bits. Residuals go in groups of three sharing
  one width, coded against a typical width per section (position, velocity, rotation; see
  `crates/ls-net/src/snapshot.rs`). `ship_motion` (packed position steps per velocity step per
  tick, 2^-20 fixed point, `Quantizer::ship_motion_scale`) is sent in full snapshots and in
  deltas only when it changes; all prediction math is integer, so native and WASM decode the
  same bits. New ships are sent in full; drifting entities use the same residual groups for
  position; projectiles and entities use the id lists above. The client keeps the same
  128-entry history, so every baseline the server may choose is available.
- **Projectile deltas (protocol 4).** A projectile in the baseline counts as unchanged (1 bit)
  when it is exactly where `base.pos + base.vel x projectile_motion x age` puts it with nothing
  else changed; otherwise a meta bit (kind, owner, state), a position residual group against
  that prediction and a velocity residual group against the baseline velocity, with their own
  typical widths. `projectile_motion` (`Quantizer::projectile_motion_scale`) travels next to
  `ship_motion` (only snapshots that carry projectiles fill it). Before encoding, the server
  **settles** each client's snapshot (`ls_net::settle_projectiles`): a projectile within
  `SETTLE_TOLERANCE` (4) position steps (about 0.016 u horizontally, 0.008 u vertically) of its prediction is sent as
  predicted, and that settled snapshot is what both ends store, so a straight shot costs one
  bit per snapshot and its position is never more than 4 steps off. A projectile new against
  the baseline is predicted from its owner: owner (6 bits below 32), velocity as a residual
  against the owner's newest projectile already in the list (same kind and state; else kind,
  state and the raw velocity), position as a 5-bit flight time in quarter ticks plus a
  residual against `ship.pos + (vel - ship.vel) x time` (the muzzle offset folds into the
  flight time); id gaps below 16 cost 5 bits.
- **Joins.** Snapshots to a client start with its first `Input` (sent once its session is
  built), so a join costs one full snapshot; the client gets no further snapshot until its
  `ack_snapshot` arrives or `FULL_SNAPSHOT_RETRY_TICKS` (30 ticks, 0.5 s) passed (then another
  full one: the first may have been lost). After that every snapshot is a delta. Until M3 a
  full snapshot went out every snapshot interval until the first acknowledgement, so a slow
  round trip or a client busy after a transition cost 2 or more (1.02-1.09 per join or
  transition in C3's e2e runs on a busy machine; milestone-3-plan.md "Milestone soak"). Sending
  snapshots during level generation made each join cost ~8 full snapshots, starvation reports
  and prediction snaps (M1 issue B; regression test
  `a_join_costs_one_full_snapshot_and_starves_once` in `crates/ls-game/tests/netplay.rs`).
- Budget: 32 ships x 18 B x 30 Hz = 17,280 B/s (576 B per snapshot).
  `bandwidth_32_ships_at_30_hz` in `ls-net` proves full snapshots fit before delta
  compression. `bandwidth_32_ships_64_debris` (ls-net) and `busy_fight_fits_the_bandwidth_budget`
  (ls-game, real ship physics via `simulate_movement`) measure 32 busy ships (thrusting,
  turning up to 4 to 9 rad/s), 64 drifting debris and 48 resting items: whole deltas of about
  460 / 520 / 557 B against baselines 2 / 4 / 6 ticks old (up to 16.7 KB/s), inside the
  budget; protocol 2 needed about 515 B for the busy ships alone.
  `busy_fight_with_projectiles_fits_the_budget` (ls-net) adds a busy fight's projectiles: every
  ship fires in bursts (60 % of the time) with a weapon mix from the data (lasers, rivets,
  plasma, scatter fans, homing Seekers, stuck mines), about 100 projectiles in flight and 4 new
  per snapshot, settled as the server does; the test runs four seeds. Whole deltas average
  392-405 / 476-491 / 544-560 B against baselines 2 / 4 / 6 ticks old (at most 12.2 / 14.7 /
  16.8 KB/s), inside the budget at every age. The busiest snapshots reach 471-482 / 565-612 /
  614-683 B: within 17,280 B/s at 2 ticks, up to about 6 % over at 4 ticks (about 66 ms round
  trip) and up to about 19 % over at 6 ticks (100 ms). **Known shortfall (M2):** the busiest
  snapshots at 100 ms round trip exceed the per-snapshot budget. The projectile section is
  90-102 / 140-155 / 186-202 B (protocol 3: 580-632 / 672-737 / 734-796 B). New projectiles
  dominate it: a baseline 6 ticks old repeats about 12 of them at about 10 B each, most of it
  the velocity residual against the owner's previous shot (about 43 bits) and the position
  residual against the owner's ship (about 28 bits). Cutting it needs a velocity predictor from
  the owner's orientation or coarser new-projectile velocities (open).
  More moving debris (a Tectonic Charge jostles every piece) on top of a busy fight also goes
  over. In practice a 3-bot local run measures about 4 KiB/s of snapshots for the whole
  server.

## Relevance filter

Before a client's snapshot is packed, the room removes what that client may not or need not
see: `Snapshot::retain_relevant(viewer, &impl ls_net::Relevance)`, with one callback per ship,
projectile and entity (default: keep). The room's implementation (`SimRelevance` in
`crates/ls-game/src/server/mod.rs`):

| Item | Rule |
| --- | --- |
| Own ship | always kept |
| Enemy ship with an active gadget | the gadget's `GadgetBehavior::visible_to` decides (`World::ship_visible_to`). The Cloak hides the carrier from enemies unless they are within `reveal_distance` (6 u) or it fired within `reveal_after_fire_secs` (0.5 s); teammates always see it (and draw a shimmer) |
| Debris | resting pieces always; moving pieces only within `DEBRIS_RELEVANCE_RADIUS` (100 u) of the viewer's ship. Debris goes out without velocity |
| Projectiles, other entities, players | always kept (a cloaked ship's shots are visible; firing also reveals it) |

The filter runs before delta encoding and projectile settling, and the filtered snapshot is
what goes into the client's history, so a ship that turns invisible simply leaves the ship
presence mask of the next delta (so it is no longer drawn) and reappears as a new, fully sent
ship. Lag compensation works on the server's world, not on what a
client was sent.

## Events, cues and other messages

Events (hits, kills, respawns, match end, and game cues) are batched per snapshot
into an `Events` message on the reliable channel and sent to every client (no distance
filter: level-wide cues such as `TectonicLaunch` and `Shockwave` rely on it). A cue
(`NetEvent::Cue`, from `ls_sim::Cue`) carries a `CueKind` byte, source and target ship ids, a
position and a 16-bit value; the kinds are Polite warning, blinding, charge start, Tectonic
launch and armed, shockwave, Leech beam, swap, rewind, muzzle flash, gadget, decoy return,
overcharge and paint colour (`crates/ls-sim/src/behavior.rs`; values in
[arsenal-framework.md](arsenal-framework.md#replication-ls-net-protocol-4)). The browser turns
them into effects and HUD state (`web/src/game/mechanics.ts`); some also correct client
guesses: the `PaintColor` cue confirms the colour the client guessed on V, and the `Gadget`
cue's value (`ls_sim::gadget_cue`: on, off by key, out of energy, instant use) starts the
client's cooldown estimate only for a key-press off or an instant use. Bots use `Swapped` and
`Rewound` to tell a teleport from a desync.

| Message | Channel | Direction | Contents |
| --- | --- | --- | --- |
| `Hello` | reliable | client to server | magic, protocol version, name, preferred team, data hash, optional first ship type, optional private room code (`R-..`) and party code (`P-..`) |
| `Welcome` / `Reject` | reliable | server to client | see the handshake in [overview.md](overview.md#from-share-code-to-rendered-frame); the `Welcome` also carries the match epoch and the room code (empty for the public room) and comes again on the open connection at every match transition. `Reject` has a frozen layout and a reason (`DataMismatch` names both hashes; an unknown room or party code or a closed room is `Other` with a message such as "unknown room R-..", "room closed") |
| `Input` | unreliable | client to server | newest 3 commands, `ack_snapshot` |
| `Snapshot` | unreliable (reliable when too big) | server to client | above |
| `Events` | reliable | server to client | events and cues of one snapshot interval |
| `Ping` / `Pong` | unreliable | both | clock sync and round trip |
| `SelectShip` | reliable | client to server | ship type for the next respawn |
| `Inventory` | reliable | server to client | the own ship's weapons, ammo, levels, shot counts, module state bytes, selection and gadget, when they change |
| `LobbyState` | reliable | server to client | lobby phase and its ticks left, room code, next mode, rule set and world, player range, teams, players (team pick, ready, spectator) |
| `MatchResults` | reliable | server to client | once when the mode ends (and to players joining afterwards): winner side, team scores, rows (kills, deaths, score, name) |
| `Ready` / `PickTeam` / `Spectate` | reliable | client to server | lobby requests (ready for the next match, team for the next match or auto, spectate from the next match on), queued for the host's lobby (`Room::take_lobby_requests`) |
| `Disconnect` | reliable | both | reason |

## Match transitions, lobby and mode state (protocol 5)

- **Mode state.** The last section of every snapshot is the generic mode state
  (`ls_net::NetModeState`, the wire form of `ls_sim::ModeView`): mode kind, teams, phase and
  winner, timer end, stage and its end, team scores, the paintable surface, the score target,
  view flags, per-player rows (wire id, side, score, lives, meter, flags; the top 32 scores)
  and up to 8 objective markers (id, kind, team, holder, state byte, timer end tick, radius,
  position). A full snapshot sends it in full (1 bit when empty), a delta 1 bit "unchanged" or
  per section (header, team scores, players with a presence mask and per-player change bits,
  objectives with meta and position diffs); see `crates/ls-net/src/mode.rs`. Once the mode
  ended, `end_tick` is the last simulated tick and `match_ticks_left` is 0, so clients freeze
  their prediction exactly where the server stopped (also for modes that end early).
- **Match transition.** A host that starts the next match sends every connected client a new
  `Welcome` on the open connection (a new `Room` per match, `RoomConfig::epoch` incremented).
  `ClientSession` rebuilds itself in place (level, rule set, terrain, predictor, clock,
  snapshot history; counters, ping nonces and the lobby state carry over) and reports
  `Incoming::MatchTransition`. Inputs carry the `Welcome`'s epoch and snapshots the room's;
  both ends drop packets of another epoch (unreliable packets of the old match still in
  flight). The new room sends snapshots only after the client's first input of the new
  epoch, so a transition costs one full snapshot like a join
  (`crates/ls-game/tests/transition.rs`).
- **Lobby and results.** The room queues `Ready`, `PickTeam` and `Spectate` for its host
  (`Room::take_lobby_requests`); the host sends `LobbyState` (`Room::send` /
  `Room::broadcast`). `MatchResults` goes out once when the mode ends (kills and deaths are
  tallied from the simulation's kill events) and to clients joining a finished match.
- **Hello.** Optional room code (private rooms) and party code (`P-XXXXXX`); the `Welcome`
  echoes the room code. Everything else of the lobby features (presence, parties, custom
  matches, duel challenges, the server list) goes over HTTP: the game server's JSON API and the
  directory ([modes-and-lobby.md](modes-and-lobby.md#identity-parties-invite-links-duels)), not
  protocol messages.
- **Epochs.** `Welcome::epoch` is a `u8` that changes with every room on a connection (the
  server gives each new room the previous one's epoch + 1, wrapping). `Input` carries the
  client's epoch, `Snapshot` the room's; `Message::peek_snapshot_epoch` reads it before the
  delta decode, so a delta against a baseline of the old match is dropped instead of failing.

## Protocol 6: moods, The Architect, Belt Up

Added in M4 wave A (`docs/milestone-4-plan.md` §4 "Protocol"); wave B packages add their
items inside version 6 against this layout. Codecs: `crates/ls-net/src/mode.rs` (mood, race),
`wire.rs` (`ship_ext`, `proj_wire`, `well_state`), `snapshot.rs` (ship trailer), `message.rs`
(`ArchitectAction`).

| Item | Wire | Cost |
| --- | --- | --- |
| Mood view (`NetModeState::mood`, `ls_net::NetMoodView`) | after the objectives in the mode state: 1 bit "present" (full) or "changed" (delta), then u8 mood, u8 event (`EventKind` index, 255 none), u8 phase (`mood_phase` = `ls_sim::EventPhase`: announce, active, ending), u8 target (wire ship id, 255 none), varint active tick and phase end tick, 8 parameter bytes, `up` as 1 bit "+Y" or 3 raw `f32` | 1 bit per delta while unchanged; about 30 B when it changes (+12 B while `up` is turned) |
| Architect role | `mode_player_flags::ARCHITECT` (bit 3) on the architect's mode player row; `meter` = stroke budget (others: their architect points); `FINISHED` (bit 4) on a runner through the exit; objectives `EXIT` 5 and `STROKE` 6 (see contracts.md "Game data ids") | none |
| Ship `ext` (`NetShip::ext`, `ls_net::ship_ext`) | bit 0 pigmented, bits 3-7 hats (0-31, saturating). Full ship: flag bit 15 now means "trailer": 1 bit status [4 B], 1 bit ext [u8]. Delta: the flags change bit covers flags and ext (u16 + 1 bit [u8]) | 0 for ships without pigment or hats (still 18 B); 10 bits for one with them |
| Rubber chicken (`NetProjectile::chicken`, sim `proj_flags::CHICKEN`) | bit 7 of the projectile kind byte (`proj_wire::CHICKEN`; weapon kinds stay below 128) | none |
| `ArchitectAction` (client to server, reliable, tag 17) | u8 stroke kind (`the_architect::StrokeKind`), centre and radius as raw `f32`; the server's world validates it (`World::architect_action`, refusals counted by the room) | 17 B |
| Entity kinds (`ls_sim::ReplicatedKind`, append only) | 10 pigment pickup, 11 drone (`state[0]`: 0 tax, 1 pizza, 2 survey), 12 duck, 13 hat, 14 cat paw, 15 gossip face, 16 disco ball, 17 mimic, 18 well | as any entity |
| Cues (`ls_sim::CueKind`) | 14 `CaveInWarning` (pos, value = ticks to the fall) and 15 `Pigment` (A4), 16 `EventAnnounce` (value = `EventKind`, target = named player, pos = the tell; `ls_game::convert` also emits it from the director's announcement), 17 `StrokeTelegraph` (source = architect, pos, value = stroke << 8 \| radius) | as any cue |
| Well entities (kind 18) | every `ls_sim::gravity::DynamicWell` (Paperweight, Architect well) as an entity built by `ls_game::convert::well_entities`: id `0x8000 \|` key, position = centre, `state[0]` range in 0.25 u, `state[1]` peak pull in 0.125 u/s^2, `state[2..4]` low 16 bits of the end tick, `visual` low 16 bits of the start tick (both decoded nearest to the snapshot tick, so the entity never changes while the well lives; `ls_net::well_state`). The predictor rebuilds the wells: a telegraphed well starts on its tick, and a changed well list replays the stored inputs from the anchored state, so a well learned of late pulls from its exact start. Exact when the simulation creates wells on that grid (`well_state::quantize_range`, `quantize_peak`); clients draw wells from `Game.wells()`, not as entities | as any entity |
| Race section (B6, Belt Up) | reserved: 1 bit after the mood (encoder writes 0, decoder rejects 1 until B6). Layout for B6: u8 heat, u8 heats, u8 heat phase, varint phase end tick, u32 racer mask, per racer: u8 next gate (8 = lap done), varint finish tick (0 none), u8 place (0 none), varint hold end tick (0 none), u8 slot; sent in full when anything changes | 1 bit |

## Terrain edit replication

Craters, paint and surface states are `ls_math::terrain::TerrainEdit` values
(`SubtractSphere`, `AddSphere`, `Paint`, `Surface`) produced by `World::step` and applied to
`GameTerrain` (craters come already scaled by rock strength by the simulation, see
[contracts.md](contracts.md#terrain-strength-ls_worldgenmaterial)). The level is never sent;
instead every peer keeps the same ordered **edit log**:

1. `GameTerrain` logs every edit it is handed (even ones the chunk store rejects), so an edit's
   log index is its sequence number on every peer.
2. `Welcome::edit_log` carries the log (up to 2^18 edits) to a joining client, which replays
   it on its freshly generated level, and `Welcome::edit_count` the number of edits so far.
   The server leaves out surface-state edits whose timed states expired before the join and
   can no longer matter (`GameTerrain::welcome_log`: hot, charged and glow edits with
   `until <= now`, unless an earlier still-running edit sharing a state bit may share a cell;
   a scorch mark keeps its scorch bit), so long matches full of laser hits do not inflate the
   Welcome. The late joiner counts the left-out sequence numbers
   (`GameTerrain::replay_welcome`, `edit_count()`), so later edits line up.
3. Each snapshot carries up to 64 edits starting at the client's acknowledged edit count
   (`edit_base`); unacknowledged edits are resent until a snapshot containing them is acked.
   The client applies only the unseen tail (`apply_sequence`); a gap is a hard error
   (`GameError::EditGap`, which bots count as a desync). Edits from stale (out-of-order)
   snapshots are still applied.
4. On the wire an edit is a 2-bit kind, centre and radius as **raw `f32` bits**, for paint an
   8-bit team and a 2-bit colour, and for surface states 8 state bits plus the 32-bit expiry
   tick (130, 140 or 170 bits). Raw bits, not quantized values, so every peer applies the
   exact same CSG and stays bit-identical ([determinism.md](determinism.md)).
5. Applying an edit marks chunks dirty; the browser remeshes dirty chunks first (surface nets,
   at most 8 chunks for a 3 u crater).

`GameTerrain::state_hash()` (edit count and lasting edits + SDF on a 4 u lattice + painted
and scorched cells; independent of time) is the cheap terrain desync check between peers;
tests compare it with the server's, and the browser exposes it as `Game.terrainHash()` for
debugging. `state_hash_at(tick)` adds the timed surface states running at `tick` (peers
agree for every tick from their join on).

## Lag compensation

Remote ships are shown about `RTT + 100 ms` in the past, so the server lets projectiles hit
where the shooter saw the target (up to 250 ms / 15 ticks). The room keeps a short history of
ship positions, estimates from each command which tick the shooter was looking at, and gives
every projectile the rewind of the input that fired it. When a projectile would hit a rewound
target, the target is displaced for that one step so `World::step` applies its normal hit
rules, then moved back. The view-tick estimate, guards and caveats are in
[game-glue.md, Lag compensation](game-glue.md#lag-compensation).

## Transports

`ls_net::Transport` is poll-based and has two channels: `send_unreliable` (may be lost,
duplicated or reordered; at most `max_unreliable_size`, default 1200 B) and `send_reliable`
(exactly once, in order). `Message::channel()` picks the channel: `Input`, `Snapshot`, `Ping`,
`Pong` are unreliable; `Hello`, `Welcome`, `Reject`, `Events`, `Disconnect` and the lobby
messages are reliable. An
unreliable message larger than the limit (for example a big full snapshot) is sent reliably.
Every connection starts with `Hello` (magic `LSCR`, `PROTOCOL_VERSION`, data hash); the
server answers `Welcome` or `Reject` (whose layout is frozen across versions so old clients
can show the reason). `SelectShip`, `Inventory` and `Disconnect` are reliable too.

| | WebSocket | WebTransport |
| --- | --- | --- |
| Endpoint | `GET /ws` on the `--bind` TCP port | `https://host:port/wt` on UDP, `--wt-bind` (defaults to the same port number) |
| Framing | one binary WebSocket message per encoded `Message` | reliable: one bidirectional stream with 4-byte big-endian length prefixes (first client frame is `Hello`); unreliable: one datagram per message |
| Unreliable channel | sent reliably (TCP) | datagrams; oversized ones fall back to the stream |
| Certificate | none (plain HTTP) | self-signed ECDSA P-256, 13 days, rotated every 6 days; the client trusts it by hash from `GET /cert-hash` (`serverCertificateHashes`) |
| Server code | `crates/ls-server/src/ws.rs` | `crates/ls-server/src/wt.rs` (`wtransport` 0.7) |
| Browser code | `web/src/net/transport.ts` | `web/src/net/webtransport.ts` |
| Bots | `--transport ws` (default) | `--transport wt` |

The browser (`web/src/net/connect.ts`) tries WebTransport first when the browser supports it
and `/cert-hash` offers it, and falls back to WebSocket; `?transport=ws|wt` forces one and the
F3 overlay shows which is active. WebTransport needs a secure context, so a page opened as
`http://<LAN-IP>:7777` always uses WebSocket. On the server both endpoints feed the same
`Hub` and `SimLink`, so the room never knows which transport a client uses. Tests use
`MemoryTransport::pair_with(LinkConditions)` to simulate latency, loss, reordering and
duplication without sockets.
