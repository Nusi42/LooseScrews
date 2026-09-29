# Server, bots and end-to-end checks

`crates/ls-server` hosts the authoritative `ls_game::server::Room` (see
[game-glue.md](game-glue.md)); `crates/ls-bots` are headless clients built on
`ls_game::client::ClientSession`; `tools/e2e.*` runs both together. How to run them locally:
[dev-guide.md](../dev-guide.md); transport and netcode overview: [netcode.md](netcode.md).

## ls-server

```
tokio I/O tasks                                  ls-sim thread (60 Hz)
  HttpEndpoint (TCP): static files, /ws, /cert-hash
  WebTransportEndpoint (UDP): /wt  ---Join{hello, SimLink}-->  Sim: Room + clients
  ws / wt task per connection  <--PeerLink | SimLink-->          receive -> step -> dispatch
```

Endpoints on the `--bind` address (TCP) and the `--wt-bind` address (UDP, same port by default):

| Endpoint | What |
| --- | --- |
| `GET /`, static files | the web client from `--web-root` |
| `GET /ws` (WebSocket) | one binary message = one encoded `ls_net::Message`, both directions; unreliable messages travel reliably |
| `GET /cert-hash` | WebTransport certificate as JSON `{"algorithm":"sha-256","hash":"<hex>","port":N,"path":"/wt","expires":<unix s>}` with `Access-Control-Allow-Origin: *`; 404 with `--no-webtransport` |
| `https://host:port/wt` (WebTransport, UDP) | see below; `/` is accepted too |

- `link.rs`: `link_pair` creates the two halves of a connection. `SimLink` (sim thread)
  implements `ls_net::Transport` and never blocks; `PeerLink` (socket task) awaits outgoing
  frames. At most 8 unreliable frames wait (about 260 ms of snapshots plus pongs; older ones are
  dropped), reliable frames are never dropped but more than 16 MiB queued closes the link as too
  slow. Incoming packets use a bounded channel (256), so a flooding client stalls only its own
  socket.
- `timing.rs`: deadline scheduler, bursts of up to 4 ticks to catch up, at most 250 ms backlog.
- `endpoint.rs`: the transport seam (`Endpoint`, `Hub`, `CertHash`); `ws.rs` (WebSocket) and
  `wt.rs` (WebTransport) plug in here. Handshake and idle timeouts (`ConnSettings`, 10 s idle),
  shutdown grace and the timeout counters are the same for both. Incoming packets are tagged
  with the channel they arrived on; `PeerLink::try_push_datagram` drops a datagram instead of
  waiting when the sim's queue is full.

### WebTransport (`wt.rs`, `wtransport` 0.7 with the `ring` backend)

- Wire mapping: the client opens one bidirectional stream. Reliable packets are frames on it: a
  4-byte big-endian length, then the encoded message. The first client frame must be the
  `Hello`; `Welcome` / `Reject` come back on the same stream. Unreliable packets (inputs, pings,
  snapshots, pongs) are datagrams, one message each. The link's datagram limit is the session's
  `max_datagram_size` at accept time, so larger unreliable packets use the existing reliable
  fallback; a datagram the connection still refuses also goes on the stream.
- On a server-side close the stream is flushed (the final `Disconnect` arrives) before the
  connection closes.
- Certificate: self-signed ECDSA P-256, generated at startup, valid from 1 h ago for 13 days
  (browsers accept at most 14 for `serverCertificateHashes`). Every 6 days a new one replaces
  it for new sessions and `/cert-hash` follows; running sessions are unaffected. Clients fetch
  the hash right before connecting.
- `--wt-bind` defaults to the `--bind` address (same port number over UDP). For `0.0.0.0` the
  UDP socket is dual-stack (browsers may reach `localhost` over `::1`). `--no-webtransport`
  turns the endpoint off; the library default (`ServerConfig::webtransport = None`) is off.
- WebTransport needs a secure context in the browser: `localhost` works, a page opened as
  `http://<LAN-IP>:7777` has no WebTransport and falls back to WebSocket. The Vite dev server
  proxies `/ws` and `/cert-hash` to `127.0.0.1:7777`; WebTransport then goes straight to UDP 7777.
- `ls-server` and `ls-bots` need Rust 1.88 (`wtransport`); the rest of the workspace keeps
  `rust-version = "1.85"` (which also sets clippy's MSRV).

### Sim thread policies (`sim.rs`)

| Situation | Behaviour |
| --- | --- |
| Room full / bad version | `Reject`, link closed, `rejected` counted |
| More than 32 bad packets | `Disconnect { "too many protocol errors" }`, removed, `kicked` counted |
| Client sends `Disconnect` | removed; its remaining packets are ignored (not counted as errors) |
| Reliable queue overflow | removed as too slow, `kicked` counted |
| Unreliable packet over the transport's datagram limit | sent on the reliable channel instead |
| Match over, players connected | result shown for `--intermission` seconds (default 10), then every client gets `Disconnect { ls_game::MATCH_RESTART_REASON }` and is removed (not a kick); clients rejoin |
| First join after a finished match | the room is rebuilt (same share code and rules; under a second in release builds, on the sim thread while nobody plays); the scheduler restarts instead of counting skipped ticks. An idle or exiting server never rebuilds |
| Stats window with starvation | more than `10 + 2 %` of the player ticks without input: `input starvation` warning, `desync_warnings` counted (missing inputs count only after a client's first input) |

Counters (`StatsSnapshot`) keep growing across matches: `total_ticks`, `total_edits`, snapshots,
inputs; `tick` and `edit_count` are the current match's. The `tick stats` log line (every
`--stats-interval` s) has p50/p99/max tick time, the longest time of each tick phase
(`recv_max_us`: commands and received packets, `step_max_us`: `Room::step`, `send_max_us`:
dispatch and the rest; a slow tick whose time sits in the receive or send phase is the host
descheduling the sim thread, not game work), input counters, bandwidth, edits, craters,
networked debris now and at its peak in the window (`debris`, `debris_peak`; cap 64), paint
cells per team, match time left, `match_over`, `matches` and `encode_errors`; `final stats`
is logged at shutdown.

### CLI

```
ls-server --bind 0.0.0.0:7777 [--wt-bind <addr> | --no-webtransport] --code <share code> \
    --ruleset data/rulesets/paint_the_void.ron --max-players 8 --web-root web/dist \
    --stats-interval 10 --intermission 10 [--exit-when-empty]
ls-server --ruleset LR6V-69CM-G006-58R --ban tectonic_charge --balance-list balance.ron
```

Rules (`crates/ls-server/src/rules.rs`): `--ruleset` takes a file or an `LR...` rule set code
(an existing file wins; text that looks like a code is decoded). `--ban <weapon_id>`
(repeatable) and `--balance-list <file>` (`ls_sim::ruleset::BalanceList`: `bans`,
`gadget_bans`, `tuning` multipliers applied on top of the rule set's (rounded to twentieths),
`shield_overrides` that replace the rule set's) apply on top: effective rules = rule set, then
balance list, then bans, validated at startup. The balance list is re-read before every match;
if it fails to load, the server warns and keeps the previous rules. The effective rules are
what the `Welcome` carries and the simulation enforces, logged with their code at each match
start (`ServerHandle::rules_code` keeps the first match's code). Built-in rule sets:
`data/rulesets/paint_the_void.ron` (default) and `data/rulesets/sandbox_all_weapons.ron`
(every weapon and gadget as a pickup, every ship type; used by the arsenal e2e run).

`--web-root` defaults to `web/dist` if it exists, so `cargo run -p ls-server --release` after
`npm run build` in `web/` serves the client at `http://localhost:7777`. `--exit-when-empty` shuts
down 2 s after the last player left (scripted runs).

## Browser client lifecycle (`web/src/main.ts`)

- Transport (`web/src/net/connect.ts`, `connectTransport`): WebTransport first when the
  browser has it and `GET /cert-hash` offers it (the certificate is trusted by hash), WebSocket
  otherwise or when WebTransport fails to connect; `?transport=ws|wt` forces one. The F3
  overlay shows the active one (`transportLabel`). `droppable` packets are only ls-net
  `Channel::Unreliable` messages (Input, Ping): WebTransport sends them as datagrams.
- Every animation frame: drain packets, `Game::frame`, send inputs (droppable: skipped while the
  send buffer is backed up, e.g. 16 KiB of WebSocket `bufferedAmount`; the `Hello` is never
  dropped; the F3 overlay counts skipped sends).
- Hidden tab (no animation frames): a 250 ms timer (throttled to ~1 Hz by browsers) runs
  `GameSession::idleFrame`, which drains packets and sends idle input plus the periodic ping, so
  the server's idle timeout does not fire and held buttons are released at once.
- Session end (server `Disconnect`, closed socket, desync, GPU device lost): the session is
  disposed (socket closed, wasm game freed) and an error screen shown.
- Match restart (`Disconnect` with `MATCH_RESTART_REASON`): the game is cleaned up, the renderer
  kept, and the client rejoins on its own (`web/src/game/rejoin.ts`: 0.5 s, then 1, 2, 3, 4, 4,
  ... s, up to 12 attempts, then an error screen). Name, horizon assist, look settings and the
  F3 overlay state carry over.
- Overlays: Tab holds the scoreboard (player names by team, paint %); the end-of-match screen
  shows the winner, paint per team and the scoreboard; F3 toggles the debug overlay (FPS, ping,
  transport, prediction corrections, shot prediction counters, chunks, skipped sends).

### Drawing entities and projectiles (M2, data-driven)

- ls-wasm exports the match's `VisualTable` once (`Game.visuals()`: shape index, colour, scale
  per visual id; shape names from `visualShapeNames()`, `ls_sim::Shape::ALL` order), and every
  frame the projectiles (with visual id and replicated `proj_flags` byte), the entities
  (`Game.entities()`: kind, visual, pose, owner, team, 4 state bytes, radius when known: online
  it is decoded from the state bytes for debris and dust clouds, `scene::net_radius`; kind
  names from `entityKindNames()`). Online, entities are interpolated at the same render tick as
  remote ships (`ClientSession::entities`),
  the players (`playerInfo()` + `playerNames()`) and the weapon panel (`loadout()`). Layouts are
  in `web/src/wasm/api.ts` and `crates/ls-wasm/src/lib.rs`.
- `web/src/game/scene.ts` maps a shape name to a style (`SHAPE_STYLES`: mesh kind, size, glow,
  trail, spin, bob, translucency) and builds the renderer's instances; meshes are procedural
  (`web/src/renderer/primitives.ts`, `MESH_BUILDERS`). A new weapon or entity needs only a
  `visual` in its data file; a new `Shape` variant needs one style entry (a test checks that
  every ls-sim shape has one). Pickups bob and spin, scrap piles grow with their scrap, armed
  mines blink, ghosts and dust clouds are translucent, decoys take their team's hull colour,
  and inside a dust cloud the fog closes in (`dustFog`).
- Name tags (`web/src/ui/nameTags.ts`) project a point above each ship; teammates are labelled
  to 90 u, enemies to 45 u.
- Terrain surface states: `ChunkMesh.surface()` gives per-vertex bits (hot 1, charged 2, glow
  4, permanent scorch 8; `game::surface_flags` from `ls_worldgen::Mesh::states`); with the
  rock material (`materials()`) and the paint effect colour (`paintColors()`) they are packed
  above the paint byte into the terrain's team attribute (`web/src/renderer/mesh.ts`:
  bits 8-15, 16-23, 24-31) and drawn by the terrain shader. Every frame (`pull_dirty`) the
  terrain's display tick moves to the match time (`GameTerrain::advance_to`: the server time
  estimate online, the world tick in practice), so chunks whose states expired are remeshed.
- Online the server sends the own inventory (`Message::Inventory`: carried weapons, ammo,
  levels, selection, gadget) whenever it changes; `crates/ls-wasm/src/inventory.rs` keeps it
  for the HUD and weapon selection, showing a local selection until the server has processed
  it (`Online::select_tick`).
- Keys: 1-9 and 0 select the n-th carried primary in design-doc order (registry order), the
  mouse wheel cycles primaries, X cycles secondaries, G uses the gadget (`Buttons::GADGET`), V
  is the selected primary's mode (`Buttons::WEAPON_MODE`, bit 7: the Paint Thrower's colour),
  B opens the ship bay (1-4 pick a ship there), F stays overdrive. The browser sends weapon
  keys as intents (controls indexes 8-10); `GameCore` resolves them against the carried
  weapons and holds a selection until a tick sends it. The simulation acts on the press edge
  of G and V; `GameCore` keeps such a press from a frame that ran no tick for the next tick
  (`EDGE_BUTTONS`), and the browser counts a tap shorter than a frame as held for that frame.
- Mechanics state for the HUD comes from ls-wasm, not from local guesses where the
  simulation owns it: the Paint Thrower colour (practice: the ship's weapon state; online:
  the server's `PaintColor` cue, guessed ahead on a V press, red after a respawn; 0 unless the
  Paint Thrower is the selected primary), the gadget cooldown (practice: the ship; online:
  from `Gadget` cues, see `ls_sim::gadget_cue`), weapons offline after a Borrowed Time rewind
  (`weaponsOfflineSecs`; online the predictor starts it as the rewind fires).

## ls-bots

Each bot runs on its own thread with a current-thread tokio runtime and plays at 60 Hz.
`--transport ws|wt` (default `ws`) picks WebSocket or WebTransport (`src/transport.rs`, one
`Link` type): for `wt` the bot fetches `/cert-hash` from the host and port of `--server` and
trusts the certificate by that hash only. The `joined` log line names the transport.
Every socket write has a 5 s limit and a session in which nothing arrives for 5 s is lost
(`BotConfig::write_timeout`, `silence_timeout`), so a hung server cannot block `--duration`.

`BotCore::from_welcome` builds the session and also derives the level's spawn points with
`ls_game::match_setup` (about 190 ms per join on cave worlds: the level is generated a second
time), so bots know where the other team starts.

Behaviour (`src/brain.rs`, weapon and threat handling in `src/brain/combat.rs`), priorities
every tick: **guide** an Eyeball the bot launched (its look input steers the missile at the
nearest enemy); **flee** Tectonic Charge zones (a charge in flight or stuck, the launch and
crack cues) along a route out of the collapse zone; **scavenge** when hull is low;
**combat** when an enemy is in range with line of sight (orbit at the chosen primary's
distance, lead aim for its projectile speed, fire); **collect** wanted weapon, ammo and
gadget pickups within 60 u (within 30 u during a fight); **hunt** when an enemy is alive but out of
sight: follow a navigation route to it (to the enemy spawn area when none is alive), painting
on the way, with a break of `hunt_break_secs` (8 s) after `hunt_secs` (90 s) without a fight
or at the end of a route that cannot reach it; otherwise **paint and wander** (cycling the
paint colour now and then). `BrainCounters::hunt_ticks` counts ticks in Hunt mode.

Arsenal use (`src/arsenal.rs`): every weapon of the match's table is a `Gun` (ballistics,
cost, slot, charge) with a `Role` from its data id (`Role::Other` for unknown ids: fired like
a laser or a rocket). Primaries are picked by distance band (lasers mid range, kinetic long
range, Scrap Shotgun and Leech Arc close), weighted towards weapons not fired yet; charge
weapons are held (the Fusion Charger released before its overcharge). Secondaries by
situation: mines when chased, homing ordnance at fleeing targets, the Tectonic Charge on
clusters, Borrowed Time when hurt, Swap Harpoon out of a Tectonic zone, otherwise a random
fitting one. Gadgets: Shield while splash threatens, Cloak while approaching, Decoy when
chased (`GadgetKind::from_id`; unknown gadgets are never used). Each bot picks a random
allowed ship type (`bot::preferred_ship`). A prediction snap waits 1 s for a Swap Harpoon or
Borrowed Time cue before it counts as a desync. The summary lists shots and hits per weapon.

Global navigation (`src/nav.rs`, M1 issue A):

- `NavGrid`: 2 u cells over the level; a cell is a node where some point in it has 0.55 u of
  free space (terrain SDF, refined towards passage axes); 26-neighbour links keep 0.5 u along
  the segment or a two-leg elbow. Built incrementally within 25k SDF evaluations per tick (a
  256 x 128 x 256 u level takes 2-3 s of CPU in release builds, 5-10 s of play). Geometry edits
  queue their region; updates are resumable cell by cell within the same budget (a 35 u
  collapse or a merged barrage box spreads over ticks, the old data stays in use meanwhile).
  Connected components are labelled after the build and at most once a second after changes;
  a goal in another component gets only a short search (20k expansions) for the closest node.
- `NavGrid::find_path`: weighted A* (narrow links cost extra), partial route to the closest
  node when the goal is out of reach. Changed regions are remembered (`changed_near`), so a
  route re-plans only when a change touches it.
- `Navigator`: follows the route (skips ahead to waypoints in view, steps back after a push).
  When the distance left stops shrinking for 2 s while it is followed every tick, the link
  ahead is blocked and the route re-planned; a pause in following (combat orbit, holding still)
  restarts the stall clock instead, and blocked links are re-tested after 30 s.

Tests: `tests/nav_sweep.rs` (release only, about 20 s): three bots on the soak world
`LS08-PNEY-V86Z-VXZM-K4M6-8001-8F8P` and nine random worlds must hit each other within 60 s,
and a 5-minute match on the soak world must still see hits in its last minute. `LS_NAV_TRACE=1`
prints positions, modes and routes.

| Exit code | Meaning |
| --- | --- |
| 0 | all good (match restarts are fine: bots rejoin after 0.5 s and count `match_restarts`) |
| 1 | a protocol error or desync |
| 2 | a bot never joined |
| 3 | a bot lost its connection (crash, kick, timeout); `--allow-disconnects` turns this off |

## End-to-end run (`tools/e2e.mjs`)

A script, not a Rust test: it runs the real `ls-server` and `ls-bots` binaries over real
sockets and checks their logs (server started with `--exit-when-empty` and a short
`--stats-interval`). Commands, options and the full list of checks:
[dev-guide.md, End-to-end run](../dev-guide.md#6-end-to-end-run).
