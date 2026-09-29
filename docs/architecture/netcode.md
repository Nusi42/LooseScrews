# Netcode

An authoritative server runs the simulation at 60 Hz. Clients send input commands, predict
their own ship with the same `ls-sim` code, interpolate everyone else, and rebuild the level
locally from the share code plus a replicated log of terrain edits. This page is the
end-to-end picture; the exact rules and numbers live next to the code:

- `crates/ls-net` (wire format, `Message`, snapshots, `Transport`);
- [game-glue.md](game-glue.md) (`ClientSession`, `Room`, the [wire
  conventions](game-glue.md#wire-conventions) table, lag compensation);
- [server.md](server.md) (sockets, WebSocket and WebTransport mapping, backpressure).

## Tick model

| Item | Value | Where |
| --- | --- | --- |
| Simulation rate | 60 Hz, `DT = 1/60` (`ls_math::TICK_HZ`, `Tick(u64)`) | `ls-math` |
| Snapshot rate | every 2nd tick, 30 Hz (`RoomConfig::snapshot_interval = 2`) | `ls-game` server |
| Wire tick | `u32` | `ls-net` |
| Server scheduler | deadline based; bursts of up to 4 ticks to catch up, at most 250 ms backlog, the rest counted as skipped | `ls-server/src/timing.rs` |

Server step `k` (`Room::step`): take every client's input stamped `k` (or repeat its last
command), plan lag compensation, run `World::step`, record ship positions for rewinds, and
on snapshot ticks queue one snapshot per client. `Snapshot::tick` is the tick after that step.
Once the match timer runs out the room stops stepping the world but keeps sending snapshots.

The client runs its own 60 Hz clock ahead of the server: its first command is stamped
`Welcome::server_tick + 4` (`INITIAL_LEAD_TICKS = 3`, plus one), so the command for tick `k`
arrives before the server steps `k`. `ClientClock` keeps that lead right:

- a snapshot with `ack_input < tick` means the server ran out of our input (starvation), so the
  client runs ahead by the missing ticks (at most 30 per report);
- a client that is not ahead at all (for example after slow level generation in WASM) jumps
  to `tick + 13` in one go;
- a `Ping` sent right after the input for tick `c` comes back as a `Pong` with `server_tick`;
  `server_tick - c` is the input slack. Below 1 tick the client runs ahead, a minimum above 4
  ticks over four pongs skips one tick. Pings go out every 30 ticks.

Until a snapshot proves the server had our input in time (`ClientClock::is_synced`), the
client sends idle commands: look input is a per-tick delta, and a command the server replaced
by a repeat would leave a permanent orientation difference.

## Input and redundancy

- `InputCmd` per tick: thrust axes and roll (quantized to 1/127), look deltas (1/8192 rad),
  button bits (`ls_sim::Buttons`: boost, fire primary/secondary, scavenge, overdrive, horizon
  assist) and a weapon selection. The client quantizes each command exactly as the server will
  decode it, so both simulate the same numbers.
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

Shots: the browser also predicts its own projectiles (`crates/ls-wasm/src/shots.rs`,
`ShotPredictor`) with the simulation's firing rules, draws them at once, and hands over to the
server's projectile when a snapshot shows a match (the difference fades as a render offset).
Unmatched predictions disappear after 4 ticks. Bots do not predict shots.

## Interpolation (other ships and projectiles)

- Remote ships are drawn at `server time - 6 ticks` (100 ms, `INTERP_DELAY_TICKS`) from the
  buffered snapshots: position and velocity lerped, orientation nlerped. Gaps larger than 8 u
  (respawns) are not interpolated; at most 6 ticks of extrapolation when snapshots run late.
- Projectiles are straight-line extrapolated from the newest snapshot to the client's
  server-time estimate (at most 8 ticks).
- The server's lag compensation assumes the same 6-tick delay when it rewinds targets.

## Snapshots and deltas

Wire layout (`crates/ls-net/src/snapshot.rs`): `tick`, optional baseline (as a `u8` age),
`ack_input`, `match_ticks_left`, terrain edits (`edit_base`, count, edits), scores, ships
(a 32-bit presence mask, then each ship), projectiles.

- A full ship is exactly **18 bytes** (`SHIP_CORE_BITS = 144`): position 3 x 16 bit inside the
  quantization box (level bounds + 2 u), smallest-three orientation in 32 bits, velocity
  3 x 16 bit (±64 u/s), 16 flag bits (alive, boosting, team, selected primary weapon id).
  Flag bit 15 tells the decoder a 4-byte `ShipStatus` (hull, energy, boost, scrap, each
  `ceil(value / max * 255)`) follows; only the receiving client's own ship carries it.
- **Deltas.** The server keeps a `SnapshotHistory` per client (128 snapshots, 256 ticks) and
  encodes against the client's newest acknowledged snapshot (`choose_baseline`: age 1..=255
  ticks and still in history, else a full snapshot). Fields are compared as packed integers,
  so a delta decodes to exactly the packed snapshot the server built. Ships and projectiles in
  the baseline send a change bit plus per-field differences (2-bit size class: unchanged, 4-bit
  or 8-bit zigzag difference, or the raw 16-bit value); new ones are sent in full. The client
  keeps the same 128-entry history, so every baseline the server may choose is available.
- Budget: `bandwidth_32_ships_at_30_hz` in `ls-net` proves 32 ships x 18 B x 30 Hz fits
  17 KiB/s before delta compression. In practice a 3-bot local run measures about 4 KiB/s of
  snapshots for the whole server.
- Events (hits, kills, respawns, match end) are batched per snapshot into an `Events` message
  on the reliable channel.

## Terrain edit replication

Craters and paint are `ls_math::terrain::TerrainEdit` values (`SubtractSphere`, `AddSphere`,
`Paint`) produced by `World::step` and applied to `GameTerrain`. The level is never sent;
instead every peer keeps the same ordered **edit log**:

1. `GameTerrain` logs every edit it is handed (even ones the chunk store rejects), so an edit's
   log index is its sequence number on every peer.
2. `Welcome::edit_log` carries the whole log (up to 2^18 edits) to a joining client, which
   replays it on its freshly generated level.
3. Each snapshot carries up to 64 edits starting at the client's acknowledged edit count
   (`edit_base`); unacknowledged edits are resent until a snapshot containing them is acked.
   The client applies only the unseen tail (`apply_sequence`); a gap is a hard error
   (`GameError::EditGap`, which bots count as a desync). Edits from stale (out-of-order)
   snapshots are still applied.
4. On the wire an edit is a 2-bit kind, centre and radius as **raw `f32` bits** and, for paint,
   an 8-bit team (130 or 138 bits). Raw bits, not quantized values, so every peer applies the
   exact same CSG and stays bit-identical ([determinism.md](determinism.md)).
5. Applying an edit marks chunks dirty; the browser remeshes dirty chunks first (surface nets,
   at most 8 chunks for a 3 u crater).

`GameTerrain::state_hash()` (edit log + SDF on a 4 u lattice + paint around every edit) is the
cheap terrain desync check between peers; tests compare it with the server's, and the browser
exposes it as `Game.terrainHash()` for debugging.

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
`Pong` are unreliable; `Hello`, `Welcome`, `Reject`, `Events`, `Disconnect` are reliable. An
unreliable message larger than the limit (for example a big full snapshot) is sent reliably.
Every connection starts with `Hello` (magic `LSCR`, `PROTOCOL_VERSION`); the server answers
`Welcome` or `Reject` (whose layout is frozen across versions so old clients can show the
reason).

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
