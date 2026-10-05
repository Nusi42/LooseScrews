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
| `sdf` | `Generator::sample` bits on an 8 u lattice (offset 0.37 u) over the level box plus one step, then the Heist site's bits (core, radii, access point, extraction zones; generator version 3) |
| `surface_nets` | the surface-nets mesh of a probe chunk (found by raycasting from an open point near the room centre) |
| `dual_contouring` | the dual-contouring mesh of the same chunk |
| `edited` | surface-nets meshes of every dirty chunk after a 3 u crater, a 1.5 u added rock and a 4 u team-1 paint splat |

The fixture `crates/ls-worldgen/tests/fixtures/golden_hashes.txt` holds the expected hex
digests for seven share codes (open sky, sky/cavern mix, caverns, cavern/mine mix, dense mines,
every slider at 100, and the Heist world added with generator version 3). `tests/golden.rs`
checks them:

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

Modes add their own replay goldens next to their module (wave B): Heist's
`modes::heist::tests::heist_replay_golden_hash` (`HEIST_REPLAY_GOLDEN_HASH`) runs 8 scripted
bots for 60 s on a box room with a vein (mining with rockets, the collapse, carrying,
deliveries) and checks the same hash on native and wasm32; the test prints the new value
after a deliberate change.

When a behaviour change re-blesses `REPLAY_GOLDEN_HASH` without a `data/` change, also bump
`ls_sim::SIM_BEHAVIOR_VERSION` (see [below](#data-hash-and-behaviour-version)).

The scripted match must keep exercising the systems: `replay_same_inputs_same_state_hash`
asserts that it fires, damages, craters, kills, respawns, uses gadgets, takes pickups and
switches paint colours. Its weapon selections are drawn from the weapon table's length, so
adding a weapon changes the match; if a system drops out, adjust `scripted_inputs` or the
seed, then re-bless.

Sibling tests prove the replay is repeatable, that a cloned world continues identically, and
that one changed input diverges the hash from that tick on.

### Other pinned values

| Constant or fixture | Test | Changes when |
| --- | --- | --- |
| `material_field_hash_is_pinned` constant | `cargo test -p ls-worldgen --lib material` | the rock material field changes |
| `web/src/rules/catalog.ts` (generated) | `rulecode::tests::web_catalog_is_current` | weapon or gadget ids, names, slots, costs, levels, the registries or the default rule set change: `LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode` |
| Rule set codes quoted in docs and examples (the default is `LR8V-6097-G000-TMBG`; the catalog holds the current one) | `rulecode::tests::web_catalog_is_current` | the code format, the default rule set, or the registries change: a new weapon or gadget changes every code, the default included (codes made by older builds still decode) |

### Data hash and behaviour version

`ls_sim::data_hash()` hashes every embedded `data/` file by path and content (weapons,
gadgets, ships, the built-in rule sets) plus `ls_sim::SIM_BEHAVIOR_VERSION` (currently 5).
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
| Sim behaviour without data change | `REPLAY_GOLDEN_HASH` and `SIM_BEHAVIOR_VERSION` |
| Data file (numbers, new weapon or gadget) | `REPLAY_GOLDEN_HASH`; the web catalog if names, slots, costs, levels, registries or the default rule set changed |
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
   native fixture and compares all four digests:

   ```sh
   cd web && npm run build:wasm && cd ..
   node tools/determinism-check.mjs            # ok / FAIL per code, exit 1 on any mismatch
   ```

   `--pkg <dir>` points it at another wasm-bindgen output directory (for example one you built
   with your own `CARGO_TARGET_DIR`), `--fixture <file>` at another fixture.

Both run in `tools/ci.ps1` / `tools/ci.sh`. The sim replay hash is only covered by the first
(`ls-wasm` does not export it).

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
