# Milestone 3 — Modes and lobby

Status: planned (no wave started) · Design source: [design/game-design-doc.md](design/game-design-doc.md) (Game modes, The Squeeze, Mutators, Match rules, Servers) · Previous milestone: [milestone-2-plan.md](milestone-2-plan.md) · Overall plan: [implementation-plan.md](implementation-plan.md)

Milestone 3 turns the single-mode server (Paint the Void, one room, a restart by disconnect after every match) into a game with every PvP mode from the design doc, the Instagib mutator, duels, and a lobby: each `ls-server` hosts its own lobby and match rotation, a small optional directory service feeds the server list, and invites and party codes bring friends into the same match. It also takes over the five carried-over M3 items from [implementation-plan.md §8](implementation-plan.md#carried-over-items).

Decisions taken before planning:
- Full M3 scope: all listed modes, duels, lobby, server list, invites, party codes, and all five carried-over items.
- The lobby runs inside `ls-server`: every server hosts its own lobby and match rotation. A small, optional directory service collects servers for the server list; a server works without it (direct link or address).

## 1. Scope

| Area | In scope |
| --- | --- |
| Mode framework | One module per mode (`crates/ls-sim/src/modes/<id>.rs`) behind a hook trait like weapons; 1 to 4 teams or free-for-all; lives and elimination; replicated mode state (scores, phase, timer, objectives) for every mode; Paint the Void ported onto it unchanged in behaviour |
| Team Brawl | 2 to 4 teams, 4 to 32 players, team deathmatch with respawns; first team to the kill target wins |
| The Squeeze | Free-for-all, no respawns; walls grow inward from the level boundary in phases (20 s warning, 30 s close), safe-zone centre from the seed and inside the previous zone, crushing damage, closing walls shed scrap debris; last ship flying wins |
| Instagib (mutator) | Railspike only, unlimited ammo, any hit kills; no pickups, no gadgets; hull and energy hidden, boost kept; Railspike stays a 270 u/s projectile; lasting effects on; instant respawn in modes with respawns. Works on every PvP mode |
| Heist | Mine a core deep in a collapsing mine and carry it to the surface; the carrier cannot boost |
| Hot Reactor | Pass the reactor around; whoever holds it at detonation loses a life; the fuse runs faster the more ships are near it |
| Last Color Standing | Monochrome level; kills drain the victim's colour; the last player with any colour wins |
| Duels | Two-player matches; challenge any player on the server list directly, with a custom seed (and optional rule set code) |
| Lobby (in `ls-server`) | Lobby state between matches (results, next mode and world, ready-up/countdown, team choice), match rotation from a playlist file, the next match built off the sim thread, players stay connected across matches |
| Server list | Optional directory service (new crate `ls-directory`): servers register and heartbeat; the browser lists servers (name, mode, players, region, ping) and joins one. Servers without it stay reachable by link |
| Invites and party codes | Invite links carry server address + world code + rule set code (+ party code); party codes group players so they join the same server, team and private room |
| Carried over (from M1) | **Match rebuild blocks joins**: build the next match (0.6–1.6 s) off the sim thread and swap it in; matters more with lobby-driven rotation |
| Carried over (from M2) | **Tectonic collapse tick spike**: spread the about 30 crack craters over the following ticks (sim behaviour change, bump `SIM_BEHAVIOR_VERSION`) |
| Carried over (from M2) | **Weapon state on the wire**: `WeaponState::shots` in `Message::Inventory` (no inferred Scatter/Corkscrew fan phase) and a weapon-state byte in `NetWeapon` (Paint Thrower colour survives a rejoin) |
| Carried over (from M2) | **Own Leech Arc beam drawn locally** instead of one round trip late |
| Carried over (from M2) | **Bot navigation cost**: share one nav grid per match between bots in one process, or cache it per share code (2–3 s CPU per bot per match today) |

Out of scope (M4+): coop modes (Deep Dig, Tethered, Escort the Intern), The Architect, pigment pickups and the stealth/power trade-off of monochrome levels (M4, comedy director and sliders), ranked playlists with a live balance list pushed from a central service, accounts and persistent friends lists, matchmaking by skill, regional orchestration (Agones/Nomad, M5), WebRTC fallback (M5). The remaining M5 carried-over items stay in M5.

## 2. Current state (start of M3)

- `ls_sim::mode` holds only `PaintTheVoid` (timer, coverage, `MatchPhase::{Running, Ended}`); `World::mode()` returns it directly. `ls_sim::world::TEAMS = 2` is a constant; `PaintCoverage::team_cells` is `[u64; 2]`; `add_player(team: None)` picks the smaller of two teams.
- `RuleSet::mode` is `ModeRules::PaintTheVoid(PaintTheVoidRules { teams, match_secs, respawn_secs })`; `Mutator::Instagib` exists in the type but is not implemented; rule set codes are format version 3; two built-in rule sets (`paint_the_void.ron`, `sandbox_all_weapons.ron`).
- `ls-server` runs one `Room` on one sim thread (`crates/ls-server/src/sim.rs`). At match end it shows the result for `--intermission` (10 s), then disconnects everyone with `MATCH_RESTART_REASON`; the first join afterwards rebuilds the match on the sim thread (`Sim::new_match`), a 0.6–1.6 s stall. There is no lobby, no rotation, no second room.
- The HTTP endpoint (`http.rs`) serves `/ws`, `/cert-hash` and the static web root; there are no JSON endpoints.
- Protocol 4, `SIM_BEHAVIOR_VERSION` 4, generator version 2.

## 3. Design decisions for gaps in the design doc

**Every row was accepted** (marked *Accepted*). Numbers live in `data/` (mode settings in rule sets) and are tuning starting points. Rows rejected or changed in review are updated here before the wave that needs them starts.

### 3.1 Framework and lobby

| Topic | Decision |
| --- | --- |
| Mode modules | *Accepted.* A `ModeModule` trait mirroring the weapon registry: hooks `on_start`, `on_tick`, `on_kill(victim, killer, weapon)`, `on_damage` (for modes that rewrite damage, e.g. Instagib, Last Color Standing), `on_spawn`, `allow_respawn`, `on_pickup`, `end_check`, plus a `ModeView` that the world replicates. One file per mode in `crates/ls-sim/src/modes/`, registered in `modes/mod.rs`. Mutators are a second, smaller hook set (`mutators/<id>.rs`) applied after the mode |
| Teams | *Accepted.* `teams: 0` means free-for-all (every ship its own side; friendly fire not applicable); 2–4 teams otherwise. `TEAMS` becomes per-match; team colours 0–3 come from a fixed accessible palette (adds two team colours to the client). Paint coverage grows to `[u64; 4]` |
| Generic mode state on the wire | *Accepted.* One `ModeState` wire item (protocol 5) for every mode: phase, phase end tick, up to 4 team scores or the top 32 player scores, lives per player, and up to 8 objective markers (id, kind, position, holder, timer). Sent on change, delta-coded like entities. Mode-specific visuals are data-driven where possible |
| Lobby state machine | *Accepted.* `Lobby → Countdown (10 s) → Match → Results (10 s, `--intermission`) → Lobby`. In the lobby, players fly in the current or next level with weapons off ("warm-up") rather than sitting in a menu; the countdown starts when at least the mode's minimum players are present and either everyone is ready or 30 s have passed |
| Match transition | *Accepted.* Players stay connected: the server sends a new `Welcome` (new world code, rule set, mode, team) on the same connection instead of `Disconnect`. The client tears down and rebuilds the world without a reconnect. The next match is built on a worker thread during Results and swapped in at the end of Results (carried-over item) |
| Rotation | *Accepted.* `ls-server --playlist <file>`: a RON list of entries (rule set file or `LR` code, world code or `random` with slider ranges, min/max players). Default playlist: Team Brawl, The Squeeze, Paint the Void, Hot Reactor, Heist, Last Color Standing, in order, random worlds. `--ruleset` keeps working as a one-entry playlist |
| Next-match vote | *Accepted.* None in M3: rotation order only (keeps the lobby small). A vote of three options is listed as an open question |
| Mode player limits | *Accepted.* A playlist entry is skipped while the player count is outside its range (e.g. Team Brawl needs 4). If nothing fits, the server falls back to Team Brawl with 2 teams, then free-for-all Team Brawl ("deathmatch") for 2–3 players |
| Join mid-match | *Accepted.* Modes with respawns: join immediately on the smaller team. Elimination modes (The Squeeze, Last Color Standing, Hot Reactor after its first detonation): join as a spectator until the next match |
| Spectators | *Accepted.* A minimal free-fly spectator camera (no ship, server sends snapshots) for eliminated players and late joiners; no spectator-only slots in M3 |
| Multiple rooms per server | *Accepted.* `ls-server` hosts one public room (the rotation) plus up to `--private-rooms N` (default 4) private rooms for duels and parties, each on its own sim thread. `Hello` gains an optional room code; private rooms close when empty. The tick budget (4 ms p99) applies per room; total rooms are capped so the machine's cores suffice |
| Party codes | *Accepted.* 6-character codes (Crockford base32, `P-XXXXXX`), created by the server for a party leader; valid while the party exists on that server. Members join the leader's room and the same team (team modes) where team balance allows (max team size difference 1, else the party is split by join order). Parties live on one server only; no cross-server party state in M3 |
| Invite links | *Accepted.* `https://<host>/?join=<address>&code=<LS..>&rules=<LR..>&party=<P-..>`; every part optional. Opening it joins that server; with a world/rule set code but no party code it opens a private room with those settings (a "custom match") |
| Duels | *Accepted.* A duel is a private two-player room with a seed (world code) and rule set chosen by the challenger (default: the duel rule set, Team Brawl free-for-all, first to 10 kills or 5 minutes). Challenge flow: from the server list's player list or the in-game scoreboard; the challenged player gets a toast with accept/decline (30 s); accepting moves both to a new private room on the challenger's server. Cross-server challenges go through the directory (it only relays the invite; the duel runs on the challenger's server) |
| Directory service | *Accepted.* New binary crate `ls-directory` (Rust, same HTTP style as `ls-server`, no database: in-memory with a TTL): `POST /servers` heartbeat every 10 s (name, public address, WebTransport cert hash, region tag, mode, map code, players/max, version + data hash), entries expire after 30 s; `GET /servers` returns the list as JSON (CORS for the web client); `POST /invites` / `GET /invites?player=` relays duel challenges by short-lived player token. Servers opt in with `--directory <url>`; `--server-name`, `--region` and `--public-address` describe them. Clients measure ping themselves (WebSocket ping to each listed server, parallel, capped) |
| Directory trust | *Accepted.* No authentication in M3 beyond a per-server random secret for updating its own entry; the list shows only servers whose protocol and data hash match the client's build. Abuse handling (rate limit per IP, max entries) is basic; hardening is M5 |
| Player identity | *Accepted.* No accounts. A random player token stored in browser `localStorage` (with fallback to per-session) plus the chosen name identifies a player for invites and parties for the lifetime of the token |

### 3.2 Modes

| Topic | Decision |
| --- | --- |
| Team Brawl | *Accepted.* Kill target scales with players: `max(15, 5 × players / teams)`, capped at 50; 10 minute timer, most kills wins on time, draw if equal. Respawn 3 s (instant under Instagib). Suicide and team kills cost the team one point. Default rule set: Scrap Laser + 3 Bolt Rockets, standard pickups, friendly fire 50 % |
| The Squeeze: walls | *Accepted.* Implemented as a runtime terrain layer in `ls-game`'s `GameTerrain` (a solid shell SDF `max(shell(t), terrain)` evaluated per chunk and remeshed as it moves), not as a generator change, so `GENERATOR_VERSION` and the worldgen golden hashes stay. Each phase's safe sphere (centre and radius) is derived deterministically from the seed through a new RNG stream (`squeeze`); the shell moves in steps of 0.5 u per tick-batch to bound remeshing |
| The Squeeze: numbers | *Accepted.* 5 phases; phase 1 starts 60 s after the match start; each phase warns 20 s and closes over 30 s; the safe radius shrinks to 60 %, 45 %, 30 %, 18 %, 8 % of the level's radius; crushing: 25 hull per second of contact with a moving wall (ignores shields); closing walls spawn 3 scrap debris pieces per phase per 100 u² of closed opening, capped by the debris cap. After the last phase the shell keeps closing slowly until one ship is left |
| The Squeeze: no respawns | *Accepted.* Each player has one life; ties (last two die in the same tick) are a draw. Eliminated players spectate |
| Instagib | *Accepted.* Implemented as a mutator that rewrites the effective rule set (only the Railspike, loadout-only, unlimited ammo; no pickups; gadgets banned) and a damage hook (any hit from an enemy Railspike kills; self-hits and splash do not). The Railspike keeps piercing ships and walls. The HUD hides hull and energy bars. Friendly fire still uses the rule set setting |
| Heist: level | *Accepted.* The generator places a "core vein" chamber deep in the level (at the farthest reachable point from the surface openings) and marks extraction zones at the level's open surface (Enclosure sliders below 100 %) or, for fully enclosed levels, at each team's spawn. This is a generator change (bump to generator version 3, re-bless golden hashes) |
| Heist: mining and carrying | *Accepted.* The core is an entity embedded in strong rock; it is freed by removing the rock around it (any weapon's craters count) and picked up by touch. The carrier cannot boost, has 75 % top speed and is shown to everyone. "Collapsing mine": once the core is freed, the chamber region's rock strength drops and the ceiling sheds debris for 30 s. The carrier dropping it (death) leaves the core loose for 20 s, then it returns to the vein |
| Heist: teams and scoring | *Accepted.* Team mode (2 teams): a delivery scores 1 point, the core respawns in the vein after 15 s; first to 3 deliveries or most in 12 minutes wins. Free-for-all variant for fewer than 4 players |
| Hot Reactor: passing | *Accepted.* The reactor is held by one ship and passes on any hit the holder takes from another ship (the shooter becomes the holder) or by ramming; the holder can't drop it. The new holder is immune to passing it back for 1 s |
| Hot Reactor: fuse | *Accepted.* Base fuse 20 s, shown to all; each other ship within 15 u of the holder adds 0.5× fuse speed (so 2 ships near = 2× speed). At detonation the holder loses a life (3 lives each) and the blast (10 u, 60 damage) hurts nearby ships too; a new reactor appears on a random living player after 3 s. Last player with lives wins; respawns use the normal timer while lives remain |
| Last Color Standing: colour | *Accepted.* Each player has a colour meter (100). Kills drain the victim's colour by 34 (three deaths drain it fully); damage alone does not. A player at 0 colour is out (spectates). Free-for-all only. The level renders monochrome (client shader flag from the mode); ships are drawn in their player colour scaled by the meter, so colourful ships are easy to spot. Pigment pickups that restore colour are M4 (with monochrome levels as a general feature) |
| Last Color Standing: stalemate | *Accepted.* After 8 minutes every remaining player loses 1 colour per 2 s, so the match always ends |
| Mode end and results | *Accepted.* Every mode ends with a winner (team or player) or a draw and a results table (kills, deaths, mode score); the results feed the lobby's Results phase. No persistent stats in M3 |
| Bots in modes | *Accepted.* `ls-bots` plays every mode: objective goals (carry the core home, flee the reactor holder or pass it, stay inside the safe zone, chase the leader in Last Color Standing) layered on top of the existing hunt/fight behaviour. Bots also fill the server list end to end (`--directory`) in e2e |
| Bot navigation cost | *Accepted.* Cache the nav grid per share code (+ generator version) in a process-wide `Arc` map in `ls-bots`; bots of the same match share one grid; built once on a worker thread |
| Tectonic cracks | *Accepted.* Spread the crack craters over the next 6 ticks (5 per tick), in the existing deterministic order; the 35 u core crater stays in the collapse tick |

## 4. Determinism and handshake impacts

| Item | Change | Owner of the change |
| --- | --- | --- |
| `SIM_BEHAVIOR_VERSION` | 4 → 5 in wave A (mode framework, per-match teams, Tectonic crack spreading). Each mode module that lands in wave B changes behaviour only for its own rule sets, but the version still bumps once per wave integration if any default behaviour changed (integrator decides; the version is mixed into the data hash, so a stale client is rejected) | A1, then the wave integrator |
| Data hash | Changes automatically with every new or edited `data/rulesets/*.ron` (one default rule set per mode, the duel and instagib rule sets) and every playlist that is embedded. Playlists are server-side files, not embedded, so they don't enter the hash; the rule sets they reference do through the `rules_code` check | every B package |
| `REPLAY_GOLDEN_HASH` | Re-blessed in wave A (crack spreading changes the replay if the replay uses a Tectonic Charge; per-match teams must not change it). New per-mode replay goldens (`crates/ls-sim/src/tests.rs`, one short replay per mode) added in wave B and checked on wasm32 too | A1, each B mode package |
| Worldgen golden hashes | Unchanged by The Squeeze (runtime layer). Heist's core vein and extraction zones bump `GENERATOR_VERSION` 2 → 3 and re-bless (`LS_BLESS_GOLDEN=1`), plus re-encoded share codes in tests and tools (codes carry the generator version) — scheduled once, in B3 | B3 |
| Protocol | 4 → 5 in wave A: `ModeState` item, `Welcome` on an open connection (match transition), lobby messages (`LobbyState`, `Ready`, `PickTeam`, `Spectate`), optional room/party code in `Hello`, `InventoryMsg` with `shots`, `NetWeapon` state byte. Further additions in wave B (duel challenge relay) stay within protocol 5 if made before wave B integration, else 6 | A2 |
| Rule set codes | Format version 4: new `ModeRules` variants and their settings, mutators list, teams 0–4. Version 3 codes keep decoding. `LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode` regenerates `web/src/rules/catalog.ts` | A1 (format), each B package (catalog) |
| WASM cross-check | `tools/determinism-check.mjs` gains one Heist world (generator version 3) and the per-mode replay hashes | B3, C3 |

## 5. Waves

Each wave is a set of work packages that run in parallel in separate work folders (`loosescrews.win2`+), one package per folder, as in M2. Packages in one wave touch disjoint files except where listed under "Shared files"; the integrator (win1) merges them in the listed order. Dependencies between waves are explicit; a package may start before its dependency is merged only against the listed day-one contract.

### Wave A — foundations

| Package | Work | Owner areas | Depends on |
| --- | --- | --- | --- |
| A1 Mode framework | `ModeModule` trait, mode registry, `modes/paint_the_void.rs` (port, behaviour unchanged), stub modules for the six new modes and the Instagib mutator (generic fallback, like M2's weapon stubs), per-match teams 0–4, lives and elimination, spectator state in `World`, `ModeView` for replication, `ModeRules` variants with their settings, rule set code format 4; Tectonic crack spreading (carried over); `SIM_BEHAVIOR_VERSION` 5, `REPLAY_GOLDEN_HASH` re-bless | `crates/ls-sim` (`mode*`, `modes/`, `mutators/`, `world.rs`, `ruleset.rs`, `rulecode*`, `weapons/ordnance/tectonic_charge.rs`, `tests.rs`), `data/rulesets/` | — ; **day-one contract**: `ModeView` struct and `ModeRules` variant list pushed on the A1 branch first, so A2 and A3 code against it |
| A2 Protocol 5 and client glue | `ModeState` wire item and its delta coding, lobby messages, `Welcome` on an open connection, room/party code in `Hello`, `InventoryMsg::shots` and `NetWeapon` state byte (carried over); `ClientSession` world teardown and rebuild on a new `Welcome`; `Room` mode-agnostic (scoreboard and results from `ModeView`); `ls-wasm` exposes mode state, lobby state and the weapon-state fields | `crates/ls-net`, `crates/ls-game`, `crates/ls-wasm` (no `web/`) | A1 day-one contract |
| A3 Server lobby, rotation, rooms | Lobby state machine, playlist file and `--playlist`, next match built on a worker thread and swapped in (carried over), no disconnect at match end, room registry with one public and `--private-rooms N` private rooms (each on its own sim thread), routing by room code, `tick stats` per room; `ls-server --help` and `userDocs/hosting-guide.md` flags | `crates/ls-server` | A1, A2 day-one contracts (A2 pushes its message enum first) |
| A4 Directory service | New crate `crates/ls-directory` (binary + small client lib for `ls-server`), JSON API from §3.1, TTL expiry, CORS, version/data-hash filter, invite relay; tests with servers on `127.0.0.1:0`; `tools/check-binds.mjs` allowlist if needed | `crates/ls-directory`, workspace `Cargo.toml` | — |
| A5 Bot navigation cache | Process-wide nav grid cache per share code, shared by bots of one match, built off the bot loop (carried over); measure CPU per bot before/after. Follow-up from VQ0: the nav clearance still uses the SDF bound (conservative); it may switch to the contact estimate `ls_sim::terrain::surface_estimate` so bots use the gaps humans fit through | `crates/ls-bots` (`nav.rs`, bot setup) | — |

Shared files: workspace `Cargo.toml` (A4 adds a member), `CLAUDE.md`/`docs/dev-guide.md` (each package adds its commands; integrator resolves).

Merge order: A1, A2, A3, A4, A5.

Acceptance (wave A):
- A1: Paint the Void replays bit-identically through the framework except for the deliberate re-bless (the re-bless is explained by the Tectonic change alone: a replay without a Tectonic Charge keeps its hash); every new mode stub builds a world and ends by timer; teams 0–4 spawn and balance (unit tests); rule set code format 4 round-trips every built-in rule set and still decodes format 3 codes; a single Tectonic collapse tick costs under 2 ms in the existing scenario test, and the crack set after 6 ticks equals the old one.
- A2: protocol 5 round-trip tests for every new message; a client session survives three consecutive match transitions on one connection in an in-process test (no reconnect, no full-snapshot storm: at most 1 full snapshot per transition); Paint Thrower colour survives a rejoin; the shot predictor uses `shots` (no fan-phase inference left).
- A3: two matches in a row on one connection in an integration test; joins during the next-match build get their `Welcome` within 200 ms; a private room is created by code, runs alongside the public room, and closes when empty; playlist entries outside their player range are skipped; `--ruleset` alone still works.
- A4: register, heartbeat, expiry and list over HTTP in tests; mismatched version/data hash filtered out; server runs unchanged without `--directory`.
- A5: second bot in the same match does not rebuild the grid (counter in a test); CPU per bot per match at least 5× lower for 8 bots on one world.

#### As built (wave A)

Integrated on WIN1 in package order A1-A5; the notes per package record what differs from the table above and what stays open.

A1 as built (notes for the integrator and wave B):
- `REPLAY_GOLDEN_HASH` is **unchanged** (`0x8321_c0ab_902b_8ee5`, native and wasm32): the scripted replay uses no Tectonic Charge, so Paint the Void runs bit-identically through the framework and nothing needed a re-bless. The crack change is pinned instead by `tectonic_charge::tests::cracks_spread_over_six_ticks_and_match_the_m2_set` (the crack edits after 6 ticks hash to the M2 collapse-tick set at Instability 50 and 100). `SIM_BEHAVIOR_VERSION` is 5; the data hash also changes through the new `crack_spread_ticks` parameter.
- Collapse tick: the cracks (16 per tick at Instability 100) now cost about 0.5 ms per tick on a generated level, but the collapse tick itself stays at about 3.5-5 ms in release (`cargo test -p ls-game --release --test tectonic_tick`): the single exact 35 u collapse crater's sample bake in `ls_worldgen::ChunkStore::apply_geometry` / `bake_edit` (every corner of the crater's reach box, with a `room_open` generator call each) is that cost by itself. The "< 2 ms" criterion is therefore not met; it needs a cheaper bake in `ls-worldgen` (for example skipping `room_open` for corners the edit leaves unchanged, or baking big craters lazily per chunk), a worldgen package's job. The test asserts < 2 ms for the crack ticks and < 8 ms for the collapse tick.
- Rule set codes: format 4 (`CODE_VERSION`), full/diff bit and 4-bit mode index in the header, mode settings as the mode's generic field list; formats 2 and 3 still decode (tested with M2 codes). The default Paint the Void code is now `LR8V-6097-G000-TMBG`. Each mode's diff base is `RuleSet::default_for(kind)`: its own file once a B package adds `data/rulesets/<file>.ron` to `BUILTIN_RULESETS` (`ModeKind::ruleset_file`; The Squeeze's is `squeeze.ron`), else Paint the Void's rules with the mode's settings and without the Paint Thrower. Adding a mode's file or a settings field changes that mode's codes (re-bless the web catalog). The web editor encodes and validates every mode; picking a mode in the editor UI is not built yet.
- Free-for-all ships carry their own side id in `ShipState::team` (0-253). Resolved at integration without a wire change: `NetPlayer::team`, `NetModePlayer::side`, `ResultRow::side` and `Welcome::team` are full bytes; `ship_flags` keeps its 2 team bits (the low bits of the side). Readers take the side from the players list (`ClientSession::side_of`, the own `team()`, bots' contacts, the wasm scoreboard and ship views), and `NetConv::apply_net_ship` keeps a full side id that agrees with the flag bits.
- Open: the collapse tick (3.5-5 ms against the < 2 ms target, see above) waits for an `ls-worldgen` bake optimisation.

A2 as built (protocol 5 and client glue):
- As planned: `NetModeState` mirrors `ls_sim::ModeView`; results carry kind, teams, winner side and kills/deaths; `LobbyState::next_mode` is a `ModeKind` index (A3 had coded against a string; fixed at integration). `Room` is mode-agnostic, keeps `RoomConfig::epoch`, drops inputs of another epoch and sends `MatchResults` once when the mode stops running (and to clients joining a finished match). `ClientSession` rebuilds in place on a new `Welcome` and drops snapshots of another epoch.
- Deviation: A2 first built its own mode view, kill/death `Tally` and results in `ls_game::mode` because A1's `World::mode_view()`/`World::results()` were not on its branch. At integration the room switched to A1's (`World` counts kills and deaths per ship); `ls_game::mode` keeps only the wire conversions (`mode_view_to_net`, `results_to_net`, `legacy_scores`).
- Integration fix: a delta snapshot of the previous match arriving after the new `Welcome` failed to decode (its baseline is gone) and ended the session; `ClientSession::handle_packet` now drops other-epoch snapshots by `Message::peek_snapshot_epoch` before decoding (found by `ls-bots` `tests/match_transition.rs`).
- Bots: A2's interim "rejoin on a match transition" is replaced at integration by an in-place transition (`BotCore::next_match` rebuilds the brain, navigation grid and spawn points for the new level on the same connection; counted as `match_restarts`). The `MATCH_RESTART_REASON` rejoin path stays for older hosts.

A3 as built (server lobby, rotation, rooms):
- As planned: lobby state machine, `--playlist`, default rotation of the built-in mode rule sets with player-range skipping and the Team Brawl fallback, next room built on an `ls-build` worker thread and swapped in without a disconnect, one public room plus `--private-rooms N`, routing by `Hello::room`, rooms closing when empty.
- Deviation: the warm-up is a separate room (the next match's level with every weapon, gadget and pickup removed and a 1 h timer), built and swapped in like a match room, rather than flying in the finished match's level. A warm-up of a mode that can end early (kill target, last side standing) would stop and send results; the warm-up rules should also clear the mode's early-end conditions once wave B fills in those modes.
- Integration fix: every room built by the sim thread gets the next match epoch (`RoomConfig::epoch` = current room's + 1, wrapping; A3 built every room with epoch 0), so inputs and snapshots of the previous room still in flight are dropped (test: `finished_match_moves_everyone_to_the_next_match_on_the_same_connection` checks the epochs, exactly one `MatchResults` per match, and a stale input counted in `stale_epoch_inputs`). `MatchResults` comes from `Room` alone; `ls-server` does not send its own.
- `tools/e2e.mjs` and the browser smoke run the server with `--warmup 0 --countdown 0`.

A4 as built (directory):
- As planned: `crates/ls-directory` (binary + library), register/heartbeat/expiry/list, CORS, protocol and data-hash filters, invite relay, rate limit; `DirectoryClient` and `spawn_heartbeats` for B7.
- Deviation: plain HTTP only (no TLS in the binary); a public directory belongs behind a TLS-terminating reverse proxy (hosting guide). The browser on an HTTPS page cannot fetch a plain-HTTP directory (mixed content), which B7 must account for.
- Integration: the branch had committed ~560 MB of build output under `crates/ls-directory/target`; WIN1 merged a rewritten copy of the commit without it, and `.gitignore` now also ignores `crates/*/target/`.

A5 as built (bot navigation cache):
- As planned: process-wide nav grid cache per share code, generator version and `NavConfig`, built once on a `nav-build` worker thread; `nav_cache::builds()` counter; measured 8.4-9.1x less navigation CPU per bot for 8 bots of one world (criterion: at least 5x). A failed build falls back to the per-bot incremental build.

### Wave B — modes and lobby features

| Package | Work | Owner areas | Depends on |
| --- | --- | --- | --- |
| B1 Team Brawl + Instagib | `modes/team_brawl.rs` (2–4 teams and free-for-all deathmatch), `mutators/instagib.rs`, default rule sets `team_brawl.ron`, `instagib_brawl.ron`, `duel.ron`, scenario tests (kill target, team kill penalty, instant respawn, any hit kills, no pickups/gadgets) | `crates/ls-sim/src/modes/team_brawl.rs`, `mutators/`, `data/rulesets/` (own files) | A1 |
| B2 The Squeeze | `modes/the_squeeze.rs` (phases, safe zone RNG stream, crushing, scrap from walls, elimination), shell layer in `GameTerrain` with bounded remeshing, `squeeze.ron`, scenario tests; contracts.md: new RNG stream and the shell layer | `crates/ls-sim/src/modes/the_squeeze.rs`, `crates/ls-game/src/terrain.rs` (shell layer only), `data/rulesets/squeeze.ron`, `docs/architecture/contracts.md` | A1, A2 |
| B3 Heist | Generator: core vein chamber and extraction zones (generator version 3, golden re-bless, share codes re-encoded in tests/tools); `modes/heist.rs` (core entity, mining free, carrier limits, drop/return, collapse, deliveries), `heist.ron`, scenario tests | `crates/ls-worldgen`, `crates/ls-sim/src/modes/heist.rs`, `data/rulesets/heist.ron`, tools/test codes | A1 |
| B4 Hot Reactor | `modes/hot_reactor.rs` (holder, passing, proximity fuse, blast, lives), `hot_reactor.ron`, scenario tests | `crates/ls-sim/src/modes/hot_reactor.rs`, `data/rulesets/hot_reactor.ron` | A1 |
| B5 Last Color Standing | `modes/last_color_standing.rs` (colour meter, drain on kill, stalemate drain, elimination), `last_color_standing.ron`, scenario tests | `crates/ls-sim/src/modes/last_color_standing.rs`, `data/rulesets/last_color_standing.ron` | A1 |
| B6 Lobby client | Browser: lobby/warm-up overlay (next mode, world, countdown, ready, team pick), results screen per mode, spectator camera, match transition without reload, generic mode HUD from `ModeState` (scores, timer, lives, objective markers) | `web/src/ui/` (new `lobby.ts`, `results.ts`, `modeHud.ts`), `web/src/game/` (transition) | A2, A3 |
| B7 Server list, invites, parties, duels | `ls-server` directory client (`--directory`, `--server-name`, `--region`, `--public-address`), party creation and routing, duel challenge flow (local and via directory relay), invite links; browser: server list screen with ping, invite link handling in `main.ts`, party panel, challenge toast | `crates/ls-server` (new `directory.rs`, `party.rs`, `duel.rs`), `web/src/ui/` (new `serverList.ts`, `party.ts`), `web/src/net/` (ping probe) | A3, A4 |

Shared files: `crates/ls-sim/src/modes/mod.rs` registry and `data/rulesets` list in `ruleset.rs` (`BUILTIN_RULESETS`): each B1–B5 package adds one line; the integrator resolves. `web/src/rules/catalog.ts` is regenerated after the merge, not by hand. `web/src/main.ts` is touched by B6 (transition) and B7 (invite links): B6 owns it, B7 adds a single call into its own module.

Merge order: B1, B4, B5 (sim only), B3 (generator bump), B2 (terrain layer), B6, B7.

Acceptance (wave B):
- Each mode B1–B5: a scenario test per core rule from §3.2; a 60 s replay per mode with a golden hash that matches on native and wasm32; the default rule set loads, encodes to an `LR` code and back, and runs a full match with 8 in-process bots-in-sim (headless `Room` test) to a result.
- B2: shell remeshing stays within the per-frame remesh budget in the client benchmark (no chunk remesh above 2 ms per tick-batch); crushing and safe-zone placement deterministic across seeds (8 seeds test).
- B3: worldgen golden hashes re-blessed with generator version 3; the core vein is reachable (connectivity check like `CONNECT_MARGIN`) on the 8-seed × 25-slider setup sweep; `node tools/determinism-check.mjs` passes.
- B6: the browser smoke test (extended in C) can go lobby → match → results → next match without reload in headless Chrome.
- B7: server list shows two local servers registered with a local directory; invite link with party code puts two clients in the same room and team; a duel challenge creates a private two-player room with the challenger's seed.

### Wave C — client polish, bots, tests

| Package | Work | Owner areas | Depends on |
| --- | --- | --- | --- |
| C1 Mode visuals | Squeeze wall glow/grind and next-zone marker, Heist core and carrier markers, extraction zones, reactor fuse ring and holder highlight, monochrome shader for Last Color Standing with colour-meter ship tint, Instagib HUD (hidden bars), 4 team colours; own Leech Arc beam drawn locally (carried over) | `web/src/renderer/`, `web/src/game/mechanics.ts`, `crates/ls-wasm/src/mechanics.rs` | B1–B6 |
| C2 Bots in modes | Objective behaviours per mode, spectating when eliminated, accepting duels (flag), `--directory` join; `tests/mode_matches.rs`: one release test per mode (8 bots, full match to a result, 0 desyncs) | `crates/ls-bots` | B1–B5, B7 |
| C3 End-to-end and smoke | `tools/e2e.mjs`: `--playlist` runs a rotation of all modes on one server with reconnect-free transitions, `--directory` with a local directory; checks per mode (result reached, transitions = matches − 1 without disconnects); browser smoke: lobby, transition, server list, invite link, duel; per-mode replay hashes in `determinism-check.mjs` | `tools/`, `tools/browser-smoke/` | B6, B7, C2 (bots) |

Acceptance (wave C): see the milestone criteria below; each package's tests are part of `tools/ci.*` (release bot tests, browser smoke).

### Wave D — review, docs, full CI

Review of all packages, fixes, `pwsh tools/ci.ps1` green, e2e rotation soak (below), docs: new `docs/architecture/modes-and-lobby.md` (mode framework, lobby, rooms, directory, parties, duels), updates to `netcode.md` (protocol 5), `server.md`, `game-glue.md`, `contracts.md`, `determinism.md`; `userDocs/player-manual.md` (every mode, lobby, server list, invites, parties, duels), `userDocs/hosting-guide.md` (playlists, private rooms, directory, all new flags), a directory hosting section; `node tools/gen-weapon-docs.mjs` extended to a generated mode reference (`userDocs/reference/modes.md`) from the rule sets; carried-over items closed or rescheduled in `implementation-plan.md §8`.

## 6. Acceptance criteria (milestone)

- Every PvP mode in the design doc (Team Brawl, The Squeeze, Heist, Hot Reactor, Last Color Standing, Paint the Void) and the Instagib mutator exist as modules with a default rule set in `data/rulesets/`, behave as described in the design doc and §3.2, and have scenario tests for each core rule.
- Instagib combines with Team Brawl, The Squeeze and a duel (tests for each combination).
- Replay determinism and the WASM golden-hash check pass, including one replay per mode; `REPLAY_GOLDEN_HASH`, worldgen golden hashes and `SIM_BEHAVIOR_VERSION` changed deliberately and documented.
- A server rotates through all modes of the default playlist with 8 bots for 60 minutes: every match reaches a result, players stay connected across transitions (0 disconnects caused by match end), 0 desyncs, 0 protocol errors, at most 1 full snapshot per join or transition, no join waits more than 200 ms for its `Welcome` during a next-match build, tick p99 < 4 ms per room on a quiet machine.
- The Squeeze's walls close in every phase on 8 seeds without a chunk remesh spike above the client's frame budget, and no ship survives inside the shell.
- A private duel and a party room run alongside the public room on one server, each within its tick budget.
- The server list shows servers registered with `ls-directory`, with players, mode and ping; servers without `--directory` stay reachable by link.
- Invite links (world + rule set + party code) and party codes put friends in the same room and, in team modes, on the same team; a duel challenge from the server list reaches the other player and starts a private match with the challenger's seed.
- Bots play every mode toward its objective (release tests: each mode reaches a result with 8 bots), and bot nav grids are built once per world per process.
- All five carried-over items are closed with evidence (tests or measurements).
- docs/ and userDocs/ describe the modes, the lobby, the server list, invites, parties, duels and all new server flags as built.

## 7. Risks

| Risk | Mitigation |
| --- | --- |
| The Squeeze remeshes many chunks every tick | Move the shell in steps, remesh only chunks the shell crosses, spread over frames; measure in B2 before the visuals in C1 |
| Several rooms per process break the tick budget | Rooms on their own threads, cap private rooms, per-room `tick stats`; e2e with public + 2 private rooms |
| Match transition without reconnect leaks client state (old world, old terrain, predictor) | A2's three-transition in-process test; browser smoke runs several transitions and checks memory/handles |
| Generator version 3 (Heist) churns share codes across tests, tools and docs | One package (B3) owns the bump and re-encodes every code in the same change, as in M2 |
| Wave B file overlap in the mode registry and rule set list | One-line additions only; integrator merges in the listed order |

## 8. Decisions

Every proposal in this plan was accepted as written, including the proposed answers to the former open questions: warm-up flying between matches and rotation order without a vote; Heist delivers at the open surface, with the team spawn as the fallback on enclosed levels (generator version 3); Last Color Standing ships without pigment pickups (they come in M4); cross-server duels run on the challenger's server, with the directory only relaying the invite; no public directory by default (hosts pass `--directory`, the client takes `?directory=`); an anonymous browser token for identity; one public room plus `--private-rooms 4` per server; free-for-all Team Brawl as the default duel mode; the live balance list stays with ranked playlists (later).
