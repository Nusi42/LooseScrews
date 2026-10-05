# Browser renderer

The WebGPU renderer in `web/src/renderer/` (visual-quality-plan.md VQ2, VQ6 and VQ8). Terrain and
objects come from the game (`game/chunks.ts`, `game/scene.ts`); this page covers what happens
from there to the screen. Wave D of M3 extends it into the full renderer reference.

## Frame

| Step | Target | Code |
| --- | --- | --- |
| Lights (compute, skipped without lights): one invocation per froxel of a 16x9x24 grid, every light's view-space sphere tested against the froxel's box, one 128-bit mask per froxel | storage buffer (3456 x 16 bytes) | `lights.ts`, `shaders/lights.ts` |
| Scene: terrain chunks (frustum culled per chunk AABB), then opaque instances, then translucent instances (no depth write) | `rgba16float` HDR colour, MSAA 4x at "high"/"medium", resolved into the post chain's HDR texture; `depth24plus` | `renderer.ts`, `shaders/terrain.ts`, `shaders/objects.ts` |
| Bloom down: 13-tap filter, the first step with a Karis average and a soft-knee threshold | up to 6 half-size `rgba16float` mips | `post.ts`, `shaders/post.ts` `BLOOM_DOWN_WGSL` |
| Bloom up: 3x3 tent, added onto the next larger mip | the same mips | `BLOOM_UP_WGSL` |
| Composite: bloom (averaged over the mips, added), exposure, colour grade, vignette, AgX tonemap with a look, sRGB encode, half-LSB dither, optional film grain | the swapchain (`getPreferredCanvasFormat()`, non-sRGB, so the shader encodes) | `COMPOSITE_WGSL` |

With `timestamp-query` every pass writes timestamps (`gpuTimer.ts`), labelled `terrain` and
`objects` (or `scene` when `Renderer.detailedTiming` is off) and `post`; later packages add
`particles`, `lights` and `sky`. Budgets per label are in `web/src/gallery/scenes.ts`
(`PASS_BUDGETS_MS`); the F3 overlay and the visual gallery show the times
([dev-guide.md](../dev-guide.md#visual-gallery-and-gpu-budget)).

## Colour and units

- The scene writes **linear HDR radiance**. Nothing clamps to 1; the tonemap does the rolloff.
- Colours are **authored as display (sRGB) values**: `RenderSettings` (fog, rock, team colours),
  instance colours from `game/scene.ts`, literals in the shaders. The renderer converts them to
  linear (`srgbToLinear` for the frame uniforms, `srgb_to_linear` in WGSL) before lighting.
- **Emissive** light is in physical HDR units and blooms above `bloomThreshold` (scene radiance
  1.0). Instances map their emissive amount `a` (0..1) to `colour x (1 + 5a²)` (`emissiveGain`,
  peak `EMISSIVE_PEAK` = 6 at a = 1: bolt cores and flashes bloom, a 0.1 hull glow barely
  shows). Terrain: hot rock about 3, charged sparks 5, glow 1.2, paint 0.3-0.4 times its colour.
- Lighting (`shaders/lighting.ts`, `light(n, p)`): sun 0.9, hemisphere ambient 0.035-0.11, the
  camera headlight (a soft spot light, see below) and the dynamic point lights. Model materials
  call `light_spec(n, p, v, power, &spec)`, which also returns the dynamic lights' Blinn-Phong
  highlight (`models/shading.ts` scales it per material: metal and glass glint, paint less).

## Dynamic lights

Up to `RenderQuality.maxLights` point lights per frame (high 128, medium 32, low 16), clustered
forward (VQ6). The game hands the renderer a list every frame (`Renderer.setLights`); the
renderer adds hot-rock lights from the chunk meshes, keeps the in-view lights with the best
score (`lights.ts` `selectLights`: intensity, range and distance to the camera, objectives with
a priority), uploads them and builds the froxel masks in a compute pass timed as `lights`. The
terrain and object shaders find their fragment's froxel from its clip position
(`cluster_index`: 16 x 9 screen tiles, 24 exponential depth slices from 0.5 to 240 u) and loop
over the set bits only.

| Piece | File |
| --- | --- |
| `PointLight` (position, display colour, `intensity`, `range`, `priority`), selection, packing, froxel maths (CPU mirrors of the WGSL, unit tested), hot spots, `LightSystem` (buffers, compute pipeline) | `renderer/lights.ts` |
| Froxel builder (compute), the `PointLight` / `LightInfo` structs, grid constants | `renderer/shaders/lights.ts` |
| `light`, `light_spec`, `dynamic_lights`, the headlight spot; bindings 1-3 of group 0 (light info uniform, lights, froxel masks) next to the Frame uniform | `renderer/shaders/lighting.ts` |
| What lights up in the game: attached lights (`sceneLights`) and timed pulses (`LightFx`) | `game/lightFx.ts` |

Units: `intensity` is the light's HDR radiance multiplier 1 u from it (the sun is 0.9);
falloff `intensity / (1 + d^2)`, windowed to 0 at `range` by `(1 - (d / range)^4)^2`, so a
froxel miss never cuts light off visibly. Without a range, `lightRange` puts it where the light
drops to 0.02 (at most 40 u). Diffuse is slightly wrapped (lights grazing a wall still tint
it). Colours are display values like every other authored colour. Typical intensities: bolt 7,
orb or arc 12, engine 4 idle / 12 boosting, muzzle flash 12, explosion 90 (white-hot cooling to
orange, growing), mine or charge blast 160, ship death 130, reactor 14 pulsing.

`game/lightFx.ts`:

- `sceneLights(input)`: lights rebuilt every frame from the state: glowing projectiles by shape
  (`SHAPE_LIGHTS`: carried intensity and range, a fixed colour for rocket motors, the impact
  pulse, the muzzle pulse), engines behind every living ship (none while cloaked), armed mines
  blinking, pickups, wreck embers, returning decoys, Leech Arc beams, the mechanics flashes
  (muzzle flashes, shockwaves, swaps; `FLASH_LIGHT_PER_RADIUS`) and mode objectives (reactor
  pulsing faster as it nears detonation, core, extraction; zones give none).
- `LightFx`: timed pulses, the **emitter API** for effect systems: `emit(preset or LightPulse,
  position, now, { color, scale })`; `LIGHT_PRESETS` (muzzle, spark, plasma, explosion,
  bigExplosion, shipDeath, pickup) with attack, decay ("fade", "flash", "flicker"), growth and a
  cooling end colour; `frame(now)` returns the live pulses' lights (at most 96 alive).
  `observe(shots, visuals, now)` emits a muzzle pulse for a projectile that appeared and an
  impact pulse for one that vanished (matched by owner, visual and predicted position), so
  every weapon lights the cave without per-weapon code; `observeShips` emits the death blast.
  An effect system that emits its own impacts sets `watchProjectiles = false`.

`main.ts` calls `observe`, `observeShips`, `sceneLights` and `frame` each frame and passes the
result to `Renderer.setLights`; the gallery does the same for its synthetic scenes.

Hot rock: `LightSystem.setChunk` gathers the vertices of each chunk with a glowing surface
state (hot, charged, glow) into 6 u cells and puts one light at each cell's centroid, 0.6 u off
the rock, brighter for bigger patches; hot rock breathes, charged rock crackles (`hotSpotAt`).

Headlight: a soft spot along the view (`HEADLIGHT_SPOT`: full inside 14 degrees, fading to 30 %
at 40 degrees, distance falloff `1 / (1 + 0.0025 d^2)`), strength `RenderSettings.headlight`.

Cost (gallery, RTX 5080, 1920x1080, high): 12 lights (`light-volley`) +0.01-0.02 ms; 64 lights
in one cave tunnel (`light-storm`, most froxels hold 15-30 lights) about +0.07 ms (terrain
+0.06, objects +0.01, compute 0.01). `node tools/browser-smoke/gallery.mjs --lights off`
renders the same frames without lights for the comparison.

## Post settings and the grade hook

`Renderer.settings.post` (`PostSettings`, defaults `DEFAULT_POST`): `exposure` 1.4,
`bloomStrength` 1.5, `bloomThreshold` 1.0, `vignette` 0.35, `grain` 0 (off), AgX look
`agxPower` 1.3 and `agxSaturation` 1.25, and `grade` (`ColorGrade`): ASC CDL slope, offset and
power plus saturation in scene-linear light before the tonemap, and a monochrome blend that can
keep one hue (`monoTint`, `monoKeep`: saturated colours of that hue survive). `GRADE_PRESETS`
has `neutral` (the default), mild `caves`, `mines` and `sky` starting points for VQ7, and
`lastColor` (monochrome) for C1's Last Color Standing, which sets `monoTint` to the colour to
keep. Change a grade by assigning `renderer.settings.post.grade`; it applies on the next frame.
The gallery previews one with `--grade <preset>` (`?grade=`).

## Quality presets

`quality.ts`: one `RenderQuality` object per level; later packages add their switches to it.

| Level | MSAA | Bloom mips | Lights | Notes |
| --- | --- | --- | --- | --- |
| high | 4x | 6 | 128 | default |
| medium | 4x | 4 | 32 | WebGPU has no 2x MSAA (the plan's medium 2x); medium saves on bloom, lights and, later, particles and shafts |
| low | 1x | 0 | 16 | still HDR + tonemap (rgba16float render targets are core WebGPU), one composite pass |

`?quality=low|medium|high` forces a level. Otherwise the renderer starts at "high" and, once it
has 5 s of GPU-timed frames, steps down once if the median GPU frame exceeds 8 ms ("medium") or
14 ms ("low"). Without `timestamp-query` it stays at "high" (frame intervals would also count
CPU work such as the initial terrain load). Scene pipelines are built per MSAA sample count when
a level first needs them.

## Shader files

`shaders/frame.ts` (the `Frame` uniform block, shared by every scene pipeline, the terrain
attribute bit layout, sRGB helpers), `shaders/lighting.ts`, `shaders/terrain.ts`,
`shaders/objects.ts` (instanced and translucent), `shaders/lights.ts` (the light froxel
builder), `shaders/post.ts`. Each package edits its own
file (VQ3b terrain, VQ5 particles, VQ6 lighting, VQ7 sky); keep `FRAME_UNIFORM_FLOATS` and
`Renderer.writeFrameUniforms` in sync with the `Frame` struct.

The object shaders include `models/shading.ts` (VQ4): `shade_model` picks a material by the
mesh vertex's part (`models/kit.ts` `PART_*`, uploaded as the mesh's `team` buffer at vertex
buffer slot 3, location 6); instance floats 14 and 15 carry the engine glow (0 idle .. 1 boost)
and the nav-light blink phase. `PART_PLAIN` (untagged meshes) keeps the generic `shade` look.
Material colour literals are display values linearised at build time (`lin(...)`); the engine,
nav-light and lamp strengths are HDR radiance constants in `models/shading.ts` (VQ6 retuned
them for HDR: engines 2.6 idle and 7.5 boosting, nav lights 1.4 steady plus a 7.0 flash, lamps
4.5), so idle engines and steady nav lights bloom softly and boost and the nav flash hard.
