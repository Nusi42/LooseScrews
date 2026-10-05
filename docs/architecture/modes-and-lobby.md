# Modes and lobby (M3)

How a server plays more than one mode: the mode framework in `ls-sim`, the lobby and match
rotation in `ls-server`, rooms, the optional directory, parties, invite links and duels, and
what the browser and the bots do with them. Plan and decisions:
[milestone-3-plan.md](../milestone-3-plan.md); the numbers of every built-in rule set:
[userDocs/reference/modes.md](../../userDocs/reference/modes.md) (generated).

| Piece | Where | Details |
| --- | --- | --- |
| Mode framework, modes, mutators | `crates/ls-sim/src/mode.rs`, `modes/<id>.rs`, `mutators/<id>.rs`, `data/rulesets/*.ron` | [below](#mode-framework); each module's `//!` docs hold its rules and gap decisions |
| Mode state on the wire, match transitions, lobby messages | `crates/ls-net/src/mode.rs`, `message.rs` (protocol 5) | [netcode.md](netcode.md#match-transitions-lobby-and-mode-state-protocol-5) |
| Room, client session (mode-agnostic) | `crates/ls-game/src/server`, `client`, `mode.rs` | [game-glue.md](game-glue.md#wire-conventions) |
| Lobby, rotation, rooms, parties, duels, JSON API, directory client | `crates/ls-server/src/{lobby,playlist,rooms,party,duel,api,directory}.rs` | [server.md](server.md#rooms-lobby-and-rotation-roomsrs-lobbyrs-playlistrs-m3) |
| Directory service | `crates/ls-directory` | [directory.md](directory.md) |
| Browser: lobby, results, mode HUD, spectator, server list, party panel | `web/src/game/{modeState,matchFlow,spectator,sides,modeVisuals}.ts`, `web/src/ui/{lobby,results,modeHud,modeIcons,serverList,party}.ts` | [below](#browser) |
| Bots in modes | `crates/ls-bots/src/brain/objective.rs`, `social.rs` | [server.md](server.md#ls-bots) |

## Mode framework

Every mode is a module (`crates/ls-sim/src/modes/<id>.rs`) implementing `modes::ModeModule`,
registered in `modes::MODULES` in `ModeKind` order (the rule set code's 4-bit mode index and
the wire's mode kind; append only). Settings live in the mode's `ModeRules` variant
(`ruleset.rs`, declared with `mode_settings!`: field, default, code precision); every mode has
`teams`, `match_secs` and `respawn_secs`. Mutators (`crates/ls-sim/src/mutators/<id>.rs`,
`MutatorModule`, listed in a rule set's `mutators`) rewrite the effective rule set before it
resolves (`RuleSet::effective`), may change the respawn delay and damage, let weapons fire
without paying their cost (`unlimited_ammo`, resolved into `ResolvedRules::free_fire`), and
set HUD flags; their hooks run after the mode's.

| Hook | When | Context |
| --- | --- | --- |
| `validate`, `team_counts` | `RuleSet::resolve` | the settings |
| `on_join`, `late_join_spectates` | `World::add_player` | `ModeMut` / `ModeRef` |
| `on_start` | first tick | `ModeCtx` (`Ctx` commands + mutable `ModeState`) |
| `allow_respawn`, `on_spawn` | respawns | `ModeRef` / `ModeMut` + the ship |
| `on_damage` | damage pipeline, after friendly fire | `ModeRef` + `ModeHit` (may change the amount, skip the gadget) |
| `on_hit` | after the mode's and mutators' `on_damage`, before the gadget absorbs the hit | `ModeMut` + the final `ModeHit` (Hot Reactor passes the reactor to the shooter) |
| `on_kill` | a ship died | `ModeMut` + `Kill` (victim, killer, sides, weapon) |
| `on_pickup` | a ship touches a pickup | `ModeState` + the ship |
| `on_tick`, `end_check` | last stage of `World::step` | `ModeCtx`, `ModeRef` |
| `view` | `World::mode_view` | `ModeState` |
| `move_limits` | a ship moves | the rules + the player's replicated mode flags; returns `MoveLimits` (speed cap scale, no boost). The client predictor applies the same limits from the flags in each snapshot (`Predictor::set_limits`) |
| `material_at` | a hook asks for the rock material (`Ctx::material_at`, `Ctx::crater`) | `ModeState` + tick + point; `Some(id)` overrides the terrain's (Heist's hard vein knot and weak collapsing chamber) |
| `terrain_shell` | start of every tick, before anything moves | `ModeState` + tick; the solid shell the terrain adds (`SimTerrain::set_shell`, The Squeeze; [contracts.md](contracts.md#shell-layer-the-squeeze)) |

The world keeps the generic `mode::ModeState` (part of the state hash: kind, teams, phase,
timer, stage and stage end tick, paint coverage, team scores, score target, per-player
`PlayerMode` with score, lives, meter and flags, objectives with position, holder and timer,
module scratch values) and replicates it as `mode::ModeView` (`World::mode_view`: teams, phase
with the winning side, timer, stage, team scores, the top 32 players with lives, meter and
flags, up to 8 objective markers, HUD `view_flags` such as `INSTAGIB`, `MONOCHROME`, `METER`).
`World::results` is the results table (winner or draw, kills, deaths and mode score per
player; kills and deaths are counted per ship by the simulation). A hook a module does not
override gets the generic fallback (kills score for the killer and their side, a suicide or
team kill costs a point, lives where the mode has them, the match ends when one side is left,
at `score_target` or on the timer). A two-team Paint the Void match runs exactly as in M2:
the mode state beyond the M2 fields is hashed only when it differs from a fresh two-team
Paint the Void state, so `REPLAY_GOLDEN_HASH` did not change.

**Teams and sides.** Teams are per match: `teams` 2-4 (a joining player gets the preferred
team unless that puts it two ahead of the smallest, else the smallest; ties to the lowest
index) or 0 = free-for-all, where every ship is its own side with its own side id in
`ShipState::team` (lowest free id, 0-253), so friendly fire, kill credit and every weapon's
"enemy" test work unchanged. The match setup (`ls_game::setup`) places spawn points for at
most two sides: team 0 at the low-x end, team 1 at the high-x end, or one set over the whole
level in free-for-all. Teams 2 and 3 of a 3- or 4-team match have no spawn points of their
own and cycle through all of them; free-for-all spawns take the spawn point farthest from the
other living ships. The client draws four team colours (Orange, Blue, Lime, Magenta) and a
12-hue free-for-all palette (`web/src/game/sides.ts`).

**Lives, elimination, spectators.** A player whose `PlayerMode` is eliminated or spectating
never respawns and leaves play at the end of the tick (`Event::Eliminated` when eliminated);
the view keeps listing it with its flags. Players joining after
`modes::LATE_JOIN_GRACE_TICKS` (10 s) in modes with lives spectate until the next match (Hot
Reactor: after its first detonation); `World::set_spectator` switches a player by choice.
Spectators have no ship; the server keeps sending them snapshots and the browser flies a free
camera.

### The modes as built

| Mode (module) | Teams | Core rules | Replicated through |
| --- | --- | --- | --- |
| Paint the Void (`paint_the_void.rs`) | 2-4 | Paint coverage per team (refreshed every 30 ticks), most painted surface on time wins; kills do not score | team scores = painted cells, `surface` |
| Team Brawl (`team_brawl.rs`) | 0, 2-4 | A kill scores 1; a suicide or killer-less death costs the victim's side and a team kill the killer's side `team_kill_penalty`; target `kill_target`, or with 0 `max(15, 5 x players / sides)` capped at 50, set on join and never lowered; first to the target, else best score on time, a tie is a draw | scores, `score_target` |
| The Squeeze (`the_squeeze.rs`) | 0 | One life; 5 phases (warning, then close) of a solid shell around a shrinking safe sphere whose centres come from `RNG_STREAM_SQUEEZE`; crushing through shields while touching a moving wall; scrap debris from closing walls; last ship flying | objectives `SAFE_ZONE` / `NEXT_ZONE` carry the whole schedule; the shell is a pure function of them and the tick |
| Heist (`heist.rs`) | 0, 2 | Core in a hard-rock knot in a vein chamber (generator version 3 Heist site, `MatchSetup::heist`); freed by craters, taken by touch; carrier at 75 % speed without boost; drop returns after 20 s; the chamber collapses for 30 s once the core is free; delivery to the own extraction zone (any zone in free-for-all) scores; first to 3 | objective 0 = core (`state` embedded/free/carried/dropped/respawning), 1-2 = extraction zones; carrier flag |
| Hot Reactor (`hot_reactor.rs`) | 0 | 3 lives, lost only to a detonation in your hands; any hit by another ship or a ram passes the reactor (1 s immunity); 20 s fuse, +0.5x per other ship within 15 u; blast 60 damage over 10 u; last pilot with lives | objective `REACTOR` (holder, detonation tick in `timer_end`, fuse radius), `CARRIER` flag, lives |
| Last Color Standing (`last_color_standing.rs`) | 0 | 100 colour, every death drains 34, out at 0; from 8 min every player loses 1 colour per 2 s; last pilot with colour | `meter` per player, `MONOCHROME | METER` view flags |
| Instagib (mutator, `mutators/instagib.rs`) | any | Railspike only, loadout-only and free to fire; an enemy's direct hit takes the whole hull through shields; no secondaries, gadgets or pickups (Paint the Void keeps its Paint Thrower); instant respawn | `INSTAGIB` view flag (HUD hides hull and energy) |

Each module's docs list the decisions the plan left open (for example: a Hot Reactor holder
killed by another ship hands the reactor to the killer; The Squeeze's level radius is half
the bounds diagonal). Replay goldens per mode live next to the modules and in the per-mode
replay fixture ([determinism.md](determinism.md#per-mode-replay-hashes)).

**Adding a mode:** a `ModeKind` variant (append), a `ModeRules` variant with its settings
(`mode_settings!`), the module file registered in `modes::MODULES`, its default rule set in
`data/rulesets/<file>.ron` and `BUILTIN_RULESETS` (`ModeKind::ruleset_file`), scenario tests
per rule and a replay golden, the per-mode replay fixture re-blessed
(`LS_BLESS_MODE_REPLAYS=1`), the rules catalog re-blessed (`LS_BLESS_RULES_CATALOG=1`), a
meaning for every new setting in `tools/gen-weapon-docs.mjs` (`MODE_FIELDS`; its test fails
otherwise) and the reference regenerated, the mode HUD wording (`web/src/ui/modeHud.ts`,
`modeIcons.ts`), its visuals (`web/src/game/modeVisuals.ts`), the bots' objective
(`brain/objective.rs`), a mode name in `web/src/ui/serverList.ts` and, if it belongs in the
default rotation, `ls_server::playlist::DEFAULT_ROTATION`.

## Match lifecycle on a server

Each room of `ls-server` runs a pure lobby state machine (`lobby.rs`, in sim-thread ticks):

```
Warmup --(players fit the entry, all ready or --warmup s passed)--> Countdown (--countdown s)
   ^                                                                     |
   |                                                                     v
Results (--intermission s, next room built meanwhile) <-- mode ended -- Match
```

- **Warm-up** is a room of its own: the next match's level with every weapon, gadget, pickup
  and mutator off, playing a timer-only mode whatever the next mode is (`lobby::warmup_mode`:
  Paint the Void without paint, same team count, two teams for a free-for-all match), so kill
  targets, lives and objectives can never end or eliminate in a warm-up; the sim thread keeps
  its one-hour clock from running out. `LobbyState` announces the real next mode, rule set,
  world, player range and teams.
- **Countdown** starts once the player count fits the playlist entry and everyone pressed
  Ready, or `--warmup` seconds after players arrived; it falls back to Warmup when the count
  stops fitting. If the warm-up time passes with a count the entry does not fit, the room
  picks another entry (a new warm-up room); with nothing better the entry is played anyway.
- **Match**: a fresh room built from the same world on an `ls-build` worker thread during the
  warm-up and swapped in at zero.
- **Results**: `MatchResults` goes out once when the mode stops; meanwhile the next entry's
  warm-up room is built off the sim thread and swapped in after `--intermission`.
- `--warmup 0 --countdown 0` turns the lobby off: no warm-up rooms; Results swaps the next
  match room in directly and a match starts when its first player joins.

**Swaps without reconnects.** Every swap (warm-up to match, match to next warm-up) joins every
connected player into the new room with its stored `Hello` (updated by `PickTeam`) and sends
the new room's `Welcome` on the same connection. Each room has the next match epoch
(`RoomConfig::epoch`, wrapping); inputs and snapshots of another epoch are dropped on both
ends, so packets of the old room still in flight never reach the new one. The client rebuilds
in place (`ClientSession::transition`), and a transition costs one full snapshot like a join.
Joins never wait for a build: they join the room that is running (also during Results) and
move with everyone else. The old room is dropped on an `ls-drop` thread.

**Rotation** (`playlist.rs`): `--playlist <file>` lists entries (`rules`: built-in id, file or
`LR` code; `world`: `Random`, `RandomSliders(..)` or `Code(..)`; `min_players`,
`max_players`). `Rotation::pick(players)` takes the next entry in order whose range fits, else
the first `team_brawl` entry (2 teams from 4 players, free-for-all below), else the next
entry. Without `--playlist`: `--ruleset` is a one-entry playlist on one world; neither plays
`DEFAULT_ROTATION` (Team Brawl from 4 players, then The Squeeze, Paint the Void, Hot Reactor,
Heist, Last Color Standing on random worlds, or on `--code`'s world). A team Heist planned for
1-3 players plays free-for-all (`sim::heist_variant`). Hosting view of all this:
[hosting-guide.md](../../userDocs/hosting-guide.md#lobby-match-end-and-rotation).

## Rooms

One public room (the rotation) plus up to `--private-rooms N` (default 4, at most one per CPU
core beyond the public room's) private rooms, each a `Sim` on its own thread with its own
tick budget and `tick stats`. Private rooms have codes `R-XXXXXX` (6 Crockford base32
characters, parsed leniently), are reached with `Hello::room`, put their code into
`Welcome::room_code`, and close when their last player left (or 60 s after creation if nobody
came). A private room plays a one-entry playlist (`PrivateRoomSpec`): a custom match's rule
set on its world (a new random world per match without one) or a duel; it runs the server's
lobby timing (warm-up, countdown, results) and takes at most the server's player limit
(duels: 2). A custom match without a rule set code plays the public room's current
rules. `Hub::open` routes each connection: party code first, then room code, else the public
room; unknown codes get a `Reject` ("unknown room", "unknown party ...").

## Identity, parties, invite links, duels

Lobby features beyond the game connection talk to a small JSON API on the game port
(`/api/info|players|presence|party|rooms|duel|duel/answer`, CORS open; request and reply
table in [server.md](server.md#server-list-invites-parties-and-duels-apirs-partyrs-duelrs-directoryrs-m3-b7)),
not to protocol messages.

- **Identity**: no accounts. The browser keeps a random public id (`p-` + 16 hex) and a
  private token (32 hex) in `localStorage` (`ls.identity`; per tab when storage is blocked).
  `POST /api/presence` (every 3 s in game) claims the id with its token, refreshes the name
  and returns the player's inbox; players seen within 15 s form the server's roster.
- **Parties**: `POST /api/party` makes a code `P-XXXXXX` for the leader's room (public room,
  the leader's private room, or a new private custom match with a world and rule set code);
  it lives on that server only, while members are connected and 10 minutes after the last left
  (or with its private room). A `Hello` with the party code joins the party's room and, in
  team modes, asks for a party mate's team; `Room::join` still grants only a team within one
  of the smallest, so a party stays together only where balance allows, else it is split by
  join order. At every swap the sim thread re-plans the joins (`party::plan_joins`: whole
  parties on one team where possible).
- **Invite links**: `https://<page>/?join=<host:port>&code=<LS..>&rules=<LR..>&party=<P-..>`
  (every part optional; `world=` is accepted for `code=`, `room=<R-..>` joins a private room,
  `directory=<url>` names a directory, `servers` opens the server list). With a party or room
  code the page joins that room on that server; with a world or rule set code but neither, it
  first opens a private custom match there (`POST /api/rooms`) and joins it; with nothing, the
  public room. Moving to another server or room is a page navigation to an invite link (a new
  connection), not a transfer of the connection.
- **Duels**: `POST /api/duel` on the challenger's server records a challenge (30 s) with the
  challenger's world and optional rule set code. A challenged player present on that server
  sees it in the presence inbox; otherwise the server relays it through the directory
  (`POST /invites`, kind `duel`), and the challenged browser, which polls the directory by its
  public id, answers on the challenger's server (`POST /api/duel/answer`). Accepting opens a
  private two-player room on the challenger's server (`duel::duel_spec`: the challenger's
  world, the challenger's rule set code or the built-in `duel` rule set: Team Brawl
  free-for-all, first to 10 kills or 5 minutes); both browsers follow the room's invite link
  (the challenger through the inbox note `accepted`).
- **Directory**: `ls-server --directory <url>` sends heartbeats with the public room's mode,
  world and rule set code, players over all rooms, the WebTransport certificate hash and the
  roster (public ids only); details and the API in [directory.md](directory.md). The
  directory speaks plain HTTP: a browser on an `https://` page needs it (and every listed
  game server) behind a TLS reverse proxy
  ([hosting guide](../../userDocs/hosting-guide.md#https-and-a-public-directory)).

## Browser

- **Match flow** (`game/matchFlow.ts`): from the lobby state, the mode state and the results it
  decides the stage (warm-up, countdown, match, results), which overlay shows and whether the
  own player spectates (out, or no ship in a running match). The countdown runs locally between
  `LobbyState` updates. A new `Welcome` swaps the level, visual table, camera and effects in
  place (`Game.takeMatchTransition()`, also reported from hidden-tab frames).
- **Overlays**: `ui/lobby.ts` (next mode with a one-line summary, world, rules, room code,
  players with ready tick and team pick; Enter ready, T team, O spectate, also as buttons),
  `ui/results.ts` (Victory/Defeat/Draw, team bars or a free-for-all podium, the table, "Next:
  <mode> in N s"), `ui/modeHud.ts` + `ui/modeIcons.ts` (mode title, timer, scores or the top
  three, per-mode target wording, The Squeeze's stage line, lives, colour meter, alerts and
  edge-clamped objective markers with icons; Instagib hides hull and energy), the scoreboard
  for 2-4 teams or a ranked free-for-all list.
- **Spectator** (`game/spectator.ts`): free flight (Shift faster, through rock), left click
  follows the next ship, right click returns to free flight.
- **Mode visuals** (`game/modeVisuals.ts`, `renderer/modeFx.ts`): The Squeeze's walls and next
  zone, Heist core, carrier and extraction zones, the reactor and its fuse ring, the Last Color
  Standing grade with per-ship colour ([renderer.md](renderer.md)).
- **Server list and party panel** (`ui/serverList.ts`, `ui/party.ts`, `net/ping.ts`,
  `net/social.ts`): `?servers` lists the directory's servers of this build (filtered by the
  page server's protocol and data hash from `/api/info`), pings each over a WebSocket open
  (6 at a time, 3 s timeout), and offers Join and Duel; the in-game "Party & duels" panel
  creates party invites, lists the server's players with a Duel button and shows challenge
  toasts (Accept/Decline, 30 s).

## Tests and evidence

| What | Test |
| --- | --- |
| Rules of each mode | `crates/ls-sim/src/modes/*/tests.rs`, `modes/tests.rs`, `mutators/instagib.rs` (one scenario per rule) |
| Determinism per mode | module replay goldens and `ls_sim::golden` (fixture `crates/ls-sim/tests/fixtures/mode_replays.txt`, native, wasm32 and `tools/determinism-check.mjs`) |
| Transitions on one connection | `crates/ls-game/tests/transition.rs`, `crates/ls-bots/tests/match_transition.rs`, `crates/ls-server/tests/rooms.rs` |
| Rooms, joins during a build, `--ruleset` alone | `crates/ls-server/tests/rooms.rs`, `sim.rs` `joins_during_the_next_match_build_are_welcomed_at_once` |
| Lobby, rotation, warm-up rules | `ls-server` `lobby::tests`, `playlist::tests` |
| Parties, duels, directory, server list | `crates/ls-server/tests/social.rs`, `party.rs`, `duel.rs`, `directory.rs` unit tests, `crates/ls-directory/tests/http.rs`, `web/src/ui/party.test.ts`, `web/src/net/ping.test.ts` |
| Bots play every mode | `crates/ls-bots/tests/mode_matches.rs` (8 bots, full match to a result, release), `social.rs` |
| End to end | `node tools/e2e.mjs --playlist quick|default [--directory local]`; browser smoke phases `lobby` and `social` ([dev-guide.md](../dev-guide.md#12-browser-smoke-test)) |

## Known gaps

- **Spectate by choice** (O in the lobby): the server stores the choice and announces it in
  `LobbyState`, but the next room still gives the player a ship (`ls-server` never calls
  `World::set_spectator`), so the browser only spectates once the player is out. Elimination
  and late-join spectating work.
- **Server bans in private rooms**: custom matches and duels resolve their rule set code
  without the server's `--ban` and `--balance-list` (`api::custom_rules`, `duel::duel_rules`
  build `ServerRules` with no bans), so an invite's code can bring back a weapon the host
  banned. The public rotation applies them to every entry.
- **Third and fourth teams** have no spawn zone of their own (see Teams and sides).
- **Directory over plain HTTP** only, no forwarded-for handling behind a proxy (rate limits
  apply to the proxy's address); no authentication beyond the per-server secret (M5 hardening).
- **Parties and identity** live on one server; there are no accounts or friends lists (M4+).
