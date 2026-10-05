// Sun shadow map (VQ9, renderer/sunShadow.ts): rock blocks the sun. One
// orthographic depth map from the sun over the whole level, rendered from
// the terrain chunks' back faces (seen from the sun) and updated only where
// chunks change. Scene shaders sample it through `sun_visibility` (lighting.ts
// multiplies the sun with it); the volumetric pass marches it along view rays.
//
// Bindings 8-10 of group 0 (next to the Frame uniform, the lights' 1-3 and
// the environment cube's 6-7): renderer/sunShadow.ts SUN_SHADOW_BINDINGS.

/** Floats in the SunShadow uniform (clip matrix, params, vol). */
export const SUN_SHADOW_UNIFORM_FLOATS = 24;

/** The SunShadow struct, its bindings and `sun_visibility` (included by lighting.ts). */
export const SUN_SHADOW_WGSL = /* wgsl */ `
struct SunShadow {
  // World to shadow clip space: x, y in -1..1 over the level, z depth 0..1 (0 nearest the sun).
  clip: mat4x4f,
  // x mode (0 = no shadow map: the sun is never blocked; 1 = the map; -1 =
  // sealed level: rock all around, the sun never reaches in), y PCF radius (uv),
  // z normal offset (u), w depth bias (depth units).
  params: vec4f,
  // Volumetric sun shafts (shadow.ts VOLUMETRIC_WGSL): x scattering per u, y
  // phase asymmetry g, z march length (u), w steps.
  vol: vec4f,
};
@group(0) @binding(8) var<uniform> sun_shadow: SunShadow;
@group(0) @binding(9) var sun_shadow_map: texture_depth_2d;
@group(0) @binding(10) var sun_shadow_samp: sampler_comparison;

// Shadow map uv (xy) and depth (z) of a world position.
fn sun_shadow_coord(p: vec3f) -> vec3f {
  let q = sun_shadow.clip * vec4f(p, 1.0);
  return vec3f(q.x * 0.5 + 0.5, 0.5 - q.y * 0.5, q.z);
}

// 0 (rock between the point and the sun) .. 1 (sunlit); soft edges from an
// 8-tap PCF over bilinear compares. Outside the map (beyond the level) 1.
fn sun_visibility(p: vec3f, n: vec3f) -> f32 {
  if (sun_shadow.params.x < -0.5) { return 0.0; }
  if (sun_shadow.params.x < 0.5) { return 1.0; }
  let c = sun_shadow_coord(p + n * sun_shadow.params.z);
  if (any(c.xy <= vec2f(0.0)) || any(c.xy >= vec2f(1.0)) || c.z >= 1.0) { return 1.0; }
  let z = c.z - sun_shadow.params.w;
  let r = sun_shadow.params.y;
  var s = textureSampleCompareLevel(sun_shadow_map, sun_shadow_samp, c.xy + vec2f(-0.613, 0.162) * r, z);
  s += textureSampleCompareLevel(sun_shadow_map, sun_shadow_samp, c.xy + vec2f(0.170, -0.954) * r, z);
  s += textureSampleCompareLevel(sun_shadow_map, sun_shadow_samp, c.xy + vec2f(0.951, 0.287) * r, z);
  s += textureSampleCompareLevel(sun_shadow_map, sun_shadow_samp, c.xy + vec2f(-0.276, 0.874) * r, z);
  s += textureSampleCompareLevel(sun_shadow_map, sun_shadow_samp, c.xy + vec2f(0.442, 0.473) * r * 0.5, z);
  s += textureSampleCompareLevel(sun_shadow_map, sun_shadow_samp, c.xy + vec2f(-0.402, -0.556) * r * 0.5, z);
  s += textureSampleCompareLevel(sun_shadow_map, sun_shadow_samp, c.xy + vec2f(-0.927, -0.336) * r, z);
  s += textureSampleCompareLevel(sun_shadow_map, sun_shadow_samp, c.xy + vec2f(0.618, -0.598) * r, z);
  // Slightly sharpened: a soft but defined edge.
  return smoothstep(0.05, 0.95, s * 0.125);
}

// One bilinear compare (volumetric march): 1 sunlit, 0 in rock's shadow.
// Outside the map (beyond the level, open space) 0: no dust out there.
fn sun_visibility_point(p: vec3f) -> f32 {
  let c = sun_shadow_coord(p);
  if (any(c.xy <= vec2f(0.0)) || any(c.xy >= vec2f(1.0)) || c.z <= 0.0 || c.z >= 1.0) { return 0.0; }
  return textureSampleCompareLevel(sun_shadow_map, sun_shadow_samp, c.xy, c.z - sun_shadow.params.w);
}
`;

/**
 * Caster pass: chunk positions only, depth only (no fragment stage), and a
 * clear of the scissored rectangle (a full-screen triangle at depth 1).
 */
export const SHADOW_CASTER_WGSL = /* wgsl */ `
struct SunShadow {
  clip: mat4x4f,
  params: vec4f,
  vol: vec4f,
};
@group(0) @binding(0) var<uniform> sun_shadow: SunShadow;

@vertex
fn vs_caster(@location(0) pos: vec3f) -> @builtin(position) vec4f {
  return sun_shadow.clip * vec4f(pos, 1.0);
}

@vertex
fn vs_clear(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return vec4f(uv * 2.0 - 1.0, 1.0, 1.0);
}
`;
