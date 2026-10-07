# Game data

All gameplay numbers live here as [RON](https://github.com/ron-rs/ron) files, not in code.
The schemas are owned by `crates/ls-sim` (the Rust types it deserializes into are the
reference); change the types and the files together. The player-facing number tables in
`userDocs/reference/` are generated from these files: run `node tools/gen-weapon-docs.mjs`
after a change (CI fails when they are stale).

| Path | Contents |
| --- | --- |
| `weapons/*.ron` | One file per weapon (28: the design doc's 27 and the Paperweight of M4; schema `ls_sim::WeaponDef`): speed, lifetime, damage, fire interval, cost (energy / scrap / count), splash, knockback, terrain effect (crater radius, paint radius), shield interaction (`Blocked` / `Wears(f)` / `PassThrough(f)`), generic pattern (`projectiles`, `spread_deg`), `charge`, `gravity` (M4: wells bend its projectiles; ordnance and the Seed Grenade), `visual`, and free `params` read by the weapon's module. |
| `ships/*.ron` | One file per ship type (`skitter`, `tinker`, `hauler`, `tug`; schema `ls_sim::ShipTuning`): radius, per-axis cruise/boost caps, accelerations, turn rates, horizon assist, hull/energy/boost/scrap maxima and spawn values, regen and refill rates, scavenging, overdrive cost, damage multiplier, secondary slots, wreck scrap, scrap passing (Tug), `mass` (M4: Belt Up's bumps). |
| `gadgets/*.ron` | One file per gadget (`shield`, `cloak`, `decoy`; schema `ls_sim::GadgetDef`): activation (`Toggle` / `Instant`), energy cost, drain per second, moving drain multiplier, cooldown, duration, visual, `params`. |
| `world/*.ron` | World physics of M4 (`docs/architecture/contracts.md` "World physics"): `gravity.ron` (well force law, slider mapping, the 40 % ship clamp; schema `ls_sim::gravity::GravitySettings`), `instability.ron` (crater growth, strain, cave-in shape and limits; `ls_sim::instability::InstabilitySettings`), `pigment.ron` (pigment duration, damage bonus, splash, Last Color Standing's colour restore; the pickup counts are the rule set's `pigment_pickup`; `ls_sim::pigment::PigmentSettings`), `radiation.ron` (the belt dose law; `ls_sim::radiation::RadiationSettings`), `boulders.ron` (drifting boulders on belts: contact damage and bounce; `ls_sim::obstacle::BoulderSettings`). |
| `events/*.ron` | The comedy director (M4, `docs/architecture/moods-and-world.md`): `director.ron` (cadence, announce time, moods; schema `ls_sim::director::DirectorDef`) and one file per event (18; `ls_sim::events::EventDef`: name, `enabled`, `weight`, `active_secs`, `ending_secs`, `moods`, `targeted`, free `params` read by the event's module). |
| `rulesets/*.ron` | One default rule set per mode (schema `ls_sim::RuleSet`, v2): weapon and gadget availability (`LoadoutAndPickup` / `PickupOnly` / `LoadoutOnly` / `Banned`), starting loadout (primaries, secondaries with counts, gadget), weapon / upgrade / gadget pickups (weight, respawn, ammo, `max_on_map`), scrap pickup, shield overrides and tuning multipliers per weapon, friendly fire percent (0 = off, otherwise 10..=100), allowed ship types, absurdity (the comedy events' cap), mutators, `events` (event bans), `pigment_pickup` (M4). |

The files are embedded into `ls-sim` with `include_str!` (weapons by
`crates/ls-sim/src/weapons/registry.rs`, gadgets by `crates/ls-sim/src/gadgets/mod.rs`, ships by
`crates/ls-sim/src/tuning.rs`, events by `crates/ls-sim/src/events/mod.rs` and `director/mod.rs`, world files by `crates/ls-sim/src/data.rs`), so the WASM client needs no file system. Their hash
(`ls_sim::data_hash`) is checked in the handshake, so clients and servers must be built from the
same files. Changing any number changes the sim replay golden hash (`REPLAY_GOLDEN_HASH` in
`crates/ls-sim/src/tests.rs`); update it in the same change. How weapon modules use these files:
`docs/architecture/arsenal-framework.md`.

Conventions:
- `crater_radius` and `paint_radius` are radii in u. The design doc gives crater sizes as
  diameters, so store `radius = diameter / 2`.
- Knockback is the level-1 direct-hit velocity change; like damage it scales with weapon
  level and the ship's `damage_mult`.
- Paint team ids in `TerrainEdit::Paint` are the simulation team (0-based); 255 means
  unpainted / erase (see `docs/architecture/contracts.md`).

Units follow the design doc: 1 u = one ship length, speeds in u/s, durations in seconds
(the simulation converts them to 60 Hz ticks with `ls_math::Tick::ticks_for_secs`).

The server loads a rule set with `--ruleset data/rulesets/<mode>.ron`; see
`docs/dev-guide.md` sections 8 (adding a weapon) and 9 (adding a rule set) for the full
field lists.
