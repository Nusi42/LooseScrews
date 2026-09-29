# Milestone 2 — Arsenal and physics

Status: in progress (waves A and B integrated, wave C next) · Design source: [design/game-design-doc.md](design/game-design-doc.md) · M1 plan: [implementation-plan.md](implementation-plan.md)

Milestone 2 turns the four-weapon vertical slice into the full combat sandbox from the design doc: every weapon, all four ship types, gadgets, lasting physical effects (debris, surface states, dust) and configurable match rules. It also fixes two M1 issues found in soak testing and closes the M1 gaps that M2 depends on.

## 1. Scope

| Area | In scope |
| --- | --- |
| M1 issues | (A) bots never fight on some worlds; (B) clients receive a full snapshot resync far too often (284 full vs 36 joins in a 1 h soak) |
| M1 gaps closed | Pickups, scrap piles, wrecks and debris drawn in the browser; player names shown; game-data hash checked in the handshake |
| Ship types | Skitter, Tinker, Hauler, Tug from `data/ships/*.ron`; ship choice before spawn; rule sets restrict allowed types; Tug can pass scrap to teammates |
| Energy primaries | Scrap Laser levels 1–6 (upgrade pickups), Scatter Laser, Plasma Repeater, Fusion Charger, Corkscrew, Ricochet Arc, Leech Arc |
| Kinetic primaries | Rivet Gun, Railspike (pierces ships and walls up to 2 u), Scrap Shotgun — all fire scrap |
| Ordnance | Bolt Rocket, Seeker, Quicksilver, Eyeball (guided), Swarm Pod, Heavy Rocket, Flash Pod, Prox Mine, Stalker Mine, Tectonic Charge |
| Specials | Echo Cannon, Swap Harpoon, Paint Thrower, Borrowed Time, Magnet Sneeze, Polite Laser, Seed Grenade (with the rare rubber duck) |
| Gadgets | Shield (self-damage protection + per-weapon shield interaction), Cloak (3x energy drain while moving), Decoy (rubber-band return hit) |
| Physics | Knockback on every hit, debris entities (bounce, block shots, pushable, scavengeable), terrain strength per region, surface states (hot, charged, glow, paint), dust clouds |
| Match rules | Full rule set from the design doc (availability, loadout, pickup spawns, shield interaction, weapon tuning multipliers, ship types, friendly fire), rule set share codes, in-browser rule set editor, server ban list |

Out of scope (M3+): other game modes, Instagib, lobby, comedy director, gravity wells, remaining sliders.

## 2. Design decisions for gaps in the design doc

The design doc gives behavior but not every number; these defaults live in `data/` and are tuning starting points.

| Topic | Decision |
| --- | --- |
| Damage scale | Tinker hull is 100. Laser level 1 bolt 6, level 6 bolt 14; Railspike 35; Bolt Rocket 30 direct + splash; Heavy Rocket 70; Tectonic core kills. The content packages pick the rest to keep time-to-kill 3–8 s for primaries |
| Weapon selection | Number keys 1–0 select carried primaries in design-doc order; mouse wheel cycles primaries; X cycles secondaries; G activates the gadget; F stays overdrive |
| Instant weapons | Only Leech Arc is instant (a 12 u arc); every other weapon is a projectile, per the speed model |
| Eyeball | While guiding, the player's look input steers the missile and its camera is shown; the ship keeps drifting without thrust (the malus) |
| Borrowed Time | Server keeps 3 s of per-ship history; rewind restores position, velocity and hull, leaves a ghost entity at the old spot for 3 s, weapons offline 3 s |
| Swap Harpoon | On hit, positions swap and velocities swap (you inherit their momentum); 4 s cooldown |
| Echo Cannon | Records its path and returns along it after 2 s; the return leg can hit its owner |
| Polite Laser | 1.5 s charge; the target receives a `PoliteWarning` event (HUD "excuse me" warning; audio arrives with the audio milestone) |
| Seed Grenade | Grows a deterministic cluster of `AddSphere` edits seeded by the projectile id; 2% chance of a giant rubber duck built from spheres |
| Cloak | Server omits cloaked enemies from snapshots unless within 6 u or they fired in the last 0.5 s; clients draw teammates' cloak as a shimmer |
| Shield interaction | Per weapon in data: `Blocked`, `Wears(fraction)` or `PassThrough(fraction)`; self-damage reduced by 75% |
| Decoy | Integrity equals owner's max hull; lasts 6 s; return hit = 99% of remaining integrity to the first ship on its path |
| Debris | Server-simulated spheres (radius 0.3–1.5 u) with damping, bounce and a cap of 64 networked pieces; small cosmetic chips are client-only |
| Surface states | Stored per surface cell next to paint: hot (10 s, visual), charged (15 s, drains 5 energy/s from ships scraping it), glow (30 s, light) |
| Rule set codes | Rule sets serialize to a compact code like share codes (`LR` prefix); invite links carry world + rule set codes |
| Paint Thrower colours | Paint always claims the cell for the shooter's team (Paint the Void scoring is unchanged); the V key cycles the effect colour stored with it: red = bouncy (ships rebound at 0.9 restitution), blue = slippery (no wall friction, speed kept), yellow = sticky (ships touching it drop to 40% speed). Painting over replaces both team and colour |
| Data hash | `Hello` carries a hash of all loaded `data/` files; the server rejects mismatches with a clear message |
| Time to kill (wave B) | Primaries tuned to 3-8 s on a Tinker at point blank: Scatter Laser 3 per bolt, Plasma Repeater 3 (1.5 s reach), Corkscrew 0.5 s between volleys (2 s reach), Rivet Gun 10/s at half a scrap per rivet, Railspike 1.5 s between shots. The Scrap Laser at level 6 (1.75 s) is above the band by the table's numbers, on purpose: it takes five upgrade pickups |
| Balance list | `ls-server --balance-list` bans apply on top of any rule set; its tuning multiplies the rule set's (rounded to twentieths), so a list meant to weaken a weapon never strengthens it; its shield overrides replace the rule set's |
| Level connectivity | Generator version 2 carves a network of caverns and 4 u tunnels linking both team ends into every level; spawns and pickups must reach it (`ls_game::setup::CONNECT_MARGIN`) |

## 3. Waves

| Wave | Work | Owner areas |
| --- | --- | --- |
| A1 | Fix issue A: global navigation for bots (coarse open-space grid + A*), hunting enemies across the level; regression test on the soak world `LS04-PNEY-V86Z-VXZM-K4M6-8001-8F8T` (generator version 2: `LS08-PNEY-V86Z-VXZM-K4M6-8001-8F8P`, same seed and sliders) | `crates/ls-bots` |
| A1 | Fix issue B: find why full snapshots and input starvation repeat after joins/restarts; fix root cause; regression test asserting ≤ 1 full snapshot per join in steady state | `crates/ls-game`, `crates/ls-server`, `crates/ls-net` |
| A2 | Framework: weapon behavior registry with one module per weapon (stubs for all), projectile kinds, charge/guided/beam mechanics hooks, ship types from data, gadget slot, debris entities, generic entity replication in `ls-net` with data-driven visuals, data hash in handshake, rule set v2 + codes | `crates/ls-sim`, `crates/ls-net`, `crates/ls-game`, `data/` |
| A3 | Surface states and terrain strength in worldgen; generic entity rendering in the client (pickups, scrap, wrecks, debris, names, data-driven projectile visuals) | `crates/ls-worldgen`; `crates/ls-wasm` + `web/` |
| B | Content in parallel, each package fills its own weapon modules + data: energy; kinetic + ordnance I; ordnance II incl. Tectonic Charge; specials; gadgets + ship types; rule set editor + codes + ban list | per-module files |
| C | Client mechanics UI (eyeball camera, charge meters, blind, cloak, ghost, decoy, shield, dust, tectonic warning, ship/loadout selection, weapon selection HUD); bots use all weapons and ship types; scenario tests for design numbers; e2e with mixed ships and all weapons | `web/` + `crates/ls-wasm`; `crates/ls-bots`; `crates/ls-sim/tests` + `tools/` |
| D | Review, fix, full CI + e2e; docs/ and userDocs/ updates | all |

### Wave A status (integrated)

Done: issue A (bot navigation: `crates/ls-bots/src/nav.rs`, Hunt mode, `tests/nav_sweep.rs`
over the soak world and 9 random worlds plus a 5-minute soak-world match; stall blocks no
longer wear out links, edit updates are resumable within the per-tick budget), issue B (no
snapshots before a client's first input; pong catch-up only before sync; regression test with
late-input bounds), the framework (weapon and gadget modules with generic fallbacks, ship
types, entities and debris, rule set v2 and codes with module counts, data hash with
`SIM_BEHAVIOR_VERSION`, generic entity replication with id lists and coarse entity positions,
`Message::Inventory`), surface states and rock strength (craters scaled in `ls_sim::Ctx::crater`,
`crater_exact` for exact zones, surface states drawn in the browser and expiring), and the
generic client rendering. Hook API additions from the wave A review: `&mut Entity` in entity
hooks (write-back, expiry extension, `kinematic` entities), `ImpulseProjectile` /
`RemoveProjectile` and `ctx.projectiles()`, `ctx.surface_states_near` / `ctx.material_at`,
`ctx.view.instability`, `Shot::shots`, a charged-rock energy drain step, pass-through hits
counted once per ship (`Projectile::passed`).

Wave B must fill: the 23 weapon modules still marked `TODO(wave-B)` and the Decoy return hit
(see `docs/architecture/arsenal-framework.md`); Tectonic Charge collapse with
`ctx.crater_exact` scaled by `ctx.view.instability`; Leech Arc charged surfaces as
`TerrainEdit::Surface` edits (the drain is already a framework step); Magnet Sneeze with
`ctx.impulse_projectile` and `ctx.material_at`; Scatter Laser / Corkscrew fans from
`Shot::shots`. (Done in wave B, see below.)

### Wave B status (integrated)

Done: every weapon module (energy, kinetic, ordnance I and II with the Tectonic Charge,
specials) and every gadget (Decoy return hit) implements its design-doc behaviour with
scenario tests next to the module; `weapons::tests` now fails if any module falls back
(`docs/architecture/arsenal-framework.md#the-arsenal-as-built`). Plan numbers applied (Scrap
Laser 6 to 14, Bolt Rocket 30) and primaries retuned to the 3-8 s time to kill (see the
decisions table). Integration fixes: one guided Eyeball at a time (the world skips the
secondary while guiding), Magnet Sneeze pulls pickups and scrap piles (`Command::MovePickup`,
`PickupSpawn::offset`, `View::pickups` / `scrap_piles`), lag compensation keeps Swap Harpoon
swaps and Borrowed Time rewinds (`restore_offsets`, `World::shift_ship`), terrain edit and
debris rates capped (Corkscrew grooves, Echo Cannon scorch, `weapons::debris_crowded`),
`CueKind::Overcharge` for the Fusion Charger burn, Leech Arc beam cue every 3rd arc, Tectonic
Charge rides debris and decoys, Prox Mine honours friendly fire off, Stalker Mine can be
pulled loose. Rule set codes version 3 (diffs against the default, `LR6V-69CM-G006-58R`),
`ls-server --ruleset <file|code> --ban --balance-list`, the browser rule set editor
(`?editor=rules`) and `data/rulesets/sandbox_all_weapons.ron`. Generator version 2 carves a
cavern and tunnel network into every level (share codes now start `LS08`/`LS0B`; the M1 codes
in tests and tools were re-encoded with the same seeds and sliders). Protocol 3 compresses
ship deltas (busy 32-ship fights fit the 17,280 B/s budget for baselines up to 6 ticks old).
`SIM_BEHAVIOR_VERSION` 3; `REPLAY_GOLDEN_HASH` re-blessed (native and wasm32 agree).

Checks at integration: workspace tests, clippy, fmt, wasm32 unit tests, bots release tests
(navigation sweep over the LS08 soak world), web typecheck/tests/build, determinism
cross-check; e2e over WebSocket and WebTransport (7 bots, 90 s) and a full sandbox match with
8 bots (0 desyncs, worst window tick p99 3.7 ms, under the 4 ms budget but with little margin
while other builds ran on the machine).

Open for wave C/D: the client (`ls-wasm` ShotPredictor) still predicts straight bolts (it draws
a stray bolt for the instant Leech Arc and mis-predicts Scatter / Corkscrew fans); client
visuals for the new cues and states (Leech beam, muzzle flash, decoy return, overcharge,
ghost, spike marker, glow, riding Tectonic Charge); bots use only the laser, Rivet Gun and
secondaries; Leech Arc re-acquires its target every arc (a real lock needs per-weapon state);
paint colour effects are M3; no permanent scorch surface state (scorch is long-lived `HOT`);
projectile deltas have no motion prediction yet (busy fights with many shots in flight exceed
the budget); the rule set editor has been checked by its Node tests only, not in a browser;
`userDocs/` describe none of wave B yet (wave D).

## 4. Acceptance criteria

- Every weapon, gadget and ship type in the design doc exists in `data/` and behaves as described; scenario tests cover each one's bonus, malus and lasting effect.
- Tectonic Charge zones match the design table (core 4 u kill, damage 25 u, collapse 35 u, shockwave 60 u, 1.5 s fuse, 6 sub-charges).
- Decoy return damage equals 99% of remaining integrity and hits the first ship on its path, teammates included.
- Replay determinism and the WASM golden-hash check still pass; `REPLAY_GOLDEN_HASH` updated deliberately.
- Bots fight on every world in a sweep of 10 random share codes (hits > 0 in each 60 s run).
- Steady-state full snapshots ≤ 1 per join across a 10-minute soak.
- e2e with 8 bots using mixed ship types and all weapons: 0 desyncs, 0 protocol errors, tick p99 < 4 ms with debris at cap.
- Rule set editor produces a code that the server accepts and enforces (banned weapons never spawn or fire).
- docs/ and userDocs/ describe every weapon, gadget, ship type and the rule set editor as built.
