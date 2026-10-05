# Shared contracts

These types live in `crates/ls-math` so that crates developed in parallel (`ls-worldgen`,
`ls-sim`, `ls-net`, `ls-wasm`) agree on them without depending on each other. Changing any
of them is a cross-crate change: update this page, and every implementer and caller, in
the same change.

Overview of all crates: [overview.md](overview.md); determinism rules and golden hashes in
depth: [determinism.md](determinism.md).

## Coordinates, units and time

| Item | Convention |
| --- | --- |
| Handedness | Right-handed, `+Y` up |
| Ship / camera axes | Forward `-Z`, right `+X`, up `+Y` (`Quat::forward/right/up`) |
| Quaternion order | `a * b` = rotate by `b`, then by `a`; `Quat::rotate(v)` or `q * v` |
| Length | 1 u = one ship length (`f32`) |
| Time | `Tick(u64)` at `TICK_HZ = 60`; `DT = 1.0 / 60.0` (`f32`, bits `0x3c888889`) |
| Durations from data | seconds in RON, converted with `Tick::ticks_for_secs` |
| Level box | `ls_worldgen::Generator::bounds()`: `[0,256] x [0,128] x [0,256]` u at the default size, min corner at the origin (not centred); sealed by 2 u walls, outside counts as solid |
| Connection network | Generator version 2: every level carves 8 caverns (4 x 2 grid; the outer columns in the team spawn zones) and 9 + 3 tunnels of radius 4 u linking them, applied last as `max(terrain, carve)` (`ls_worldgen::network`); tunnel centre lines keep at least 3 u of clearance. `ls_game::setup` only places spawns and pickups with a straight 1.5 u (`CONNECT_MARGIN`) link to it |

## Determinism

The rules (floats, `ls_math::det`, no `mul_add`, ordering, no environment) and the golden-hash
tests are in [determinism.md](determinism.md). The contract part is the RNG streams:

- `SplitMix64` derives seeds; `Pcg32::new(seed, stream)` (reference PCG32) or
  `Pcg32::from_seed(seed, stream)` (seed scrambled first) gives each subsystem its own stream,
  so adding a draw in one place never shifts another.
- Stream ids in use:

  | Constant | Value | Use |
  | --- | --- | --- |
  | `ls_sim::world::RNG_STREAM_PICKUPS` | `0x5349_4d5f_5049_434b` ("SIM_PICK") | Weapon pickup rolls, seeded with `Pcg32::from_seed(match_seed, ..)` |
  | `ls_game::setup::RNG_STREAM_SPAWNS` | `0x4741_4d45_5f53_504e` ("GAME_SPN") | Team spawn point placement, `Pcg32::from_seed(world_seed, ..)` |
  | `ls_game::setup::RNG_STREAM_PICKUP_POINTS` | `0x4741_4d45_5f50_4b50` ("GAME_PKP") | Pickup point placement, `Pcg32::from_seed(world_seed, ..)` |
  | `ls_sim::modes::the_squeeze::RNG_STREAM_SQUEEZE` | `0x5349_4d5f_5351_5a45` ("SIM_SQZE") | The Squeeze: safe-zone centres, `Pcg32::new(mix64(match_seed ^ mix64(phase ^ stream)), ..)` per phase, and wall scrap, `Pcg32::new(mix64(match_seed ^ mix64(tick ^ stream.rotate_left(17))), ..)` per batch |
  | `ls_sim::behavior::RNG_STREAM_MODULES` | `0x5349_4d5f_4d4f_4453` ("SIM_MODS") | Weapon and gadget modules (`Ctx::rng(key)`): `Pcg32::new(mix64(match_seed ^ mix64(key ^ tick.rotate_left(32))), ..)`, a private stream per key and tick. Seed Grenade growth uses the same stream id keyed only by the match seed and the projectile id (no tick: `seed_grenade::growth`) |

  The simulation's `match_seed` (`WorldConfig::seed`) is
  `mix64(world_seed ^ ls_game::setup::MATCH_SEED_SALT)` with the salt `0x4d41_5443_485f_5345`
  ("MATCH_SE"). Spawn and pickup placement reads only the pristine generated level, so every
  peer derives the same `MatchSetup` from the share code and rule set.

  `ls-worldgen` draws no PCG streams: its noise and features are integer hashes of the seed,
  and the connection network (generator version 2, `ls_worldgen::network`) draws from a
  `SplitMix64` seeded by its own `Seeds::connect` value, appended last to `Seeds` so the
  earlier derived seeds are unchanged. The Heist site (generator version 3: core vein
  chamber, vein tunnels, extraction caverns; `Network::heist`) draws from a second
  `SplitMix64` seeded with `Seeds::connect ^ 0x4845_4953_545f_5349` ("HEIST_SI"), so the
  version 2 hubs and tunnels stay where they were. The Heist mode's collapsing mine draws
  from `Ctx::rng` (key `0x4845_4953_5443_4f4c`, "HEISTCOL").
  (`ls-bots` uses its own stream for bot behaviour; it is not part of the deterministic core.)
- `det::tests::golden_bits` and the RNG reference-vector tests run on native and on wasm32
  (commands in [determinism.md](determinism.md#the-wasm-cross-check)); they must stay
  identical on both.

## Terrain (`ls_math::terrain`)

### Sign convention

The signed distance field is **negative inside rock, positive in open space**; the surface
is the zero level set. `sdf` must be 1-Lipschitz (a true distance or a lower bound on it),
because the default raycast sphere-traces it.

### Terrain distance: bound vs. contact estimate

`sdf` (and `ls_sim::terrain::world_sdf`, which adds the solid level bounds) is a **lower
bound** on the distance, not the distance: the cave generator's noise fields report about
0.3 of it (noise divided by its worst-case slope `NOISE_LIPSCHITZ = 3.6`), while boxes,
planes and crater/add-rock spheres are exact. Code uses it in two ways:

- **Sphere tracing and "is it clear?" proofs** use the bound as is: stepping by it never
  crosses the surface, and `d >= r` proves a sphere of radius `r` clear. Projectile terrain
  hits are sphere-traced rays (exact at the hit; a projectile's radius only applies to ships
  and entities).
- **Sphere contact** (ships, debris, decoys: `ship::move_sphere_with`; paint touching;
  the near-miss scrape gap) uses the first-order estimate `d / |grad d|`
  (`ls_sim::terrain::surface_estimate`, `sphere_contact`, `sphere_contact_distance`), but
  only when the bound is below the test's reach (radius plus margin), so open flight costs one
  evaluation. The gradient is one central difference (`NORMAL_EPS`, 6 evaluations) that also
  gives the contact normal; the slope is clamped to `[MIN_CONTACT_SLOPE = 0.2, 1]` (never
  below the bound), and inside rock (`d <= 0`) the bound is used. Plain IEEE ops and `sqrt`
  only (deterministic, checked on wasm32 by `ship::tests::bound_field_contact_golden_hash`).
  A hull at contact sits within about 0.01 u of its radius from the true surface in cavern
  worlds (`crates/ls-game/tests/contact_fit.rs`).
- Bot navigation (`ls-bots`, the clearance grid in `nav.rs`) keeps the bound: conservative, bots stay out of
  gaps a human can now fly through.

A terrain that adds a new field must keep it 1-Lipschitz; it does not have to be exact.

### Shell layer (The Squeeze)

A mode can add a solid shell to the terrain: `ls_sim::ShellSphere { center, radius }`, rock
everywhere outside the sphere, field `radius - |p - center|` (exact, 1-Lipschitz, negative
in its rock). A terrain with a shell answers `sdf` as `min(terrain, shell)`, so ships,
projectiles, debris and raycasts treat it like rock.

- The world sets it at the start of every tick: `SimTerrain::set_shell(mode.terrain_shell(state,
  tick))` (`ls_sim::modes::ModeModule::terrain_shell`; `None` in modes without walls).
  `SimTerrain::set_shell` defaults to ignoring it; `ls_sim::ShelledTerrain<T>` is the reference
  wrapper (tests, tools) and `ls_game::GameTerrain` implements it for real levels. The Squeeze's
  own rules (crushing, swallowed ships) do not depend on the terrain implementing it.
- It is a runtime layer, not a `TerrainEdit`: not logged, not on the wire, not a generator
  change (worldgen golden hashes and share codes stay), and not in `GameTerrain::state_hash`
  (that hashes the chunk store), so peers compare terrain at any time.
- Clients derive it: the shell is a pure function of the replicated mode state and the tick
  (`ls_sim::modes::the_squeeze::ShellSchedule::shell_at` over the `SAFE_ZONE` / `NEXT_ZONE`
  objectives). Zone centres are whole units, so `ls_game::mode::squeeze_shell` snaps the
  quantized marker positions back and gets the server's shell bit for bit;
  `ClientSession::tick` sets it on its terrain for the predicted tick.
- While closing, the shell moves in steps of at most `SHELL_STEP` = 0.5 u of surface travel
  (centre shift plus radius change; quantized progress), so each step changes meshes only in a
  thin band. `GameTerrain::take_dirty_chunks` adds the chunks a shell change since its last call
  can change: those with corners near open space (terrain above -2.5 u, tracked per 8 u block)
  where one of the two shells is within 2.5 u of its surface; `GameTerrain::mesh_chunk` meshes
  a chunk the shell reaches from its padded samples combined with the shell (same surface-nets
  mesher, paint, states and materials). See `docs/milestone-3-plan.md` (B2 as built) for the
  measured remesh cost.

### `TerrainEdit`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TerrainEdit {
    SubtractSphere { center: Vec3, radius: f32 }, // crater
    AddSphere { center: Vec3, radius: f32 },      // add rock
    Paint { center: Vec3, radius: f32, team: u8, color: u8 }, // claim cells, no geometry change
    Surface { center: Vec3, radius: f32, states: u8, until: u32 }, // surface states
}
```

- Edits are applied **in tick order** and logged; late joiners replay the log. Two peers that
  apply the same log to the same share code must end up with identical terrain. The one
  exception is deliberate: a `Welcome` leaves out surface-state edits whose timed states
  expired before the join and can no longer matter (`ls_game::GameTerrain::welcome_log`, see
  [netcode.md](netcode.md#terrain-edit-replication)); the late joiner then has the same
  geometry, paint and scorch, and the same timed states from its join tick on, and counts
  the left-out edits so sequence numbers still agree. Desync hashes compare only that
  (`GameTerrain::state_hash`, time-independent; `state_hash_at(tick)` with the running timed
  states).
- Helpers: `center()`, `radius()`, `changes_geometry()`, `bounds()` (affected `Aabb`), and
  `apply_to_sdf(d, p)`, the reference CSG: subtract = `max(d, radius - |p - c|)`,
  add = `min(d, |p - c| - radius)`.
- `ls-worldgen` owns applying edits to chunks and returning dirty chunks; `ls-net` owns the
  wire encoding (serde derives are for RON/debugging, not the hot path).
- `radius` is in u. The design doc gives crater sizes as **diameters**, so weapon data
  stores `crater_radius = diameter / 2` (e.g. the 2 u Bolt Rocket crater is
  `crater_radius: 1.0`).
- `Surface` sets surface states on the surface cells whose centre is within `radius` (the
  query `GameTerrain::surface_states_near` instead counts every 1 u surface cell that
  overlaps the query sphere, so a ship scraping a wall finds the state of the cell under it).
  Timed states: for every timed bit in `states` (`ls_math::SurfaceStates`: `HOT` 1, `CHARGED`
  2, `GLOW` 4; `SurfaceStates::TIMED` = 7) the cell's expiry tick becomes `until` (absolute
  simulation tick; `0` clears). A timed state is active while `tick < until`; the cell's
  other states are kept. Default durations (design doc): hot 10 s, charged 15 s, glow 30 s
  (`SurfaceStates::DEFAULT_TICKS`, `TerrainEdit::surface_for_default(center, radius, state,
  now)`). The edit carries the expiry, not a duration, so peers that apply it late
  (snapshots, late-join replay) agree. Charged cells drain 5 energy/s from ships scraping
  them (simulation rule; the query is `GameTerrain::surface_states_near`).
- **Scorch** (`SurfaceStates::SCORCH` = 8) is permanent: a `Surface` edit with that bit
  scorches its cells for the rest of the match (`until = 0` clears it instead; only a crater
  that removes the surface also removes the mark). Queries report it next to the timed states
  that are running; meshes carry it in `states` (bit 8). Scorch trails and marks use it instead
  of long-lived `HOT` edits: `TerrainEdit::scorch(center, radius, now, hot_ticks)` scorches and
  heats for `hot_ticks` (0: scorch only, `until = u32::MAX`); `ls_sim::Ctx::scorch` skips
  already scorched spots. Other bits (16 and up) are ignored.
  `TerrainEdit::without_expired_states(now)` is the part of an edit that still matters once
  its timed states expired (the scorch bit, or nothing).
- `Paint.team` is the simulation team (0-based): 0-3 in team modes; in free-for-all modes
  (`teams: 0`) the painting ship's side id (`ShipState::team`, 0-253). `255`
  (`ls_worldgen::NO_TEAM`, `ls_sim::NO_PAINT`) means unpainted; a paint edit with team 255
  erases. The same ids appear in `PaintCoverage.team_cells` (teams 0-3), in mesh vertex
  `teams`, and in the web client's `MeshData.team`. A crater clears paint and surface states from cells that stop being
  surface.
- `Paint.color` is the effect colour stored with the team (`ls_math::terrain::PAINT_*`):
  `PAINT_PLAIN` 0 (no effect: generic weapon paint), `PAINT_RED` 1 (bouncy), `PAINT_BLUE` 2
  (slippery), `PAINT_YELLOW` 3 (sticky); other values are stored as plain
  (`paint_color`), and the wire carries 2 bits. Painting over replaces team and colour.
  Paint the Void scoring counts teams only. Ships react to the colour under them through
  `TerrainQuery::paint_colors_near` (M2 plan: red rebounds at 0.9 restitution, blue has no wall
  friction and keeps the speed, yellow holds ships to 40% of their speed caps; numbers in
  `data/ships/*.ron`, see [arsenal-framework.md](arsenal-framework.md#ship-types)).

### Terrain strength (`ls_worldgen::material`)

Rock has a material per region: `Material::Weak` (id 0), `Rock` (1), `Hard` (2), from a
Voronoi partition with one hashed site per 24 u cube (`MaterialField::new(world_seed)`; about
30 % weak, 45 % rock, 25 % hard). It is a pure function of the seed and position, separate
from the SDF (it does not change the generator golden hashes; a unit test pins it on native
and wasm32). The ids and multipliers are contract constants in `ls_math::terrain`
(`MATERIAL_WEAK`/`ROCK`/`HARD`, `material_crater_scale`: weak 1.4, rock 1.0, hard 0.6), so
weak rock breaks first and repeated fire wears through hard rock slowly. The simulation does
the scaling: `ls_sim::Ctx::crater` multiplies the radius by the scale of the material at the
centre (read through `SimTerrain::material_at`; `GameTerrain` answers from the field), and
`Ctx::crater_exact` does not (Tectonic Charge collapse zones keep the design-table size).
Every terrain applies and logs edits verbatim, `Event::TerrainEdit` carries the edit as
applied, and the log and the wire carry the scaled crater. Test terrains (`BoxRoomTerrain`,
...) are ordinary rock everywhere (the `SimTerrain::material_at` default).

A game mode may override the material in a region (`ModeModule::material_at`, read by the
world before the terrain's answer, from the replicated mode state, so every peer agrees):
Heist makes the core vein's knot hard rock and, for `collapse_secs` after the core is mined
free, the vein chamber region weak rock (`ls_sim::modes::heist`).

### `RayHit`

```rust
pub struct RayHit { pub point: Vec3, pub normal: Vec3, pub distance: f32 }
```

`normal` is unit length and points into open space; `distance` is measured along the
normalized ray direction.

### `TerrainQuery`

```rust
pub trait TerrainQuery {
    fn sdf(&self, p: Vec3) -> f32;
    fn normal(&self, p: Vec3) -> Vec3;                       // default: central differences
    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32)
        -> Option<RayHit>;                                    // default: sphere tracing
    fn bounds(&self) -> Aabb;                                 // outside counts as solid
    fn sphere_overlap(&self, center: Vec3, radius: f32) -> bool; // default: sdf(center) < radius
    fn paint_colors_near(&self, center: Vec3, radius: f32) -> u8; // default: 0 (no paint)
}
```

- `paint_colors_near` returns the paint colours (bit set `1 << PAINT_*`) of the painted
  surface cells overlapping the sphere; the ship flight model reads it
  (`ls_sim::simulate_movement`), so every terrain a ship flies through must answer it the same
  way on every peer (`ChunkStore`, `GameTerrain` and `ls_sim::PaintedTerrain` do; test
  terrains without paint report none).

- Only `sdf` and `bounds` are required; implementers may override the rest with faster exact
  versions as long as results stay deterministic.
- `raycast` normalizes `dir` (a zero direction never hits), searches `t` in `[0, max_dist]`,
  and returns distance 0 if the origin is already inside rock.
- The trait is object safe (`&dyn TerrainQuery` works).
- Tuning constants: `NORMAL_EPS = 1/512`, `HIT_EPS = 1/1024`, `MAX_TRACE_STEPS = 512`.
  Free functions `sdf_gradient` and `sphere_trace` are exported for reuse.

### Test terrains

| Type | Shape | Use |
| --- | --- | --- |
| `PlaneTerrain` | Rock below a plane (`PlaneTerrain::floor(y, bounds)`), analytic raycast | Simple collision and flight tests |
| `BoxRoomTerrain` | Open box room (`BoxRoomTerrain::centered(size)`), rock outside, edits applied with `apply(edit)` | Wall collisions, craters, `AddSphere` obstacles, paint logging (`paint_log()`) |

The real implementation (`ls-worldgen`'s chunk store) implements the same trait, so
`ls-sim` code written against the test terrains runs unchanged against generated worlds.
`ls_game::GameTerrain` wraps it and also implements `ls_sim::SimTerrain` (`apply_edit` plus
`paint_coverage`); it logs every edit it is handed, so the log index is the edit's sequence
number on the wire.

## Chunk meshes (`ls-worldgen` -> `ls-wasm` -> web client)

`ls_worldgen::Mesh` holds `positions: Vec<[f32; 3]>`, `normals: Vec<[f32; 3]>`,
`indices: Vec<u32>`, `teams: Vec<u8>` (one paint id per vertex, see `Paint.team`),
`colors: Vec<u8>` (paint colour id per vertex, `PAINT_*`, 0 where unpainted),
`states: Vec<u8>` (surface state bits per vertex: timed states active at the chunk store's
display tick, `GameTerrain::advance_to`, plus scorch 8) and `materials: Vec<u8>` (material
id per vertex; filled by the `ChunkStore` meshers) and `ao: Vec<u8>` (baked ambient occlusion
per vertex, 255 = open, 0 = fully occluded; surface nets only, empty for dual contouring).
`Mesh::hash` covers colours and states only when a vertex has one, and occlusion when the mesh
carries it.
Surface-nets vertices lie on the collision surface (`ChunkStore::eval`): each mass point is
snapped by one Newton step (tricubic grid interpolation, the exact field at creases); the
normals are the smooth grid gradient (continuous across cells and chunks). Shared border
vertices carry bit-identical positions, normals and occlusion in both chunks. A chunk's
`SampleGrid` holds corner values with a 4-corner halo (`GRID_HALO`) read from the neighbouring
chunks; meshing a chunk generates its in-level neighbours.
Vertices are in world space and faces are counter-clockwise seen from open space. `ls-wasm`
hands the client a `MeshData` (`web/src/renderer/mesh.ts`) by flattening only:
`positions` and `normals` as `Float32Array` (xyz triples), `indices` as `Uint32Array`, and
`teams` as the `Uint8Array` field `team` with values unchanged; `states` travel as the
optional `surface` field, `materials` and `colors` through `ChunkMesh.materials()` and
`ChunkMesh.paintColors()` (unknown colour ids become plain), occlusion through `ChunkMesh.ao()`
as the optional `MeshData.ao` (`Uint8Array`, one byte per vertex; the terrain shader multiplies
its ambient light by it, VQ3b). The web client packs the first ones into
one `u32` terrain vertex attribute (`web/src/renderer/shaders.ts`): bits 0-7 paint team (255
unpainted), bits 8-15 surface states, bits 16-23 material + 1 (0 unknown), bits 24-31 paint
colour; without the optional layers the packing is the M1 one. Coloured paint draws a
per-effect pattern (red stripes, blue waves, yellow dots) in a dark shade of the team colour,
so neither the effect nor the owner relies on hue alone. The client renders with back-face
culling and `frontFace: "ccw"`.

## Share codes

`LS` followed by exactly 26 Crockford base32 symbols (130 bits: version u8, seed u64,
7 sliders x 7 bit, CRC-8, 1 pad bit), printed as `LSab-cdef-ghij-klmn-opqr-stuv-wxyz`
(34 characters). Decoding ignores case, dashes and whitespace and reads O as 0 and I/L as 1.
`ls_worldgen::ShareCode` is the reference. The web start screen checks the format in
TypeScript (`web/src/ui/shareCode.ts`) and then decodes the code, checksum and generator
version through `ls-wasm`'s `checkShareCode`.

## Game data ids (`ls-sim`)

Every peer derives these from the same embedded data (the data hash in the handshake makes
sure it is the same data); details in [arsenal-framework.md](arsenal-framework.md).

| Id | Definition |
| --- | --- |
| `WeaponId` | Index in the weapon table sorted by weapon id (27 built-in weapons: `borrowed_time` 0 ... `tectonic_charge` 26); travels as projectile `kind`, ship flag bits 10-14, `InputCmd::weapon_select - 1` |
| Weapon registry order | `weapons::registry::MODULES`, design-doc order; rule set codes index it (append only) |
| `GadgetId` | Index in the gadget table sorted by id (`cloak` 0, `decoy` 1, `shield` 2); registry order (`shield`, `cloak`, `decoy`) is the rule set code order |
| `ShipType::index` | Skitter 0, Tinker 1, Hauler 2, Tug 3 (wire, rule set codes) |
| `VisualId` | Index in `VisualTable::new(weapons, gadgets)`: 8 built-ins, then 2 per weapon (projectile, pickup), then 1 per gadget |
| Entity wire id | `entity id & 0x7fff`, unique among live entities (the allocator skips ids in use) |
| `ReplicatedKind` | Entity `kind` byte: Debris 0, WeaponPickup 1, UpgradePickup 2, GadgetPickup 3, ScrapPile 4, Wreck 5, Decoy 6, Ghost 7, DustCloud 8, Marker 9 |
| `CueKind` | Cue `kind` byte (`ls_sim::behavior::CueKind`) |
| Data hash | `ls_sim::data_hash()`: FNV-1a over all embedded `data/` files sorted by path (path, 0, length, bytes without CR) |

## Network quantization (`ls-net`)

`QuantConfig::default()` covers the default level box plus a 2 u margin
(`[-2,258] x [-2,130] x [-2,258]` u), ship and entity velocities up to 64 u/s per axis and
projectiles up to 320 u/s (Railspike 270 u/s). The server fills `Welcome.quant` with
`ls_game::convert::quant_config_for(Generator::bounds(), weapons)` (bounds plus 2 u) so
clients always use the real level box. How game state maps onto the wire types (ids, tick
numbering, `ack_input`, status scaling, scores, edit delivery) is specified in
[game-glue.md](game-glue.md#wire-conventions).
`SnapshotHistory::default()` holds 128 snapshots (256 ticks at 30 Hz), covering
`MAX_BASELINE_AGE = 255`; server and client must use the same capacity.
