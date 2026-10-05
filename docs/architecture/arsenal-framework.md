# Arsenal framework (M2)

The framework that lets every weapon, gadget and ship type of the design doc live in its own
file, so they can be filled in parallel without touching shared code. It is part of
`ls-sim` (deterministic: follow [determinism.md](determinism.md)), plus the generic replication
in `ls-net` and its glue in `ls-game`. The M2 scope and design decisions are in
[milestone-2-plan.md](../milestone-2-plan.md); step-by-step checklists for adding a weapon,
gadget or rule set (with a worked example and the tests that need updating) are in
[dev-guide.md](../dev-guide.md#8-adding-a-weapon).

| Piece | Where |
| --- | --- |
| Weapon numbers | `data/weapons/<id>.ron` (`ls_sim::WeaponDef`) |
| Weapon behaviour | `crates/ls-sim/src/weapons/{energy,kinetic,ordnance,specials}/<id>.rs` (`WeaponBehavior`) |
| Weapon registry (the one place that lists them) | `crates/ls-sim/src/weapons/registry.rs` (`MODULES`) |
| Hook context and commands | `crates/ls-sim/src/behavior.rs` (`Ctx`, `Command`, `Cue`) |
| Gadgets | `data/gadgets/<id>.ron` (`GadgetDef`), `crates/ls-sim/src/gadgets/<id>.rs` (`GadgetBehavior`, registry `gadgets::MODULES`) |
| Ship types | `data/ships/{skitter,tinker,hauler,tug}.ron` (`ShipTuning`, `ShipTable`) |
| Entities | `crates/ls-sim/src/entity.rs` (`Entity`, `EntitySpawn`, `EntityView`) |
| Visuals | `crates/ls-sim/src/visual.rs` (`Visual`, `VisualTable`) |
| Rule set v2, codes | `crates/ls-sim/src/ruleset.rs`, `crates/ls-sim/src/rulecode.rs` |
| Data hash | `crates/ls-sim/src/data.rs` (`data_hash`, `GameData`) |
| Framework (hook calls, command application) | `crates/ls-sim/src/world.rs` |
| Replication | `ls_net::{NetEntity, NetPlayer, NetEvent::Cue, Relevance}`, `ls_game::convert`, `ls_game::server::Room` |

## Implementing a weapon module

Every weapon already has a module file, a data file and a registry entry. A module starts as
the **generic fallback**: all trait defaults, which together are the M1 weapon (straight
projectile, or the data's `projectiles`/`spread_deg` pattern, direct damage, splash,
knockback, the data's crater and paint, charge from the data's `charge`). Since wave B every
one of the 27 weapon modules and the 3 gadget modules implements its design-doc behaviour and
reports `implemented() == true`; `weapons::tests::every_weapon_has_a_module_and_a_data_file`
fails if a module falls back again. What each one does is in
[The arsenal as built](#the-arsenal-as-built) below.

To implement one (say `crates/ls-sim/src/weapons/kinetic/railspike.rs`):

1. Put every number in the data file. Common fields are typed (`speed`, `damage`, `splash`,
   `terrain`, `shield`, `charge`, `projectiles`, `spread_deg`, `visual`, ...); anything else goes
   into `params: { "pierce_thickness": 2.0 }` and is read with `def.param("pierce_thickness",
   2.0)`. No shared struct has to change.
2. Override only the hooks you need (see the table below). Hooks read the world through `ctx`
   and change it by queuing commands (`ctx.damage`, `ctx.crater`, `ctx.spawn_projectile`, ...);
   the world applies them right after the hook returns.
3. Return `true` from `implemented()` and add scenario tests for the bonus, the malus and the
   lasting effect (next to the module or in `crates/ls-sim/src/tests_arsenal.rs`-style files).
   `weapons::tests::every_weapon_fires_with_its_fallback` checks that every weapon fires and
   ends cleanly.
4. Behaviour or data changes change the replay hash: update `REPLAY_GOLDEN_HASH` in
   `crates/ls-sim/src/tests.rs` and say so. A behaviour change without a data change also
   bumps `ls_sim::SIM_BEHAVIOR_VERSION` (mixed into the data hash, so stale clients are
   rejected at the handshake).

Adding a *new* weapon (not in the design doc): add the module file, add it to its family's
`mod.rs`, add a `data/weapons/<id>.ron` and append one entry to `registry::MODULES` (append,
never reorder: rule set codes index this list). `data::tests::embedded_files_match_disk`
fails for a data file that is not registered.

Rules for modules:

- Deterministic only: IEEE-exact float operations, `ls_math::det` for transcendentals, no
  `HashMap` iteration, no clocks. For randomness use `ctx.rng(key)` (a private PCG32 keyed by
  the match seed, the tick and your key, usually the projectile id); never share a stream.
- Keep per-projectile state in the generic fields (`charge`, `generation`, `bounces`,
  `flags`, `arm_tick`, `fuse_tick`, `target`, `vars[4]`, `path`), per-entity state in
  `Entity::vars`/`state` (entity hooks get `&mut Entity`), per-weapon phase in
  `Shot::shots` (paid shots of that weapon before this one, `WeaponState::shots`: alternating
  or rotating fans), and other per-weapon state of a ship (a colour, a lock) in
  `ShipState::weapon_state` (a `u32` per carried weapon, 0 when unset: read it with
  `ship.weapon_state_of(weapon)`, write it with `ctx.set_weapon_state(ship, weapon, value)`,
  `Command::SetWeaponState`; it survives weapon switches and is dropped with the weapon and at
  respawn). Everything is hashed by `World::state_hash`.
- Each hook call may queue at most `behavior::MAX_COMMANDS_PER_HOOK` (256) commands; mind the
  terrain edit rate (every edit is networked and remeshed).

### Hooks (`WeaponBehavior`)

| Hook | Called | Default |
| --- | --- | --- |
| `fire(ctx, shot)` | a paid shot (cooldown over, cost paid; for charge weapons on release) | `fire_pattern`: `def.projectiles` projectiles along the aim (fan up to 3, else centre + ring) |
| `charge_start(ctx, ship, w)` | trigger of a charge weapon goes down (nothing paid yet; a `ChargeStart` cue is emitted) | nothing |
| `charge_hold(ctx, ship, w, held)` | every tick the trigger stays down | `Release` at `max_secs` if `auto_release`, else `Hold` |
| `update(ctx, p)` | every tick before the move | nothing |
| `on_ship_hit(ctx, p, hit)` | the swept move hits a ship | `ctx.detonate` (direct damage, splash, knockback), `Stop` |
| `on_entity_hit(ctx, p, hit)` | hits debris or a decoy | detonate, damage the entity's `hp`, push it, `Stop` |
| `on_terrain_hit(ctx, p, hit)` | hits terrain | detonate with the data's crater and paint, `Stop` |
| `on_fuse(ctx, p)` | `now >= p.fuse_tick` | detonate in place; the projectile ends unless the hook sets a later fuse |
| `on_expire(ctx, p)` | lifetime over | nothing (split into sub-projectiles here) |
| `entity_tick(ctx, e)` / `entity_expired(ctx, e)` | every tick (from the spawn tick on) / at expiry for entities with `Source::Weapon(w)`; `e` is `&mut Entity`: changes (pose, `vars`, `state`, `expires_tick`, `hp <= 0` removes it) are written back before the hook's commands run, and a later `expires_tick` set in `entity_expired` keeps the entity | nothing |
| `on_mode(ctx, ship, w)` | the weapon mode key (V, `Buttons::WEAPON_MODE`, wire bit 7) goes down while `w` is the ship's selected primary | nothing (Paint Thrower: next colour) |

`HitResponse` after a hit: `Stop` (ends), `Continue` (flies through the ship or entity; it is
not hit again until it has left that sphere, `Projectile::passed`; on terrain it counts as
`Stop`), `Pierce { exit }` (continues from the
far side), `Bounce` (reflects off the surface normal, `bounces += 1`), `Stick` (stays at the
hit point, `proj_flags::STUCK`). Up to 8 hits are resolved per projectile per tick.

Charge: with `charge: Some((max_secs, min_secs, auto_release, damage_mult))` in the data the
framework handles hold-to-charge: nothing fires or is paid while charging; on release the shot
fires with `charge = min(held / max_secs, 1)` and damage and knockback times `1 + (damage_mult
- 1) * charge`; releasing before `min_secs` cancels; switching weapons drops the charge.

Guided projectiles: set `proj_flags::CONTROLS_OWNER` (usually in `fire`); while such a
projectile lives, the owner's ship gets no thrust or look (it drifts: the Eyeball malus), the
world does not fire the owner's secondary (one guided missile at a time), and `update` reads
the owner's look input with `ctx.input(p.owner)`. The client predicts the
freeze from the replicated state bit (`ls_game::client::GUIDED_STATE_BIT`), and the browser
shows the missile's camera while it is guided (`web/src/game/camera.ts`).

Instant weapons (Leech Arc): `fire` spawns no projectile and queues its effect directly
(`ctx.target_in_cone`, `ctx.drain_energy`, `ctx.damage_proportional`), plus a `CueKind::Beam`
cue every `beam_every`-th arc whose `value` is the ticks it covers (a cue per arc would cost
10 events per second per user). They emit no `Event::Fired`; the beam cue is their shot. The
Leech Arc keeps its lock in `ShipState::weapon_state` (see [the arsenal](#the-arsenal-as-built)).

### Context helpers (`Ctx`)

Reading: `ship`, `tuning_of`, `def`, `input`, `ships_within`, `nearest_ship`,
`target_in_cone` (with line of sight), `raycast`, `thickness` (rock depth for pierce tests),
`surface_states_near` (hot, charged, glow now), `material_at` (rock material id,
`ls_math::terrain::MATERIAL_*`), `projectiles()` (every projectile in flight in any stage,
except the one whose hook runs), `trail` (a ship's position, velocity and hull up to 3 s ago),
`rng`, and `ctx.view` for the tables (`weapons`, `gadgets`, `ship_types`, `rules`, `visuals`),
the level's `instability` slider (0-100, Tectonic collapse scaling) and state (`ships`,
`projectiles` plus `projectiles_stepping` / `projectiles_stepped` while projectiles are
stepped, `entities`, `pickups` (content at `PickupSpawn::content_pos`), `scrap_piles`,
`terrain`, `layers`, `inputs`). `weapons::debris_crowded(ctx, point, max)` tells whether
`max` debris pieces already float within 2 u (Rivet Gun and Scrap Shotgun spray no more then,
so sustained fire does not fill the debris cap).

Writing (each queues a `Command`): `damage` / `damage_proportional` (knockback proportional to
damage), `splash`, `shockwave` (knockback only), `edit` / `crater` (scaled by rock strength) /
`crater_exact` (not scaled) / `grow` / `paint` (plain) / `paint_color` (with an effect colour)
/ `scorch` (permanent scorch mark, optionally hot for a while; skips scorched spots) /
`channel`, `set_weapon_state`, `spawn_projectile` (see
`weapons::projectile_for` and `sub_projectile`), `spawn_entity` / `spawn_debris`,
`impulse_ship`, `impulse_entity`, `impulse_projectile` (another projectile; a stuck one is
pulled loose: Magnet Sneeze pulling mines in), `move_pickup` (moves a scrap pile, or a pickup's
content off its spawn point, by a delta: Magnet Sneeze), `remove_projectile`, `drain_energy`, `blind`,
`swap`, `rewind`, `disable_weapons`, `adjust_hull`, `cue`, `emit`, and `detonate` (the default
impact).

`ctx.later(delay, |ctx| ...)` defers the terrain edits the closure queues by `delay` ticks
(`Command::EditLater`, applied at the start of that tick before respawns, part of the state
hash while pending); the Tectonic Charge spreads its cracks with it.

Damage goes through one pipeline: friendly-fire scaling between teammates, then the mode's and
the mutators' `on_damage` hooks, then the target's active gadget (`GadgetBehavior::on_damage`,
the Shield; a mode hook may skip it), then hull, `Damaged` event and kill (the mode's
`on_kill`).
Every hit pushes: the projectile's `knockback` scales with its damage (level, ship type, rule
set tuning and charge multipliers), splash knockback scales with the falloff, and
`damage_proportional` uses `WeaponDef::knockback_per_damage`.

### Terrain layers for weapons

- **Surface states**: queue `ctx.edit(TerrainEdit::surface_for_default(center, radius,
  SurfaceStates::HOT, tick))` (or a `TerrainEdit::Surface` with an explicit `until` tick) to
  heat, charge or light surface cells; they expire by themselves. Query them with
  `ctx.surface_states_near(center, radius)` (`SimTerrain::surface_states_near`; test terrains
  report none). Charged rock drains ships that scrape it: a framework step right after each
  ship's movement (`in_contact` and `CHARGED` within radius + 0.2 u) takes the Leech Arc data's
  `charged_drain_per_sec` (default 5 energy/s, `world::CHARGED_DRAIN_PER_SEC`).
  `GameTerrain::surface_states_near` counts every 1 u surface cell that overlaps the query
  sphere (not only cells whose centre is inside), so a ship scraping a wall finds the state of
  the cell under it.
- **Scorch**: `ctx.scorch(center, radius, hot_secs)` marks the surface for good
  (`SurfaceStates::SCORCH`, a permanent bit, see
  [contracts.md](contracts.md#terrainedit)) and heats it for `hot_secs` (0: no heat); already
  scorched spots are skipped, so a trail does not flood the edit log. Use it for scorch marks
  and trails instead of `HOT` edits with a long `until`: those stay in every late joiner's
  Welcome until they expire, and still expire. The Stalker Mine trail, the Echo Cannon trail
  and impacts and the Scatter Laser marks use it (`scorch_hot_secs` / `hot_secs` in their
  data); the Scrap Laser's 10 s hot metal stays a `HOT` edit (a heat glow, not a scorch).
- **Paint colours**: `ctx.paint_color(center, radius, team, color)` claims cells for a team
  with an effect colour (`ls_math::terrain::PAINT_*`); ships react to the colour under them in
  the flight model (see [Ship types](#ship-types)).
- **Materials**: `ctx.material_at(p)` (`SimTerrain::material_at`) for weak-wall effects
  (Magnet Sneeze ripping chunks off weak rock).
- **Strength**: `ctx.crater` (and `detonate`, `channel`) scales a crater by the rock material
  at its centre (weak 1.4x, hard 0.6x, `ls_math::terrain::material_crater_scale`); weapon data
  keeps the design-doc size for ordinary rock. `ctx.crater_exact` keeps the radius (Tectonic
  collapse zone, 35 u in the design table). The scaled edit is what is applied, logged, sent
  and reported in `Event::TerrainEdit`. Splash and shockwave radii are not scaled.

## Hook order per tick

`World::step`:

0. On the first tick the mode's `on_start`; then the deferred terrain edits due this tick.
1. Respawns (with the ship type picked for the next life; never for eliminated or spectating
   players, and only if the mode's `allow_respawn` agrees).
2. Per ship in id order: weapon selection; movement (`simulate_movement` with the paint
   effects of the surface it touches, frozen for ships guiding a projectile); charged-rock
   drain; scavenging (scrap piles, then debris) and Tug scrap passing; gadget
   (G press edge: toggle on/off or instant use; drain; `tick` hook); weapon mode (V press
   edge: the selected primary's `on_mode`); then firing, primary then
   secondary (charge hooks, `fire`). Commands of each hook are applied immediately.
3. Projectiles in list order: `on_fuse`, `on_expire` or `update`, then the swept move with hit
   hooks. Projectiles spawned during this stage start moving next tick. While stepping, the
   not-yet-stepped and already-stepped lists live in the world, so `ImpulseProjectile` /
   `RemoveProjectile` and `View::all_projectiles` reach every projectile.
4. Entities: debris physics, decoy movement (kinematic entities, `Entity::kinematic`, only move
   by their velocity, without terrain collision, so a module can drive them), ships push
   debris, `entity_tick` / `entity_expired` hooks of the owning module (changes to the entity
   written back, then its commands), expiry.
5. Scrap piles, pickup collection, pickup respawns.
6. Ship trails (3 s of position, velocity, hull per ship for Borrowed Time).
7. Mode: `on_tick` (Paint the Void refreshes its coverage every 30 ticks), newly eliminated
   players leave play (`Event::Eliminated`), then `end_check` (`Event::MatchEnded`).

## Gadgets

One slot per ship (`ShipState::gadget`). `GadgetDef` (data): `activation` (`Toggle` or
`Instant`), `energy_cost` per activation, `drain_per_sec` while on, `moving_drain_mult`,
`cooldown_secs`, `duration_secs`, `params`. The G key (`Buttons::GADGET`, wire bit
`buttons::GADGET`) acts on the press edge. Hooks (`GadgetBehavior`): `on_activate`,
`on_deactivate`, `tick`, `on_damage` (modify an `IncomingHit`), `visible_to` (snapshot
relevance), `entity_tick` / `entity_expired`.

| Gadget | Status |
| --- | --- |
| Shield | Implemented: self-damage -75%; enemy hits by the weapon's `ShieldInteraction` (rule set override applied): `Blocked` (energy pays, hull never), `Wears(f)` (each damage point costs `f x energy_per_damage` energy; the hull takes only what the energy cannot cover), `PassThrough(f)` (`f` reaches the hull, the rest is absorbed for free); upkeep `drain_per_sec`; drops at 0 energy; knockback is not reduced |
| Cloak | Implemented: hidden from enemies beyond 6 u unless it fired in the last 0.5 s (`World::ship_visible_to`, used by the server's snapshot filter); 3x drain while moving |
| Decoy | Implemented: launches a shootable decoy entity (integrity = the owner's max hull) that enemy projectiles hit and Seekers and Swarm blobs lock onto. After 6 s it turns `kinematic` and rubber-bands back at `return_speed` (60 u/s), sweeping its path each tick: the first living ship on it (the owner if nobody is in the way, teammates included) takes 99% of the decoy's remaining integrity with `return_knockback_per_damage` knockback, without friendly-fire scaling; the owner gets the kill credit; a `DecoyReturn` cue goes out. It vanishes if its owner is gone and fizzles after `return_max_secs` (3 s) |

## Ship types

`ShipTable` holds the four types from `data/ships/*.ron`; `ShipType::index` (Skitter 0, Tinker
1, Hauler 2, Tug 3) is used on the wire and in rule set codes. Top speed relative to the 30 u/s
laser, damage multiplier, hull, secondary slots, hitbox radius and turn rate follow the design
doc (`tuning::tests::ship_types_match_design_doc`). The Tug has `can_pass_scrap`: holding
scavenge next to a teammate passes `scrap_pass_rate` scrap per second (hull first).

Paint Thrower colours (M2 plan) act in the flight model (`ls_sim::ship`), for every ship on
every peer (client prediction included, it runs the same code): the terrain reports the paint
colours under the ship (`TerrainQuery::paint_colors_near`). Red paint: a wall hit rebounds
at `paint_bounce_restitution` (0.9) instead of `restitution`. Blue paint: no damping while
touching it (within `PAINT_TOUCH_MARGIN` 0.2 u of the surface), and a wall hit keeps the speed
(turned along the wall, at most doubling what is left after the hit). Yellow paint: while
touching it the per-axis caps and the ship's local velocity are held to
`paint_sticky_speed` (0.4) of the cruise or boost cap. Plain paint has no effect. The numbers
are `ShipTuning` fields in `data/ships/*.ron`.

Ship choice: `Hello::ship_type` picks the first ship (the room falls back to the Tinker or the
first allowed type); `Message::SelectShip` / `ClientSession::select_ship` / `World::set_ship_type`
pick the type for the next respawn. The rule set's `allowed_ship_types` is enforced for both.

## Entities

Sim storage: `World::projectiles` (weapon projectiles and mines), `World::entities` (generic:
`Debris`, `Decoy`, `Ghost`, `DustCloud`, `Marker`), `World::pickups` (weapon, upgrade and gadget
pickups at the level's pickup points) and `World::scrap_piles` (scrap pickups, level junk and
wrecks). `World::entity_views` flattens everything but ships and projectiles into generic
`EntityView`s: `ReplicatedKind` (wire byte), `VisualId`, pose, owner, team and 4 state bytes:

| Kind | State bytes |
| --- | --- |
| Weapon pickup | weapon id, ammo, 0, 0 (position: `PickupSpawn::content_pos`, the spawn point plus the offset a Magnet Sneeze moved it by) |
| Upgrade / gadget pickup | weapon / gadget id, 0, 0, 0 |
| Scrap pile, wreck | scrap (whole units, max 255), 0, 0, 0 |
| Debris | scrap, resting (0/1), radius in tenths of a unit, 0 |
| Decoy | integrity percent, module state (ship type index in the low nibble, `0x80` while returning), 0, 0 |
| Dust cloud | radius in whole units, module state bytes 1-3 |
| Ghost, marker | the module's `Entity::state` |

Debris: spheres of 0.3-1.5 u with drag, terrain bounce and rest (no gravity), pushed by ships
that ram them and by splash and shockwaves, blocking projectiles (with `hp`), scavengeable
(emptied debris disappears), at most `entity::MAX_DEBRIS` (64; the oldest goes first).

Ids: entities, pickups and scrap piles share one id counter; the allocator skips ids whose low
15 bits are in use, so the wire id (`id & 0x7fff`) is unique among live entities even for
long-lived ones. Projectiles have their own counter; once it passes 65536 its allocator skips
ids whose low 16 bits belong to a live projectile (a long-lived mine keeps its wire id).

Visuals: `VisualTable` (ids: 8 built-ins, then per weapon its projectile and pickup visual,
then per gadget its pickup visual) is derived from the data on every peer; projectiles carry
the weapon's visual, entities their own. Renderers map `Shape` to a mesh and tint with `color`;
`Shape::ALL` lists every shape in index order (declared together with the enum, so it cannot
miss one).

## Replication (`ls-net` protocol 4)

| Wire item | Contents |
| --- | --- |
| `Snapshot::entities` | `NetEntity`: u16 id, kind, u16 visual, owner, team, position (8x coarser than ships, about 0.03 u), velocity (ship range; zero for debris) and rotation (each with a presence bit when zero/identity), 4 state bytes; an id list (1 bit for an unchanged entity, see [netcode.md](netcode.md#snapshots-and-deltas)) |
| `Snapshot::players` | `NetPlayer`: ship id, team, ship type, gadget id (255 none), name; 1 bit when unchanged against the baseline, else only the changed players |
| `Message::Inventory` | the own ship's weapons (id, ammo, level; protocol 5: paid shots and the module state byte, e.g. the Paint Thrower colour), selected primary and secondary, gadget; reliable, sent when it changes |
| `NetProjectile::state` | replicated projectile flags (`proj_flags` low byte: stuck, armed, hits owner, guided, returning; `CUSTOM_A`/`CUSTOM_B` are module bits: a Paint Thrower blob's colour id, `paint_thrower::projectile_color`) |
| Projectile deltas | predicted from the baseline (straight shots cost a bit per snapshot, the server settles them) and new projectiles from their owner; see [netcode.md](netcode.md#snapshots-and-deltas) |
| `NetEvent::Cue` | `CueKind` byte, source and target ship ids, position, value (Polite warning, blinding, charge start, Tectonic launch and armed, shockwave, Leech beam, swap, rewind, Railspike muzzle flash, gadget on / off / used (`ls_sim::gadget_cue`: the gadget id when switched on, 255 off by its key, 254 off because the energy ran out, `0x100 | id` an instant gadget used; 255 and `0x100 | id` start the cooldown), decoy return, Fusion overcharge, Paint Thrower colour switch `PaintColor` 13 with the colour id); sent to every client |
| `Welcome::edit_count` | edits so far; `Welcome::edit_log` may leave out expired surface-state edits ([netcode.md](netcode.md#terrain-edit-replication)) |
| Terrain edits | `Paint` carries a 2-bit effect colour; `Surface` states may carry the permanent scorch bit 8 |
| Input buttons | bit 7 `WEAPON_MODE` (V) |
| Ship flags | bit 6 `GADGET_ACTIVE` (alias `SHIELDED`), weapon id in bits 10-14 (0-31) |
| `Hello` | adds `data_hash` and `ship_type`; `Welcome` adds `ship_type` and `data_hash` |
| `Message::SelectShip` | ship type for the next respawn (reliable) |
| `RejectReason::DataMismatch` | the client's data hash differs (message names both hashes) |

Per-client relevance: `ls_net::Relevance` (`ship`, `projectile`, `entity`) is applied with
`Snapshot::retain_relevant` before each client's snapshot is packed; the room's
implementation hides ships that `World::ship_visible_to` hides (Cloak) and moving debris more
than 100 u from the viewer's ship. The viewer's own ship is always kept.

## Data hash

`ls_sim::data_hash()` hashes every embedded `data/` file (weapons, ships, gadgets, rule sets)
by path and content (CR bytes ignored; comment edits count), plus
`ls_sim::SIM_BEHAVIOR_VERSION` (bumped with behaviour changes that change no data, together
with `REPLAY_GOLDEN_HASH`; 5 since M3 wave A: mode framework, per-match teams, Tectonic cracks
spread over 6 ticks). Clients send it in `Hello`
(`ls_game::client_hello`); `Room::join` rejects a mismatch with `DataMismatch`, and
`ClientSession::from_welcome_with` refuses a `Welcome` whose hash differs from its own
built-in data. Custom tables (`MatchConfig` with non-built-in data) hash their serialized form.

## Rule set v2 and codes

`RuleSet` adds `gadgets` (availability), `upgrade_pickups`, `gadget_pickups`, `max_on_map` per
pickup (Tectonic Charge: 1), `shield_overrides` and `tuning` (damage, fire rate and ammo
multipliers) per weapon, `mutators` (see the mode framework below), and the loadout `gadget`.
`RuleSet::resolve_with` validates everything and precomputes per-weapon tables
(`ResolvedRules::shield_interaction`, `tuning`, `max_ammo`). Enforcement in the world: banned
weapons never fire (`can_fire`), are never granted (`give_weapon`, pickups) and never spawn
(`update_pickup_spawns` zeroes their weight); tests in `tests_arsenal.rs` and the room tests.

Rule set codes (`RuleSet::to_code` / `from_code`, format in `rulecode.rs`): `LR` + Crockford
base32 in groups of four. Format 4 (M3) carries every mode: a full/diff bit and a 4-bit mode
index after the header, and the mode's settings as its generic field list
(`ModeRules::fields`: each field `bits` bits in steps of `1 / steps`). The diff form (what
`to_code` prints) stores only the differences from the mode's default rule set
(`RuleSet::default_for`: `data/rulesets/<mode>.ron`, or Paint the Void's rules with the
mode's settings and without the Paint Thrower until the mode has its own file) plus a 16-bit
fingerprint of that default: the default Paint the Void rules are `LR8V-6097-G000-TMBG` (19
characters), a ban adds about 3 symbols. A diff code made against a different default is
rejected ("ask for the full code"). The full form (`to_full_code`, about 270 characters)
stores every setting; `to_code` uses it when it is shorter. The M2 formats 2 (full) and 3
(diff), Paint the Void only, still decode. All carry the weapon and gadget counts (the
registry is append-only: codes from an older build decode with the missing weapons banned,
codes from a newer build, or with an unknown mode or mutator, are rejected) and a CRC-16. Encoding fails for values the format cannot carry exactly (tuning in
twentieths, shield fractions in hundredths), so a code always decodes to the rule set it was
made from. `RuleSet::ban_weapon` / `ban_gadget` remove the item from availability, pickups and
the loadout at once.

Server-side bans and balance list: `ls-server --ban <weapon>` and `--balance-list <file>`
(`BalanceList`: bans, gadget bans, tuning multiplied onto the rule set's and rounded to
twentieths, shield overrides that replace the rule set's), re-read before every match; see
[server.md](server.md#cli). The browser rule set editor (`web/src/rules/`, open with
`?editor=rules`) reproduces the encoder, decoder and validation bit for bit; its catalog
(`web/src/rules/catalog.ts`) is generated from the data by the Rust test
`web_catalog_is_current` (`LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode`).

Built-in rule sets: `paint_the_void.ron` (default) and `sandbox_all_weapons.ron` (every weapon
and gadget as a pickup with even weights and short respawns, every ship type; the arsenal e2e
run uses it), plus one default rule set per implemented mode (`ModeKind::ruleset_file`):
`last_color_standing.ron`.

## Mode framework (M3)

Every mode is a module (`crates/ls-sim/src/modes/<id>.rs`) implementing `modes::ModeModule`,
registered in `modes::MODULES` in `ModeKind` order (the rule set code's mode index; append
only). Settings live in the mode's `ModeRules` variant (`ruleset.rs`, declared with
`mode_settings!`: field, default, code precision); every mode has `teams`, `match_secs` and
`respawn_secs`. Mutators (`crates/ls-sim/src/mutators/<id>.rs`, `MutatorModule`) rewrite the
effective rule set before it resolves (`RuleSet::effective`), may change the respawn delay and
damage, and set HUD flags; they run after the mode's hooks.

| Hook | When | Context |
| --- | --- | --- |
| `validate`, `team_counts` | `RuleSet::resolve` | the settings |
| `on_join`, `late_join_spectates` | `World::add_player` | `ModeMut` / `ModeRef` |
| `on_start` | first tick | `ModeCtx` (`Ctx` commands + mutable `ModeState`) |
| `allow_respawn`, `on_spawn` | respawns | `ModeRef` / `ModeMut` + the ship |
| `on_damage` | damage pipeline, after friendly fire | `ModeRef` + `ModeHit` |
| `on_kill` | a ship died | `ModeMut` + `Kill` |
| `on_pickup` | a ship touches a pickup | `ModeState` + the ship |
| `on_tick`, `end_check` | stage 7 | `ModeCtx`, `ModeRef` |
| `view` | `World::mode_view` | `ModeState` |

The world keeps the generic `mode::ModeState` (part of the state hash: kind, teams, phase,
timer, stage and stage end tick, paint coverage, team scores, score target, per-player
`PlayerMode` with score, lives, meter and flags, objectives with position, holder and timer,
module scratch values) and replicates it as `mode::ModeView` (`World::mode_view`: teams, phase
with the winning side, timer, stage, team scores, top 32 players with lives and flags, up to 8
objective markers, HUD `view_flags`). `World::results` is the results table (winner or draw,
kills, deaths, mode score per player). Modules that do not override a hook get the generic
fallback (kills score, a suicide or team kill costs a point, lives where the mode has them,
end on one side left, `score_target` or the timer); the M3 modes not yet implemented
(`ModeModule::implemented` false) are such stubs until their wave B package lands.

Teams are per match: `teams` 2-4 (the smallest team gets a joining player; a team without
spawn points of its own uses any spawn point) or 0 = free-for-all, where every ship is its own
side with its own side id in `ShipState::team` (lowest free id, at most 254 ships), so
friendly fire, kill credit and every weapon's "enemy" test work unchanged; free-for-all spawns
pick the spawn point farthest from the other living ships. Lives and elimination: a player
whose `PlayerMode` is eliminated or spectating never respawns, leaves play at the end of the
tick (`Event::Eliminated` when eliminated) and appears in the view with its flags. Players
joining after `modes::LATE_JOIN_GRACE_TICKS` (10 s) in modes with lives spectate until the
next match; `World::set_spectator` switches by choice.

Last Color Standing (`modes/last_color_standing.rs`, free-for-all only): every player's
`PlayerMode::meter` starts at `colour` (100) and every death drains `drain_per_kill` (34),
whoever killed the ship; damage alone does not. At 0 the player is eliminated (spectates). From
`stalemate_secs` (480) on, every player still in play loses 1 colour per
`stalemate_drain_secs` (2 s), alive or dead, so the match always ends (`ModeState::stage` 0
before the stalemate with `stage_end_tick` at its start, 1 during it). The mode score mirrors
the colour (results list the most colourful first). The match ends when at most one player has
colour (that player wins, nobody is a draw) or on the timer (most colour wins, a tie is a
draw); late joiners spectate. The view sets `MONOCHROME` and `METER`. Tests:
`modes/last_color_standing/tests.rs` (one per rule, default rule set and code, 60 s replay
golden hash `LCS_REPLAY_GOLDEN_HASH`) and `ls-bots` `tests/last_color_standing_match.rs`
(8 bots to a result in a `Room`, release).

A two-team Paint the Void match runs exactly as in M2 through the framework (its state hash
and `REPLAY_GOLDEN_HASH` are unchanged): the mode state beyond the M2 fields is hashed only
when it differs from a fresh two-team Paint the Void state.

## The arsenal as built

Numbers are in each weapon's data file; the module file's docs list the parameters it reads.
Crater sizes in the design doc are diameters (`crater_radius` is half). Cone parameters are
named for what they hold: `cone_deg` is a full apex angle (Seeker: 30 = 15 degrees either
side), `*_half_deg` / `lock_half_angle_deg` are half angles (Leech Arc, Magnet Sneeze).
Damage is balanced for a 3-8 s time to kill on a 100-hull Tinker at point blank (M2 plan);
the Scrap Laser at level 6 (14 per 0.25 s, 1.75 s) is above that band by the plan's numbers.

| Weapon | Bonus / mechanics | Malus | Lasting effect |
| --- | --- | --- | --- |
| Scrap Laser | 30 u/s bolts, 6 damage at level 1 to 14 at level 6 (upgrade pickups) | weak at level 1 | hit rock stays `HOT` 10 s (no re-edit while hot) |
| Scatter Laser | 3 bolts 8 degrees apart; fans alternate horizontal / vertical per shot (`Shot::shots`) | damage and knockback fall to 40% at the end of the bolt's life | fan of permanent scorch marks (hot for 10 s) |
| Plasma Repeater | 33 u/s, 10 bolts/s, 3 damage, 1.5 s reach | 30 energy/s against 10/s regeneration | every 2nd bolt melts a 0.4 u pit; hits stay hot |
| Fusion Charger | hold up to 2 s: up to 4x damage and knockback (framework charge) | past 2 s it burns 10 hull/s (never below 1; `Overcharge` cue) | melts a channel 1-5 u deep with charge, through thin walls; shoves debris aside |
| Corkscrew | 5 bolts in a line that spins around the aim axis (1 turn/s), widening up to `max_radius` 4 u; knockback along the shot axis | only the centre bolt connects at range | the 4 spinning bolts etch 2-crater grooves (already carved spots skipped): spirals |
| Ricochet Arc | bounces off walls up to 3 times; can hit its owner after the first bounce | self-hits | 0.3 u chip at every bounce |
| Leech Arc | instant 12 u arc to the nearest visible enemy in a 30 degree half-cone: 2 damage and 2 energy drained per arc, 10 arcs/s; the lock persists (`ShipState::weapon_state`: locked id + 1) while the target lives, stays within 12 u in line of sight and inside a 60 degree half-cone (`lock_keep_half_angle_deg`), even if another enemy comes closer | nothing beyond 12 u (still pays) | touched rock `CHARGED` 15 s; the framework drains 5 energy/s from ships scraping it |
| Rivet Gun | 180 u/s, 10 rivets/s, 3 damage, half a scrap each | burns 5 scrap/s | rivets embed (impact, no crater); weak rock: 20% chance of a chip crater and a debris chunk (not while 2 pieces float near) |
| Railspike | 270 u/s, 35 damage, pierces every ship and debris piece and walls up to 2 u (straight hole) | 1.5 s between shots; `MuzzleFlash` cue, firing uncloaks for 0.5 s | straight holes through thin rock; 0.3 u dent in thick rock |
| Scrap Shotgun | 8 pellets at 200 u/s in a random 6 degree cone | 30 u range, damage falls to 30% by 24 u | pockmarks; the last 2 pellets spray debris (not while 2 pieces float near) |
| Bolt Rocket | 40 u/s dumbfire, 30 direct + 2.5 u splash | slow | 2 u crater |
| Seeker | re-targets every tick to the nearest enemy ship or enemy Decoy in its cone within 60 u and line of sight, 2.5 rad/s turn | fooled by decoys; a boosting pilot outflies it | 2 u crater |
| Quicksilver | fastest ordnance, 45 u/s | small 1.5 u blast | 2 u crater |
| Eyeball | guided by the owner's look input (turn capped at 3 rad/s); flies straight if the owner dies | the owner's ship drifts without thrust or look; one guided at a time | 4 u crater |
| Swarm Pod | the pod hits like a rocket and releases 6 homing blobs on any impact | blobs are weak alone | scatter of small craters |
| Heavy Rocket | 70 direct, 8 u splash | 20 u/s; its splash hurts the shooter | 6 u crater that opens passages |
| Flash Pod | bursts on impact or after 3 s: blinds every pilot within 15 u in line of sight for 2 s (`blind`, `Blinded` cue) | blinds you and teammates too | nearby surfaces `GLOW` 30 s |
| Prox Mine | dropped behind, arms after 1 s, triggers within 2 u, 40 + 5 u splash | hurts owner and teammates at full damage (unless friendly fire is off) | sticks to walls; 3 u crater |
| Stalker Mine | armed, creeps at 4 u/s toward the nearest visible enemy within 10 u | slow; a pilot who notices it flies away | permanent scorch every 1.5 u (hot for 10 s) |
| Tectonic Charge | the design table: core 4 u kill, damage 25 u, collapse 35 u (`crater_exact`), shockwave 60 u, 1.5 s fuse, 6 sub-charges; cracks scale with Instability | slow, 1 per map, hurts the owner | dust cloud 20 s, debris field, level-wide jostle; rides ships, debris and decoys it sticks to |
| Echo Cannon | records its path, flies back along it after 2 s (can hit twice) | the return leg hits its owner (the Shield cuts it) | scorch marks along the path (every 2nd point, capped) and at impacts |
| Swap Harpoon | swaps positions and velocities with the ship it hits (`Swapped` cue; lag compensation keeps the swap) | 4 s cooldown | spike marker in walls; holding Scavenge next to it grabs it |
| Paint Thrower | paints the team colour (Paint the Void scoring) with an effect colour: V cycles red (bouncy, 0.9 restitution), blue (slippery: no wall friction, speed kept), yellow (sticky: 40% speed while touching); the colour is the weapon state (red when unset), blobs carry it in their replicated bits, a `PaintColor` cue tells the HUD | no damage | paint stays until painted over (team and colour) |
| Borrowed Time | rewinds position, velocity and hull by 3 s (`trail`), ghost entity at the old spot | weapons offline 3 s | craters made meanwhile stay |
| Magnet Sneeze | cone pull: debris, enemy ships, pickups and scrap piles (they jump up to 10 u towards you) | pulls every other projectile in too, stuck mines included | rips chunks off weak rock |
| Polite Laser | 1.5 s charge, 75 damage; `PoliteWarning` cue to the target (repeated to a new target in the aim) | the target is warned | clean holes through thin walls |
| Seed Grenade | grows a deterministic cluster of `AddSphere` cover (never inside a ship; floating cover in mid-air) | the cover blocks your shots too | lasting rock; 2% rubber duck |
