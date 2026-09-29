# Game data

All gameplay numbers live here as [RON](https://github.com/ron-rs/ron) files, not in code.
The schemas are owned by `crates/ls-sim` (the Rust types it deserializes into are the
reference); change the types and the files together.

| Path | Contents |
| --- | --- |
| `weapons/*.ron` | One file per weapon: speed, lifetime, damage, fire interval, cost (energy / scrap / count), splash, knockback, terrain effect (crater radius, paint radius), shield interaction. |
| `rulesets/*.ron` | One default rule set per mode: per-weapon availability (`LoadoutAndPickup` / `PickupOnly` / `LoadoutOnly` / `Banned`), starting loadout, pickup spawn weights and respawn times, optional scrap pickup (`scrap_pickup`), friendly fire percent (0 = off, otherwise 10..=100), allowed ship types, absurdity. |
| `ships/*.ron` | One file per ship type (`ls_sim::ShipTuning`): radius, per-axis cruise/boost caps, accelerations, turn rates, horizon assist, hull/energy/boost/scrap maxima and spawn values, regen and refill rates, scavenging, overdrive cost, damage multiplier, secondary slots, wreck scrap. M1 has only `tinker.ron`. |

The files are embedded into `ls-sim` with `include_str!`, so the WASM client needs no file
system. Changing any number changes the sim replay golden hash
(`REPLAY_GOLDEN_HASH` in `crates/ls-sim/src/tests.rs`); update it in the same change.

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
`docs/implementation-plan.md` section 4.3 for the full field list.
