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

/** Size of the Frame uniform struct in f32 units (192 bytes). */
export const FRAME_UNIFORM_FLOATS = 48;

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
};
@group(0) @binding(0) var<uniform> frame: Frame;

// Display (sRGB-encoded) colour to linear, exact piecewise curve.
fn srgb_to_linear(c: vec3f) -> vec3f {
  let lo = c / 12.92;
  let hi = pow((max(c, vec3f(0.0)) + 0.055) / 1.055, vec3f(2.4));
  return select(hi, lo, c <= vec3f(0.04045));
}

fn apply_fog(color: vec3f, world_pos: vec3f) -> vec3f {
  let d = distance(world_pos, frame.camera_pos.xyz);
  let fd = frame.fog_color.w * d;
  let fog = 1.0 - exp2(-fd * fd);
  return mix(color, frame.fog_color.rgb, clamp(fog, 0.0, 1.0));
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
