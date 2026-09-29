# Shared contracts

These types live in `crates/ls-math` so that crates developed in parallel (`ls-worldgen`,
`ls-sim`, `ls-net`, `ls-wasm`) agree on them without depending on each other. Changing any
of them is a cross-crate change: update this page, and every implementer and caller, in
the same change.

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

## Determinism

- Transcendentals only through `ls_math::det` (`sin, cos, sin_cos, tan, asin, acos, atan2,
  sqrt, exp, ln, powf, floor, ceil, round, fmod`), which wraps `libm`.
- No `mul_add`. `Vec3`, `Quat`, `Mat3` use plain `+ - * /` only.
- Randomness: `SplitMix64` to derive seeds, `Pcg32::new(seed, stream)` (reference PCG32) or
  `Pcg32::from_seed(seed, stream)` (seed scrambled first) per subsystem. Give each subsystem
  its own stream id so adding a draw in one place never shifts another.
- Stream ids in use:

  | Constant | Value | Use |
  | --- | --- | --- |
  | `ls_sim::world::RNG_STREAM_PICKUPS` | `0x5349_4d5f_5049_434b` ("SIM_PICK") | Weapon pickup rolls, seeded with `Pcg32::from_seed(match_seed, ..)` |
  | `ls_game::setup::RNG_STREAM_SPAWNS` | `0x4741_4d45_5f53_504e` ("GAME_SPN") | Team spawn point placement, `Pcg32::from_seed(world_seed, ..)` |
  | `ls_game::setup::RNG_STREAM_PICKUP_POINTS` | `0x4741_4d45_5f50_4b50` ("GAME_PKP") | Pickup point placement, `Pcg32::from_seed(world_seed, ..)` |

  The simulation's `match_seed` (`WorldConfig::seed`) is
  `mix64(world_seed ^ ls_game::setup::MATCH_SEED_SALT)` with the salt `0x4d41_5443_485f_5345`
  ("MATCH_SE"). Spawn and pickup placement reads only the pristine generated level, so every
  peer derives the same `MatchSetup` from the share code and rule set.

  `ls-worldgen` draws no RNG streams: its noise and features are integer hashes of the seed.
- `det::tests::golden_bits` and the RNG reference-vector tests run on native and on wasm32
  (see `CLAUDE.md` for the command); they must stay identical on both.

## Terrain (`ls_math::terrain`)

### Sign convention

The signed distance field is **negative inside rock, positive in open space**; the surface
is the zero level set. `sdf` must be 1-Lipschitz (a true distance or a lower bound on it),
because the default raycast sphere-traces it.

### `TerrainEdit`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TerrainEdit {
    SubtractSphere { center: Vec3, radius: f32 }, // crater
    AddSphere { center: Vec3, radius: f32 },      // add rock
    Paint { center: Vec3, radius: f32, team: u8 },// paint surface cells, no geometry change
}
```

- Edits are applied **in tick order** and logged; late joiners replay the log. Two peers that
  apply the same log to the same share code must end up with identical terrain.
- Helpers: `center()`, `radius()`, `changes_geometry()`, `bounds()` (affected `Aabb`), and
  `apply_to_sdf(d, p)`, the reference CSG: subtract = `max(d, radius - |p - c|)`,
  add = `min(d, |p - c| - radius)`.
- `ls-worldgen` owns applying edits to chunks and returning dirty chunks; `ls-net` owns the
  wire encoding (serde derives are for RON/debugging, not the hot path).
- `radius` is in u. The design doc gives crater sizes as **diameters**, so weapon data
  stores `crater_radius = diameter / 2` (e.g. the 2 u Bolt Rocket crater is
  `crater_radius: 1.0`).
- `Paint.team` is the simulation team (0-based). `255` (`ls_worldgen::NO_TEAM`,
  `ls_sim::NO_PAINT`) means unpainted; a paint edit with team 255 erases. The same ids
  appear in `PaintCoverage.team_cells`, in mesh vertex `teams`, and in the web client's
  `MeshData.team`. A crater clears paint from cells that stop being surface.

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
}
```

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
`indices: Vec<u32>` and `teams: Vec<u8>` (one paint id per vertex, see `Paint.team`).
Vertices are in world space and faces are counter-clockwise seen from open space. `ls-wasm`
hands the client a `MeshData` (`web/src/renderer/mesh.ts`) by flattening only:
`positions` and `normals` as `Float32Array` (xyz triples), `indices` as `Uint32Array`, and
`teams` as the `Uint8Array` field `team` with values unchanged. The client renders with
back-face culling and `frontFace: "ccw"`.

## Share codes

`LS` followed by exactly 26 Crockford base32 symbols (130 bits: version u8, seed u64,
7 sliders x 7 bit, CRC-8, 1 pad bit), printed as `LSab-cdef-ghij-klmn-opqr-stuv-wxyz`
(34 characters). Decoding ignores case, dashes and whitespace and reads O as 0 and I/L as 1.
`ls_worldgen::ShareCode` is the reference; the web start screen only checks this format.

## Network quantization (`ls-net`)

`QuantConfig::default()` covers the default level box plus a 2 u margin
(`[-2,258] x [-2,130] x [-2,258]` u). The server fills `Welcome.quant` with
`ls_game::convert::quant_config_for(Generator::bounds(), weapons)` (bounds plus 2 u) so
clients always use the real level box. How game state maps onto the wire types (ids, tick
numbering, `ack_input`, status scaling, scores, edit delivery) is specified in
[game-glue.md](game-glue.md#wire-conventions).
`SnapshotHistory::default()` holds 128 snapshots (256 ticks at 30 Hz), covering
`MAX_BASELINE_AGE = 255`; server and client must use the same capacity.
