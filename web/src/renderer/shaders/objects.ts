// Instanced objects (ships, projectiles, pickups, debris, wrecks, ...): an
// opaque pass and a translucent pass (dust volumes, ghosts, trails, glow
// shells, flashes) drawn after it without depth writes. Instance colours
// are display values (game/scene.ts); the emissive amount 0..1 maps to HDR
// radiance up to EMISSIVE_PEAK, so fully glowing bolts and flashes bloom.

import { FRAME_WGSL } from "./frame.ts";
import { LIGHTING_WGSL } from "./lighting.ts";
import { MODEL_SHADING_WGSL } from "../models/shading.ts";
import { KEEP_ALPHA } from "./post.ts";

/** HDR radiance of a fully emissive instance (emissive = 1), in multiples of its colour. */
export const EMISSIVE_PEAK = 6;

/** HDR gain of an instance with emissive amount `a` (0..1): 1 at 0, EMISSIVE_PEAK at 1, quadratic between. */
export function emissiveGain(a: number): number {
  return 1 + (EMISSIVE_PEAK - 1) * a * a;
}

const INSTANCED_VS = /* wgsl */ `
struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) world_pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) color: vec4f,
  @location(3) opacity: f32,
  // VQ4 model materials: part (kit.ts PART_*), model-space position and
  // normal (panel seams), x = engine glow, y = nav light phase.
  @location(4) @interpolate(flat) part: u32,
  @location(5) local_pos: vec3f,
  @location(6) local_normal: vec3f,
  @location(7) fx: vec2f,
};

fn quat_rotate(q: vec4f, v: vec3f) -> vec3f {
  let t = 2.0 * cross(q.xyz, v);
  return v + q.w * t + cross(q.xyz, t);
}

// Glowing radiance of an instance with emissive amount a (see emissiveGain in objects.ts).
fn emissive_glow(albedo: vec3f, a: f32) -> vec3f {
  return albedo * (1.0 + ${EMISSIVE_PEAK - 1}.0 * a * a);
}

// Lit colour blended toward glowing radiance by the emissive amount.
fn shade(albedo: vec3f, a: f32, n: vec3f, world_pos: vec3f) -> vec3f {
  // Fully glowing (bolts, flashes, shells): no lighting to blend in.
  if (a >= 0.999) { return emissive_glow(albedo, a); }
  let lit = albedo * light(n, world_pos);
  return mix(lit, emissive_glow(albedo, a), a);
}

@vertex
fn vs_main(
  @location(0) pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) offset_scale: vec4f, // xyz world position, w = uniform scale
  @location(3) rotation: vec4f,     // unit quaternion (x, y, z, w)
  @location(4) color: vec4f,        // rgb display colour, a = emissive amount 0..1
  @location(5) extra: vec4f,        // x = stretch along local Z, y = opacity, z = glow, w = blink phase
  @location(6) part: u32,           // mesh vertex part (low byte; models/kit.ts PART_*)
) -> VsOut {
  var out: VsOut;
  let stretch = max(extra.x, 1e-3);
  let scaled = vec3f(pos.xy, pos.z * stretch) * offset_scale.w;
  let world = quat_rotate(rotation, scaled) + offset_scale.xyz;
  out.clip = frame.view_proj * vec4f(world, 1.0);
  out.world_pos = world;
  // Non-uniform stretch along Z: scale the normal's Z inversely.
  out.normal = quat_rotate(rotation, normalize(vec3f(normal.xy, normal.z / stretch)));
  out.color = vec4f(srgb_to_linear(color.rgb), color.a);
  out.opacity = extra.y;
  out.part = part & 0xffu;
  out.local_pos = pos;
  out.local_normal = normal;
  out.fx = extra.zw;
  return out;
}
`;

export const INSTANCED_WGSL = /* wgsl */ `
${FRAME_WGSL}
${LIGHTING_WGSL}
${INSTANCED_VS}
${MODEL_SHADING_WGSL}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let n = normalize(in.normal);
  let color = shade_model(in.part, n, in.world_pos, in.local_pos, normalize(in.local_normal), in.color, in.fx.x, in.fx.y);
  // Alpha above 1 marks colour a monochrome grade keeps (Last Color Standing; the opacity slot carries 1 + keep).
  return vec4f(apply_fog(color, in.world_pos), 1.0 + clamp(in.opacity - 1.0, 0.0, 1.0) * ${KEEP_ALPHA});
}
`;

/**
 * Translucent instances, drawn after the opaque pass without depth writes.
 * Opacity is strongest where the surface faces the camera, so spheres read
 * as soft volumes.
 */
export const TRANSLUCENT_WGSL = /* wgsl */ `
${FRAME_WGSL}
${LIGHTING_WGSL}
${INSTANCED_VS}
${MODEL_SHADING_WGSL}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4f {
  var n = normalize(in.normal);
  if (!front) { n = -n; }
  let v = normalize(frame.camera_pos.xyz - in.world_pos);
  let facing = abs(dot(n, v));
  let color = shade_model(in.part, n, in.world_pos, in.local_pos, normalize(in.local_normal), in.color, in.fx.x, in.fx.y);
  let alpha = clamp(in.opacity * (0.2 + 0.8 * facing * facing), 0.0, 1.0);
  return vec4f(apply_fog(color, in.world_pos), alpha);
}
`;
