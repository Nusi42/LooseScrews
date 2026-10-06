# Server, bots and end-to-end checks

`crates/ls-server` hosts the authoritative `ls_game::server::Room` (see
[game-glue.md](game-glue.md)); `crates/ls-bots` are headless clients built on
`ls_game::client::ClientSession`; `tools/e2e.*` runs both together. How to run them locally:
[dev-guide.md](../dev-guide.md); transport and netcode overview: [netcode.md](netcode.md); how
modes, the lobby, rooms, parties and duels fit together across the crates:
[modes-and-lobby.md](modes-and-lobby.md).

## ls-server

```
tokio I/O tasks                                  one sim thread per room (60 Hz)
  HttpEndpoint (TCP): static files, /ws, /cert-hash
  WebTransportEndpoint (UDP): /wt  --Hub: route by Hello.room-->  public room: Sim (Room + lobby + rotation)
  ws / wt task per connection  <--PeerLink | SimLink-->           private rooms R-XXXXXX: Sim each
                                                                  receive -> step -> dispatch -> lobby
                                         ls-build worker threads: next Room built, swapped in
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
  `wt.rs` (WebTransport) plug in here. `Hub::open` routes each connection by the room code in
  its `Hello` (`rooms.rs`; none = the public room, an unknown code gets a `Reject` "unknown
  room"). Handshake and idle timeouts (`ConnSettings`, 10 s idle),
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

### Rooms, lobby and rotation (`rooms.rs`, `lobby.rs`, `playlist.rs`, M3)

- **Rooms** (`rooms.rs`): one public room plus up to `--private-rooms N` (default 4, capped at
  one per CPU core beyond the public room's) private rooms, each a `Sim` on its own thread with
  its own `SharedStats` (the public room's carry the server-wide connection counters).
  Private rooms are created with `ServerHandle::create_private_room(PrivateRoomSpec)` (duels,
  parties and custom matches in wave B call it) and get a code `R-XXXXXX` (6 Crockford base32
  characters; parsing accepts lower case, a missing `R-` and O/I/L look-alikes). Clients join
  with `Hello::room`. A private room closes when its last player has left, or 60 s after its
  creation if nobody joined; joins that raced the close get `Reject { "room closed" }`.
  `ServerHandle::rooms()` lists every room's `RoomStatus` (phase, playlist entry, share code,
  rules, players) and counters; `total_players()` sums them.
- **Lobby** (`lobby.rs`, pure state machine on sim-thread ticks): `Warmup -> Countdown -> Match
  -> Results -> Warmup`. Warm-up flies the next match's level in a *warm-up room*: the same
  world, ships and team count with every weapon, gadget and mutator off (empty loadout, no
  pickups; `warmup_rules`). Whatever the next mode, the warm-up room plays a timer-only mode
  (`warmup_mode`: Paint the Void without paint, 2 to 4 teams, two for a free-for-all match), so
  kill targets, lives, last-side-standing checks and objectives can never end or eliminate in a
  warm-up; the sim thread keeps its one-hour clock from running out (`extend_warmup_clock`).
  `LobbyState` announces the real next mode. The countdown starts once the player count fits the playlist entry
  and everyone is ready (`Ready`) or `--warmup` seconds (default 30) passed since players
  arrived; it is cancelled when the count stops fitting. If the warm-up passes with a count
  the entry does not fit, the room picks another entry (a new warm-up room); if the rotation
  has nothing better, the entry is played anyway. After `--countdown` (default 10 s) the match
  room (built meanwhile) is swapped in. When the mode ends, the result shows for
  `--intermission` seconds while the next entry's warm-up room is built. `--warmup 0
  --countdown 0` (and the library default `ServerConfig::new`) turn the lobby off: no warm-up
  rooms, Results swaps the next match room in directly. A private room's `Welcome` carries its
  room code (`Room::set_room_code`).
- **Rotation** (`playlist.rs`): `--playlist <file>` (RON: `Playlist(entries: [(rules: "team_brawl",
  world: Random | RandomSliders(enclosure: (lo, hi), density: (lo, hi)) | Code("LS.."),
  min_players: 4, max_players: 32), ...])`; `rules` is a built-in rule set id (file name in
  `data/rulesets/` without `.ron`), a file (relative to the playlist) or an `LR` code; bans and
  the balance list apply to every entry). `Rotation::pick(players)` takes the next entry in
  order whose player range fits, else the first `team_brawl` entry (2 teams from 4 players,
  free-for-all below), else the next entry. Without `--playlist`: `--ruleset` plays that rule
  set on every match (one entry, one world), neither flag plays the default rotation (Team
  Brawl, The Squeeze, Paint the Void, Hot Reactor, Heist, Last Color Standing: the built-in
  rule sets that exist; random worlds, or `--code`'s world). A Heist entry with teams plays
  free-for-all (`teams: 0`) when it is planned for 1-3 players (`sim::heist_variant`, plan
  §3.2; with 0 players, i.e. nobody connected yet, it keeps its teams).
- **Swaps**: the next room is built on an `ls-build` worker thread (`Room::new`, 0.6-1.6 s in
  release) while the current room keeps running, so joins never wait for a build: they join the
  current room (also during Results) and move with everyone else. At the swap every connected
  player joins the new room with its stored `Hello` (`PickTeam` updates its team) and gets the
  new room's `Welcome` on the same connection, no `Disconnect`; the old room is dropped on an
  `ls-drop` thread. With parties in the room the joins follow `party::plan_joins` (whole parties
  on one team where balance allows, in an order `Room::join` grants). `ServerConfig::build_delay`
  stretches builds for tests.
- **Lobby messages**: the sim thread handles `Ready`, `PickTeam` and `Spectate` itself (the
  room never sees them) and sends `LobbyState` (phase, ticks left, room code, next mode / rule
  set / share code, player range, teams, players with team pick, ready and spectate flags) to
  everyone whenever it changes (the countdown alone is no change), always after the room's own
  packets of the tick, so a new `Welcome` arrives before its lobby state. `Spectate` is stored
  per connection and announced; when the player moves into the next match room the sim thread
  calls `World::set_spectator(player, true)`, so they have no ship in that match (warm-up rooms
  stay open to everyone; test `spectate_applies_from_the_next_match_on`). `SelectShip` also
  updates the connection's stored `Hello`, so a ship picked mid-match carries over to the next
  rooms, and a leaving player's `Ready` is cleared with its wire id (ids are reused).

### Server list, invites, parties and duels (`api.rs`, `party.rs`, `duel.rs`, `directory.rs`, M3 B7)

- **JSON API** on the game port (`api.rs`, routed by `http.rs` for `/api/...`; CORS `*`,
  `OPTIONS` preflight, POST bodies with `Content-Length` up to 16 KiB, errors
  `{"error":"..."}`):

  | Request | Body | Reply |
  | --- | --- | --- |
  | `GET /api/info` | - | the directory entry (name, address, region, mode, world, rules code, players, protocol, data hash, roster) plus `directory`, `rooms` and `private_rooms_free` |
  | `GET /api/players` | - | `{"players":[{"id","name"}]}`: present players |
  | `POST /api/presence` | `{"id","token","name"}` | `{"invites":[note],"address","directory"}` |
  | `POST /api/party` | `{"room"?,"player"?,"world_code"?,"rules_code"?}` | `{"party","room","address"}` |
  | `POST /api/rooms` | `{"world_code"?,"rules_code"?}` | `{"room","address"}`: a private custom match |
  | `POST /api/duel` | `{"id","token","name","to","to_name"?,"world_code"?,"rules_code"?}` | `{"challenge","relayed"}` |
  | `POST /api/duel/answer` | `{"from","id","accept"}` | `{"room","address"}` |

  Status codes: 400 invalid body or field, 403 wrong token, 404 unknown player, challenge, room
  or path, 405, 502 directory failed, 503 no private room free. `Social` holds the state and is
  shared with the `Hub`; `RoomFactory` (also behind `ServerHandle::create_private_room`) opens
  private rooms.
- **Presence and identity** (`duel.rs`): the browser keeps a random public player id and a
  private token (localStorage). `POST /api/presence` (polled every 3 s in game) registers it
  (the first token claims an id; a later one gets 403), refreshes the name and returns the
  player's inbox. Players seen within 15 s form the roster (`/api/players`, the directory
  entry's `roster`).
- **Parties** (`party.rs`): codes `P-XXXXXX` (6 Crockford base32, parsed leniently like room
  codes), at most 512 per server. A party points at the public room, a private room (the
  leader's), or a new private custom match (`world_code`/`rules_code`). `player` (the leader's
  `Welcome::player_id`) tags the leader's connection (`SimCommand::SetParty`). A `Hello` with a
  party code is routed to the party's room (`Hub::open`; the party wins over a room code;
  unknown or ended parties get `Reject { "unknown party ..." }`); in team modes the member asks
  for the team of a party mate in the room, and `Room::join` still grants only a smallest team,
  so a party stays together only where the team sizes stay within one (else it is split by join
  order). At every swap the sim thread re-plans the joins (`plan_joins`). Sim threads count
  members (`Parties::member_joined/left`); a party ends 10 minutes after its last member left
  (or after creation if nobody joined) and with its private room.
- **Duels** (`duel.rs`, `api.rs`): `POST /api/duel` on the challenger's server records a
  challenge (30 s). A challenged player present on that server sees it in the inbox; otherwise
  the server relays it through the directory (`POST /invites`, kind `duel`, `address` = this
  server, the challenger's world and rules codes), and the other player's browser, which polls
  the directory by its public id, answers on the challenger's server. Accepting opens a private
  room (`duel_spec`): 2 players, the challenger's world (a fixed random one without), the
  challenger's rule set code or the built-in `duel` rule set (`data/rulesets/duel.ron`: Team
  Brawl free-for-all, 10 kills or 5 minutes; a Team Brawl fallback with the same numbers is
  built in code if the file is missing). Like custom matches (`api::custom_rules`), duel rules
  are resolved without the server's `--ban` and `--balance-list` (known gap). Both players
  get the room code (the answer's reply, the challenger's inbox note `accepted`) and join by
  invite link; a declined challenge shows up as `declined`.
- **Directory** (`directory.rs`): with `--directory <url>` the server sends
  `ls_directory::spawn_heartbeats` (every `heartbeat_secs`, 10 s by default) with
  `Social::server_info()`: `--server-name`, `--region`, `--public-address` (default: the bound
  address, `localhost:<port>` for an unspecified one), the WebTransport certificate hash, the
  public room's mode id, world and rules code, players over all rooms (capped at the public
  room's limit) and the roster; shutdown removes the entry. Without `--directory` nothing
  changes (the API still serves invites, parties and local duels).

### Sim thread policies (`sim.rs`)

| Situation | Behaviour |
| --- | --- |
| Room full / bad version | `Reject`, link closed, `rejected` counted |
| More than 32 bad packets | `Disconnect { "too many protocol errors" }`, removed, `kicked` counted |
| Client sends `Disconnect` | removed; its remaining packets are ignored (not counted as errors) |
| Reliable queue overflow | removed as too slow, `kicked` counted |
| Unreliable packet over the transport's datagram limit | sent on the reliable channel instead |
| Match over | Results for `--intermission` seconds (default 10) while the next room is built on a worker thread; then everyone moves to it on the same connection (new `Welcome`, see above). Nobody is disconnected |
| Join at any time (also during a build) | joins the current room at once (`Welcome` on the next tick) |
| Next room would not take a player | `Disconnect` with the reason, removed |
| Stats window with starvation | more than `10 + 2 %` of the player ticks without input: `input starvation` warning, `desync_warnings` counted (missing inputs count only after a client's first input) |

Counters (`StatsSnapshot`) keep growing across matches: `total_ticks`, `total_edits`, snapshots,
inputs; `tick` and `edit_count` are the current match's. The `tick stats` log line (every
`--stats-interval` s) has p50/p99/max tick time, the longest time of each tick phase
(`recv_max_us`: commands and received packets, `step_max_us`: `Room::step`, `send_max_us`:
dispatch and the rest; a slow tick whose time sits in the receive or send phase is the host
descheduling the sim thread, not game work), input counters, bandwidth, edits, craters,
networked debris now and at its peak in the window (`debris`, `debris_peak`; cap 64), paint
cells per team, match time left, `match_over`, `matches` and `encode_errors`, plus the
room (`room`: `public` or the code), its lobby `phase` and whether it is a warm-up room;
every room logs its own line (the tick budget, p99 < 4 ms, holds per room). `matches`
counts match rooms started (warm-up rooms do not count). `final stats` is logged at
shutdown.

### CLI

```
ls-server --bind 0.0.0.0:7777 [--wt-bind <addr> | --no-webtransport] --code <share code> \
    --ruleset data/rulesets/paint_the_void.ron --max-players 8 --web-root web/dist \
    --stats-interval 10 --intermission 10 [--exit-when-empty]
ls-server --ruleset LRAW-740M-8S00-01JD-4 --ban tectonic_charge --balance-list balance.ron
ls-server --playlist playlist.ron --warmup 30 --countdown 10 --intermission 10 --private-rooms 4
ls-server --directory http://127.0.0.1:7780 --server-name "Den" --region eu \
    --public-address play.example.com:7777
```

Rules (`crates/ls-server/src/rules.rs`): `--ruleset` takes a file, a built-in rule set id
(`paint_the_void`, `sandbox_all_weapons`, ...) or an `LR...` rule set code (an existing file
wins, then built-in ids; text that looks like a code is decoded). `--ban <weapon_id>`
(repeatable) and `--balance-list <file>` (`ls_sim::ruleset::BalanceList`: `bans`,
`gadget_bans`, `tuning` multipliers applied on top of the rule set's (rounded to twentieths),
`shield_overrides` that replace the rule set's) apply on top: effective rules = rule set, then
balance list, then bans, validated at startup. The balance list is re-read before every match;
if it fails to load, the server warns and keeps that playlist entry's previous rules. The effective rules are
what the `Welcome` carries and the simulation enforces, logged with their code at each match
start (`ServerHandle::rules_code` keeps the first match's code). Built-in rule sets
(`ls_sim::ruleset::BUILTIN_RULESETS`, ids = file names): `paint_the_void`, `team_brawl`,
`squeeze`, `heist`, `hot_reactor`, `last_color_standing` (one per mode, the default rotation),
`instagib_brawl`, `duel` and `sandbox_all_weapons` (every weapon and gadget as a pickup, every
ship type; used by the arsenal e2e run); their numbers are in
[userDocs/reference/modes.md](../../userDocs/reference/modes.md).

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
- Match transition (M3, a new `Welcome` on the open connection: warm-up room, next match):
  `GameSession` reports `FrameResult.transition` once (also after hidden-tab frames) and
  re-reads the match's visual table; `main.ts` keeps the renderer and the page, resets the
  camera to the new level, the effects and the spectator camera, and streams the new level
  (the old level's chunks come back from `takeUnloads`). No reload, no reconnect.
- Invites, server list, parties and duels (`web/src/ui/party.ts`, `ui/serverList.ts`,
  `net/social.ts`, `net/ping.ts`): `startSocial` reads the page URL
  (`?join=<host:port>&party=P-..&room=R-..&code=LS..&rules=LR..&directory=<url>`; `world=` is
  accepted for `code=`) and hands `main.ts` the WebSocket URL of the `join` server, a wrapped
  game core whose `Hello` carries the room and party code, and a connection opener that first
  opens a custom match (`POST /api/rooms`) for a link with a world/rules code but no party or
  room. `?servers` (the start screen's "Server list" link) shows the server list: the
  directory's servers of this build (protocol and data hash of the page's server, from
  `/api/info`), pinged in parallel by WebSocket open time (6 at once, 3 s limit), with Join
  (invite link) and Duel per roster entry. In game (`attach`): presence poll, directory invite
  poll, challenge toasts (Accept/Decline) and the "Party & duels" panel (party invite link for
  the current room and team, Duel buttons for the players on the server, on the current world).
  Moving to a duel room or another server is a page navigation to its invite link.
- Match restart from an older host (`Disconnect` with `MATCH_RESTART_REASON`): the game is
  cleaned up, the renderer kept, and the client rejoins on its own (`web/src/game/rejoin.ts`:
  0.5 s, then 1, 2, 3, 4, 4, ... s, up to 12 attempts, then an error screen). Name, horizon
  assist, look settings and the F3 overlay state carry over.
- Mode and lobby UI (M3): `GameSession.frame` parses ls-wasm's `modeState()`, `lobbyState()` and
  `results()` JSON (`web/src/game/modeState.ts`); `game/matchFlow.ts` decides the stage
  (warm-up, countdown, match, results) and whether the player spectates (eliminated or
  spectator flag), and `LobbyClock` counts the lobby's `phaseTicksLeft` down between
  `LobbyState` updates (the server sends it only on change). `ui/modeHud.ts` draws the
  scoreline (mode, timer, team scores or the free-for-all top three, target), the stage, own
  lives and colour meter, alerts and the objective markers (projected, clamped to the screen
  edge); `ui/lobby.ts` the lobby card (next mode, world, players with ready and team, Enter
  ready, T team, O spectate, also as buttons); `ui/results.ts` the results screen (team bars or
  a podium, the table, the next mode's countdown; in practice built from the local mode
  state); `game/spectator.ts` the free-fly / follow camera. Lobby requests (`Ready`,
  `PickTeam`, `Spectate`) are sent reliably. `game/sides.ts` colours sides: four team colours,
  and in free-for-all a distinct colour per side id (ships, shots, name tags, tables).
  `#ui[data-stage]` and `#ui[data-matches]` expose the stage and the match count to the
  browser smoke test.
- Overlays: Tab holds the scoreboard (per team or one free-for-all list: mode score, kills,
  deaths, lives); F3 toggles the debug overlay (FPS, ping, transport, prediction corrections,
  shot prediction counters, chunks, skipped sends).

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
  (`web/src/renderer/models/index.ts`, `MESH_BUILDERS`: the modelling-kit hulls and props of
  `models/*`, the remaining primitives of `primitives.ts`). Ships draw the hull of their type
  (`shipMeshKind`), decoys and ghosts their owner's, wrecks the dead ship's
  (`game/wrecks.ts`). A new weapon or entity needs only a
  `visual` in its data file; a new `Shape` variant needs one style entry (a test checks that
  every ls-sim shape has one). Pickups bob and spin, scrap piles grow with their scrap, armed
  mines blink, ghosts and dust clouds are translucent, decoys take their team's colour,
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

Mode objectives (`src/brain/objective.rs`, M3 C2), from the replicated mode state
(`ModeSense::from_mode`: own player flags, objective markers, colour meters):

- **Spectate**: an eliminated player (or a late joiner of an elimination mode) has no ship;
  the brain idles in `Mode::Spectate` (`spectate_ticks`) until the next match.
- **The Squeeze**: the zone to keep is the `NEXT_ZONE` once announced (inside the current one,
  so safe through the whole close), else the `SAFE_ZONE`. Outside its comfort radius
  (radius minus a quarter, 4-25 u) the bot flies (boosting) back until it is within half of
  it, right after Tectonic fleeing and before scavenging or fighting; wander points and pickups
  outside the zone are ignored.
- **Heist**: the carrier flies to its team's extraction zone (the nearest in free-for-all)
  before anything else, without boost, shooting on the way; a free or dropped core is fetched
  by touch; an embedded core is dug out with crater ordnance (Bolt Rocket, then Quicksilver,
  Heavy Rocket) fired at the core from 6.5 u when the ray hits rock within 4 u of it. About half
  the bots are miners (`RNG_STREAM_ROLE`) that head for the core even under fire; the others dig
  when no enemy is in sight; at the core everyone keeps digging. The enemy carrier counts as
  much closer when choosing targets and hunts; team mates escort their own carrier.
- **Hot Reactor**: the holder rams the nearest ship (straight at it with boost, wall clearance
  lowered); everyone else never targets the holder (`Perception::is_hostile`: a hit would pass
  it to the shooter) and flies away from it while within fuse radius + 12 u.
- **Last Color Standing**: target and hunt distances are weighted by colour
  (`Perception::target_weight`: x2 at 0 colour, x1 at 100), so the most colourful go first.

`BrainConfig::objectives = false` plays every mode like a deathmatch (comparisons).
Lobby: while the `LobbyState` (warm-up or countdown) lists the bot as not ready, `BotCore`
sends `Ready { ready: true }` (again after 1 s if the lobby still says not ready;
`BotStats::ready_presses`).

Server list and duels (`src/social.rs`): `--directory <url>` asks `ls-directory`
(`DirectoryClient::list` with this build's protocol and data hash) before every connection
and joins the fullest server that still has a free slot. `--accept-duels` registers a presence
on the game server (`POST /api/presence` every 2 s, id `bot-<name>-<hash>`), so the bot is in the
roster; every `duel` note in its inbox (and every relayed `duel` invite in the directory when
`--directory` is given) is accepted (`POST /api/duel/answer` on the challenger's server). The
runner then leaves (`Disconnect`), joins the duel room with `Hello::room`, plays until the duel
match is over (`MatchOver`) and returns to its server (`BotStats::duels`). `--room <code>` joins
a private room directly.

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
Borrowed Time cue before it counts as a desync; a correction (a teleport shorter than the snap
distance) waits the same 1 s and, if such a cue explains it, counts as `teleport_corrections`
instead of toward the largest reconciliation error. The summary lists shots and hits per weapon.

Global navigation (`src/nav.rs`, M1 issue A):

- `NavGrid`: 2 u cells over the level; a cell is a node where some point in it has 0.55 u of
  free space (terrain SDF, refined towards passage axes); 26-neighbour links keep 0.5 u along
  the segment or a two-leg elbow. Built incrementally within 25k SDF evaluations per tick (a
  256 x 128 x 256 u level takes 2-3 s of CPU in release builds, 5-10 s of play). Geometry edits
  queue their region; updates are resumable cell by cell within the same budget (a 35 u
  collapse or a merged barrage box spreads over ticks, the old data stays in use meanwhile).
- Grid cache (`src/nav_cache.rs`, M3 A5): bots on a share-code world don't build their own
  grid. The first bot of a world (key: share code, generator version, `NavConfig`) starts one
  build on a `nav-build` worker thread from the pristine level; every bot of the process
  attaches to it (`SharedNav`), steers locally until it is done, then copies it
  (`Navigator::shared`) and re-samples the regions of the edits it has seen (the `Welcome`'s
  edit log and later ones), so edits and blocked links stay per bot. Worlds with a live handle
  stay cached, plus the 4 most recent; a failed build falls back to the incremental per-bot
  build. `nav_cache::builds()` counts builds. Measured (`tests/nav_cache.rs`, release, 8 bots
  on one world): 3.7-5.7 s of navigation time per bot before, 0.44-0.63 s after (8.4-9.1x).
  In-process tests step faster than real time, so they call `BotCore::wait_for_nav` after
  `from_welcome` (a real bot never blocks; it steers locally until the grid arrives).
  Connected components are labelled after the build and at most once a second after changes;
  a goal in another component gets only a short search (20k expansions) for the closest node.
- `NavGrid::find_path`: weighted A* (narrow links cost extra), partial route to the closest
  node when the goal is out of reach. Changed regions are remembered (`changed_near`), so a
  route re-plans only when a change touches it.
- `Navigator`: follows the route (skips ahead to waypoints in view, steps back after a push).
  When the distance left stops shrinking for 2 s while it is followed every tick, the link
  ahead is blocked and the route re-planned; a pause in following (combat orbit, holding still)
  restarts the stall clock instead, and blocked links are re-tested after 30 s.

Mode tests (release only): `tests/mode_matches.rs` plays every mode's built-in rule set,
unchanged, with 8 bots to a result (0 desyncs, 0 protocol errors), about 60 s for all seven
tests: Team Brawl to the kill target, The Squeeze to the last ship flying (plus an unarmed
Squeeze in which only the walls decide), Heist with deliveries, Hot Reactor with passes besides
detonations, Last Color Standing decided by kills before the stalemate drain, Paint the Void on
time; eliminated bots spectate. `LS_BOTS_NO_OBJECTIVES=1` plays them with the objective layer
off and only prints the measurements. `tests/social.rs` runs bots against a real server: Ready
starts the countdown long before a 10-minute warm-up ends, `--directory` bots find their server,
and an `--accept-duels` bot plays a challenger's duel and comes back.

### Bots in moods (M4 package C5)

The perception carries the comedy director's state (`MoodSense`: event, phase, target), the
event props, dynamic wells and pigment pickups (`PropSeen`), the clamped well pull on the ship,
the radiation dose rate, the monochrome flag and the own pigment bit, Belt Up's `RaceView`, and
The Architect's exit, strokes, role and budget (`ArchitectSense` in `ModeSense`).

- **World physics** (`brain/moods.rs`): axes without input get a one-step counter-thrust against
  the well pull (idle dampers would drift toward the well); ducks and dynamic well stones push the
  plan away; a `CaveInWarning` cue is a danger zone (as the Tectonic Charge's); pigment pickups
  are fetched above 60 % hull when not recently hit. Static well cores are rock (wall avoidance).
- **Events**: Tax drones, mimics and other players' pizza drones are targets when no ship is
  (a contact with id 255 through the usual fire logic, primaries only); bots on even seeds also
  rate the Customer survey (the column a shot passes from where they are is the star; the leader's
  own shots go through the middle); the Cat paw prop is a 40 u danger zone during the tell and
  the sweep; the pizza recipient holds still below 90 % hull; hats in 35 u are collected.
- **The Architect** (`brain/architect.rs`): the architect (no ship, so before the alive check)
  sends an `ArchitectAction` every 2.3-3.3 s when the budget allows: weighted among the strokes
  it can pay (Enclosure+ most), centred where the runner nearest the exit will be when the
  telegraph ends (rock strokes half a radius further toward the exit, kept clear of it); the
  server validates (refusals only cost the cooldown). Runners take a navigation route to the exit,
  treat telegraphed rock and well strokes as danger zones, never target ships, and blast rock
  toward the exit when the distance stops shrinking for 5 s.
- **Belt Up racing** (`race_line.rs`, `brain/race.rs`): a lattice over slot 0's course (a station
  every 1.5 degrees of azimuth; per station 21 radii from 76 to 156 u by 21 heights of +-30 u,
  plus the generator's clear corridor and the slot; nodes keep 1.6 u from rock, links 1.2 u; at
  and within 6 degrees of a gate only points inside the gate), built once per world per process
  from the pristine level and shared. A line is the cheapest path by dynamic programming for
  `time + weight x dose` (time from a speed that falls in squeezes, dose from the dose law at the
  edge's middle), the straight dive to the exit added at the last station; lines for other slots
  are rotated by whole wedges. Racers plan from where they are: the hugger with a fixed weight 4
  (the belt line), the skimmer and the daredevil with the lowest weight (bisected) whose dose to
  the finish fits 60 % or 100 % of their hull above a reserve (6 + 10 % of full hull); they plan
  again at a heat start, after a respawn and when the hull strays 8 hp from the plan. Flying: a
  point 5-10 u ahead on the line, the planned speed, over or under boulders, boost on clear
  straights above 70 % (the rest is kept for the dive), the dive once the way to the exit is
  clear, a navigation route back to the line after 4 s without progress. Racers scavenge scrap in
  40 u when even the belt line's dose would not leave the reserve (late in the lap, or below 35 %
  hull), hold still for their pizza in the back half of the field, steer through wanted pickups
  in 14 u ahead, lean into a rival alongside (not the hugger), and fight the nearest racer ahead
  in the running order within 60 u and 46 degrees of the flight direction (primaries, the usual
  secondaries: Seekers, Magnet Sneeze, Swap Harpoon, Paperweights, mines when chased).

Tests (release only): `tests/race_match.rs`: `race_full_match` (8 racers of all three
personalities play the built-in Belt Up rule set to a result: 3 heats, every bot finishes at
least 2 of them, 0 desyncs), `time_trials` (8 belt seeds, a Tinker alone per line: the mixed
line is fastest, the open line without scavenging is not faster than the belt line, every line
inside the 180 s limit; `LS_TIME_TRIAL_SEEDS=n`, `LS_TIME_TRIAL_ONLY=i`, `LS_TRIAL_TRACE=1` and
`LS_BELT_UP_<FIELD>=<value>` help tuning), `skim_lap_and_dive_doses` (the dose law against its
targets); `tests/mood_matches.rs`: `absurdity_rotation` (Team Brawl on a monochrome level,
Heist and Paint the Void at Absurdity 100 with wells and cave-ins, six forced events each so all
18 run, 8 bots: results, 0 desyncs, reactions counted, reconciliation error near wells and ducks
at most 0.1 u, `BotStats::max_reconcile_error_moods`) and `architect_match` (6 bots, 3 rounds, a
bot architect per round). The harness is shared (`tests/support/mod.rs`).

Tests: `tests/nav_sweep.rs` (release only, about 20 s): three bots on the soak world
`LS0G-2TNV-VD0V-ZFQY-JC58-T2XM-57QY` and nine random worlds must hit each other within 60 s,
and a 5-minute match on the soak world must still see hits in its last minute. `LS_NAV_TRACE=1`
prints positions, modes and routes.

| Exit code | Meaning |
| --- | --- |
| 0 | all good (the next match is fine: bots follow a new `Welcome` in place, or rejoin after 0.5 s on a `MATCH_RESTART_REASON` disconnect; both count `match_restarts`) |
| 1 | a protocol error or desync |
| 2 | a bot never joined |
| 3 | a bot lost its connection (crash, kick, timeout); `--allow-disconnects` turns this off |

## End-to-end run (`tools/e2e.mjs`)

A script, not a Rust test: it runs the real `ls-server` and `ls-bots` binaries over real
sockets and checks their logs (server started with `--exit-when-empty` and a short
`--stats-interval`). Commands, options and the full list of checks:
[dev-guide.md, End-to-end run](../dev-guide.md#6-end-to-end-run).
