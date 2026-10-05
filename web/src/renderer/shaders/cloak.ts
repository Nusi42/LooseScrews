// Cloak refraction (visual-quality plan VQP): a cloaked ship is drawn as a
// lens. After the scene (and the sky and light shafts) a copy of the HDR
// scene is taken, and the hull is drawn into the scene again sampling that
// copy through its surface: bent by the hull's normal (strongest at the
// silhouette, where real glass bends most), split slightly into its colour
// channels, plus a faint team-tinted rim and bands of shimmer that travel
// along the hull. Like the particles, the pass has no depth attachment:
// each fragment reads the scene depth (multisampled or not) and is dropped
// behind rock and the cockpit frame.

import { FRAME_WGSL } from "./frame.ts";

/** Floats in the cloak uniform block: camera right, up, viewport size and its inverse. */
export const CLOAK_UNIFORM_FLOATS = 12;

/** Screen-space bend (fraction of the viewport) at the hull's centre and its silhouette. */
export const CLOAK_BEND_CENTER = 0.016;
export const CLOAK_BEND_EDGE = 0.05;
/** HDR strength of the rim shimmer (times the tint). */
export const CLOAK_RIM = 0.6;

/** WGSL of the cloak pipeline; `multisampledDepth` picks the scene depth binding type. */
export function cloakWgsl(multisampledDepth: boolean): string {
  const depthType = multisampledDepth ? "texture_depth_multisampled_2d" : "texture_depth_2d";
  return /* wgsl */ `
${FRAME_WGSL}

struct Cloak {
  right: vec4f,   // camera right (world)
  up: vec4f,      // camera up (world)
  size: vec4f,    // viewport width, height, 1 / width, 1 / height (px)
};
@group(1) @binding(0) var<uniform> cloak: Cloak;
@group(1) @binding(1) var scene_copy: texture_2d<f32>;
@group(1) @binding(2) var copy_samp: sampler;
@group(1) @binding(3) var scene_depth: ${depthType};

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) world_pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) tint: vec3f,
  @location(3) strength: f32,
  @location(4) local_pos: vec3f,
  // Normal of a smooth ellipsoid around the hull: the lens bends the scene
  // smoothly instead of in facets (the hulls are flat-shaded parts).
  @location(5) lens_normal: vec3f,
};

fn quat_rotate(q: vec4f, v: vec3f) -> vec3f {
  let t = 2.0 * cross(q.xyz, v);
  return v + q.w * t + cross(q.xyz, t);
}

@vertex
fn vs_main(
  @location(0) pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) offset_scale: vec4f,
  @location(3) rotation: vec4f,
  @location(4) color: vec4f,
  @location(5) extra: vec4f,   // y = cloak strength 0..1
) -> VsOut {
  var out: VsOut;
  let world = quat_rotate(rotation, pos * offset_scale.w) + offset_scale.xyz;
  out.clip = frame.view_proj * vec4f(world, 1.0);
  out.world_pos = world;
  out.normal = quat_rotate(rotation, normal);
  out.tint = srgb_to_linear(color.rgb);
  out.strength = clamp(extra.y, 0.0, 1.0);
  out.local_pos = pos;
  out.lens_normal = quat_rotate(rotation, normalize(pos * vec3f(2.4, 3.2, 1.2) + vec3f(0.0, 0.0, 1e-4)));
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  // Behind rock, other ships or the cockpit frame: the scene stays.
  let sd = textureLoad(scene_depth, vec2i(in.clip.xy), 0);
  if (in.clip.z > sd) { discard; }
  let n = normalize(in.normal);
  let to_eye = frame.camera_pos.xyz - in.world_pos;
  let dist = length(to_eye);
  let v = to_eye / max(dist, 1e-4);
  let ndv = clamp(dot(n, v), 0.0, 1.0);
  let fres = pow(1.0 - ndv, 2.5);
  let t = frame.camera_pos.w;
  let s = in.strength;
  // The bend follows the normal on screen; a slow ripple keeps it alive.
  let ripple = 0.5 * sin(dot(in.local_pos, vec3f(23.0, 17.0, 31.0)) + t * 5.0)
             + 0.5 * sin(in.local_pos.z * 37.0 - t * 8.0);
  let ln = normalize(mix(normalize(in.lens_normal), n, 0.25));
  let bend = vec2f(dot(ln, cloak.right.xyz), -dot(ln, cloak.up.xyz));
  // Nearer hulls bend more of the screen (a lens of fixed world size).
  let near = clamp(6.0 / max(dist, 1.0), 0.35, 2.0);
  let lfres = pow(1.0 - clamp(abs(dot(ln, v)), 0.0, 1.0), 2.0);
  let amount = (${CLOAK_BEND_CENTER} + ${CLOAK_BEND_EDGE - CLOAK_BEND_CENTER} * lfres + 0.003 * ripple) * s * near;
  let uv = in.clip.xy * cloak.size.zw;
  let split = 0.12 * amount;
  let o = bend * amount;
  let d = bend * split;
  let r = textureSampleLevel(scene_copy, copy_samp, uv + o + d, 0.0).r;
  let g = textureSampleLevel(scene_copy, copy_samp, uv + o, 0.0).g;
  let b = textureSampleLevel(scene_copy, copy_samp, uv + o - d, 0.0).b;
  let behind = textureSampleLevel(scene_copy, copy_samp, uv, 0.0).rgb;
  var col = mix(behind, vec3f(r, g, b), s);
  // A faint tint through the hull and a shimmering rim (team-tinted, so
  // teammates and enemies read apart), with bands running along the hull.
  col *= mix(vec3f(1.0), 0.86 + 0.14 * in.tint, s);
  let band = pow(0.5 + 0.5 * sin(in.local_pos.z * 16.0 + in.local_pos.y * 5.0 - t * 6.5), 10.0);
  let flicker = 0.65 + 0.35 * sin(t * 11.0 + in.local_pos.x * 29.0);
  let fade = exp(-dist * 0.02);
  col += in.tint * ((fres * fres * flicker + band * (0.03 + 0.25 * fres)) * ${CLOAK_RIM} * s * fade);
  return vec4f(col, 1.0);
}
`;
}
