# Moods and world (M4)

Milestone 4 made the level a participant: a comedy director rolls 18 events, the world sliders
all shape the level, gravity wells pull, tunnels cave in, monochrome levels hand out pigment, one
player can reshape the level live (The Architect), and a second level archetype, the asteroid
belt, hosts a racing mode (Belt Up). This page is the map of how those pieces fit together as
built; the exact rules and numbers live next to the code and in these pages:

- [contracts.md](contracts.md): share code layout, `ls_math::level` (boxes, `WellSite`,
  `BeltSite`), the world physics contracts (gravity law, Instability, pigment, radiation,
  boulders, the Belt Up race), RNG streams, every id table (`EventKind`, `Mood`, cues, entity
  kinds, stroke kinds, player flags);
- [netcode.md](netcode.md#protocol-6-moods-the-architect-belt-up): protocol 6 (mood view,
  race section, ship `ext`, well entities, `ArchitectAction`);
- [determinism.md](determinism.md): the replay goldens of the world physics, every event, The
  Architect and Belt Up, and the Milestone 4 record;
- [modes-and-lobby.md](modes-and-lobby.md): the mode hooks M4 added (`allows_event`, `leader`,
  `ship_contact`, `pickup_group`) and The Architect and Belt Up as modes;
- [game-glue.md](game-glue.md): `MatchSetup` from the generator, the predictor's moods, wells
  and obstacles, lag compensation with `World::preview_movement`;
- [renderer.md](renderer.md) (ink look, world effects, the event visuals, Belt Up, The
  Architect, forcing events), [audio.md](audio.md) (sounds and the announcer),
  [server.md](server.md#bots-in-moods-m4-package-c5) (bots in moods, racing bots);
- [milestone-4-plan.md](../milestone-4-plan.md): the design decisions (§3) and every
  package's "as built" notes with measurements.

Player-facing descriptions: [player manual](../../userDocs/player-manual.md#the-level-fights-back),
[comedy event reference](../../userDocs/reference/events.md) (generated from `data/events/`),
[world physics reference](../../userDocs/reference/world.md) (generated from `data/world/`).

## Overview

| Piece | Where | Replicated as | Predicted by the client |
| --- | --- | --- | --- |
| Level shape, wells, belt | `ls-worldgen` (generator version 4: `generator.rs`, `network.rs`, `wells.rs`, `belt.rs`) | the share code (every peer generates) | yes: `MatchSetup` from the share code |
| Comedy director and events | `ls-sim` `director/`, `events/<id>.rs`, `data/events/` | `ModeView::mood` (`MoodView`), event props as entities, cues | movement effects, obstacles, the up (`MoodView::advanced`) |
| Gravity | `ls-sim` `gravity.rs`, `data/world/gravity.ron` | static wells: the share code; dynamic wells: well entities | yes (`Predictor::set_wells`) |
| Instability | `ls-sim` `instability.rs`, `data/world/instability.ron` | terrain edits, `CaveInWarning` cue | no (server-side edits) |
| Pigment | `ls-sim` `pigment.rs`, `data/world/pigment.ron` | pigment pickup entities, ship `ext` bit, `Pigment` cue | no |
| Radiation | `ls-sim` `radiation.rs`, `data/world/radiation.ron` | hull in snapshots; the dose rate is a pure function of the belt | the HUD's dose (`doseRateAt`) |
| Boulders and ducks | `ls-sim` `obstacle.rs`, `data/world/boulders.ron` | nothing (pure functions of tick, seed and setup) | yes (pushes) |
| The Architect | `ls-sim` `modes/the_architect.rs`, `world/architect.rs`; `ls-game` reshape sampling | mode view (objectives, flags), `StrokeTelegraph` cue, edits | no (the architect has no ship) |
| Belt Up | `ls-sim` `modes/belt_up.rs` | race section (`RaceView`), `HELD` flag | holds (`move_limits`), boulders |
| Paperweight | `ls-sim` `weapons/specials/paperweight.rs`, `data/weapons/paperweight.ron` | its projectile plus a well entity | its well (dynamic wells) |
| World workshop | `web/src/ui/workshop.ts` | the share code it makes | n/a |
| Audio and announcer | `web/src/audio/`, `web/src/ui/announcer.ts` | cues, mood view (client only) | n/a |

Everything in `ls-sim` runs inside `World::step` and is part of the state hash once it does
anything; each system is inert when its input is off (Absurdity 0, no wells, Instability 0, no
monochrome level, no belt, world +Y up), and the hash then skips its state, so the pre-M4
replays kept their hashes through wave A ([determinism.md](determinism.md#milestone-4-record)).

## A tick with M4

`World::step` (`crates/ls-sim/src/world.rs`, module docs) gained two stages and a few calls
inside the existing ones:

1. respawns;
2. ships in id order: the running event's movement effects (before and after the move), the
   clamped well pull in the movement environment (`ship::MoveEnv`: wells, the level's up),
   kinematic obstacles (ducks, belt boulders) pushing the ship after its move; then boulder
   contact damage and the mode's `ship_contact` (Belt Up's bumps);
3. projectiles: the well pull for `gravity: true` weapons, the event's projectile flags
   (`CHICKEN`, `PLANAR`) at spawn, obstacles absorbing shots, the event's hit radius scale and
   hit plane;
4. entities: debris pulled by wells, event props posed by their event;
5. scrap piles and pickups (`allows_pickup`: Moving day, mimics);
6. ship trails;
7. **world physics**: radiation on belt levels, pigment pickups, ended dynamic wells,
   Instability's strain decay and cave-ins, The Architect's strokes and rounds;
8. mode (`on_tick`, `end_check`);
9. **comedy director**: mood drift, the running event's lifecycle hooks, rolls.

Crater sizes pass through `Ctx::crater`: rock strength, then Instability's growth, then the
event's crater scale (Everything is cake).

## Comedy director

`crates/ls-sim/src/director/mod.rs` (module docs), numbers in `data/events/director.ron`
(`DirectorDef`).

- **Effective Absurdity** = `min(MatchSetup::absurdity, RuleSet::absurdity)`: the world's
  slider is its temperament, the rule set's value the host's cap. At 0 the director is off
  and its pristine state is not hashed. Warm-up rooms run at 0. Built-in rule sets cap at 100
  (duel 50); rule set codes of format 2-4 decode with the old cap of 20.
- **Cadence**: no announcement in the first 45 s or the last 20 s of a match (and never in
  warm-up, countdown or results); a roll every 20 s with chance `0.6 x absurdity / 100`; one
  event at a time; at least 30 s from an event's end to the next announcement (x1.5 when the
  mood is calm). A successful roll during an event or a gap waits (one at most) and is announced
  when the gap has passed; an event does not repeat in a match while others are eligible.
  Measured over 200 simulated 5-minute matches: 0 / 1.20 / 3.06 / 4.50 events at Absurdity
  0 / 20 / 50 / 100 (`director/tests.rs`, `event_statistics`).
- **Choice**: among events that are `enabled` in their data file, not banned by the rule set
  (`RuleSet::events`, rule set code format 5), allowed by the mode and mutators
  (`ModeModule::allows_event`, `MutatorModule::allows_event`) and `eligible` (their needs:
  wells, a damaged player, a leader, 4 pickups, a boundary face near ships); weight
  `EventDef::weight`, x2 (`mood_bias`) when the event lists the current mood.
- **Moods**: calm, bored, grumpy, giddy, drifting every 60-90 s. Weights and presentation only
  (grade tint, motes, ambient drone, the HUD badge; `web/src/game/moodFx.ts`).
- **Phases**: announce (3 s: card, announcer line, the event's tell), active (`active_secs`),
  ending (`ending_secs`, a wind-down: drones fly off, the face sinks), over. Damage from events
  never credits a killer.
- **Determinism**: every draw comes from `director::mood_rng(match_seed, tick, key)` on
  `RNG_STREAM_MOOD` ("SIM_MOOD"), a fresh generator per draw (contracts.md, RNG streams).
- **Forced events**: `World::schedule_event(tick, kind)` starts an event at its tick (or after
  the running one) whatever the Absurdity, bans, vetoes, `enabled` and eligibility. Used by
  practice `?event=<id>[@secs]` / `?event=all` (`web/src/game/eventParam.ts`, ls-wasm
  `Game.practiceEvent`), the server's testing flag `--event-schedule` (`ls-server`
  `event_schedule.rs`, logs a warning; [hosting guide](../../userDocs/hosting-guide.md#command-line-reference)),
  The Architect's Absurdity+ and Verticality strokes, and the replay harnesses.

Vetoes as built: The Architect runs no director (its strokes force events); The Squeeze vetoes
Cat paw and Duck parade from phase 4; Belt Up vetoes Bored level, Wrong genre and Moving day and
holds events during start lights, a heat's first 15 s and heat results; the Instagib mutator
vetoes Tax drone and Pizza delivery. Leader events (Tax drone, Customer survey) target
`ModeModule::leader` (default: most mode score; Belt Up: the best racer still racing).

### Events

Each event is a module `crates/ls-sim/src/events/<id>.rs` implementing `EventModule` and a data
file `data/events/<id>.ron` (`EventDef`: `name`, `enabled`, `weight`, `active_secs`,
`ending_secs`, `moods`, `targeted`, `params`), registered in `events::MODULES` in `EventKind`
order (append only: the index is the wire id and the rule set code's ban bit).

| Hook | Kind | Used by |
| --- | --- | --- |
| `eligible` | lifecycle (`EventRef`) | Coffee break (wells), Pizza (a damaged player), Tax drone and Survey (a leader), Moving day and Mimics (4 pickups), Cat paw (ships near a boundary face) |
| `on_announce`, `on_start`, `on_tick`, `on_end` | lifecycle (`EventCtx`: the hook `Ctx`, `EventState`, `EventCtx::rng`, kills since the last step) | target and parameter picks, props, damage, edits, awards |
| `move_effects` | effect (pure) | Level hiccups (impulses), Wrong genre (plane lock, `Facing`) |
| `level_up` | effect | Bored level (turns `World::up`, which stays turned) |
| `well_scale` | effect | Gravity coffee break (0) |
| `hit_radius_scale`, `hit_plane` | effect | Big head day (3x), Wrong genre (depth-free hits) |
| `projectile_flags` | effect | Rubber chickens (`CHICKEN`), Wrong genre (`PLANAR`) |
| `crater_scale`, `scavenge_scale` | effect | Everything is cake (1.44, 2) |
| `reveal` | effect | Gossip wall (everyone), Hat rain (3 or more hats) |
| `allows_pickup` | effect | Moving day (during the move), Mimic pickups (mimicked pickups) |
| `obstacles` | effect | Duck parade (`obstacle.rs`) |

Effect hooks run in the active and ending phases and are pure functions of the replicated
`MoodView`, the match seed and the tick, so the client derives them itself
(`director::event_effects`, `director::move_effects`, `MoodView::advanced(now)` for ticks past
the snapshot). Values an event picks at its announce (an axis, an origin, mimicked pickups, a
route) go into the view's 8 parameter bytes (`events::params`). Lifecycle hooks change the world
only through commands (`Ctx`) and keep their state in the hashed `EventState`.

The 18 events by family (rules: [milestone-4-plan.md §3.2](../milestone-4-plan.md#32-the-18-events)
and the packages' as-built notes; numbers: the generated [event reference](../../userDocs/reference/events.md)):

- **The level's own jokes** (B1): Bored level, Level hiccups, Gravity coffee break, Wrong genre,
  Big head day, Rubber chickens, Moving day, Everything is cake. Movement effects are predicted;
  lag compensation keeps each history tick's hit scale and plane.
- **Visitors** (B2): Tax drone, Pizza delivery, Customer survey (drones on the shared
  `events::drone` flight code: a `NavGraph` of the level's hubs and tunnel centre lines from
  `ls_game::setup::nav_graph`, Dijkstra, string pulling, local avoidance; straight flight on
  belts), Mimic pickups, Gossip wall. The survey rating is an award in the results.
- **Spectacles** (B3): Duck parade (kinematic obstacles along `MatchSetup::routes`, the ring
  retrograde on belts), Hat rain (hats on ships in the `ext` bits; the Hat Baron award), Cat paw
  (sweep, knockback, three claw grooves as `SubtractSphere` chains spread over ticks: the claws
  rake the first rock toward the boundary face, each groove sphere `groove_inset` inside it,
  `claw_gap` 7 u apart so ridges stay between them), Disco inferno
  (presentation only), Tired announcer (`KillCam` cues scheduled back to back, the same on
  every client).

Event props (drones, mimics, the gossip face, ducks, hats, the paw, the mirror ball) are
`EntityKind::Prop(ReplicatedKind)` entities that their event spawns, poses every tick
(`Command::PoseEntity`) and removes in `on_end`; drones and mimics block shots while alive.
Awards (`events::Award`: Hat Baron, Customer rating) reach `MatchResults::awards` and the wire
(`MatchResultsMsg::awards`).

On the client, `web/src/game/events/index.ts` (C1's hub) draws the telegraph of every event (the
card and pill, `web/src/ui/eventBanner.ts`, and the announcer's `event.<id>` line) and the
looks of B1's events; `visitors.ts` (C2) and `spectacles.ts` / `killcam.ts` (C3) run beside it
from `main.ts`. `Game.eventFx(tick)` (`crates/ls-wasm/src/events.rs`) exposes the effects the
client needs (well scale, hit scale, chicken flag, the up and Bored level's axis, hiccup ticks,
the cake origin).

### Adding an event

1. Append an `EventKind` and a `MODULES` entry (never reorder), write
   `events/<id>.rs` (`implemented() == true`) and `data/events/<id>.ron`. The data file is
   embedded and hashed; the rule set code covers events by count, so codes made before the event
   decode with it allowed.
2. Movement, hit and pickup effects only through effect hooks, pure in the mood view; anything
   chosen at random goes into the parameter bytes at the announce.
3. Tests: scenario tests per rule, a replay golden (`events::replays` picks up every built event
   without its own harness: an `event/<id>` line in
   `crates/ls-sim/tests/fixtures/mood_replays.txt`, `LS_BLESS_MOOD_REPLAYS=1`), native and
   wasm32, and an in-process netplay test in `ls-game` when it moves ships.
4. Client: a module in `web/src/game/events/` (its look, sounds in `web/src/audio/sounds/`,
   announcer lines in every pack of `web/src/audio/announcer/`), a gallery strip
   `event-<id>-{a,b,c}`, bot reactions in `crates/ls-bots/src/brain/moods.rs`.
5. `node tools/gen-weapon-docs.mjs` regenerates the event reference;
   `LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode` the rule catalog (event ids
   and the editor's ban boxes).

## Levels: generator version 4

`ls_worldgen::GENERATOR_VERSION` is 4. A version 4 share code carries a 3-bit level archetype
(`ls_math::level::Archetype`: 0 classic, 1 asteroid belt, append only) and the seven sliders
packed as one base-101 number, so codes stay 26 symbols (`LS0G...`); codes of versions 1-3 are
refused with a message naming both versions (contracts.md, "Share codes").

### Classic levels

The cave, mine and open-sky generator of M1-M3 with all seven sliders active:

| Slider | Effect (as built) |
| --- | --- |
| Enclosure, Density | as before: open sky and islands to caverns to mine shafts; pillars, boulders and floating rock |
| Verticality | the level box (`BOX_SHAPES`, about the same chunk count): 0-19 320 x 64 x 320 u, 20-39 288 x 96 x 288, 40-59 256 x 128 x 256, 60-79 224 x 160 x 224, 80-100 192 x 224 x 192; chambers stretched 0.5x to 2.2x vertically (and the inverse square root across), mine vertical tunnel chance 0.05-0.6, island and rock tiers from 60 (one 64 u layer below 20), an upper hub layer from 60 (8 hubs, 11 tunnels) |
| Gravity Chaos | `1 + slider / 15` well sites (at most 7; none at 0) from the "GRAVWELL" stream: a rock core (1.5-2.5 u) in a carved hollow (8 u) joined to the terrain with a smooth maximum, in caverns, hubs or open space that reaches the network; at least 30 u apart, 40 u plus a spawn cavern's radius from every spawn cavern, outside the Heist vein (`Generator::gravity_wells`). Measured 6.6-7.0 wells at Gravity Chaos 100 |
| Palette | render only (the ink look, renderer.md); 80 and above makes a monochrome level (`MatchSetup::monochrome`: pigment) |
| Absurdity | the director's input (above) |
| Instability | craters grow and tunnels cave in (below); the Tectonic Charge's crack reach (M3) |

Every classic level also has the team 2/3 caverns at the z ends (linked to the hubs; three- and
four-team matches spawn there, `ls_game::setup::spawn_zone`) and the Heist site of version 3.
Readers take the real bounds from `Generator::bounds` or `Welcome` (renderer extents, nav grid,
The Squeeze's radius, `ls-wasm`'s view radius 240 u).

### Asteroid belts

`crates/ls-worldgen/src/belt.rs` (module docs): a fixed box of 320 x 96 x 320 u, a ring of
voxel asteroids (inner radius 95 u, outer 145 u, half-thickness 8-18 u by Verticality) around
an open centre with a 30 u dust cocoon and the 6 u exit. One 45-degree wedge is generated from
the "BELT_GEN" stream and copied by rotation (8-fold) and by mirroring in the belt plane, so
every start slot flies the same course: the field is symmetric within 3.5e-5 u (limit 1e-4,
`field_has_eightfold_rotation_and_plane_mirror_symmetry`), and the belt-only lap lengths of
the 8 slots differ by 0.04-0.55 % (8 worlds). Each wedge holds a meandering clear corridor (at
least 4 u wide), a start slot with a clear dive lane to the exit, 3 pickup points, its gate,
3-10 boulder orbits in lanes (Density) and, from Gravity Chaos 20, a shepherd moon (a
`WellSite` of kind `Shepherd`). Enclosure sizes the asteroids (1.5-3 u at 0 to 4-10 u at 100,
hollow with a bore from 60), Density counts them (15-60 per wedge, twice as dense in the inner
third, densest at the plane). `Generator::belt()` returns the `BeltSite` (slots, gates, exit,
pickup points, boulder orbits; contracts.md "Levels"), which `ls_game::setup` puts into
`MatchSetup::belt` (spawns = slots). Belts generate 36-49 % faster than a default classic level.

Belt worlds are made in the workshop, shared and invited like any world. A Belt Up playlist
entry or rule set paired with a classic code plays it as a belt on the same seed and sliders
(`ls-server` `playlist::belt_code`). Other modes on belt worlds are out of scope (M5 candidate).

## World physics

Contracts and numbers: [contracts.md "World physics"](contracts.md#world-physics-ls_sim-m4).

- **Gravity** (`gravity.rs`): softened inverse-square pull, tapered to 0 at the well's range;
  static wells from the level (peak 12 u/s^2 x strength, range 20-40 u, off at Gravity Chaos 0)
  plus dynamic wells (`DynamicWell`: Paperweight, the Architect's Gravity Chaos+ stroke) with a
  start and end tick and an owner. The total on a ship is clamped to 40 % of its per-axis
  thrust, so stacked wells never trap it; an idle ship's dampers aim at a drift speed, so it
  visibly falls. Ships, debris, unstuck mines and `gravity: true` ordnance are pulled; energy
  bolts and kinetic rounds fly straight. The predictor applies the same field.
- **The level's up** (`World::up`): +Y unless Bored level turned it; horizon assist levels to it,
  Level hiccups and Hat rain follow it, cave-in rubble hangs from the first rock along it.
- **Instability** (`instability.rs`): craters grow by `1 + 0.4 x instability / 100`; every
  crater adds strain to its 16 u cell (decaying 3 % per second); a cell over the threshold caves
  in after a 1.5 s `CaveInWarning`: a hollow in the ceiling and 5-9 weak-rock rubble spheres,
  debris with scrap, 20 damage within 3 u; one per 8 s, never near spawns, the Heist chamber or an
  extraction zone, never sealing a ship in (a flood-fill escape check). Rubble regions are weak
  rock for the simulation's material queries.
- **Pigment** (`pigment.rs`): on monochrome levels (Palette 80+) and in Last Color Standing,
  pigment pickups from the rule set's `pigment_pickup` (one per 4 players, at least 2, 30 s
  respawn; Last Color Standing 45 s). Taking one pigments the ship for 40 s (+25 % damage, drawn
  in colour and marked through walls for everyone; a 15 s splash pickup on death); in Last Color
  Standing it restores 17 colour instead. Grey ships keep their name tag hidden from enemies
  beyond 20 u (client only; relevance is unchanged).
- **Radiation** (`radiation.rs`): on belt levels only; hull loss past shields and gadgets by the
  distance to the belt or the cocoon, with a knee (C5's tuning: 0.8 hp/s at the surface, 1.6 at
  4 u out, 2.8 at 6 u, cap 7 at 13 u), times the mode's `radiation_scale`; no kill credit.
- **Obstacles** (`obstacle.rs`): kinematic spheres that push ships and absorb shots, from pure
  functions of tick, seed and setup: Duck parade's ducks and the belt's drifting boulders (which
  also hurt on contacts faster than 8 u/s). Server and predictor run the same push.

## The Architect

`ModeKind` 6: one architect without a ship against 1-7 runners, `min(players, 3)` rounds of
4 minutes, the role rotating by join order (modes-and-lobby.md has the rules as built).

- **Strokes**: `ArchitectAction` (protocol 6) carries the stroke kind, centre and radius;
  `Room::step` hands queued actions to `World::architect_action`, which validates role, round,
  radius (12-24 u), range, cooldown (2 s), strokes in flight (4), budget (100 + 4/s, max 250),
  Verticality once per round and refuses strokes that would change nothing (such a dud still
  holds the next stroke back `STROKE_DUD_TICKS` = 6 ticks, so a flood of duds cannot make the
  server sample the generator every tick). The room queues only the current architect's
  actions (bounded); everyone else's are refused at once. A stroke is
  telegraphed for 2 s (`CueKind::StrokeTelegraph`, an objective of kind `STROKE`), then acts.
- **Reshaping**: rock strokes sample the generator with the slider moved
  (`SimTerrain::reshape_sample`, answered by `ls_game::GameTerrain` from a cached moved
  generator) on a 4 u lattice and emit at most 32 `AddSphere` / `SubtractSphere` edits over 8
  ticks; an `AddSphere` over a ship is left out and the "never enclose a ship" escape check of
  the cave-ins applies. No new `TerrainEdit` kind. Gravity Chaos+ adds a dynamic well for 30 s,
  Instability+ a cave-in, Absurdity+ and Verticality force events, Palette is a 20 s ink
  marker.
- **Client** (`web/src/game/architect/`, `web/src/ui/architect.ts`): the god camera's map view
  with a cutaway and free flight, stroke picking (keys, wheel, the workshop's `SliderPanel`),
  client-side checks mirroring the server's, telegraph visuals for runners
  ([renderer.md](renderer.md#the-architect-m4-c4)).
- **Bots**: a bot architect and runner bots (`crates/ls-bots/src/brain/architect.rs`).

## Belt Up

`ModeKind` 7 on belt levels (rules as built in modes-and-lobby.md and contracts.md "Belt Up
race"): 2-8 racers, three heats per match, one lap through the 8 gates in order and a dive to
the exit; start lights and death holds through the `HELD` player flag (the mode's
`move_limits` stop the ship, so the predictor holds too); radiation scaled by the rule set's
`radiation_scale` while racing; bumps through the mode hook `ship_contact` (ship `mass` in
`data/ships`); identical wedge pickups through `pickup_group`; the next heat through
`Command::ResetRound`, which also ends a running comedy event in that tick (`World::end_event`).
A racer who spectates mid-heat leaves the heat without a place and may race again only in
start lights with a free seat (mode hook `may_stop_spectating`); a running heat that every
racer left starts over with the waiting players. The race state is replicated in the race section (`RaceView`). The
client (`web/src/game/race/`, `web/src/ui/raceHud.ts`, `web/src/renderer/belt.ts`) mirrors the
sim's progress function (`course.ts`, checked against `crates/ls-sim/tests/fixtures/race_progress.txt`)
for gaps and distances. Racing bots plan lines on a station lattice over the course with a
hull-aware radiation weight (`crates/ls-bots/src/race_line.rs`); their time trials gate the
radiation tuning.

## The Paperweight

A special secondary appended to the weapon registry (registry index 27, `WeaponId` 11 by sorted
id): a lobbed stone that stops on rock or after 1.2 s and is a dynamic gravity well for 6 s
(range 16 u, peak 30 % of the Tinker's thrust, pulling its thrower too), then fuses the debris it
gathered into a lump of weak rock (`Command::WeakRock`). It is a pickup in Belt Up and the
sandbox rule set only. Its well travels as a well entity (kind 18); the stone is its projectile
(arsenal-framework.md, "The arsenal as built").

## World workshop

`web/src/ui/workshop.ts` (`?workshop[=<code>]`, "Make a world" on the start screen): the seven
sliders with live meanings (belt meanings on a belt), the seed with a dice button, the Classic /
Asteroid belt switch, Save (copy the code), Fly it (practice), Host it (an invite link with
`code=`, the rule set editor link and an `ls-server` command; on a belt it proposes Belt Up's
rules). Codes are made and checked by ls-wasm (`makeShareCode`, `checkShareCode`); the live
preview is the start screen's flythrough (`web/src/attract/intro.ts`) rebuilt 300 ms after the
last change. The last world is kept in `localStorage` (`ls.workshop`). `SliderPanel` is reused by
the Architect's HUD. Smoke phase `workshop` (`tools/browser-smoke/workshop.mjs`) drives it.

## Tests and evidence

| What | Where |
| --- | --- |
| Director cadence, statistics, vetoes, bans | `crates/ls-sim/src/director/tests.rs` |
| Every event's rules | `events/world_events_tests.rs` (B1), `visitor_tests.rs` (B2), `spectacle_tests.rs` (B3) |
| Replay goldens (world physics, 18 events, The Architect, Belt Up) | `crates/ls-sim/tests/fixtures/mood_replays.txt`, native, wasm32 and `tools/determinism-check.mjs` ([determinism.md](determinism.md#world-physics-replay-hashes-m4)) |
| Prediction through moods, wells and obstacles | `crates/ls-game/tests/{mood_netplay,world_events_netplay,visitor_netplay,spectacle_netplay}.rs`, Belt Up netplay in `crates/ls-game/tests/belt_up.rs` |
| Generator version 4 | `crates/ls-worldgen/tests/golden.rs` (13 worlds incl. every shape, seven wells and two belts), the setup sweep `cargo test -p ls-game --release --lib setup` |
| Bots | `crates/ls-bots/tests/mood_matches.rs` (Absurdity 100 rotation with every event forced, The Architect), `race_match.rs` (Belt Up, time trials) |
| End to end | `tools/e2e.mjs --event-schedule` (quick playlist: an Absurdity 100 entry, The Architect, Belt Up); browser smoke phases `moods`, `architect`, `race`, `workshop`, `audio` |
