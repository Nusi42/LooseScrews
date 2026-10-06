# Determinism

The server (x86_64) and the browser (wasm32) never exchange the level: both regenerate it
from the share code and then apply the same ordered terrain edits. Client prediction also
re-runs the server's movement code. Both only work if the deterministic crates produce
**bit-identical** results on every target. This page lists the rules, the tests that enforce
them, and what to do when you change generator or simulation output on purpose.

## Scope

| Crate | Deterministic | Notes |
| --- | --- | --- |
| `ls-math` | yes | Geometry, `det`, RNGs, terrain contracts |
| `ls-worldgen` | yes | SDF, chunk store, edits, paint, surface states, material field, meshers |
| `ls-sim` | yes | `World::step`, `simulate_movement` (incl. paint effects), every weapon and gadget module, entities and debris, rule sets and rule set codes |
| `ls-net` | encoding only | Integer bit packing; clients quantize inputs exactly as the server decodes them |
| `ls-game` | by construction | Pure logic on top of the above, no clocks or threads; match setup reads only the pristine level |
| `ls-server`, `ls-bots`, `ls-wasm`, `web/` | no | Hosts: wall-clock time, sockets, threads; `ls-server` seeds random worlds from the clock on purpose |

## Rules

These apply to `ls-math`, `ls-worldgen` and `ls-sim` (and to anything in `ls-game` that feeds
the simulation):

- **Floats:** `f32` with IEEE-exact operations only: `+ - * /`, `sqrt`, comparisons, conversions.
  Every transcendental goes through `ls_math::det` (`sin, cos, sin_cos, tan, asin, acos,
  atan2, sqrt, exp, ln, powf, floor, ceil, round, fmod`), which wraps the pure-Rust `libm`
  crate. Never call `f32::sin` and friends directly: their results depend on the platform's
  math library.
- **No fused multiply-add:** never `mul_add`; `Vec3`, `Quat`, `Mat3` use plain `+ - * /`.
  No fast-math, no `target-cpu` or float-reassociation flags (see the comment on
  `[profile.release]` in the root `Cargo.toml`).
- **Ordering:** never let `HashMap`/`HashSet` iteration order reach an output. Use `BTreeMap`,
  `BTreeSet` or sorted `Vec`s (the simulation keeps ships and per-tick commands in `BTreeMap`s keyed by player).
- **No environment:** no wall-clock time (time is a `Tick`), no threads, no OS or thread-local
  randomness, no file system in code that runs in WASM (data files are embedded with
  `include_str!`).
- **RNG:** `SplitMix64` to derive seeds, `Pcg32` streams per subsystem, each with its own
  stream id so a new draw in one place never shifts another. The ids in use are listed in
  [contracts.md](contracts.md#determinism). `ls-worldgen` uses no RNG stream at all: noise and
  features are integer hashes of the seed.
- **Edits on the wire** carry raw `f32` bits, not quantized values, so every peer applies the
  exact same CSG.
- **Data:** gameplay numbers come from `data/*.ron`, embedded at compile time, so the server,
  bots and browser build with the same values; the handshake's [data hash](#data-hash-and-behaviour-version)
  refuses a client built from other files.
- **Weapon and gadget modules** (`crates/ls-sim/src/weapons/`, `gadgets/`) follow the same
  rules and add three of their own: randomness only from `ctx.rng(key)` (a private PCG32
  stream per key and tick, keyed by the match seed; usually the projectile id as key, see
  `RNG_STREAM_MODULES` in [contracts.md](contracts.md#determinism)); no state outside the
  world (module structs are unit structs; per-projectile, per-entity and per-ship-weapon state
  lives in the hashed fields `Projectile::vars`, `Entity::vars` / `state`,
  `ShipState::weapon_state`); and changes only through queued commands, which the world
  applies in a fixed order ([hook order](arsenal-framework.md#hook-order-per-tick)).
- **Registries are append-only:** `weapons::registry::MODULES` and `gadgets::MODULES` order is
  part of the rule set code format, so codes decode the same on every build that knows the
  weapon.

Reviewers check these rules; the tests below catch what slips through.

## Golden hashes

### World generation (`ls-worldgen`)

`ls_worldgen::golden::golden_hashes(code)` condenses a world into four SHA-256 digests:

| Hash | What it covers |
| --- | --- |
| `sdf` | `Generator::sample` bits on an 8 u lattice (offset 0.37 u) over the level box plus one step, then the Heist site's bits (core, radii, access point, extraction zones; generator version 3), then the archetype, the gravity well sites and the belt site (slots, pickup points, boulder orbits; generator version 4) |
| `surface_nets` | the surface-nets mesh of a probe chunk (found by raycasting from an open point near the room centre; on belt levels near start slot 0) |
| `dual_contouring` | the dual-contouring mesh of the same chunk |
| `edited` | surface-nets meshes of every dirty chunk after a 3 u crater, a 1.5 u added rock and a 4 u team-1 paint splat |

The fixture `crates/ls-worldgen/tests/fixtures/golden_hashes.txt` holds the expected hex
digests for thirteen share codes (open sky, sky/cavern mix, caverns, cavern/mine mix, dense
mines, every slider at 100 (the tall box with gravity wells), the Heist world added with
generator version 3, and with version 4 one world for each other level shape, a default box
with seven gravity wells and two asteroid belts, one with shepherd moons). `tests/golden.rs`
checks them (`golden_list_covers_shapes_wells_and_belts` keeps the list covering every shape,
seven wells and both belts), and `tools/determinism-check.mjs` checks the same codes in the
WASM build:

```sh
cargo test -p ls-worldgen --release --test golden
```

A mismatch names the share code and which of the four hashes changed, which narrows the cause:
`sdf` means the generator, `surface_nets`/`dual_contouring` only the mesher, `edited` only the
edit and paint path (or the surface-nets mesher). A mesher-only change keeps `sdf`, so it needs
no `GENERATOR_VERSION` bump: re-bless the fixture (`LS_BLESS_GOLDEN=1`) and say why in the
commit. VQ3a (snapped vertices, continuous normals, baked ambient occlusion) did that for
`surface_nets` and `edited`; `Mesh::hash` covers the per-vertex occlusion when a mesh carries
it. The mesher's square roots use `f32::sqrt` (correctly rounded, the same bits as
`ls_math::det::sqrt`).

Generator version 4 (M4 A1) changed every level: the five level boxes and inner structure of
Verticality, gravity well sites (Gravity Chaos), the team 2/3 caverns, the upper hub layer of
tall levels, the level archetype field in the share code (sliders packed base-101, still 26
symbols) and the asteroid belt archetype. The belt's 8-fold rotation and plane mirror are
exact in the generator field to rounding: rotations by multiples of 45 degrees use exact
sines and cosines (0, 1, `sqrt(1/2)`) with the products summed in `f64`
(`ls_math::level::rotate_wedges`), and `belt::tests::field_has_eightfold_rotation_and_plane_mirror_symmetry`
holds the field at 10 000 points to 1e-4 u of its rotated and mirrored copies.

The rock material field (`ls_worldgen::material`) is not part of the SDF, so it is not in the
golden fixture: `material::tests::material_field_hash_is_pinned` pins it with a constant and
runs in the wasm32 unit tests too. Changing the field changes crater sizes (gameplay): update
the constant deliberately; no generator version bump is needed because share codes still
produce the same geometry. `Mesh::hash` covers surface states only when a vertex has one, and
not materials, so the golden meshes are unaffected by both layers.

### Simulation (`ls-sim`)

`World::state_hash()` is an FNV-1a hash over the exact bit patterns of the world state
(`crates/ls-sim/src/hash.rs`). `crates/ls-sim/src/tests.rs` replays a scripted 4-player match
(one ship of each type, debris in the arena) for 1800 ticks (firing with random weapon
selection, damage, craters, kills, respawns, pickups, gadget use) and compares the final hash
with `REPLAY_GOLDEN_HASH`:

```sh
cargo test -p ls-sim replay_golden_hash
```

Each mode also has a tailored replay golden next to its module (M3 wave B), a 60 s scripted
match that exercises its rules, checked on native and wasm32 (each test prints the new value
after a deliberate change):

| Rule set | Constant | Test module | Value |
| --- | --- | --- | --- |
| Team Brawl, Instagib Brawl, duel | `TEAM_BRAWL_REPLAY_GOLDEN`, `INSTAGIB_BRAWL_REPLAY_GOLDEN`, `DUEL_REPLAY_GOLDEN` | `modes::team_brawl::tests` (8 ships, kill target 255) | `0x4324_af1e_2f87_4773`, `0x61ed_c96b_b3c7_4e0a`, `0xbf16_09a8_44fe_9b5c` |
| The Squeeze | `SQUEEZE_REPLAY_GOLDEN_HASH` | `modes::the_squeeze::tests` (4 ships, all five phases closing) | `0xb7d4_e8da_9cd6_639a` |
| Heist | `HEIST_REPLAY_GOLDEN_HASH` | `modes::heist::tests` (8 bots in a box room with a vein: mining, collapse, carrying, deliveries) | `0x9aa1_4ec8_8a94_291b` |
| Hot Reactor | `HOT_REACTOR_REPLAY_GOLDEN_HASH` | `modes::hot_reactor::tests` | `0xa94b_6f50_7b53_d4f3` |
| Last Color Standing | `LCS_REPLAY_GOLDEN_HASH` | `modes::last_color_standing::tests` (stalemate shortened to 30 s) | `0x7bcd_a0fc_22ef_666a` |

When a behaviour change re-blesses `REPLAY_GOLDEN_HASH` without a `data/` change, also bump
`ls_sim::SIM_BEHAVIOR_VERSION` (see [below](#data-hash-and-behaviour-version)).

### Per-mode replay hashes

`ls_sim::golden` (M3, C3) plays one 40 s scripted match per built-in rule set (every mode,
Instagib Brawl, the duel, the sandbox): 8 ships with pseudo-random inputs aiming at the nearest
enemy, in a box arena with a rock knot and Heist extraction zones, under a `ShelledTerrain`
(The Squeeze's walls); The Squeeze's phases, the Last Color Standing stalemate and the reactor
fuse are shortened so their rules run inside the 40 s. The hashes (the per-second state hashes
folded with the final one) live in `crates/ls-sim/tests/fixtures/mode_replays.txt`:
`golden::tests::mode_replays_match_the_fixture` checks them on native and on wasm32, and
`ls-wasm` exports them (`modeReplayHashes`) so `tools/determinism-check.mjs` compares the
browser build under Node. They are separate from the tailored per-mode goldens in `modes/*`.
After a deliberate change of simulation behaviour, tuning or `data/` (any of them can change
these hashes), re-bless on native and say so in the commit:

```sh
LS_BLESS_MODE_REPLAYS=1 cargo test -p ls-sim --lib golden
# PowerShell: $env:LS_BLESS_MODE_REPLAYS=1; cargo test -p ls-sim --lib golden; Remove-Item Env:LS_BLESS_MODE_REPLAYS
```

The scripted match must keep exercising the systems: `replay_same_inputs_same_state_hash`
asserts that it fires, damages, craters, kills, respawns, uses gadgets, takes pickups and
switches paint colours. Its weapon selections are drawn from the weapon table's length, so
adding a weapon changes the match; if a system drops out, adjust `scripted_inputs` or the
seed, then re-bless.

Sibling tests prove the replay is repeatable, that a cloned world continues identically, and
that one changed input diverges the hash from that tick on.

### World physics replay hashes (M4)

`ls_sim::world_replays` (M4, A4) plays four more 40 s scripted 8-ship replays with the same
inputs, one per world physics system: `world/gravity` (Gravity Chaos 100, three static wells and
a dynamic well from 10 s to 20 s, the sandbox rule set so ordnance bends), `world/cave_in` (a
12 u high arena at Instability 100, ten Bolt Rockets each: cave-ins happen), `world/pigment`
(a monochrome level, odd players fly at the nearest pigment) and `world/radiation` (a small belt
with half the ships above it: radiation kills). Their hashes are the `world/` lines of
`crates/ls-sim/tests/fixtures/mood_replays.txt` (the director's event replays share the file),
checked on native and wasm32 by `world_replays::tests::world_replays_match_the_fixture`, which
also asserts each system acted. Re-bless only the `world/` lines:

```sh
LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib world_replays
```

The Architect (M4 B4) adds the line `mode/the_architect`: a 40 s, 6-ship match in an arena that
answers reshape strokes analytically (`ls_sim::AnalyticReshape`), 10 s rounds so all three
rounds and the match end happen inside it, a scripted stroke every 1.25 s cycling through all
eight kinds (some refused), half the runners flying at the exit
(`ls_sim::modes::the_architect::replay`). Checked on native and wasm32 by
`the_architect::tests::architect_replay_matches_the_fixture`; re-bless only that line with
`LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib the_architect`. The new mode also adds
`rulesets/the_architect.ron` to the per-mode replays (no strokes there); the other nine lines
and `REPLAY_GOLDEN_HASH` are unchanged (the Architect state is hashed only when present).

Every world physics system is inert when its input is off (no wells, Instability 0, no
monochrome level, no belt, world +Y up) and the state hash then skips its state, so
`REPLAY_GOLDEN_HASH` and the per-mode replays stay as they were, except Last Color Standing,
whose rule set now has pigment pickups (its mode replay and `LCS_REPLAY_GOLDEN_HASH`
re-blessed in A4).

### Milestone 3 record

What M3 changed deliberately (plan [§4](../milestone-3-plan.md#4-determinism-and-handshake-impacts)):

| Item | Change | Why |
| --- | --- | --- |
| `SIM_BEHAVIOR_VERSION` | 4 -> 5 (wave A) | mode framework, per-match teams, Tectonic cracks spread over 6 ticks; later waves changed no existing behaviour (new rule set files change the data hash anyway), except D1's free-for-all side reuse (a side id stays taken while a departed owner's shots live): not bumped again, because version 5 was never released and only the server assigns sides (clients never predict a join) |
| `REPLAY_GOLDEN_HASH` | unchanged (`0x8321_c0ab_902b_8ee5`) | the scripted replay fires no Tectonic Charge and plays two-team Paint the Void, which runs bit-identically through the framework; the crack change is pinned by `tectonic_charge::tests::cracks_spread_over_six_ticks_and_match_the_m2_set` |
| `GENERATOR_VERSION` | 2 -> 3 (B3) | the Heist site (core vein chamber, extraction caverns) in every level; worldgen goldens re-blessed, share codes re-encoded (`LS0C`-`LS0F`), a seventh (Heist) world in the golden list |
| Worldgen mesh goldens | re-blessed three times on version 3 without a version bump (VQ3a mesher, VQP thin bores, D5 material blend weights and fold-minimising quad split) | mesher changes only: `sdf` and share-code geometry unchanged |
| Per-mode replays | new (C3): `ls_sim::golden` and `crates/ls-sim/tests/fixtures/mode_replays.txt`, plus the module goldens above | one replay per built-in rule set on native, wasm32 and the shipped WASM package |
| The Squeeze | no generator change | the walls are a runtime shell layer (`ShelledTerrain`, `GameTerrain::set_shell`), outside worldgen goldens and `GameTerrain::state_hash` |

### Milestone 4 record

What M4 changed deliberately (plan [§4](../milestone-4-plan.md#4-determinism-and-handshake-impacts)); wave D (D3) completes it.

| Item | Change | Why |
| --- | --- | --- |
| `SIM_BEHAVIOR_VERSION` | 5 -> 6 (wave A, assigned by A3) | the comedy director as the last stage of `World::step`, plus wave A's gravity, Instability cave-ins, pigment, radiation and dynamic wells |
| `REPLAY_GOLDEN_HASH` and the nine mode replays | unchanged | the scripted replays run with `MatchSetup::absurdity` 0 (and Instability 0, no wells): the director stays off and its pristine state is not hashed (`director::tests::inert_director_leaves_the_hash_alone`) |
| Director state | hashed once it did anything (`DirectorState::is_pristine`) | mood, roll grid, gap, the running event's state and the forced schedule |
| Data hash | `data/events/*.ron` (18 events and `director.ron`), built-in rule set Absurdity 20 -> 100 (duel 50), `pigment_pickup` lines | embedded files change the hash automatically |
| Rule set codes | format 4 -> 5 (`LRA...`): event count in the header, pigment pickup, event bans | formats 2-4 decode against the M3 defaults (`rulecode::LEGACY_ABSURDITY` 20, no pigment, no bans); tested with M2 and M3 codes in Rust and the web editor (`legacyVectors` in the catalog) |

### Other pinned values

| Constant or fixture | Test | Changes when |
| --- | --- | --- |
| `material_field_hash_is_pinned` constant | `cargo test -p ls-worldgen --lib material` | the rock material field changes |
| `web/src/rules/catalog.ts` (generated) | `rulecode::tests::web_catalog_is_current` | weapon or gadget ids, names, slots, costs, levels, the registries or the default rule set change: `LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode` |
| Rule set codes quoted in docs and examples (the default is `LRAV-741Q-J900-01HQ-M`; the catalog holds the current one) | `rulecode::tests::web_catalog_is_current` | the code format, the default rule set, or the registries change: a new weapon or gadget changes every code, the default included (codes made by older builds still decode) |

### Data hash and behaviour version

`ls_sim::data_hash()` hashes every embedded `data/` file by path and content (weapons,
gadgets, ships, the built-in rule sets) plus `ls_sim::SIM_BEHAVIOR_VERSION` (currently 6).
`Hello` carries it and the server rejects a different hash with `DataMismatch`, so a browser
that cached an old build, or a bot built from other data, is refused with a clear message
instead of predicting with other numbers. Consequences:

- Any data edit (even a comment) changes the hash: rebuild the server, the bots and the web
  client together (`cargo build --release -p ls-server -p ls-bots`, `cd web && npm run build`).
- A behaviour change without a data change must bump `SIM_BEHAVIOR_VERSION`
  (`crates/ls-sim/src/data.rs`) in the same change as the new `REPLAY_GOLDEN_HASH`; the
  version is what tells old and new builds apart.
- A rule set is not part of the hash when it comes from a file or code at run time: the
  server sends its RON text in the `Welcome` and both sides resolve it against the same
  tables.

### Between peers at run time

`GameTerrain::state_hash()` (`ls-game`: edit count and lasting edits, SDF on a 4 u lattice,
painted and scorched cells; `state_hash_at(tick)` adds the timed surface states running at
`tick`) needs no full generation and is the terrain desync check. The `ls-game`
netplay tests and the `ls-bots` room test assert that every client's hash equals the server's;
the browser exposes it as `Game.terrainHash()` for debugging. Bots also count a prediction snap
without a respawn, or a terrain edit gap, as a desync (exit code 1).

## Re-blessing after an intentional change

**Generator output changed** (noise, features, SDF, meshers, edit application):

1. Bump `ls_worldgen::GENERATOR_VERSION` (`crates/ls-worldgen/src/lib.rs`). The version is part
   of every share code, so old codes are rejected with a clear error instead of silently
   building a different level.
2. Regenerate the fixture on native (it writes `tests/fixtures/golden_hashes.txt`):

   ```sh
   LS_BLESS_GOLDEN=1 cargo test -p ls-worldgen --release --test golden
   ```

   In PowerShell: `$env:LS_BLESS_GOLDEN=1; cargo test -p ls-worldgen --release --test golden;
   Remove-Item Env:LS_BLESS_GOLDEN`.
3. Run the golden test again without the variable, then the WASM cross-check (below) to prove
   the new hashes also hold on wasm32.
4. Update share codes that are hard-coded elsewhere (the e2e default code in `tools/e2e.mjs`,
   tests, docs) since their version field changes.

**Simulation behaviour, tuning or `data/` changed:** run the replay test
(`cargo test -p ls-sim replay_golden_hash`), copy the printed hash (`replay hash 0x...`) into
`REPLAY_GOLDEN_HASH` in `crates/ls-sim/src/tests.rs`, and say so in the commit. Any number in
`data/weapons`, `data/gadgets`, `data/ships` or `data/rulesets/paint_the_void.ron` changes it,
and so does a new weapon or gadget (ids shift). Without a data change, also bump
`SIM_BEHAVIOR_VERSION`. Then run the wasm32 unit tests (below) to prove the new hash holds on
wasm32 too.

| Change | Re-bless |
| --- | --- |
| Generator, SDF, mesher, edit application | `GENERATOR_VERSION`, worldgen golden fixture, hard-coded share codes |
| Rock material field | `material_field_hash_is_pinned` |
| Sim behaviour without data change | `REPLAY_GOLDEN_HASH`, the per-mode replay fixture (`LS_BLESS_MODE_REPLAYS=1`) and `SIM_BEHAVIOR_VERSION` |
| Data file (numbers, new weapon or gadget) | `REPLAY_GOLDEN_HASH`, the per-mode replay fixture; the web catalog if names, slots, costs, levels, registries or the default rule set changed |
| Wire format | `ls_net::PROTOCOL_VERSION` (not a hash, but the same idea: old clients are refused) |

Never re-bless to make an unexplained mismatch go away: first find out why the output changed.

## The WASM cross-check

Two independent checks run the same code on wasm32 under Node:

1. **Unit tests on wasm32.** The test modules opt in with
   `#[cfg(target_arch = "wasm32")] use wasm_bindgen_test::wasm_bindgen_test as test;`, so the
   worldgen golden test, the sim replay hash, the `det` golden bits and the RNG reference
   vectors run unchanged under `wasm-bindgen-test-runner`:

   ```sh
   # bash
   export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner
   cargo test --release -p ls-math -p ls-net -p ls-sim -p ls-worldgen --target wasm32-unknown-unknown
   cargo test --release -p ls-game --lib --target wasm32-unknown-unknown
   ```

   ```powershell
   # PowerShell
   $env:CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER = 'wasm-bindgen-test-runner'
   cargo test --release -p ls-math -p ls-net -p ls-sim -p ls-worldgen --target wasm32-unknown-unknown
   cargo test --release -p ls-game --lib --target wasm32-unknown-unknown
   ```

   Do not add `-q`: cargo forwards `--quiet` to `wasm-bindgen-test-runner`, which then fails.
2. **The shipped package against the native fixture.** `tools/determinism-check.mjs` loads
   the real browser build of `ls-wasm` in Node, calls `goldenHashes(code)` for every code in the
   native fixture and compares all four digests, then `modeReplayHashes()` against
   `crates/ls-sim/tests/fixtures/mode_replays.txt` (the [per-mode replays](#per-mode-replay-hashes)):

   ```sh
   cd web && npm run build:wasm && cd ..
   node tools/determinism-check.mjs            # ok / FAIL per code, exit 1 on any mismatch
   ```

   `--pkg <dir>` points it at another wasm-bindgen output directory (for example one you built
   with your own `CARGO_TARGET_DIR`), `--fixture <file>` at another worldgen fixture,
   `--mode-fixture <file>` at another mode replay fixture.

Both run in `tools/ci.ps1` / `tools/ci.sh`. `REPLAY_GOLDEN_HASH` and the tailored per-mode
goldens in `modes/*` are only covered by the first (`ls-wasm` does not export them); the
per-mode replay fixture by both.

## When a cross-check fails

- Only on wasm32: look for a direct transcendental call, `mul_add`, a platform-dependent
  conversion, or a crate dependency that does float math internally. `det::tests::golden_bits`
  failing points at `libm` itself (pin or patch it).
- On both targets after an unrelated change: something changed the output; either it is
  intentional (re-bless as above) or a bug (often iteration order or an RNG stream shared
  between subsystems).
- Only in live play (a bot reports a desync, a client's `terrainHash` differs): suspect edit
  ordering or replication (edit log, `edit_base`) rather than the generator; the netplay tests
  in `crates/ls-game/tests/netplay.rs` reproduce multi-client runs over lossy memory links.
