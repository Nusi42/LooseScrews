# Milestone 2 — Arsenal and physics

Status: complete (waves A–D integrated; open items listed under wave C status) · Design source: [design/game-design-doc.md](design/game-design-doc.md) · M1 plan: [implementation-plan.md](implementation-plan.md)

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

Open for wave C/D at the end of wave B (see wave C status for what is left): the client
(`ls-wasm` ShotPredictor) still predicted straight bolts (it draws
a stray bolt for the instant Leech Arc and mis-predicts Scatter / Corkscrew fans); client
visuals for the new cues and states (Leech beam, muzzle flash, decoy return, overcharge,
ghost, spike marker, glow, riding Tectonic Charge); bots use only the laser, Rivet Gun and
secondaries; Leech Arc re-acquires its target every arc (a real lock needs per-weapon state);
paint colour effects are M3; no permanent scorch surface state (scorch is long-lived `HOT`);
projectile deltas have no motion prediction yet (busy fights with many shots in flight exceed
the budget); the rule set editor has been checked by its Node tests only, not in a browser;
`userDocs/` describe none of wave B yet (wave D).

### Wave C status (integrated)

Done:
- **Client mechanics** (`crates/ls-wasm/src/mechanics.rs`, `web/src/game/mechanics.ts`,
  `web/src/ui/effects.ts`, `web/src/ui/shipSelect.ts`): Flash Pod whiteout, shake by
  Tectonic zone, banners (Polite warning, Tectonic launch and crack naming the own zone,
  overcharge, decoy return, swap, rewind with the weapons-offline countdown), Leech beams,
  muzzle and decoy flashes, shockwave shells, Eyeball camera and frame, charge meter, gadget
  line with cooldown, cloak shimmer in a light team shade, shield bubble, ghost, rock
  materials, permanent scorch and paint colours in the terrain shader, the ship bay (B, 1-4)
  and ship choice on the start screen.
- **Shot prediction per weapon**: Leech Arc, Borrowed Time and Prox Mine draw nothing (cost
  kept), Scatter fans, Corkscrew helices, charge weapons on release, the Scrap Shotgun's
  random cone as its centre pellet only, Borrowed Time takes the weapons offline as it fires.
- **Paint colours end to end**: V is `Buttons::WEAPON_MODE` (bit 7) from the browser;
  `GameCore` keeps G and V presses from frames without a tick. The HUD colour is the
  simulation's (practice: weapon state; online: the server's `PaintColor` cue, guessed ahead
  on V, red after a respawn) and only shows while the Paint Thrower is selected. Blobs are
  tinted by their colour bits. Each effect has its own pattern (red stripes, blue waves,
  yellow dots) drawn in a dark team shade, so neither effect nor owner relies on hue.
  Sticky paint scales the whole local velocity (direction kept).
- **Gadget cues** (`ls_sim::gadget_cue`): instant use (`0x100 | id`) and running out of
  energy (254) are told apart from a key-press off (255), so the online cooldown is right
  for the Decoy and after an empty toggle.
- **Sim follow-ups**: protocol 4 / `SIM_BEHAVIOR_VERSION` 4 (paint colours and their flight
  effects, Leech Arc lock, permanent `SCORCH` used by the Stalker Mine trail, Echo Cannon
  and Scatter Laser marks, Welcome pruning of expired timed edits with the true edit count,
  projectile motion prediction and settling).
- **Bots use the arsenal** (`crates/ls-bots`): every weapon has a role and range band,
  primaries by range, secondaries by situation, gadgets, Eyeball guidance, Tectonic zones
  fled, pickups collected (also mid-fight within 30 u; the Tectonic Charge counts three
  times closer; a carrier fires it after 20 s at the rock on the line to its target if that is
  30 u+ away), random ship types, per-weapon stats. Swap and rewind teleports no longer count
  as desyncs (a snap waits 1 s for their cue).
- **Tests and tools**: `tests/arsenal_match.rs` (8 bots, 240 s sandbox: at least 15 distinct
  combat weapons incl. a mine and the Tectonic Charge, reconciliation under 1 u; last run 18
  plus the Paint Thrower); the browser smoke test (`tools/browser-smoke`, part of CI when
  Chrome or Edge is present) with a mechanics phase against a second server
  (`tools/browser-smoke/mechanics.ron`); e2e `--min-debris`, `--max-full-per-join` and
  `tools/e2e-rulesets/sandbox_debris.ron`; the server's `tick stats` line has `debris`,
  `debris_peak` and per-phase maxima (`recv_max_us`, `step_max_us`, `send_max_us`).

Checks at integration (CARGO_TARGET_DIR=target/integrator-c): `pwsh tools/ci.ps1` passed
(fmt, clippy, workspace tests, ls-bots release tests, wasm32 unit tests, web typecheck/tests
/build, determinism cross-check, browser smoke 60 checks); e2e over WebSocket and
WebTransport (7 bots, 90 s, worst p99 317 / 295 us); stress and soak runs in the acceptance
section below.

Open (after M2; scheduled in [implementation-plan.md §8, carried-over items](implementation-plan.md#carried-over-items)):
- Bandwidth: busy 32-ship fights average within 17,280 B/s at every baseline age, but the
  busiest snapshots are up to about 6 % over at a 4-tick and 19 % over at a 6-tick baseline
  (`busy_fight_with_projectiles_fits_the_budget`, four seeds; netcode.md). New projectiles'
  velocity residuals dominate; needs a velocity predictor from the owner's orientation or
  coarser new-projectile velocities.
- A Tectonic collapse makes one 2-7 ms tick (a 35 u exact crater plus cracks, about 30
  edits in one tick); it does not move a window's p99. Spreading the cracks over later ticks
  would cap it.
- Tick p99 in e2e is wall-clock: on a busy machine (other builds, a file sync
  client) windows go over 4 ms with the time in the receive/send phases. Stress runs need a
  quiet machine.
- Optional: `WeaponState::shots` in `Message::Inventory` (the predictor infers fan/helix
  phase from snapshots), a weapon-state byte in `NetWeapon`, drawing the own Leech beam
  locally instead of one round trip late.

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

### Acceptance status (wave C integration)

Evidence with `CARGO_TARGET_DIR=target/integrator-c` on Windows 11, 32 threads, NVIDIA GPU.

| Criterion | Status | Evidence |
| --- | --- | --- |
| Every weapon, gadget and ship type in `data/`, scenario tests for bonus, malus, lasting effect | met (code); docs part open | `cargo test --workspace` (in `pwsh tools/ci.ps1`): ls-sim 246 unit tests pass (243 on wasm32), including a scenario-test module per weapon and gadget and `weapons::tests`, which fails if any module falls back to the generic behaviour |
| Tectonic zones match the design table | met | `tectonic_charge::tests::numbers_and_limits_follow_the_design_doc`, `zones_core_damage_and_shockwave`, `collapse_caves_in_35_u_and_instability_sets_the_cracks`, `six_sub_charges_scatter_up_to_20_u_and_explode_0_5_s_later` pass |
| Decoy return = 99 % of remaining integrity, first ship on its path, teammates included | met | `decoy::tests::launched_decoy_snaps_back_and_hits_its_owner_for_99_percent`, `a_shot_up_decoy_hits_for_99_percent_of_what_is_left` pass |
| Replay determinism and the WASM golden-hash check | met | CI: `replay_golden_hash` native and wasm32 pass (hash unchanged by the wave C integration fixes); `node tools/determinism-check.mjs`: all 6 share codes match the native golden hashes |
| Bots fight on 10 random worlds (hits > 0 in each 60 s run) | met | `cargo test -p ls-bots --release`: `tests/nav_sweep.rs` 11 tests pass (soak world + 9 random worlds + the 5-minute match) |
| Steady-state full snapshots <= 1 per join in a 10-minute soak | met | `node tools/e2e.mjs ... --ruleset data/rulesets/sandbox_all_weapons.ron --count 8 --duration 600 --max-full-per-join 1`: 16 full snapshots for 16 joins (two matches), 143,168 deltas, 0 stale, all 23 checks pass (worst p99 2064 us) |
| e2e with 8 bots, mixed ship types, all weapons: 0 desyncs, 0 protocol errors, tick p99 < 4 ms with debris at cap | met on a quiet machine | `node tools/e2e.mjs ... --ruleset tools/e2e-rulesets/sandbox_debris.ron --count 8 --duration 330 --min-debris 64`: 0 desyncs, 0 protocol errors, debris at the 64 cap in 12 windows, worst p99 462 us (452 us in the windows at cap), 10 Tectonic Charges, 17 weapons fired, all 23 checks pass. The plain sandbox (`sandbox_all_weapons.ron`, same flags without `--min-debris`) passes with worst p99 438 us but peaks at 24-48 debris pieces. Runs while other builds or a file sync client loaded the machine failed the 4 ms check (up to 5.3 ms) with the time in the receive/send phases (`recv_max_us`, `send_max_us` up to 10 ms), i.e. the sim thread was descheduled; the game work itself (`Room::step`, in-process: p99 at most 345 us per 5 s window, one 4-7 ms tick per Tectonic collapse) fits |
| Rule set editor code accepted and enforced | met | `pwsh tools/browser-smoke.ps1` (CI): the editor in headless Chrome produces `LR6V-69CM-G200-C0AC-48` and a second ls-server starts with that `rules_code` (60 checks pass); `tests_arsenal::banned_weapons_never_spawn_or_fire`, `tests::banned_weapon_cannot_be_fired` / `_picked_up` pass |
| docs/ and userDocs/ describe every weapon, gadget, ship type and the editor as built | met in wave D (docs review) | `docs/architecture/arsenal-framework.md` covers the arsenal as built; `userDocs/player-manual.md` describes all 27 weapons, 3 gadgets, 4 ship types, the keys V/B, the HUD mechanics and the rule set editor; `userDocs/hosting-guide.md` covers rule set codes, the editor, bans and balance lists; the number tables in `userDocs/reference/` are generated from `data/` (`node tools/gen-weapon-docs.mjs --check` in CI). Commands, flags, log lines and example codes checked against `--help` and a running `ls-server` |

