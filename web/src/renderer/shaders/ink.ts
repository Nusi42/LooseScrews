// Ink pass (M4 A7, renderer/ink.ts): the line work of the Palette slider's
// ink look, drawn into a small rgba8 target that the composite
// (shaders/post.ts) lays over the graded image.
//
// - Outlines from the scene depth: the Laplacian of 1/depth over four axes
//   (1/depth is affine in screen space on any plane, so it is zero on flat
//   rock and large at creases and silhouettes). The measure is normalised to
//   a change of surface slope per pixel, so it does not depend on distance
//   or resolution. Silhouettes ink only their near side; concave creases
//   count only while they are small (the far side of a silhouette is a jump).
// - Hatching in world space, so it sticks to the rock: three families of
//   parallel planes (dot(p, D) = k x spacing) cut every surface in lines; a
//   family stays out where it runs nearly parallel to the surface (its lines
//   would grow into bands) and the other two cover. The spacing follows the
//   pixel footprint in octaves (every second line of one octave is the next
//   octave), blended, so the lines keep a constant screen density at any
//   distance without popping. Line width grows with darkness: layer one from
//   light grey, the cross layer in the shadows, the third in the deep dark.
//
// Output: r outline coverage, g hatch coverage (both already scaled by the
// look's strengths), b distance fade (0 near .. 1 lost in the fog), a sky.

/**
 * The cockpit hull frame's own depth range (hullFrame.ts FRAME_DEPTH_RANGE,
 * FRAME_NEAR, FRAME_FAR; ink.test.ts keeps them equal): its depth is squeezed
 * into the front of the shared buffer, so the ink pass decodes it with its
 * own projection and hatches it in camera space (it flies with the camera).
 */
export const INK_FRAME_DEPTH = { range: 0.001, near: 0.01, far: 4 } as const;

/** Floats in the Ink uniform. */
export const INK_UNIFORM_FLOATS = 24;

/**
 * Hatch plane normals (world space, unit). Not axis aligned: mine shafts are
 * axis aligned and would show one family as flat bands.
 */
export const HATCH_DIRECTIONS: ReadonlyArray<readonly [number, number, number]> = [
  normalize3(1, 0.6, 0.35),
  normalize3(-0.35, 0.75, 1),
  normalize3(0.6, -1, 0.5),
];

/** Tone (0 black .. 1 white, display) below which each hatch layer starts. */
export const HATCH_THRESHOLDS: readonly [number, number, number] = [0.62, 0.42, 0.24];

function normalize3(x: number, y: number, z: number): readonly [number, number, number] {
  const l = Math.hypot(x, y, z);
  return [x / l, y / l, z / l];
}

const v3 = (d: readonly [number, number, number]): string => `vec3f(${d.map((c) => c.toFixed(6)).join(", ")})`;

export function inkWgsl(multisampledDepth: boolean): string {
  const depthType = multisampledDepth ? "texture_depth_multisampled_2d" : "texture_depth_2d";
  const [d0, d1, d2] = HATCH_DIRECTIONS as [readonly [number, number, number], readonly [number, number, number], readonly [number, number, number]];
  const [t0, t1, t2] = HATCH_THRESHOLDS;
  return /* wgsl */ `
struct Ink {
  cam: vec4f,    // xyz camera position, w near
  fwd: vec4f,    // xyz forward, w far
  right: vec4f,  // xyz right * tan(fov x / 2), w = 2 tan(fov y / 2) / height (u per px per u of depth)
  up: vec4f,     // xyz up * tan(fov y / 2), w = line scale (height / 1080, at least 1)
  look: vec4f,   // x outline strength, y hatch strength, z exposure, w fog density
  size: vec4f,   // xy target size (px), z hatch spacing (px), w tan(fov y / 2)
};
@group(0) @binding(0) var<uniform> ink: Ink;
@group(0) @binding(1) var scene_depth: ${depthType};
@group(0) @binding(2) var scene: texture_2d<f32>;

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
  var out: VsOut;
  out.clip = vec4f(p, 0.0, 1.0);
  out.uv = vec2f(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
  return out;
}

fn depth_at(p: vec2i) -> f32 {
  let q = clamp(p, vec2i(0), vec2i(ink.size.xy) - vec2i(1));
  return textureLoad(scene_depth, q, 0);
}

// 1 / linear depth from the [0, 1] perspective depth (mat4.perspective);
// the cockpit frame's squeezed range with its own near and far planes.
fn inv_lin(d: f32) -> f32 {
  let frame = d < ${INK_FRAME_DEPTH.range};
  let n = select(ink.cam.w, ${INK_FRAME_DEPTH.near}, frame);
  let f = select(ink.fwd.w, ${INK_FRAME_DEPTH.far}.0, frame);
  let dd = select(d, d / ${INK_FRAME_DEPTH.range}, frame);
  return max(f - dd * (f - n), 1e-7) / (f * n);
}

// Signed relative Laplacian of 1/depth along o: > 0 where the centre is
// nearer than its neighbours' mean (convex crease, near side of a silhouette).
fn lap(c: vec2i, o: vec2i, w0: f32) -> f32 {
  let wa = inv_lin(depth_at(c + o));
  let wb = inv_lin(depth_at(c - o));
  return (2.0 * w0 - wa - wb) / w0;
}

// Coverage of one hatch family at world position p: lines where
// dot(p, dir) is a multiple of the spacing, half width hw px.
fn hatch_layer(p: vec3f, dir: vec3f, sa: f32, blend_b: f32, hw: f32, target_px: f32) -> f32 {
  let x = dot(p, dir);
  let g = max(length(vec2f(dpdx(x), dpdy(x))), 1e-7);
  // Screen spacing of this family's lines (px): too far apart = nearly parallel to the surface.
  let px_a = sa / g;
  let keep = 1.0 - smoothstep(2.2 * target_px, 3.5 * target_px, px_a);
  let da = abs(fract(x / sa + 0.5) - 0.5) * px_a;
  let db = abs(fract(x / (2.0 * sa) + 0.5) - 0.5) * 2.0 * px_a;
  let la = clamp(hw + 0.5 - da, 0.0, 1.0);
  let lb = clamp(hw + 0.5 - db, 0.0, 1.0);
  // Lines closer than 2 px alias: they fade to their mean coverage instead.
  let fine = smoothstep(1.6, 3.0, px_a);
  let mean = min(1.0, (2.0 * hw + 1.0) / max(px_a, 1e-3));
  return keep * mix(mean, mix(la, lb, blend_b), fine);
}

fn tone_of(rgb: vec3f) -> f32 {
  // About where the composite's AgX puts the pixel (display, 0..1).
  let l = max(dot(rgb, vec3f(0.2126, 0.7152, 0.0722)), 0.0) * ink.look.z;
  return pow(1.0 - exp(-2.2 * l), 0.4545);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let c = vec2i(in.clip.xy);
  let s = i32(max(1.0, round(ink.up.w)));
  let d0 = depth_at(c);
  let sky = select(0.0, 1.0, d0 >= 1.0);
  let w0 = inv_lin(d0);
  let lin = 1.0 / w0;

  // Outlines. Slope change per pixel: lap x height / (2 s tan(fov y / 2)).
  let norm = ink.size.y / (2.0 * f32(s) * ink.size.w);
  let lx = lap(c, vec2i(s, 0), w0) * norm;
  let ly = lap(c, vec2i(0, s), w0) * norm;
  let la = lap(c, vec2i(s, s), w0) * norm * 0.7071;
  let lb = lap(c, vec2i(s, -s), w0) * norm * 0.7071;
  let convex = max(max(lx, ly), max(la, lb));
  let concave = max(max(-lx, -ly), max(-la, -lb));
  var outline = max(smoothstep(0.7, 1.8, convex), smoothstep(1.0, 2.4, concave) * (1.0 - smoothstep(6.0, 12.0, concave)));
  // Lines thin out into the fog like the rock does.
  let fd = ink.look.w * lin;
  let fade = exp2(-fd * fd * 0.8);
  outline *= fade * (1.0 - sky);

  // Hatching (world space).
  let ndc = vec2f(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
  let ray = ink.fwd.xyz + ink.right.xyz * ndc.x + ink.up.xyz * ndc.y;
  // The cockpit frame is hatched in camera space (it moves with the camera), the world in world space.
  let cam_p = vec3f(ndc.x * length(ink.right.xyz), ndc.y * ink.size.w, 1.0) * lin;
  let p = select(ink.cam.xyz + ray * min(lin, ink.fwd.w), cam_p, d0 < ${INK_FRAME_DEPTH.range});
  let foot = lin * ink.right.w;
  let target_px = ink.size.z * ink.up.w;
  let lvl = log2(max(target_px * foot, 1e-5) / 0.05);
  let sa = 0.05 * exp2(floor(lvl));
  let blend_b = fract(lvl);
  let tone = tone_of(textureLoad(scene, c, 0).rgb);
  let maxw = 0.42 * target_px;
  let h0 = hatch_layer(p, ${v3(d0)}, sa, blend_b, maxw * smoothstep(${t0}, ${t0 - 0.3}, tone), target_px) * smoothstep(${t0}, ${t0 - 0.06}, tone);
  let h1 = hatch_layer(p, ${v3(d1)}, sa, blend_b, maxw * smoothstep(${t1}, ${t1 - 0.3}, tone), target_px) * smoothstep(${t1}, ${t1 - 0.06}, tone);
  let h2 = hatch_layer(p, ${v3(d2)}, sa, blend_b, maxw * smoothstep(${t2}, ${t2 - 0.24}, tone), target_px) * smoothstep(${t2}, ${t2 - 0.06}, tone);
  let hatch = max(h0, max(h1, h2)) * fade * (1.0 - sky);

  return vec4f(outline * ink.look.x, hatch * ink.look.y, 1.0 - fade * (1.0 - sky), sky);
}
`;
}
