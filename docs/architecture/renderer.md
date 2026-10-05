# Browser renderer

The WebGPU renderer in `web/src/renderer/` (visual-quality-plan.md VQ2 and VQ8). Terrain and
objects come from the game (`game/chunks.ts`, `game/scene.ts`); this page covers what happens
from there to the screen. Wave D of M3 extends it into the full renderer reference.

## Frame

| Step | Target | Code |
| --- | --- | --- |
| Scene: terrain chunks (frustum culled per chunk AABB), then opaque instances, then translucent instances (no depth write), then the cockpit hull frame (own depth range) | `rgba16float` HDR colour, MSAA 4x at "high"/"medium", resolved into the post chain's HDR texture; `depth24plus` | `renderer.ts`, `shaders/terrain.ts`, `shaders/objects.ts` |
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
  camera headlight. VQ6 adds dynamic lights there.

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
The gallery previews one with `--grade <preset>` (`?grade=`).

## Quality presets

`quality.ts`: one `RenderQuality` object per level; later packages add their switches to it.

| Level | MSAA | Bloom mips | Notes |
| --- | --- | --- | --- |
| high | 4x | 6 | default |
| medium | 4x | 4 | WebGPU has no 2x MSAA (the plan's medium 2x); medium saves on bloom and, later, particles, lights and shafts |
| low | 1x | 0 | still HDR + tonemap (rgba16float render targets are core WebGPU), one composite pass |

`?quality=low|medium|high` forces a level. Otherwise the renderer starts at "high" and, once it
has 5 s of GPU-timed frames, steps down once if the median GPU frame exceeds 8 ms ("medium") or
14 ms ("low"). Without `timestamp-query` it stays at "high" (frame intervals would also count
CPU work such as the initial terrain load). Scene pipelines are built per MSAA sample count when
a level first needs them.

## Shader files

`shaders/frame.ts` (the `Frame` uniform block, shared by every scene pipeline, the terrain
attribute bit layout, sRGB helpers), `shaders/lighting.ts`, `shaders/terrain.ts`,
`shaders/objects.ts` (instanced and translucent), `shaders/post.ts`. Each package edits its own
file (VQ3b terrain, VQ5 particles, VQ6 lighting, VQ7 sky); keep `FRAME_UNIFORM_FLOATS` and
`Renderer.writeFrameUniforms` in sync with the `Frame` struct.

The object shaders include `models/shading.ts` (VQ4): `shade_model` picks a material by the
mesh vertex's part (`models/kit.ts` `PART_*`, uploaded as the mesh's `team` buffer at vertex
buffer slot 3, location 6); instance floats 14 and 15 carry the engine glow (0 idle .. 1 boost)
and the nav-light blink phase. `PART_PLAIN` (untagged meshes) keeps the generic `shade` look.
Material colour literals are display values linearised at build time (`lin(...)`); the engine,
nav-light and lamp strengths are HDR radiance constants in `models/shading.ts`.
