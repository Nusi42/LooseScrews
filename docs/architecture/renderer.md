# Browser renderer

The WebGPU renderer in `web/src/renderer/` (visual-quality-plan.md VQ2, VQ5 and VQ8). Terrain
and objects come from the game (`game/chunks.ts`, `game/scene.ts`), particles from the effect
library (`game/vfx.ts`); this page covers what happens from there to the screen. Wave D of M3
extends it into the full renderer reference.

## Frame

| Step | Target | Code |
| --- | --- | --- |
| Scene: terrain chunks (frustum culled per chunk AABB), then opaque instances, then translucent instances (no depth write) | `rgba16float` HDR colour, MSAA 4x at "high"/"medium", resolved into the post chain's HDR texture; `depth24plus` (stored for the particles) | `renderer.ts`, `shaders/terrain.ts`, `shaders/objects.ts` |
| Particles: sprites at least `halfResPx` wide into a half-resolution `rgba16float` target, composited onto the resolved scene; then dust motes and the smaller sprites at full resolution. No depth attachment: each fragment reads the scene depth (soft particles, occlusion) | the resolved HDR scene texture | `particles.ts`, `shaders/particles.ts`, `spriteAtlas.ts` |
| Bloom down: 13-tap filter, the first step with a Karis average and a soft-knee threshold | up to 6 half-size `rgba16float` mips | `post.ts`, `shaders/post.ts` `BLOOM_DOWN_WGSL` |
| Bloom up: 3x3 tent, added onto the next larger mip | the same mips | `BLOOM_UP_WGSL` |
| Composite: bloom (averaged over the mips, added), exposure, colour grade, vignette, AgX tonemap with a look, sRGB encode, half-LSB dither, optional film grain | the swapchain (`getPreferredCanvasFormat()`, non-sRGB, so the shader encodes) | `COMPOSITE_WGSL` |

With `timestamp-query` every pass writes timestamps (`gpuTimer.ts`), labelled `terrain` and
`objects` (or `scene` when `Renderer.detailedTiming` is off), `particles` and `post`; later
packages add `lights` and `sky`. Budgets per label are in `web/src/gallery/scenes.ts`
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
  camera headlight. VQ6 adds dynamic lights there.

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

## Particles (VQ5)

`particles.ts` `ParticleSystem` (`Renderer.particles`, null with `?vfx=off` or
`Renderer.create(canvas, { particles: false })`, which keeps the pre-VQ5 mesh effects):

- **Storage**: one buffer of 96-byte particles (six vec4: spawn position and time, velocity
  and lifetime, start and end colour with opacity, start and end size, drag, flags; stretch,
  spin, rotation, fade-in). Two oldest-first rings (`ParticleRing`): particles living up to
  0.7 s, and longer-lived ones (30 % of the slots), so a flood of sparks never evicts smoke;
  then up to 4096 **immediate** sprites rebuilt every frame (projectile cores, engine flames,
  beams, soft glows). The CPU writes a particle once, at spawn, and uploads only the slots
  written since the last frame.
- **Motion is analytic** (position and velocity at any age from the spawn state and an
  exponential drag), so there is no update pass; dead and unborn slots collapse to a degenerate
  quad in the vertex shader. Spawns may be in the future (`t0` > now: smoke after fire).
- **Sprites**: `spriteAtlas.ts` bakes an `rg8unorm` texture array at start-up (128², full mip
  chain): glow, spark, four smoke puffs, flame, ring, star flare, rock/hull shard, streak.
  Channel r is coverage, g brightness or heat. Orientation per particle: camera-facing
  billboard (spin), stretched along the velocity, stretched along a direction to a fixed length
  (trail and beam sections, dimmed when seen end-on), or lying in a plane (shield ripples);
  stretched sprites seen along their axis turn into billboards.
- **Blending**: one premultiplied "over" state; additive light writes alpha 0, smoke, dust and
  shards write their coverage, flames are partly opaque (0.45 of their coverage) so fireballs
  keep their billows. No sorting.
- **Colours** are linear HDR radiance (`game/vfxLibrary.ts` `lin()` converts display colours):
  sparks and cores 4-10, fireballs about 2.6 at the start cooling to deep red, smoke 0.01-0.08.
  Fog applies per particle; sprites closer than their size fade out.
- **Dust motes**: 1500 at "high", generated from the instance index in a 22 u cube that wraps
  around the camera (motion parallax), streaked by the camera's smoothed velocity.
- **Cost** (`particles-stress` gallery scene, 21 900 live particles: an explosion every frame,
  120 bolts with trails, four boosting hulls; RTX 5080, 1920x1080, "high"): 0.13-0.19 ms. A
  smoke-only worst case (15 000 large smoke puffs) 0.40 ms with the half-resolution split,
  1.12 ms without it.

## Quality presets

`quality.ts`: one `RenderQuality` object per level; later packages add their switches to it.

| Level | MSAA | Bloom mips | Particles (ring, spawn density, motes, half-res above) | Notes |
| --- | --- | --- | --- | --- |
| high | 4x | 6 | 32 768, 1, 1500, 32 px | default |
| medium | 4x | 4 | 16 384, 0.6, 800, 20 px | WebGPU has no 2x MSAA (the plan's medium 2x); medium saves on bloom, particles and, later, lights and shafts |
| low | 1x | 0 | 8 192, 0.35, none, 12 px | still HDR + tonemap (rgba16float render targets are core WebGPU), one composite pass |

`?quality=low|medium|high` forces a level. Otherwise the renderer starts at "high" and, once it
has 5 s of GPU-timed frames, steps down once if the median GPU frame exceeds 8 ms ("medium") or
14 ms ("low"). Without `timestamp-query` it stays at "high" (frame intervals would also count
CPU work such as the initial terrain load). Scene pipelines are built per MSAA sample count when
a level first needs them.

## Shader files

`shaders/frame.ts` (the `Frame` uniform block, shared by every scene pipeline, the terrain
attribute bit layout, sRGB helpers), `shaders/lighting.ts`, `shaders/terrain.ts`,
`shaders/objects.ts` (instanced and translucent), `shaders/particles.ts` (particles, motes,
half-resolution composite), `shaders/post.ts`. Each package edits its own file (VQ3b terrain,
VQ6 lighting, VQ7 sky); keep `FRAME_UNIFORM_FLOATS` and
`Renderer.writeFrameUniforms` in sync with the `Frame` struct.

The object shaders include `models/shading.ts` (VQ4): `shade_model` picks a material by the
mesh vertex's part (`models/kit.ts` `PART_*`, uploaded as the mesh's `team` buffer at vertex
buffer slot 3, location 6); instance floats 14 and 15 carry the engine glow (0 idle .. 1 boost)
and the nav-light blink phase. `PART_PLAIN` (untagged meshes) keeps the generic `shade` look.
Material colour literals are display values linearised at build time (`lin(...)`); the engine,
nav-light and lamp strengths are HDR radiance constants in `models/shading.ts`.
