// Per-frame uniform block shared by every scene pipeline (terrain, objects,
// later particles and lights). Keep FRAME_UNIFORM_FLOATS and the writer in
// renderer.ts (`writeFrameUniforms`) in sync with the struct.
//
// Colour convention (VQ2): the scene renders linear HDR radiance into an
// rgba16float target; post.ts tonemaps and encodes sRGB. Colours authored as
// display values (settings, team colours, instance colours, literals in the
// shaders) are converted with `srgb_to_linear` (WGSL) or `srgbToLinear` (TS)
// before lighting. Emissive light is in physical HDR units (> 1 blooms).

/** Number of paint team colour slots, indexed by team id; ids >= TEAM_SLOTS (NO_TEAM = 255) are unpainted. */
export const TEAM_SLOTS = 4;

/** Size of the Frame uniform struct in f32 units (240 bytes). */
export const FRAME_UNIFORM_FLOATS = 60;

/**
 * Open levels (VQ7 hides the sealing walls): rock fades into the sky over
 * this distance (u) before a hidden wall (pillars that run floor to
 * ceiling, rock ends cut by the level box), seen from at least
 * ROOM_FADE_NEAR away; closer up it stays solid (it still collides). VQP.
 */
export const ROOM_FADE = 3;
export const ROOM_FADE_NEAR = 12;
export const ROOM_FADE_FAR = 30;
/**
 * Rock this close (u) to a hidden wall is cut off at any distance (D5):
 * stripping the wall triangles (atmosphere.ts stripRoomWalls, 0.5 u) leaves
 * the triangles that bridge rock and wall as a sawtooth; the cut is the
 * smooth line where the rock crosses this plane instead.
 */
export const ROOM_CLIP = 0.75;

/** Mip of the environment cube the fog samples (16 texels per face: a smooth version of the sky; VQ7). */
export const FOG_ENV_LOD = 5;

/**
 * Terrain vertex attribute `team` (u32): bits 0-7 paint team (255 =
 * unpainted), bits 8-15 surface state (see SURFACE_* in mesh.ts), bits
 * 16-23 rock material plus one (MATERIAL_* + 1; 0 = unknown), bits 24-31
 * paint effect colour (PAINT_*).
 */
export const SURFACE_SHIFT = 8;
export const MATERIAL_SHIFT = 16;
export const PAINT_COLOR_SHIFT = 24;

/** The Frame struct, its binding and the helpers every scene shader uses. */
export const FRAME_WGSL = /* wgsl */ `
struct Frame {
  view_proj: mat4x4f,
  camera_pos: vec4f,   // xyz camera position, w = time in seconds
  light_dir: vec4f,    // xyz unit vector towards the sun, w = headlight strength
  fog_color: vec4f,    // rgb fog colour (linear), w = exp2 fog density
  rock_color: vec4f,   // rgb unpainted surface colour (linear)
  team_colors: array<vec4f, ${TEAM_SLOTS}>, // linear
  fog_height: vec4f,   // height fog: x density at z (world y), y falloff per u upwards; w = weight of the sky's haze (0 = flat fog_color)
  room_min: vec4f,     // open levels: the room box's inner faces (hidden walls); w = fade width (u), 0 = no fade
  room_max: vec4f,
};
@group(0) @binding(0) var<uniform> frame: Frame;
// The world's environment cube (renderer/sky.ts, VQ7): the fog takes its colour from it.
@group(0) @binding(6) var env_cube: texture_cube<f32>;
@group(0) @binding(7) var env_samp: sampler;

// Display (sRGB-encoded) colour to linear, exact piecewise curve.
fn srgb_to_linear(c: vec3f) -> vec3f {
  let lo = c / 12.92;
  let hi = pow((max(c, vec3f(0.0)) + 0.055) / 1.055, vec3f(2.4));
  return select(hi, lo, c <= vec3f(0.04045));
}

// Open levels: coverage of rock near a hidden wall of the room box (1 = solid,
// fading to 0 at the wall), only for rock seen from afar (VQP).
fn room_fade(world_pos: vec3f) -> f32 {
  let w = frame.room_min.w;
  if (w <= 0.0) { return 1.0; }
  let lo = world_pos - frame.room_min.xyz;
  let hi = frame.room_max.xyz - world_pos;
  let d = min(min(min(lo.x, hi.x), min(lo.y, hi.y)), min(lo.z, hi.z));
  // A ragged edge: the fade line wanders by up to a third of its width.
  let wobble = 0.33 * w * sin(world_pos.x * 0.61 + world_pos.z * 0.37) * sin(world_pos.z * 0.53 - world_pos.y * 0.29);
  let fade = smoothstep(0.0, w, d + wobble);
  let far = smoothstep(${ROOM_FADE_NEAR}.0, ${ROOM_FADE_FAR}.0, distance(world_pos, frame.camera_pos.xyz));
  return mix(1.0, fade, far) * step(${ROOM_CLIP}, d);
}

// exp2-squared distance fog plus an exponential height fog (integrated
// along the view ray), coloured by the sky's haze in the view direction
// (tinted toward the sun and the nebula) or by the flat fog colour.
fn apply_fog(color: vec3f, world_pos: vec3f) -> vec3f {
  let v = world_pos - frame.camera_pos.xyz;
  let d = length(v);
  let fd = frame.fog_color.w * d;
  var trans = exp2(-fd * fd);
  let h = frame.fog_height;
  if (h.x > 0.0) {
    let k = max(h.y * v.y, -60.0);
    let ratio = select((1.0 - exp(-k)) / k, 1.0, abs(k) < 1e-3);
    trans *= exp(-h.x * d * exp(-h.y * (frame.camera_pos.y - h.z)) * ratio);
  }
  let dir = v / max(d, 1e-4);
  let haze = mix(frame.fog_color.rgb, textureSampleLevel(env_cube, env_samp, dir, ${FOG_ENV_LOD}.0).rgb, h.w);
  return mix(color, haze, clamp(1.0 - trans, 0.0, 1.0));
}
`;

/** Display (sRGB-encoded) channel value to linear (same curve as the WGSL `srgb_to_linear`). */
export function srgbToLinear(c: number): number {
  return c <= 0.04045 ? c / 12.92 : Math.pow((Math.max(c, 0) + 0.055) / 1.055, 2.4);
}

/** Linear channel value to display (sRGB-encoded), the inverse of srgbToLinear. */
export function linearToSrgb(c: number): number {
  return c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(Math.max(c, 0), 1 / 2.4) - 0.055;
}
