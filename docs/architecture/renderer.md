# Browser renderer

The WebGPU renderer in `web/src/renderer/` (visual-quality-plan.md VQ2, VQ5, VQ6, VQ7 and
VQ8; M3 C1's mode effects). Terrain and objects come from the game (`game/chunks.ts`, `game/scene.ts`), particles from
the effect library (`game/vfx.ts`); this page covers what happens from there to the screen.
Wave D of M3 extends it into the full renderer reference.

## Frame

| Step | Target | Code |
| --- | --- | --- |
| Lights (compute, skipped without lights): one invocation per froxel of a 16x9x24 grid, every light's view-space sphere tested against the froxel's box, one 128-bit mask per froxel | storage buffer (3456 x 16 bytes) | `lights.ts`, `shaders/lights.ts` |
| Scene: terrain chunks (frustum culled per chunk AABB), then opaque instances, then translucent instances (no depth write), then the cockpit hull frame (own depth range) | `rgba16float` HDR colour, MSAA 4x at "high"/"medium", resolved into the post chain's HDR texture; `depth24plus` (stored for the particles) | `renderer.ts`, `shaders/terrain.ts`, `shaders/objects.ts`, `hullFrame.ts` |
| Sky (open levels only): drawn *under* the resolved scene, `out = scene + sky x (1 - alpha)` (geometry writes alpha 1, the clear is transparent black) | the HDR scene texture, single-sampled | `sky.ts`, `shaders/sky.ts` `SKY_WGSL` |
| Light shafts ("high", open levels, sun on or near the screen): sky mask (scene alpha 0) at quarter resolution, radial blur toward the sun, added onto the scene | the HDR scene texture | `sky.ts` `encodeShafts`, `SHAFT_*_WGSL` |
| Mode effects (M3 C1; skipped without volumes): one fullscreen triangle, additive, reading the scene depth: The Squeeze's glowing walls and next zone, zones, beams and halos | the resolved HDR scene texture | `modeFx.ts`, `shaders/modeFx.ts` |
| Particles: sprites at least `halfResPx` wide into a half-resolution `rgba16float` target, composited onto the resolved scene; then dust motes and the smaller sprites at full resolution. No depth attachment: each fragment reads the scene depth (soft particles, occlusion) | the resolved HDR scene texture | `particles.ts`, `shaders/particles.ts`, `spriteAtlas.ts` |
| Bloom down: 13-tap filter, the first step with a Karis average and a soft-knee threshold | up to 6 half-size `rgba16float` mips | `post.ts`, `shaders/post.ts` `BLOOM_DOWN_WGSL` |
| Bloom up: 3x3 tent, added onto the next larger mip | the same mips | `BLOOM_UP_WGSL` |
| Composite: bloom (averaged over the mips, added), exposure, colour grade, vignette, AgX tonemap with a look, sRGB encode, half-LSB dither, optional film grain | the swapchain (`getPreferredCanvasFormat()`, non-sRGB, so the shader encodes) | `COMPOSITE_WGSL` |

With `timestamp-query` every pass writes timestamps (`gpuTimer.ts`), labelled `terrain` and
`objects` (or `scene` when `Renderer.detailedTiming` is off), `lights`, `sky` (the sky and the
shafts), `mode`, `particles` and `post`. Budgets per label are in `web/src/gallery/scenes.ts`
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
  shows). Terrain: hot rock seams about 4.5 (crust 0.15), charged sparks 7, glow 0.9-1.4, paint
  0.3-0.4 times its colour.
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

`main.ts` calls `sceneLights` and `frame` each frame and passes the result to
`Renderer.setLights`; the gallery does the same for its synthetic scenes. With VQ5's effects on
(the default) the pulses come from the effect library: `Vfx.lightSink` is the `LightFx`, and
`Vfx` emits a muzzle pulse per muzzle flash, `spark` or `plasma` per impact, `explosion` /
`bigExplosion` per blast and `shipDeath` per death, so `watchProjectiles` is off and
`observe` / `observeShips` only run with `?vfx=off`. Mechanics flashes (shockwaves and the
like) stay with `sceneLights`.

Hot rock: `LightSystem.setChunk` gathers the vertices of each chunk with a glowing surface
state (hot, charged, glow) into 6 u cells and puts one light at each cell's centroid, 0.6 u off
the rock, brighter for bigger patches; hot rock breathes, charged rock crackles (`hotSpotAt`).

Headlight: a soft spot along the view (`HEADLIGHT_SPOT`: full inside 14 degrees, fading to 30 %
at 40 degrees, distance falloff `1 / (1 + 0.0025 d^2)`), strength `RenderSettings.headlight`.

Cost (gallery, RTX 5080, 1920x1080, high): 12 lights (`light-volley`) +0.01-0.02 ms; 64 lights
in one cave tunnel (`light-storm`, most froxels hold 15-30 lights) about +0.07 ms (terrain
+0.06, objects +0.01, compute 0.01). `node tools/browser-smoke/gallery.mjs --lights off`
renders the same frames without lights for the comparison.

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
cube is bound to every scene pipeline at group 0, bindings 6 (cube texture) and 7 (sampler;
bindings 1-3 are VQ6's lights).

Per world type: open sky (Enclosure 0) has no height fog, density 0.0085 and a near-black haze
tinted by the nebula; caverns (50) a warm dark haze, density 0.012 and a low-lying height fog;
mine shafts (100) a sodium-orange haze, density 0.014. Worlds in between blend these; any
Enclosure below 50 shows the sky.

## Camera, cockpit frame and fit ring (VQ1)

- **Field of view** (`game/camera.ts`): the player sets a horizontal FOV at the 16:9 reference
  (80-120°, default 100°, i.e. 67.7° vertical). Narrower screens keep that horizontal FOV;
  wider ones keep the 16:9 vertical FOV (Hor+) up to 115° horizontal (or the setting, if
  wider). `fovYFor(setting, aspect, kick)` gives the vertical FOV `CameraView.fovY` the
  renderer uses; `FovKick` eases a +4° boost kick in and out over 250 ms. The view settings
  (FOV, cockpit frame, fit ring) live in `localStorage` key `ls.view` (`ui/viewSettings.ts`,
  every access in try/catch) and are set on the start screen.
- **Cockpit hull frame** (`renderer/hullFrame.ts`, models in `renderer/hullFrameModels.ts`):
  the nose deck, side parts and canopy pillars of the own ship type, built with the VQ4 kit at
  the VQ4 hull's dimensions and fit, seen from an eye above the hull's centre line. Drawn last
  in the scene pass in view space (`Renderer.hullFrame.state`, `null` hides it) with its own
  projection (near 0.01 u) and its depth squeezed into the front 1/1000 of the shared depth
  buffer, so rock and ships never cut it. It is lit by the scene at its world position through
  the VQ4 materials (`shade_model`), with a cockpit shade (painted parts keep 0.3 of the light
  next to the eye, 0.75 at the nose). Pipeline label `hull frame` (the browser smoke requires
  it). Cost: one draw of 1.2-2.4 k triangles, timed in the `objects` pass (about 0.005-0.01 ms
  on an RTX 5080).
- **Fit ring** (`ui/fitRing.ts`, probes in `game/scaleCues.ts`): DOM/SVG, not GPU. ls-wasm
  `Game.sweepSphere` sweeps the own hull's sphere along the view (contact measured like the
  simulation's collision, `crates/ls-wasm/src/probe.rs`) and `Game.raycast` checks the aim
  point; the ring is the hull radius projected at the contact's depth.

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

## Mode effects (M3 C1)

`modeFx.ts` `ModeFxPass` draws the mode visuals that are not meshes or particles, analytically
per pixel: the game hands `Renderer.setModeVolumes(volumes)` a list every frame
(`game/modeVisuals.ts` `modeVolumes`, at most `MODE_FX_MAX_VOLUMES` = 12); with an empty list
the pass and its depth read are skipped. The pass runs after the sky and shafts and before the
particles, as one fullscreen triangle with an additive blend that keeps the scene's alpha, and
reads the scene depth like the particles (the first sample of the multisampled depth): the
world position of every pixel is rebuilt from the camera basis (right, up and forward scaled by
the half-FOV tangents) and the linear depth. Everything fades with the exp2 distance fog.

| Volume | Look | Used for |
| --- | --- | --- |
| `wall` (centre, radius, `warn`, `grind`) | Rock within a band of the sphere (the shell's terrain wall) glows: a slowly rising energy curtain (stretched value noise), 3D cell borders (Voronoi, 2x2x2 lookups) that light up under a scan band, rare flickering cells; a bright seam 0.85 u inside the sphere where cave rock meets the wall and a soft spill onto rock in front of it. Cyan while the walls only warn, amber with molten cracks in patches while they grind inward. Where the sphere lies in open space ahead of the rock (chunk meshes that have not caught up with the moving shell yet) a faint film with the same pattern | The Squeeze's shell |
| `zone` (centre, radius, film, grid, contour, ring, pulse) | Where the view ray crosses the sphere in open space: a fresnel film with a drifting latitude/longitude grid (no line on the equator) and a scan band; where the sphere cuts rock: a dashed, moving contour, its width measured along the surface (the depth-derived normal keeps walls nearly tangent to the sphere from smearing it); an optional equator ring lit up to a fill fraction | The Squeeze's next zone, extraction zones, the embedded Heist core's "dig here" outline, Hot Reactor's fuse radius (the ring shows the fuse left) |
| `beam` (base, width, height) | A vertical light pillar (closest approach of the view ray to the vertical line, glow plus core, ripples), hidden behind rock, fading over its height, at least a pixel wide far away | Extraction zones, the Heist core and its carrier |
| `halo` (centre, radius, power) | A rim-lit bubble (front and a fainter back half) | The core carrier, the reactor holder |

Cost (gallery, RTX 5080, 1920x1080, "high", timed as `mode`): zones, beams and halos
0.03-0.05 ms (`heist-*`, `reactor`); a Squeeze wall filling the whole screen 0.08-0.10 ms
(`squeeze-warn`, `squeeze-close`). At the VQ plans' factor of about 10 for the reference mid
GPU that is 0.3-0.5 ms and 0.8-1.0 ms for the worst case; the gallery budget is
`PASS_BUDGETS_MS.mode` = 1.0 ms.

**Last Color Standing keep mask.** The grade's monochrome (`ColorGrade.monochrome`) applies
per pixel except where the scene's alpha says to keep the colour: opaque instances with
`Instance.keepColor` (0..1, packed into the opacity slot as 1 + keep, which the opaque pass
does not use otherwise) write alpha `1 + keep x KEEP_ALPHA` (`shaders/post.ts`, 1/16); the
composite reads `keep = (alpha - 1) / KEEP_ALPHA` and scales the monochrome blend by
`1 - keep`. KEEP_ALPHA is small so the blends that read the scene alpha (the sky drawn under
the scene subtracts at most 1/16 of the sky behind a kept ship) stay close to an alpha of 1;
translucent passes and smoke over a ship reduce the keep like coverage. `game/modeVisuals.ts`
sets `monochromeGrade` (a little darker and more contrasty) on top of the world's grade, eased
in over 1.2 s, and gives ships the share of their side colour their colour meter has left
(`colourMeterStyle`) with keep 1.

**Props.** `models/modeProps.ts`: the Heist core (a faceted crystal in three gyroscope hoops)
and the Hot Reactor (a glowing sphere in two containment rings with six vents), mesh kinds
`core` and `reactor` (the legacy builders use spheres). Their glowing parts take the instance
colour, an HDR display value above 1.

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

| Level | MSAA | Bloom mips | Lights | Particles (ring, spawn density, motes, half-res above) | Light shafts | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| high | 4x | 6 | 128 | 32 768, 1, 1500, 32 px | yes | default |
| medium | 4x | 4 | 32 | 16 384, 0.6, 800, 20 px | no | WebGPU has no 2x MSAA (the plan's medium 2x); medium saves on bloom, lights, particles and shafts |
| low | 1x | 0 | 16 | 8 192, 0.35, none, 12 px | no | still HDR + tonemap (rgba16float render targets are core WebGPU), one composite pass; terrain without the grain, vein and macro samples (`terrainDetail: false`) |

`?quality=low|medium|high` forces a level. Otherwise the renderer starts at "high" and, once it
has 5 s of GPU-timed frames, steps down once if the median GPU frame exceeds 8 ms ("medium") or
14 ms ("low"). Without `timestamp-query` it stays at "high" (frame intervals would also count
CPU work such as the initial terrain load). Scene pipelines are built per MSAA sample count and
terrain detail when a level first needs them.

## Terrain (VQ3b)

| Part | Code |
| --- | --- |
| Rock texture array: 512², 2 layers, `rgba8unorm`, full mip chain, generated at start-up by two compute shaders (tiling gradient noise with integer hashes; layer 0 rock slope, height and vein ramp, layer 1 grain slope, height and speckle) | `rockTextures.ts` (`TerrainMaterial`, `ROCK_GEN_WGSL`, `ROCK_MIP_WGSL`) |
| Per-world look: `TerrainParams` (strata axis tilted up to 28°, bands per u, texture offsets, strata contrast, vein amount) from `hashSeed(share code)`; set `Renderer.settings.terrainSeed` (main.ts and the gallery do) | `rockTextures.ts` `terrainParams` |
| Vertex data: the packed `team` u32 (paint dilated next to paint) plus two `unorm8x4` per vertex: AO, weak, hard, paint coverage / hot, charged, glow, scorch, blurred over the mesh near borders (chunk-face vertices pinned) | `terrainBlend.ts` `terrainVertexData`, uploaded by `Renderer.setChunk` (`GpuMesh.extraBuffer`) |
| Shading: triplanar rock and grain detail normals (surface gradient), macro blotch, strata, per-material albedo, roughness and F0, sharpened organic blends, paint picking and patterns, surface-state emission, GGX from the sun and the headlight, a normalised Blinn-Phong highlight from the dynamic lights (`light_spec`, power from the roughness, scaled by F0, capped at 3x), AO and cavity | `shaders/terrain.ts` (override `DETAIL` = `RenderQuality.terrainDetail`) |

The terrain pipeline has its own layout: group 0 the frame uniforms, group 1 the texture
array, an anisotropic (8x) trilinear repeat sampler and `TerrainParams`. Paint and state
shading run only on fragments whose triangle has paint or a state (uniform rock skips that
ALU; `fwidth` is taken before the branch). Specular from VQ6's dynamic lights is not included
(they reach the diffuse term through `light()`).

## Shader files

`shaders/frame.ts` (the `Frame` uniform block, shared by every scene pipeline, the terrain
attribute bit layout, sRGB helpers), `shaders/lighting.ts`, `shaders/terrain.ts`,
`shaders/objects.ts` (instanced and translucent), `shaders/lights.ts` (the light froxel
builder), `shaders/modeFx.ts` (M3 C1's mode effects), `shaders/particles.ts` (particles, motes, half-resolution composite), `shaders/post.ts`, `shaders/sky.ts` (VQ7: the
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
nav-light and lamp strengths are HDR radiance constants in `models/shading.ts` (VQ6 retuned
them for HDR: engines 2.6 idle and 7.5 boosting, nav lights 1.4 steady plus a 7.0 flash, lamps
4.5), so idle engines and steady nav lights bloom softly and boost and the nav flash hard.
