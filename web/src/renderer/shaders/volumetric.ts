// Volumetric sun shafts (VQ9, renderer/volumetric.ts): light scattered
// toward the camera by the air the sun reaches. At reduced resolution, each
// pixel marches its view ray up to the scene depth (or `vol.z` u) through the
// sun shadow map (shadow.ts) and sums the sunlit stretches, weighted by a
// Henyey-Greenstein phase (bright toward the sun) and the extinction to the
// camera. Where an opening lets the sun into a cave, the lit air between the
// shadowed walls reads as a shaft. A second pass adds it onto the resolved
// HDR scene with a small blur (the march is jittered per pixel).

import { SUN_SHADOW_WGSL } from "./shadow.ts";

/** Floats in the Volumetric uniform. */
export const VOLUMETRIC_UNIFORM_FLOATS = 32;

export function volumetricWgsl(multisampledDepth: boolean): string {
  const depthType = multisampledDepth ? "texture_depth_multisampled_2d" : "texture_depth_2d";
  return /* wgsl */ `
${SUN_SHADOW_WGSL}

struct Volumetric {
  cam: vec4f,       // xyz camera position, w near
  fwd: vec4f,       // xyz forward, w far
  right: vec4f,     // xyz right * tan(fov x / 2)
  up: vec4f,        // xyz up * tan(fov y / 2)
  sun: vec4f,       // xyz unit vector toward the sun
  sun_rad: vec4f,   // rgb sun radiance (linear) times the shaft strength
  size: vec4f,      // xy target size (px), zw full-resolution size (px)
  misc: vec4f,      // x frame jitter seed
};
@group(0) @binding(0) var<uniform> vol_u: Volumetric;
@group(0) @binding(1) var scene_depth: ${depthType};

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VsOut {
  let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  var out: VsOut;
  out.clip = vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
  out.uv = vec2f(uv.x, 1.0 - uv.y);
  return out;
}

// Interleaved gradient noise (Jimenez 2014): a per-pixel offset that a 2x2 blur averages out.
fn ign(p: vec2f) -> f32 {
  return fract(52.9829189 * fract(dot(p, vec2f(0.06711056, 0.00583715))));
}

@fragment
fn fs_march(in: VsOut) -> @location(0) vec4f {
  // Depth of the nearest full-resolution pixel under this one (min over a 2x2 footprint: the near side, so no shaft light shows over near rock).
  let full = vol_u.size.zw;
  let scale = full / vol_u.size.xy;
  let base = vec2i(floor(in.clip.xy * scale));
  let lim = vec2i(full) - vec2i(1);
  var d = 1.0;
  for (var k = 0; k < 4; k++) {
    let p = min(base + vec2i(k & 1, k >> 1), lim);
    d = min(d, textureLoad(scene_depth, p, 0));
  }
  let near = vol_u.cam.w;
  let far = vol_u.fwd.w;
  // Linear depth along the forward axis from the [0, 1] perspective depth (mat4.perspective).
  let lin = far * near / max(far - d * (far - near), 1e-6);
  let ndc = vec2f(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
  let ray = vol_u.fwd.xyz + vol_u.right.xyz * ndc.x + vol_u.up.xyz * ndc.y;
  let ray_len = length(ray);
  let dir = ray / ray_len;
  // Distance along the ray, capped at the march length.
  let t_end = min(lin * ray_len, sun_shadow.vol.z);
  let steps = max(sun_shadow.vol.w, 1.0);
  let dt = t_end / steps;
  let jitter = ign(in.clip.xy + vec2f(vol_u.misc.x * 5.588238, vol_u.misc.x * 3.1));
  let sigma = sun_shadow.vol.x;
  var acc = 0.0;
  let n = i32(steps);
  for (var i = 0; i < n; i++) {
    let t = (f32(i) + jitter) * dt;
    let p = vol_u.cam.xyz + dir * t;
    acc += sun_visibility_point(p) * exp(-sigma * t);
  }
  // Henyey-Greenstein, scaled so that isotropic scattering is 1.
  let g = sun_shadow.vol.y;
  let c = dot(dir, vol_u.sun.xyz);
  let phase = (1.0 - g * g) / pow(max(1.0 + g * g - 2.0 * g * c, 1e-4), 1.5);
  let light = vol_u.sun_rad.rgb * (sigma * phase * acc * dt);
  return vec4f(light, 0.0);
}
`;
}

/** Adds the shafts onto the HDR scene: a 2x2-texel tent of the low-resolution target (bilinear taps). */
export const VOLUMETRIC_COMPOSITE_WGSL = /* wgsl */ `
@group(0) @binding(0) var shafts: texture_2d<f32>;
@group(0) @binding(1) var shafts_samp: sampler;

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VsOut {
  let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  var out: VsOut;
  out.clip = vec4f(uv * 2.0 - 1.0, 0.0, 1.0);
  out.uv = vec2f(uv.x, 1.0 - uv.y);
  return out;
}

@fragment
fn fs_add(in: VsOut) -> @location(0) vec4f {
  let texel = 1.0 / vec2f(textureDimensions(shafts));
  var s = textureSampleLevel(shafts, shafts_samp, in.uv + vec2f(-0.75, -0.75) * texel, 0.0).rgb;
  s += textureSampleLevel(shafts, shafts_samp, in.uv + vec2f(0.75, -0.75) * texel, 0.0).rgb;
  s += textureSampleLevel(shafts, shafts_samp, in.uv + vec2f(-0.75, 0.75) * texel, 0.0).rgb;
  s += textureSampleLevel(shafts, shafts_samp, in.uv + vec2f(0.75, 0.75) * texel, 0.0).rgb;
  // Alpha 0 with "one" blending keeps the scene's alpha (the sky's mask).
  return vec4f(s * 0.25, 0.0);
}
`;
