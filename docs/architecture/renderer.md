# Browser renderer

The WebGPU renderer in `web/src/renderer/` (visual-quality-plan.md VQ2, VQ5, VQ6, VQ7, VQ8
and VQ9; M3 C1's mode effects). Terrain and objects come from the game (`game/chunks.ts`, `game/scene.ts`), particles from
the effect library (`game/vfx.ts`); this page covers what happens from there to the screen.
Wave D of M3 extends it into the full renderer reference.

## Frame

| Step | Target | Code |
| --- | --- | --- |
| Lights (compute, skipped without lights): one invocation per froxel of a 16x9x24 grid, every light's view-space sphere tested against the froxel's box, one 128-bit mask per froxel | storage buffer (3456 x 16 bytes) | `lights.ts`, `shaders/lights.ts` |
| Sun shadow map (only on frames after a terrain change; skipped in sealed levels): the dirty rectangles of the map cleared and redrawn from the chunks that overlap them, depth only, back faces | `depth16unorm` 4096² over the whole level | `sunShadow.ts`, `shaders/shadow.ts` |
| Scene: terrain chunks (frustum culled per chunk AABB), then opaque instances, then translucent instances (no depth write), then the cockpit hull frame (own depth range) | `rgba16float` HDR colour, MSAA 4x at "high"/"medium", resolved into the post chain's HDR texture; `depth24plus` (stored for the particles) | `renderer.ts`, `shaders/terrain.ts`, `shaders/objects.ts`, `hullFrame.ts`; M4 C1: the cake branch (`shaders/cake.ts`) and, in Wrong genre's side view, the cutaway terrain pipeline (`shaders/cutaway.ts`) |
| Volumetric sun shafts ("high"/"medium", caverns with openings to the sky): each pixel of a half (third) resolution target marches its view ray through the shadow map up to the scene depth; added onto the resolved scene with a 2x2 tent | `rgba16float` low-resolution target, then the HDR scene texture | `volumetric.ts`, `shaders/volumetric.ts` |
| Sky (open levels only): drawn *under* the resolved scene, `out = scene + sky x (1 - alpha)` (geometry writes alpha 1, the clear is transparent black) | the HDR scene texture, single-sampled | `sky.ts`, `shaders/sky.ts` `SKY_WGSL` |
| Light shafts ("high", open levels, sun on or near the screen): sky mask (scene alpha 0) at quarter resolution, radial blur toward the sun, added onto the scene | the HDR scene texture | `sky.ts` `encodeShafts`, `SHAFT_*_WGSL` |
| Mode effects (M3 C1; skipped without volumes): one fullscreen triangle, additive, reading the scene depth: The Squeeze's glowing walls and next zone, zones, beams and halos | the resolved HDR scene texture | `modeFx.ts`, `shaders/modeFx.ts` |
| World effects (M4 B5; only while a well, a cave-in warning or rubble is on screen): a copy of the screen rectangle the volumes cover, then one scissored fullscreen triangle that bends, recolours and lights it: gravity wells (lensing, glow, the core stone), cave-in cracks, rubble dust, see "World effects" | the resolved HDR scene texture (needs `COPY_SRC`) | `wells.ts`, `shaders/wells.ts` |
| Cloak refraction (only while a cloaked ship is in view): a copy of the scene, then the cloaked hulls drawn into the scene sampling the copy through their surface (no depth attachment; each fragment reads the scene depth) | the HDR scene texture (needs `COPY_SRC`) | `cloak.ts`, `shaders/cloak.ts` |
| Particles: sprites at least `halfResPx` wide into a half-resolution `rgba16float` target, composited onto the resolved scene; then dust motes and the smaller sprites at full resolution. No depth attachment: each fragment reads the scene depth (soft particles, occlusion) | the resolved HDR scene texture | `particles.ts`, `shaders/particles.ts`, `spriteAtlas.ts` |
| Ink lines (M4 A7; only from Palette 50, `needsLinePass`): outlines from the scene depth and world-space hatching from the scene's tone, see "Ink look" | `rgba8unorm` full resolution (r outline, g hatch, b fog fade, a sky) | `ink.ts`, `shaders/ink.ts` |
| Bloom down: 13-tap filter, the first step with a Karis average and a soft-knee threshold | up to 6 half-size `rgba16float` mips | `post.ts`, `shaders/post.ts` `BLOOM_DOWN_WGSL` |
| Bloom up: 3x3 tent, added onto the next larger mip | the same mips | `BLOOM_UP_WGSL` |
| Composite: bloom (averaged over the mips, added), exposure, colour grade, vignette, AgX tonemap with a look, the ink look (from Palette 50: toon bands, paper grain, ink on paper, the ink lines), sRGB encode, half-LSB dither, optional film grain; Wrong genre's pixel look (M4 C1: cells, 16 colours, dither, scanlines) instead of the dither while it runs | the swapchain (`getPreferredCanvasFormat()`, non-sRGB, so the shader encodes) | `COMPOSITE_WGSL` |

With `timestamp-query` every pass writes timestamps (`gpuTimer.ts`), labelled `terrain` and
`objects` (or `scene` when `Renderer.detailedTiming` is off), `lights`, `shadow` (map updates),
`volumetric`, `sky` (the sky and the screen-space shafts), `mode`, `wells` (M4 B5), `particles`, `ink` and `post`. Budgets per label are in `web/src/gallery/scenes.ts`
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
- Lighting (`shaders/lighting.ts`, `light(n, p)`): sun 0.9 (times its visibility from the sun
  shadow map, VQ9), hemisphere ambient 0.035-0.11, the camera headlight (a soft spot light, see
  below) and the dynamic point lights. Model materials
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
the saturation and from 50 adds the ink look, see "Ink look" below), the fog (density, haze colours, height fog)
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

**Rock at the hidden walls (VQP).** In open levels the sealing walls are hidden, so pillars
that run floor to ceiling and rock cut by the level box used to end in a sharp, jagged cut
against the sky. The `Frame` uniform carries the room box (`room_min`, `room_max`; `w` the fade
width `ROOM_FADE` = 3 u, 0 in closed levels); `room_fade` in `shaders/frame.ts` gives the rock's
coverage, falling to 0 at a wall along a slightly wandering line, and only for rock seen from
`ROOM_FADE_NEAR`-`ROOM_FADE_FAR` (12-30 u) away or more, so rock you can touch stays solid. The
terrain shader writes premultiplied colour and that coverage as alpha (the sky composite
draws the sky under it; the shafts treat it as partly sky) and discards below 0.01.
`Renderer.roomFade = false` (gallery `?roomfade=off`) cuts sharply as before. Cost: a few ALU
per terrain fragment (no measurable change). Rock closer than `ROOM_CLIP` = 0.75 u to a hidden
wall is cut at any distance (D5): stripping the wall's triangles (0.5 u) leaves the triangles
that bridge rock and wall as a sawtooth, so the cut follows the plane instead.

## Sun occlusion (VQ9)

Rock blocks the sun. `sunShadow.ts` (`Renderer.sunShadow`) keeps one orthographic depth map
from the sun over the whole level (`depth16unorm`, 4096², about 0.09 u per texel over the
256 x 128 x 256 u level, 32 MB), and the scene shaders multiply the sun by its visibility:

- **Casters**: the terrain chunks' positions and indices (the drawn meshes, so in open levels the
  hidden level-box walls do not cast), depth only, **back faces** as seen from the sun (cull
  "front"). A lit surface lies in front of its own rock's far side, so lit rock has no acne
  without a large bias, and a point is in shadow exactly when its ray toward the sun enters rock.
  Rock at or beyond The Squeeze's shell sphere (less than `SHELL_CASTER_MARGIN` = 0.75 u
  inside it) casts nothing (D5, readability first): the caster pass has a fragment stage that
  drops those fragments, and `Renderer.setModeVolumes` hands the wall volume's sphere to
  `SunShadow.setShell` (a shrinking sphere needs no redraw, the remeshed chunks mark theirs
  dirty; a new or moved one redraws the map). The shell still draws, glows and collides.
- **Ship shadows** (D5): hulls and wrecks cast into a second, small map (`SHIP_SHADOW_SIZE`
  1024², `depth16unorm`, 2 MB) from the sun over `SHIP_SHADOW_RANGE` = 40 u around the camera
  (about 0.08 u per texel, centre snapped to whole texels so edges do not crawl), cleared and
  redrawn every frame from the opaque ship and wreck instance batches (positions and the
  instance transform only, every face, bias 0.04 u; `SunShadow.encodeShips`, timed as
  `shadow`). Receivers multiply both maps (`sun_visibility` = `terrain_visibility` x
  `ship_visibility`, a 4-tap PCF; binding 11). Cloaked hulls (refract batches) cast none.
  Without ships near, a sealed level or `?shadows=off` the map is not sampled;
  `?shipshadows=off` (`SunShadow.shipShadows`) turns only the hulls' shadows off.
- **Updates**: terrain is static between edits, so the map is not redrawn per frame.
  `setChunk` reports a chunk's old and new extent (`chunkChanged`); their projected texel
  rectangles go into a dirty list (merged when they overlap, at most 6), and the next frame
  clears each rectangle (a scissored full-screen triangle at depth 1) and redraws the chunks
  whose rectangles overlap it. The region is the union of the chunks seen so far snapped to
  the 32 u chunk grid (the whole level once loaded, since the streamer's view radius covers
  it); a growing region or a new sun direction redraws everything. `clearChunks` starts over.
- **Sealed levels** (no sky: `Atmosphere.open` 0, Enclosure >= 50): the level box's walls
  enclose everything, so the sun never gets in; `setSealed(true)` makes `sun_visibility`
  return 0 without a lookup and skips the map and the volumetric pass.
- **Lookup** (`shaders/shadow.ts` `sun_visibility(p, n)`, group 0 bindings 8 uniform, 9 map,
  10 comparison sampler, 11 the ship map): the receiver offset 0.12 u (plus half a texel) along the normal,
  depth bias 0.06 u, an 8-tap Poisson PCF of bilinear compares over 1.6 texels, sharpened with a
  smoothstep (soft but defined edges). `static_light` stores the result in `sun_vis_last`, which
  the terrain's GGX sun highlight and the models' `model_spec` reuse (one lookup per fragment).
  Outside the map (beyond the level) the sun is unblocked. `?shadows=off`
  (`Renderer.create(canvas, { shadows: false })`) restores the pre-VQ9 light.
- **Volumetric shafts** (`volumetric.ts`, `shaders/volumetric.ts`): at `RenderQuality.volumetric`
  (high: half resolution, 24 steps; medium: a third, 16 steps; low: off) each pixel marches its
  view ray (jittered with interleaved gradient noise, fixed per pixel so nothing shimmers) up to
  the nearest scene depth of its 2x2 footprint or 60 u, sums the sunlit steps with the
  extinction to the camera, and weights them with a Henyey-Greenstein phase (g 0.55-0.6, scaled
  so isotropic is 1) and the sun's radiance (`SUN_RADIANCE`). The composite adds a 2x2 tent of
  bilinear taps onto the resolved scene (alpha untouched, so the sky still draws under it).
  Dust per world type (`VOLUMETRIC_LOOKS`, `volumetricLook(weights)`): 0.0013 per u in
  caverns, 0.0016 in mines, faded in (smoothstep) from an enclosed weight (cave + mine) of
  `DUST_FROM` 0.75 (Enclosure 37) to 1 (Enclosure 50). Open and half-open worlds have none and
  skip the pass (black space stays black; VQ7's screen-space shafts carry the sun there), and
  sealed worlds have no sun. So the shafts belong to caverns with a few openings to the sky
  (Enclosure about 40-49): where an opening lets the sun in, the lit air between the shadowed
  walls reads as a shaft. Points outside the shadow map hold no dust.
- Costs (gallery, RTX 5080, 1920x1080, high): see visual-quality-plan.md "VQ9 as built".
  `?shadowbench=full|chunk` redraws the whole map, or two chunks' rectangles (one crater's
  remesh), every frame, so the gallery times the update (`shadow` label).

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
  the VQ4 hull's dimensions and fit, seen from an eye above the hull's centre line. The canopy
  frame (VQP) is a window frame, not loose struts: each pillar is a dark beam with a lighter
  metal rail along its inner edge, a bow arches across the top of the view (higher in the
  middle; the Hauler keeps its heavy visor above it) and rounded corners join bow and pillars
  (`canopyPillars`, `canopyBow`). Drawn last
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

## Ink look (Palette, M4 A7)

The Palette slider (milestone-4-plan.md §3.3) turns the world from full colour into
black-and-white ink. `ink.ts` maps the slider to an `InkLook` (`inkForPalette`; the atmosphere
carries it as `Atmosphere.ink`, `setAtmosphere` puts it into `settings.post.ink`) and the grade's
saturation factor (`paletteSaturation`, used by `worldGrade`):

| Palette | Saturation | Ink |
| --- | --- | --- |
| 0-49 | 1 down to 0.5 | none (no ink pass, the composite skips the ink branch) |
| 50-79 | 0.5 down to 0.4 | over the colour image: outlines 0.35 -> 0.9, toon bands 0.3 -> 0.9, paper grain 0.25 -> 0.6 |
| 80-100 (`MONOCHROME_PALETTE`, a monochrome level) | (kept colours only) | ink on paper: outlines, toon, grain and the monochrome at 1, hatching 0.75 -> 1, contrast 0 -> 1 |

**Ink pass** (`InkPass`, `shaders/ink.ts`, timed as `ink`): one fullscreen triangle after the
particles, reading the scene depth (first sample) and the resolved HDR scene, writing an
`rgba8unorm` target. Outlines: the Laplacian of 1/depth along four axes at a step of
`lineScale` px (1 at 1080p, thicker above). 1/depth is affine in screen space on any plane, so
the measure is zero on flat rock and large at creases and silhouettes; divided by 1/depth and
scaled by `height / (2 step tan(fovY/2))` it is a change of surface slope per pixel, independent
of distance and resolution. Convex creases and the near side of silhouettes ink from a slope
change of 0.7 (full at 1.8), concave creases from 1.0 (full at 2.4) while they are small (the
far side of a silhouette is a jump, not a crease); lines fade into the fog like the rock.
Hatching is world space, so it sticks to the rock while the camera moves: three families of
parallel planes (`HATCH_DIRECTIONS`, not axis aligned) cut every surface in lines; a family
whose lines on screen grow wider apart than 2.2-3.5 x the target spacing (nearly parallel to the
surface) fades out and the others cover. The spacing follows the pixel footprint in octaves
(`HATCH_SPACING_PX` = 6 px at 1080p): every second line of one octave is the next octave, and
the two are blended by the fractional octave, so the density on screen stays constant at any
distance without popping; lines closer than 2 px fade to their mean coverage. Each family starts
below a tone (`HATCH_THRESHOLDS` 0.62, 0.42, 0.24, from an AgX-like estimate of the pixel's
display lightness) and widens with darkness up to 0.42 of the spacing. The cockpit frame's
squeezed depth range is decoded with its own projection (`INK_FRAME_DEPTH`, equal to the
constants in `hullFrame.ts`) and hatched in camera space, since it flies with the camera.

**Composite** (`shaders/post.ts` `ink`, after the AgX tonemap, display-linear): toon bands are
four soft plateaus of the display lightness (the colour scaled, so the hue stays; softer in the
sky); paper grain is screen-space value noise (tooth, mottle, fibres: the paper is the screen).
The monochrome maps the banded lightness onto ink (`INK`, blue-black) to paper (`PAPER`, warm
off-white) with a curve that lifts lit rock to paper (`smoothstep(0.04..0.08, 0.74..0.58)` from
contrast 0 to 1); distant rock (the ink target's fog fade) fades into a pale mist (0.74) and
the sky is the same pale wash with soft blotches and bright things (sun, planet, nebula cores)
as paper, so far islands dissolve into the sky like an ink-wash landscape. Outlines and hatching
then darken toward the ink (`min`, so a line over a darker sky never shows lighter).

**Keep mask: paint and pigment.** Pixels whose scene alpha marks a kept colour (see "Last
Color Standing keep mask" below) skip the monochrome, the hatching and the Palette's
desaturation (the grade uses `max(saturation, 1)` there), not the outlines: pigmented ships
(`Instance.keepColor` 1, B5) and painted rock (the terrain shader writes `1 + paint coverage x
KEEP_ALPHA`, in every level) stay in full colour. Measured on the gallery's paint decor
(`paint-edge` at Palette 0 against `palette-100-paint`): interior paint keeps 0.73 of 0.82 mean
HSV saturation with a mean hue shift of 2.5 degrees (the old 90 % desaturation left 0.13).

**Cost** (gallery, RTX 5080, 1920x1080, "high"): the ink pass 0.04-0.05 ms, the composite
+0.03-0.05 ms over Palette 0 (`post` 0.08 -> 0.11-0.13 ms) in every palette scene. At the VQ
plans' factor of about 8-10 for the reference mid GPU that is about 0.3-0.45 ms for the pass and
0.3-0.5 ms in the composite, inside the plan's 0.8 ms ink share (estimated; the integrated GPU
of the development machine was not available to headless WebGPU for a floor run).
`PASS_BUDGETS_MS.ink` = 0.8 ms.

Gallery scenes: `palette-{0,40,65,85,100}` (caves with the four hulls), `palette-<p>-mine`,
`palette-<p>-sky`, `palette-100-paint` (paint keeps its colour), `palette-100-pigment` (two hulls
with `keepColor`), `palette-85-fight`; any scene takes `--query palette=<0..100>` (`?palette=`,
render only: the share code keeps its own value).

## World effects (M4 B5)

What the sliders put into the level and the level's mood, drawn on the client
(milestone-4-plan.md §3.1, §3.3, §3.4). The game side is `game/worldFx.ts`
(`WorldVisuals`: one call per frame from `main.ts`) and `game/moodFx.ts`; the
gallery scenes are `gallery/moodScenes.ts` (data) and `gallery/worldScenes.ts`
(set-up); `?worldfx=off` draws everything without B5 (before shots).

| Piece | Look | Code |
| --- | --- | --- |
| Gravity well (static sites, shepherd moons, dynamic wells: Paperweight, Architect stroke) | Lensing: everything behind the well's centre slides toward it by E^2 / b (b the ray's impact parameter, E the lens radius, held finite near the centre), so a magnified ring of rock or sky forms around the core and the far side shows inverted inside it; samples in front of the well keep the unbent pixel (a soft depth test). A photon ring at E; a violet Gaussian glow integrated along the ray up to the scene depth (rock in front hides it); the core stone darkened to polished black with thin pulsing violet veins (ridges of two noise octaves, turning slowly) and a violet rim. An accretion swirl: about 120 glowing motes spiralling inward (Keplerian speed), 16 rock chips and a faint haze as immediate particles; a violet point light. Dynamic wells grow in and out over 0.5 s; the Gravity coffee break dims glow, lens and swirl to 8 % over 1 s | `wells.ts` `WellVolume`, `worldFx.ts` `wellLook`, `swirl` |
| Pull indicator | A violet arc and chevron on a ring around the crosshair toward the pull, width and opacity by its strength (share of the 40 % clamp), "PULL n%" (of the ship's thrust); a dot (ahead) or a broken ring (behind) when the pull runs along the view. The force law mirrors `ls_sim::gravity` (tests read `data/world/gravity.ron` and `data/ships`) | `ui/worldHud.ts` `PullIndicator`, `worldFx.ts` `shipPull` |
| Cave-in warning (`CueKind::CaveInWarning`) | The ceiling above the spot (first rock along the level's up, `Game.raycast`) cracks: polygonal cell-border cracks, only some of them, kinked by a warp, spreading from the spot, widening and heating up through the 1.5 s, finer cracks opening after a third; dust and pebbles trickle (40-200 per second), an orange light grows, the view shakes within 45 u; then a dust burst and chips, `cavein.warning` / `cavein.crash`, the announcer's `caveIn` line within 40 u | `wells.ts` `CrackVolume`, `worldFx.ts` |
| Rubble | Rock within 7 u of a fallen cave-in gets a pale broken-stone dust coat (more on faces turned up, grit specks), fresh for about 25 s, then a lasting light coat (the last 16 cave-ins of a match) | `wells.ts` `RubbleVolume` |
| Pigment pickups (monochrome levels) | Three drops of paint in the primaries (`models/worldProps.ts` `pigmentDropModel`, mesh kind `pigment`) circling a glowing core on a tilted ring; a death's splash is one big drop in the side's colour with two droplets. Kept in colour through the ink (`keepColor` 1); a light cycling the primaries. The scene builder leaves `pigmentPickup` entities to the world visuals (`SceneExtras.skipKinds`) | `worldFx.ts` `addToScene` |
| Pigmented and grey ships (Palette 80+, not Last Color Standing) | Pigmented ships (`Game.shipExt`) fly in their side's colour through the ink and drip paint (opaque kept-colour drops, 9 a second); grey teammates keep 60 % of their team colour, other grey ships are ink grey; the own cockpit frame follows the same rule. Name tags: pigmented ships to 160 u through walls with a drop marker, grey enemies only within 20 u and in sight | `worldFx.ts` `pigmentStyle`, `ui/nameTags.ts` `PigmentTagRules` |
| Moods | A slow (6 s) grade tint over the world's grade (calm cool and clear, bored washed out and flat, grumpy dark, warm and hard, giddy warm and saturated), the dust motes' colour (`ParticleSystem.moteColor`, `moteRainbow`: calm pale blue, bored grey and dim, grumpy ember, giddy a pastel hue per mote), the HUD badge (a face, the mood's name, its line for 4.5 s after a change), an ambient drone per mood crossfaded on the music bus | `moodFx.ts`, `ui/worldHud.ts` `MoodBadge` |
| Bored level (the level's up turns) | `Renderer.setWorldUp(up, centre)` (`turn.ts`): the shortest rotation from +Y to the up turns the sky pass (`Sky.turn`, the sky uniform's `turn`), the fog's haze lookup (`Frame.sky_turn`, `sky_dir`), the height fog (heights along `Frame.up`) and the sun (light and shadows). While the up keeps changing (small steps per frame) the sun shadow map is redrawn spread over frames: the map for the new direction is drawn into a second map in 8 bands of rows, one per frame, then copied over the current one (`SunSweep`, `SPREAD_SLICES`); the scene is lit from the map's sun meanwhile, so light and shadows agree (they lag the sky by about 3 degrees at the turn's speed). A jump (a new match) redraws at once. `WorldFx` smooths the replicated up | `turn.ts`, `sunShadow.ts`, `sky.ts` |

**Pass** (`WellsPass`, timed as `wells`): the volumes come nearest first, at
most `WELLS_MAX_VOLUMES` (12) and the nearest three wells within 80 u (farther
wells keep their light and swirl). The CPU projects each volume's bounding box
(`sphereRect`); the pass copies and draws only the union rectangle
(scissored). Pipeline variants by the kinds present (override constants
`HAS_WELLS`, `HAS_SURFACE`) compile the unused branches away. Alpha is copied
through, so the sky mask and the colour keep mask stay.

**Cost** (gallery, RTX 5080, 1920x1080, "high"): `wells` 0.04-0.05 ms in every
B5 scene (a well filling the screen from inside its hollow, wells in open sky,
a cave-in warning, rubble); the copy and an empty pass alone 0.02 ms. Mood
presentation adds nothing measurable (grade uniforms, mote colour). At the VQ
plans' factor of 8-10 that is 0.3-0.5 ms on the reference mid GPU against the
0.4 ms wells share (estimated, not measured). `PASS_BUDGETS_MS.wells` = 0.4 ms.

## Comedy event visuals (M4 C1)

The looks of the comedy events (milestone-4-plan.md §3.2). The game side is
`game/events/`: `index.ts` (`EventVisuals`, the hub main.ts and the gallery
call), one module per event, `fx.ts` (what the event does now), `shots.ts`
(projectiles followed between frames), `info.ts` (names, accents, icons). The
gallery strips are `gallery/eventScenes.ts` (data) and
`gallery/eventScenesRun.ts` (set-up); `?eventfx=off` draws the match without
event visuals (the banner and the announcer stay).

**Timing.** ls-wasm `Game.eventFx(tick)` (`crates/ls-wasm/src/events.rs`)
decodes the replicated mood view with the simulation's own functions at the
estimated server tick: phase (advanced like the predictor's
`MoodView::advanced`), well scale, hit scale, the chicken flag, the level's up,
Bored level's start up and axis, every hiccup tick of Level hiccups
(`level_hiccups::hiccup_ticks`, from the match seed) and the cake's origin.
Without the export (older cores) `fxFromMood` decodes the phase and the
parameter bytes in TS. Every module reads an `EventClock`: seconds since the
active phase started (-3..0 while announced) or, after the end, seconds since
it was over; the hub keeps a module running after its event until it says it
is done (bursting balloons, landing boxes, receding cake, chickens still in
flight).

**Telegraph** (all 18 events, the hub): when an event is first seen in its
announce phase, the announcer says `event.<id>` with the target's name (its
fanfare `sting.event` first) and `ui/eventBanner.ts` shows a card under the mode
bar (icon, kicker, name, target, a 3 s countdown bar) that shrinks to a pill
with a ring of the time left while the event runs. The same overlay draws the
per-event HUD hints (Bored level's down arrow, Wrong genre's title cards,
sleepy Z's, comic words like "HIC!" and "POP!"). A late join into a running
event gets the pill but no line.

| Event | Look | Code |
| --- | --- | --- |
| Bored level | Sleepy Z's while announced; at the start a yawn and a creaking loop; grit streaking toward the new down around the camera while it turns; an arrow on a ring around the crosshair pointing at the new down (B5 turns sky, sun, fog and shadows, horizon assist rolls the pilot) | `boredLevel.ts` |
| Level hiccups | At every hiccup tick: a camera bump (a dip and a nod along the up, decaying in 0.6 s; the sim's 2 u lift moves the ship), "HIC!" popping on the screen, a "hic", dust and pebbles from the ceilings above and ahead of the camera (CPU rays), bubbles rising from every ship within 60 u; the tell is one small "hic?" | `levelHiccups.ts` |
| Gravity coffee break | A coffee mug (white ceramic, blue band, heart) drops onto every static well's core stone with two bounces and a clink while announced, steams and sips during the break (a slurp from the nearest), a warm lamp, a lounge loop on the music bus; lifts off after the end. B5 dims the wells | `coffeeBreak.ts` |
| Wrong genre | Pixel glitches while announced; then the side view: the camera swings in 0.55 s to 22 u in front of the own ship's z plane (34° field of view, a platformer follow with a little lead), its near plane 1.4 u in front of the plane; the terrain's cutaway pipeline draws the rock the near plane opens as a brick fill; other ships and shots are drawn in the own plane, ships 1.9x and rolled to show their tops; the composite's pixel look (quarter resolution, 16 colours); "WORLD 1-1" and "1UP" cards, a power-on bleep, a chiptune loop, a 1-UP jingle; no cockpit frame, crosshair or fit ring | `wrongGenre.ts`, `renderer/pixelPost.ts`, `shaders/cutaway.ts` |
| Big head day | A translucent balloon in the side's colour at exactly the hit sphere (radius x hit scale) around every other ship, an opaque face on its nose side (eyes, raised brows, grin, cheeks), googly pupils (spring-damper per eye kicked by the ship's acceleration, looking at the camera); blown up in three squeaky puffs while announced, popping to size at the start; hits honk and wobble the balloon; a burst (shreds, confetti, "POP!") at the end. The own ship's balloon is not drawn (it would tint the cockpit view) | `bigHeadDay.ts` |
| Rubber chickens | Every projectile with the chicken bit (ls-wasm adds 256 to the state) is drawn as a rubber chicken sized by its shape (the Charge as a giant speckled egg), beak along the flight, rolling and flapping, a soft glow in the weapon's colour, feathers shed; squeaks on fire, a squawk and a feather burst where it ends. Its normal shot visuals are hidden (`hideShots`); the effect library still sparks at the impact | `rubberChickens.ts` |
| Moving day | Every pickup gets a cardboard box (arrows, a red stamp, tape) that pops up with open flaps, shuts and is taped while announced, hops along an arc (one flip, a dust trail) to the point the sim moved the pickup to, lands with a squash and a thump, pops open and shrinks away; the pickup is hidden while boxed | `movingDay.ts` |
| Everything is cake | A sprinkle fountain and a pink glow at the origin while announced; a party horn and confetti at the start; the terrain shader's cake branch in a sphere that grows at 32 u/s; crumbs and sprinkles where shots end inside it; debris is sponge-coloured; the cake recedes in 2.5 s after the end | `cake.ts`, `shaders/cake.ts` |

**Renderer pieces.**
- *Toy materials* (`models/toyPalette.ts`): parts `TOY_PART_BASE` (32) + i take
  palette entry i (display colour and gloss; entry 0 the instance colour), so a
  prop has many colours in one mesh. `shade_toy` (included by
  `models/shading.ts`): wrapped diffuse, a gloss-dependent highlight from the
  sun, headlight and dynamic lights, a rim sheen, cardboard fibres, a small
  self-light floor so toys read in dark caves. Props in
  `models/eventProps.ts`, mesh kinds `chicken`, `giant_egg`, `moving_box`,
  `box_flap`, `box_tape` (instance `stretch` lays the tape), `balloon`,
  `balloon_face`, `pupil`, `coffee_mug`.
- *Feather sprite*: atlas layer 11 (`SPRITE.feather`).
- *Pixel look* (`pixelPost.ts`, `PostSettings.pixel`, the composite's `pixel`
  uniform): every pixel of a cell (4 px at 1080p, `quarterCell`) takes the
  cell centre's scene, bloom and ink samples; the display colour is lifted
  (gamma 0.72) and saturated (1.5x) and snapped to the PICO-8 palette after a
  4x4 Bayer dither, matched by luma and colour difference (blues stay blue); a
  faint scanline on each cell's last row. `pixelLookAt` steps the cell in whole
  pixels as it eases in.
- *Cutaway terrain* (`Renderer.terrainCutaway`): a terrain pipeline variant
  without back-face culling; the terrain shader paints back faces (rock seen
  from inside) with `cut_fill` (bricks in the x/y plane, lit top edges).
  Front faces are unchanged, so the normal pipeline needs no branch.
- *Cake* (`RenderSettings.cake`, `CakeWave`): two more vec4 in `TerrainParams`
  (origin and radius, up and amount; `TerrainMaterial.setCake` uploads on
  change). The terrain shader's cake branch (`shaders/cake.ts`, arithmetic
  only): sponge with pores, cream and jam layers along the up, pink frosting on
  faces turned up with round-ended drips down walls, capsule sprinkles in six
  colours, a bright band with twinkles at the front.

**Cost** (gallery, RTX 5080, 1920x1080, "high", label `c1-final`): see the
milestone plan's C1 note; every strip is within budget. The side view's
terrain pass rises from about 0.21 to 0.37-0.48 ms (back faces drawn, the
cave behind the plane in view), the cake branch adds 0.1-0.15 ms to the
terrain pass when it covers the screen, props and particles stay below
0.05 ms. At the VQ plans' factor of 8-10 the worst event frame is about
1 ms on the reference mid GPU against the 1.0 ms event share (estimated).

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
| `wall` (centre, radius, `warn`, `grind`) | Rock within a band of the sphere (the shell's terrain wall) glows: a slowly rising energy curtain (stretched value noise), 3D cell borders (Voronoi, 2x2x2 lookups) that light up under a scan band, rare flickering cells; a bright seam 0.85 u inside the sphere where cave rock meets the wall and a soft spill onto rock in front of it (faded in over the band, D5: a switch at the sphere drew the meshed wall's cell-to-cell offsets as rings). Cyan while the walls only warn, amber with molten cracks in patches while they grind inward. Where the sphere lies in open space ahead of the rock (chunk meshes that have not caught up with the moving shell yet) a faint film with the same pattern | The Squeeze's shell |
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
translucent passes and smoke over a ship reduce the keep like coverage. Painted rock writes the
keep too (M4 A7: paint keeps its colour in every level, also in Last Color Standing). `game/modeVisuals.ts`
sets `monochromeGrade` (a little darker and more contrasty) on top of the world's grade, eased
in over 1.2 s, and gives ships the share of their side colour their colour meter has left
(`colourMeterStyle`) with keep 1. The own cockpit frame does the same (D5):
`HullFrameState.keepColor` (main.ts sets it and the meter colour from `playerStyleFor`) and the
frame's fragment shader writes the same alpha, so in Last Color Standing the own hull keeps its
colour like everyone else's (gallery `last-color-cockpit`).

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

### Charge-up (VQP)

`game/chargeFx.ts`: while a ship holds a charge weapon (Fusion Charger, Polite Laser) energy
gathers at its muzzle: a white-hot core in an orb of the weapon's colour that grows with the
charge (`orbRadius`), a halo, a ring around the aim axis pulsing from 2 to 9 Hz, sparks
streaming in from a shell around it (25-135 per second), crackling arcs above half charge, a
turning star flare and a second ring at full charge, and an overcharge that turns it red,
unstable and spitting sparks. A point light grows with it (`chargeLight`: intensity 1.2-6.2,
reach 3-8 u). `ChargeTracker` takes the own charge from `Game.charge()` every frame (drawn at
the cockpit nose, below the crosshair, at `OWN_CHARGE_SIZE`) and other ships' from their
`ChargeStart` cues, which ls-wasm fills with the charging weapon (`target`), ending when the
ship's next projectile of that weapon appears, the ship dies or 4 s after full charge. `Vfx`
draws `VfxFrame.charges`; `main.ts` adds the lights. Gallery: `charge-up` (full, 45 %,
overcharge), `charge-cockpit` (the own charge at 75 %). Cost: 4 immediate sprites and a few
dozen particles per charging ship; particles pass 0.02 ms in both scenes (RTX 5080).

### Cloak refraction (VQP)

Cloaked ships (`game/scene.ts` `addShips` with `refractCloak`, which `main.ts` sets) go into
the scene's `refract` list instead of a flickering translucent hull. `cloak.ts` `CloakPass`
runs after the sky and shafts and before the particles, only while such a ship is in view: it
copies the resolved HDR scene, then draws the hulls (back faces culled, no depth attachment;
fragments behind the scene depth are dropped, so rock and the cockpit frame still hide them)
sampling the copy through the hull. The bend follows the normal of a smooth ellipsoid around
the hull (mixed with a quarter of the mesh normal, so the flat-shaded parts do not shatter the
image), from 1.6 % of the viewport in the middle to 5 % at the silhouette, larger for near
hulls, with a slow ripple and a slight split into colour channels; the hull tints the view
faintly and a thin rim and travelling bands shimmer in its team colour (`CLOAK_RIM`). Gallery:
`cloak` (`?cloak=flat` for the old look). Cost: the copy plus one draw per cloaked hull,
timed in `objects` (0.01 ms with two cloaked hulls at 1080p on an RTX 5080).

## Quality presets

`quality.ts`: one `RenderQuality` object per level; later packages add their switches to it.

| Level | MSAA | Bloom mips | Lights | Particles (ring, spawn density, motes, half-res above) | Light shafts | Volumetric shafts (VQ9) | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| high | 4x | 6 | 128 | 32 768, 1, 1500, 32 px | yes | 1/2 res, 24 steps | default |
| medium | 4x | 4 | 32 | 16 384, 0.6, 800, 20 px | no | 1/3 res, 16 steps | WebGPU has no 2x MSAA (the plan's medium 2x); medium saves on bloom, lights, particles and shafts |
| low | 1x | 0 | 16 | 8 192, 0.35, none, 12 px | no | off | still HDR + tonemap (rgba16float render targets are core WebGPU), one composite pass; terrain without the grain, vein and macro samples (`terrainDetail: false`) |

`?quality=low|medium|high` forces a level. Otherwise the renderer starts at "high" and, once it
has 5 s of GPU-timed frames, steps down once if the median GPU frame exceeds 8 ms ("medium") or
14 ms ("low"). Without `timestamp-query` it stays at "high" (frame intervals would also count
CPU work such as the initial terrain load). Scene pipelines are built per MSAA sample count and
terrain detail when a level first needs them. The start screen's backdrop (FI) runs on the
game's renderer, so the pick is usually made before the match starts.

## Start screen, lobby flythrough and ship bay (FI)

The first-impression pass (`web/src/attract/`) puts the renderer to work before the match.

| Piece | File |
| --- | --- |
| `FlightPath`: an endless cinematic path planned from level probes (ls-wasm `raycast` / `sweepSphere`): candidate directions in a cone around the heading are swept with a 2.2 u sphere, scored by free distance, the view beyond, keeping the heading, a level horizon, not revisiting and an optional attractor direction; each waypoint is nudged to the middle of its passage; a centripetal Catmull-Rom spline sampled by arc length | `attract/flightPath.ts` |
| `CinematicCamera` (8.5 u/s, eased in, damped look-ahead, banking, `frameYaw`: looks 0.2 rad left of the flight on landscape screens so the action sits right of the panels docked left); `Squadron`: four synthetic ships (one per hull, two teams) weaving ahead of the camera as wide as the room around the path allows, firing bolt bursts and now and then a seeking rocket; a destroyed ship returns 4.5 s later far ahead under a new id (no respawn burst) | `attract/cinematic.ts` |
| `Backdrop`: the start screen's live level: one of `BACKDROP_WORLDS` (caverns open to the sky, curated in the gallery so the planet shows), atmosphere, path leaning toward the planet (else the sun), exposure x1.18, the game's scene builder, effect library (VQ5) and lights (VQ6) | `attract/backdrop.ts` |
| `StartIntro`: creates the game's renderer after the start screen's first paint, runs the backdrop, hosts the hull view of the ship picker, hands the renderer (all meshes built) to the game | `attract/intro.ts` |
| `LobbyCinematic`: in the lobby with the mouse free, the camera leaves the cockpit and flies a path from the current view (the own ship is drawn once the camera is 2.5 u away) | `attract/lobby.ts` |
| `ShipBayView` / `PanelBay`: the four hulls turning behind the start screen's ship cards and the in-game ship bay (`ui/shipSelect.ts` `CardBackdrop`) | `attract/shipBay.ts` |
| Gallery pages (`start-screen`, `start-flythrough`, `lobby-flythrough`, `ship-bay`) | `attract/galleryPages.ts` |

**Hull view.** A second `Renderer` on its own canvas behind the cards, sharing the game
renderer's GPU device (`Renderer.create(canvas, { device })`: `gpu.ts` `contextOn` configures
the canvas; `destroy()` leaves a borrowed device alone), at "medium" (or "low"), without
particles (exhaust plumes would cross the neighbouring cards at this scale). Each frame every
hull is placed where its card's model box projects at 7 u depth (`bayLayout`, 34° vertical FOV)
and sized so its length fills the box; it turns on a tilted turntable (the selected one faster,
boosting). Studio lighting: the sun as a key light from above left in front, a warm rim light
behind and a cool fill below per hull (point lights scaled by r²), a soft engine light, no fog,
exposure 1.45, bloom 0.9. It draws only while shown; the card elements stay the DOM's, so the
layout is CSS's (responsive down to phone width).

**Keeping the start screen fast.** The start card shows as before (no GPU work in front of
it). Then: the renderer is created after the first paint; instanced meshes are built on first
use (`Renderer.prepareMeshes()` builds the rest before play), which took the VQ4 models' ~0.1 s
out of the start; the backdrop level is a lazy practice level (ls-wasm `Game.practicePreview`:
no dummies, no up-front generation, which blocked the page 1.6 s) whose chunks are generated
one per frame nearest the camera (`Game.generateNearest`, ~10-30 ms each) before meshing, and
the path is planned in a frame of its own. Chunks mesh under 10 ms per frame while loading and
4 ms while flying. `?backdrop=off` skips the flythrough. Measured with
`tools/browser-smoke/startup.mjs` (dev-guide.md), 1920x1080, RTX 5080, median of 5: start card
shown 66 ms before / 82 ms after; long tasks after it: none before (the renderer was created
after Play) / two after (renderer creation and the backdrop level's setup, about 100 ms each;
total blocking time 62 ms; time to interactive 0.69 s); backdrop visible 6.0 s after load,
57 fps (headless). The 1.6 s level build and the renderer creation no longer happen after the
Play click for online play.

GPU (gallery, RTX 5080, 1080p, high): `start-screen` 0.37 ms (terrain 0.14, particles 0.05,
sky 0.05, post 0.08, hull view `bay` 0.04), `lobby-flythrough` 0.26 ms, `ship-bay` 0.43 ms
(bay 0.05). Budget share of the hull view: `bay` 0.8 ms (`PASS_BUDGETS_MS`).

## Terrain (VQ3b)

| Part | Code |
| --- | --- |
| Rock texture array: 512², 2 layers, `rgba8unorm`, full mip chain, generated at start-up by two compute shaders (tiling gradient noise with integer hashes; layer 0 rock slope, height and vein ramp, layer 1 grain slope, height and speckle) | `rockTextures.ts` (`TerrainMaterial`, `ROCK_GEN_WGSL`, `ROCK_MIP_WGSL`) |
| Per-world look: `TerrainParams` (strata axis tilted up to 28°, bands per u, texture offsets, strata contrast, vein amount) from `hashSeed(share code)`; set `Renderer.settings.terrainSeed` (main.ts and the gallery do) | `rockTextures.ts` `terrainParams` |
| Vertex data: the packed `team` u32 (paint dilated next to paint) plus two `unorm8x4` per vertex: AO, weak, hard, paint coverage / hot, charged, glow, scorch. Weak and hard are the mesher's display weights (`MeshData.materialWeights`, smooth in space and across chunk seams, D5); paint and states are blurred over the mesh near borders (chunk-face vertices pinned); meshes without weights blur the material ids too | `terrainBlend.ts` `terrainVertexData`, uploaded by `Renderer.setChunk` (`GpuMesh.extraBuffer`) |
| Shading: triplanar rock and grain detail normals (surface gradient), macro blotch, strata, per-material albedo, roughness and F0, material borders resolved by a steep soft-max of the weights displaced by mostly low-frequency rock noise (`MATERIAL_EDGE_SHARPNESS`: one material wins almost everywhere, borders are crisp meandering curves), sharpened organic paint and state blends, paint picking and patterns, surface-state emission, GGX from the sun and the headlight, a normalised Blinn-Phong highlight from the dynamic lights (`light_spec`, power from the roughness, scaled by F0, capped at 3x), AO and cavity | `shaders/terrain.ts` (override `DETAIL` = `RenderQuality.terrainDetail`); the sun's diffuse and GGX highlight are shadowed (VQ9) |

The terrain pipeline has its own layout: group 0 the frame uniforms, group 1 the texture
array, an anisotropic (8x) trilinear repeat sampler and `TerrainParams`. Paint and state
shading run only on fragments whose triangle has paint or a state (uniform rock skips that
ALU; `fwidth` is taken before the branch). Specular from VQ6's dynamic lights is not included
(they reach the diffuse term through `light()`).

## Shader files

`shaders/frame.ts` (the `Frame` uniform block, shared by every scene pipeline, the terrain
attribute bit layout, sRGB helpers), `shaders/lighting.ts`, `shaders/terrain.ts`,
`shaders/objects.ts` (instanced and translucent), `shaders/lights.ts` (the light froxel
builder), `shaders/shadow.ts` (VQ9: the SunShadow struct and `sun_visibility`, included by
`lighting.ts`; the caster pass), `shaders/volumetric.ts` (VQ9: the shaft march and composite), `shaders/ink.ts` (M4 A7: the ink lines), `shaders/modeFx.ts` (M3 C1's mode effects), `shaders/particles.ts` (particles, motes, half-resolution composite), `shaders/cloak.ts` (VQP: the cloak refraction), `shaders/cake.ts` and `shaders/cutaway.ts` (M4 C1, included by the terrain shader), `shaders/post.ts` (with `pixelPost.ts`'s pixel look), `shaders/sky.ts` (VQ7: the
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
