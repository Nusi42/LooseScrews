# Arsenal framework (M2)

The framework that lets every weapon, gadget and ship type of the design doc live in its own
file, so they can be filled in parallel without touching shared code. It is part of
`ls-sim` (deterministic: follow [determinism.md](determinism.md)), plus the generic replication
in `ls-net` and its glue in `ls-game`. The M2 scope and design decisions are in
[milestone-2-plan.md](../milestone-2-plan.md).

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
knockback, the data's crater and paint, charge from the data's `charge`). The four M1 weapons
(Scrap Laser, Rivet Gun, Bolt Rocket, Paint Thrower) are exactly these defaults and report
`implemented() == true`; the others are marked `TODO(wave-B)` with a list of the hooks they need.

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
  `Entity::vars`/`state` (entity hooks get `&mut Entity`), and per-weapon phase in
  `Shot::shots` (paid shots of that weapon before this one, `WeaponState::shots`: alternating
  or rotating fans). Everything is hashed by `World::state_hash`.
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
projectile lives, the owner's ship gets no thrust or look (it drifts: the Eyeball malus) and
`update` reads the owner's look input with `ctx.input(p.owner)`. The client predicts the
freeze from the replicated state bit (`ls_game::client::GUIDED_STATE_BIT`); the eyeball camera
is wave C.

Instant weapons (Leech Arc): `fire` spawns no projectile and queues its effect directly
(`ctx.target_in_cone`, `ctx.drain_energy`, `ctx.damage_proportional`, a `CueKind::Beam` cue).

### Context helpers (`Ctx`)

Reading: `ship`, `tuning_of`, `def`, `input`, `ships_within`, `nearest_ship`,
`target_in_cone` (with line of sight), `raycast`, `thickness` (rock depth for pierce tests),
`surface_states_near` (hot, charged, glow now), `material_at` (rock material id,
`ls_math::terrain::MATERIAL_*`), `projectiles()` (every projectile in flight in any stage,
except the one whose hook runs), `trail` (a ship's position, velocity and hull up to 3 s ago),
`rng`, and `ctx.view` for the tables (`weapons`, `gadgets`, `ship_types`, `rules`, `visuals`),
the level's `instability` slider (0-100, Tectonic collapse scaling) and state (`ships`,
`projectiles` plus `projectiles_stepping` / `projectiles_stepped` while projectiles are
stepped, `entities`, `terrain`, `layers`, `inputs`).

Writing (each queues a `Command`): `damage` / `damage_proportional` (knockback proportional to
damage), `splash`, `shockwave` (knockback only), `edit` / `crater` (scaled by rock strength) /
`crater_exact` (not scaled) / `grow` / `paint` / `channel`, `spawn_projectile` (see
`weapons::projectile_for` and `sub_projectile`), `spawn_entity` / `spawn_debris`,
`impulse_ship`, `impulse_entity`, `impulse_projectile` (another projectile; a stuck one is
pulled loose: Magnet Sneeze pulling mines in), `remove_projectile`, `drain_energy`, `blind`,
`swap`, `rewind`, `disable_weapons`, `adjust_hull`, `cue`, `emit`, and `detonate` (the default
impact).

Damage goes through one pipeline: friendly-fire scaling between teammates, then the target's
active gadget (`GadgetBehavior::on_damage`, the Shield), then hull, `Damaged` event and kill.
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
- **Materials**: `ctx.material_at(p)` (`SimTerrain::material_at`) for weak-wall effects
  (Magnet Sneeze ripping chunks off weak rock).
- **Strength**: `ctx.crater` (and `detonate`, `channel`) scales a crater by the rock material
  at its centre (weak 1.4x, hard 0.6x, `ls_math::terrain::material_crater_scale`); weapon data
  keeps the design-doc size for ordinary rock. `ctx.crater_exact` keeps the radius (Tectonic
  collapse zone, 35 u in the design table). The scaled edit is what is applied, logged, sent
  and reported in `Event::TerrainEdit`. Splash and shockwave radii are not scaled.

## Hook order per tick

`World::step`:

1. Respawns (with the ship type picked for the next life).
2. Per ship in id order: weapon selection; movement (`simulate_movement`, frozen for ships
   guiding a projectile); charged-rock drain; scavenging (scrap piles, then debris) and Tug
   scrap passing; gadget
   (G press edge: toggle on/off or instant use; drain; `tick` hook); then firing, primary then
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
7. Mode scores and timer.

## Gadgets

One slot per ship (`ShipState::gadget`). `GadgetDef` (data): `activation` (`Toggle` or
`Instant`), `energy_cost` per activation, `drain_per_sec` while on, `moving_drain_mult`,
`cooldown_secs`, `duration_secs`, `params`. The G key (`Buttons::GADGET`, wire bit
`buttons::GADGET`) acts on the press edge. Hooks (`GadgetBehavior`): `on_activate`,
`on_deactivate`, `tick`, `on_damage` (modify an `IncomingHit`), `visible_to` (snapshot
relevance), `entity_tick` / `entity_expired`.

| Gadget | Status |
| --- | --- |
| Shield | Implemented: self-damage -75%; enemy hits by the weapon's `ShieldInteraction` (rule set override applied): `Blocked` (energy pays, hull never), `Wears(f)` (energy pays `f` per damage point, the rest reaches the hull), `PassThrough(f)` (`f` reaches the hull); drops at 0 energy |
| Cloak | Implemented: hidden from enemies beyond 6 u unless it fired in the last 0.5 s (`World::ship_visible_to`, used by the server's snapshot filter); 3x drain while moving |
| Decoy | Fallback: launches a shootable decoy entity (integrity = max hull, 6 s); TODO(wave-B): the rubber-band return hit (the gadget's `entity_tick` can now switch the decoy to `kinematic` and drive it back through `&mut Entity`, and `entity_expired` can extend its life) |

## Ship types

`ShipTable` holds the four types from `data/ships/*.ron`; `ShipType::index` (Skitter 0, Tinker
1, Hauler 2, Tug 3) is used on the wire and in rule set codes. Top speed relative to the 30 u/s
laser, damage multiplier, hull, secondary slots, hitbox radius and turn rate follow the design
doc (`tuning::tests::ship_types_match_design_doc`). The Tug has `can_pass_scrap`: holding
scavenge next to a teammate passes `scrap_pass_rate` scrap per second (hull first).

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
| Weapon pickup | weapon id, ammo, 0, 0 |
| Upgrade / gadget pickup | weapon / gadget id, 0, 0, 0 |
| Scrap pile, wreck | scrap (whole units, max 255), 0, 0, 0 |
| Debris | scrap, resting (0/1), radius in tenths of a unit, 0 |
| Decoy | integrity percent, module state, 0, 0 |
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

## Replication (`ls-net` protocol 2)

| Wire item | Contents |
| --- | --- |
| `Snapshot::entities` | `NetEntity`: u16 id, kind, u16 visual, owner, team, position (8x coarser than ships, about 0.03 u), velocity (ship range; zero for debris) and rotation (each with a presence bit when zero/identity), 4 state bytes; an id list (1 bit for an unchanged entity, see [netcode.md](netcode.md#snapshots-and-deltas)) |
| `Snapshot::players` | `NetPlayer`: ship id, team, ship type, gadget id (255 none), name; 1 bit when unchanged against the baseline, else only the changed players |
| `Message::Inventory` | the own ship's weapons (id, ammo, level), selected primary and secondary, gadget; reliable, sent when it changes |
| `NetProjectile::state` | replicated projectile flags (`proj_flags` low byte: stuck, armed, hits owner, guided, returning) |
| `NetEvent::Cue` | `CueKind` byte, source and target ship ids, position, value (warnings, blinding, charge start, gadget on/off, shockwave, beam, swap, rewind) |
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
with `REPLAY_GOLDEN_HASH`). Clients send it in `Hello`
(`ls_game::client_hello`); `Room::join` rejects a mismatch with `DataMismatch`, and
`ClientSession::from_welcome_with` refuses a `Welcome` whose hash differs from its own
built-in data. Custom tables (`MatchConfig` with non-built-in data) hash their serialized form.

## Rule set v2 and codes

`RuleSet` adds `gadgets` (availability), `upgrade_pickups`, `gadget_pickups`, `max_on_map` per
pickup (Tectonic Charge: 1), `shield_overrides` and `tuning` (damage, fire rate and ammo
multipliers) per weapon, `mutators` (none supported yet), and the loadout `gadget`.
`RuleSet::resolve_with` validates everything and precomputes per-weapon tables
(`ResolvedRules::shield_interaction`, `tuning`, `max_ammo`). Enforcement in the world: banned
weapons never fire (`can_fire`), are never granted (`give_weapon`, pickups) and never spawn
(`update_pickup_spawns` zeroes their weight); tests in `tests_arsenal.rs` and the room tests.

Rule set codes (`RuleSet::to_code` / `from_code`): `LR` + Crockford base32 in groups of four,
format in `rulecode.rs` (version 2, the number of weapons and gadgets the code covers, name,
mode, every setting, CRC-16). Codes from an older build (fewer weapons: the registry is
append-only) decode with the missing weapons banned; codes from a newer build are rejected
with "code from a newer build". Encoding fails for values
the format cannot carry exactly, so a code always decodes to the rule set it was made from;
the default Paint the Void rules take about 210 symbols (270 characters with dashes).
