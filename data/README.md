# Game data

All gameplay numbers live here as [RON](https://github.com/ron-rs/ron) files, not in code.
The schemas are owned by `crates/ls-sim` (the Rust types it deserializes into are the
reference); change the types and the files together. The player-facing number tables in
`userDocs/reference/` are generated from these files: run `node tools/gen-weapon-docs.mjs`
after a change (CI fails when they are stale).

| Path | Contents |
| --- | --- |
| `weapons/*.ron` | One file per weapon of the design doc (27; schema `ls_sim::WeaponDef`): speed, lifetime, damage, fire interval, cost (energy / scrap / count), splash, knockback, terrain effect (crater radius, paint radius), shield interaction (`Blocked` / `Wears(f)` / `PassThrough(f)`), generic pattern (`projectiles`, `spread_deg`), `charge`, `visual`, and free `params` read by the weapon's module. |
| `ships/*.ron` | One file per ship type (`skitter`, `tinker`, `hauler`, `tug`; schema `ls_sim::ShipTuning`): radius, per-axis cruise/boost caps, accelerations, turn rates, horizon assist, hull/energy/boost/scrap maxima and spawn values, regen and refill rates, scavenging, overdrive cost, damage multiplier, secondary slots, wreck scrap, scrap passing (Tug). |
| `gadgets/*.ron` | One file per gadget (`shield`, `cloak`, `decoy`; schema `ls_sim::GadgetDef`): activation (`Toggle` / `Instant`), energy cost, drain per second, moving drain multiplier, cooldown, duration, visual, `params`. |
| `rulesets/*.ron` | One default rule set per mode (schema `ls_sim::RuleSet`, v2): weapon and gadget availability (`LoadoutAndPickup` / `PickupOnly` / `LoadoutOnly` / `Banned`), starting loadout (primaries, secondaries with counts, gadget), weapon / upgrade / gadget pickups (weight, respawn, ammo, `max_on_map`), scrap pickup, shield overrides and tuning multipliers per weapon, friendly fire percent (0 = off, otherwise 10..=100), allowed ship types, absurdity, mutators. |

The files are embedded into `ls-sim` with `include_str!` (weapons by
`crates/ls-sim/src/weapons/registry.rs`, gadgets by `crates/ls-sim/src/gadgets/mod.rs`, ships by
`crates/ls-sim/src/tuning.rs`), so the WASM client needs no file system. Their hash
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
