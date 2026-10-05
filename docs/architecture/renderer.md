# Browser renderer

The WebGPU renderer in `web/src/renderer/` (visual-quality-plan.md VQ2, VQ7 and VQ8). Terrain and
objects come from the game (`game/chunks.ts`, `game/scene.ts`); this page covers what happens
from there to the screen. Wave D of M3 extends it into the full renderer reference.

## Frame

| Step | Target | Code |
| --- | --- | --- |
| Scene: terrain chunks (frustum culled per chunk AABB), then opaque instances, then translucent instances (no depth write) | `rgba16float` HDR colour, MSAA 4x at "high"/"medium", resolved into the post chain's HDR texture; `depth24plus` | `renderer.ts`, `shaders/terrain.ts`, `shaders/objects.ts` |
| Sky (open levels only): drawn *under* the resolved scene, `out = scene + sky x (1 - alpha)` (geometry writes alpha 1, the clear is transparent black) | the HDR scene texture, single-sampled | `sky.ts`, `shaders/sky.ts` `SKY_WGSL` |
| Light shafts ("high", open levels, sun on or near the screen): sky mask (scene alpha 0) at quarter resolution, radial blur toward the sun, added onto the scene | the HDR scene texture | `sky.ts` `encodeShafts`, `SHAFT_*_WGSL` |
| Bloom down: 13-tap filter, the first step with a Karis average and a soft-knee threshold | up to 6 half-size `rgba16float` mips | `post.ts`, `shaders/post.ts` `BLOOM_DOWN_WGSL` |
| Bloom up: 3x3 tent, added onto the next larger mip | the same mips | `BLOOM_UP_WGSL` |
| Composite: bloom (averaged over the mips, added), exposure, colour grade, vignette, AgX tonemap with a look, sRGB encode, half-LSB dither, optional film grain | the swapchain (`getPreferredCanvasFormat()`, non-sRGB, so the shader encodes) | `COMPOSITE_WGSL` |

With `timestamp-query` every pass writes timestamps (`gpuTimer.ts`), labelled `terrain` and
`objects` (or `scene` when `Renderer.detailedTiming` is off), `sky` (the sky and the shafts)
and `post`; later packages add `particles` and `lights`. Budgets per label are in `web/src/gallery/scenes.ts`
(`PASS_BUDGETS_MS`); the F3 overlay and the visual gallery show the times
([dev-guide.md](../dev-guide.md#visual-gallery-and-gpu-budget)).

## Colour and units

- The scene writes **linear HDR radiance**. Nothing clamps to 1; the tonemap does the rolloff.
- Colours are **authored as display (sRGB) values**: `RenderSettings` (fog, rock, team colours),
  instance colours from `game/scene.ts`, literals in the shaders. The renderer converts them to
  linear (`srgbToLinear` for the frame uniforms, `srgb_to_linear` in WGSL) before lighting.
- **Emissive** light is in physical HDR units and blooms above `bloomThreshold` (scene radiance
  1.0). Instances map their emissive amount `a` (0..1) to `colour x (1 + 5aÂ²)` (`emissiveGain`,
  peak `EMISSIVE_PEAK` = 6 at a = 1: bolt cores and flashes bloom, a 0.1 hull glow barely
  shows). Terrain: hot rock about 3, charged sparks 5, glow 1.2, paint 0.3-0.4 times its colour.
- Lighting (`shaders/lighting.ts`, `light(n, p)`): sun 0.9, hemisphere ambient 0.035-0.11, the
  camera headlight. VQ6 adds dynamic lights there.

## Sky, fog and per-world look (VQ7)

`atmosphere.ts` derives everything from the world's share code (`atmosphereFor(code, bounds)`,
decoded in TS: `decodeShareCode`, same layout and CRC as ls-worldgen): the world type weights
from Enclosure (sky / cave / mine, blended like the generator's openness fields), the colour
grade (`GRADE_PRESETS` caves, mines and sky blended by those weights; the Palette slider lowers
the saturation toward the design doc's ink look), the fog (density, haze colours, height fog)
and, from the seed, the sky: sun direction (28-58° up; also the terrain's sun light), sun
colour, one of six nebula palettes, the galactic band, and a planet (gas giant with optional
rings, ocean world or dry/icy world, 8-15° across, 75-145° from the sun). The game calls
`Renderer.setAtmosphere(atmosphereFor(game.shareCode, game.bounds()))` at the start of a match
and on every match transition (`main.ts`, the gallery); `null` restores the defaults. It must
come before the world's chunks: in open levels `setChunk` hides the level box's sealing walls
(`stripRoomWalls`: triangles whose three vertices lie on a room face, or its rounded edges,
with the normal facing into the room). Collision is unchanged; a faint blue grid shows the
boundary within about 9 u of the camera.

`sky.ts` (`Sky`) owns the GPU side:
- the **environment cube** (512² per face, full mip chain, `rgba16float`), baked once per world
  by `ENV_BAKE_WGSL`: haze gradient (zenith, horizon, nadir), the sun's wide halo, the nebula
  (domain-warped gradient noise, dust lanes, bright cores, concentrated in the galactic band)
  and unresolved star dust. Every mip is evaluated directly with fewer octaves, so high mips
  are a smooth sky;
- the **planet surface** texture (1024 x 512 equirectangular, `rgba8unorm-srgb`), baked with
  the cube, so the planet costs one texture read per pixel;
- the **sky pass**: the cube, two procedural star layers (one candidate per cube-face cell,
  Gaussian footprint about a pixel wide, denser along the band), the sun disc with limb
  darkening and corona (HDR, blooms), the planet (lit by the sun, soft terminator, faint night
  side, atmosphere rim and limb glow, rings with the planet's shadow; skipped outside its cone),
  and the boundary grid (only computed within `GRID_RANGE` of a wall);
- the **light shafts** (`RenderQuality.shafts`, "high" only).

**Fog** (`shaders/frame.ts` `apply_fog`): the exp2-squared distance fog plus an exponential
height fog (`Frame.fog_height`: density at a world height and falloff per u, integrated along
the view ray), coloured by the environment cube's mip `FOG_ENV_LOD` (16² per face) in the view
direction: distant rock fades into the sky behind it, tinted toward the sun and the nebula.
`RenderSettings.skyHaze` (0..1, `Frame.fog_height.w`) blends between that haze and the flat
`fogColor`; `main.ts` lowers it inside dust clouds, which keep their flat dust colour. The
cube is bound to every scene pipeline at group 0, bindings 1 (cube texture) and 2 (sampler).

Per world type: open sky (Enclosure 0) has no height fog, density 0.0085 and a near-black haze
tinted by the nebula; caverns (50) a warm dark haze, density 0.012 and a low-lying height fog;
mine shafts (100) a sodium-orange haze, density 0.014. Worlds in between blend these; any
Enclosure below 50 shows the sky.

## Post settings and the grade hook

`Renderer.settings.post` (`PostSettings`, defaults `DEFAULT_POST`): `exposure` 1.4,
`bloomStrength` 1.5, `bloomThreshold` 1.0, `vignette` 0.35, `grain` 0 (off), AgX look
`agxPower` 1.3 and `agxSaturation` 1.25, and `grade` (`ColorGrade`): ASC CDL slope, offset and
power plus saturation in scene-linear light before the tonemap, and a monochrome blend that can
keep one hue (`monoTint`, `monoKeep`: saturated colours of that hue survive). `GRADE_PRESETS`
has `neutral` (the default), mild `caves`, `mines` and `sky` starting points for VQ7, and
`lastColor` (monochrome) for C1's Last Color Standing, which sets `monoTint` to the colour to
keep. Change a grade by assigning `renderer.settings.post.grade`; it applies on the next frame.
`setAtmosphere` sets the world's grade (VQ7, see above), so C1's monochrome must be applied after
it (or on top of `renderer.atmosphere.grade`). The gallery previews a preset with `--grade
<preset>` (`?grade=`).

## Quality presets

`quality.ts`: one `RenderQuality` object per level; later packages add their switches to it.

| Level | MSAA | Bloom mips | Light shafts | Notes |
| --- | --- | --- | --- | --- |
| high | 4x | 6 | yes | default |
| medium | 4x | 4 | no | WebGPU has no 2x MSAA (the plan's medium 2x); medium saves on bloom, shafts and, later, particles and lights |
| low | 1x | 0 | no | still HDR + tonemap (rgba16float render targets are core WebGPU), one composite pass |

`?quality=low|medium|high` forces a level. Otherwise the renderer starts at "high" and, once it
has 5 s of GPU-timed frames, steps down once if the median GPU frame exceeds 8 ms ("medium") or
14 ms ("low"). Without `timestamp-query` it stays at "high" (frame intervals would also count
CPU work such as the initial terrain load). Scene pipelines are built per MSAA sample count when
a level first needs them.

## Shader files

`shaders/frame.ts` (the `Frame` uniform block, shared by every scene pipeline, the terrain
attribute bit layout, sRGB helpers), `shaders/lighting.ts`, `shaders/terrain.ts`,
`shaders/objects.ts` (instanced and translucent), `shaders/post.ts`, `shaders/sky.ts` (VQ7: the
environment and planet bakes, the sky pass, the shafts, gradient noise). Each package edits its
own file (VQ3b terrain, VQ5 particles, VQ6 lighting, VQ7 sky); keep `FRAME_UNIFORM_FLOATS` and
`Renderer.writeFrameUniforms` in sync with the `Frame` struct (`sky.test.ts` checks the sizes of
every uniform struct against its TS constant). A new scene pipeline uses the renderer's frame
bind group layout (uniform, environment cube, sampler), so `apply_fog` works in it.

The object shaders include `models/shading.ts` (VQ4): `shade_model` picks a material by the
mesh vertex's part (`models/kit.ts` `PART_*`, uploaded as the mesh's `team` buffer at vertex
buffer slot 3, location 6); instance floats 14 and 15 carry the engine glow (0 idle .. 1 boost)
and the nav-light blink phase. `PART_PLAIN` (untagged meshes) keeps the generic `shade` look.
Material colour literals are display values linearised at build time (`lin(...)`); the engine,
nav-light and lamp strengths are HDR radiance constants in `models/shading.ts`.
