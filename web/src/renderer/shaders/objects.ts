// Instanced objects (ships, projectiles, pickups, debris, wrecks, ...): an
// opaque pass and a translucent pass (dust volumes, ghosts, trails, glow
// shells, flashes) drawn after it without depth writes. Instance colours
// are display values (game/scene.ts); the emissive amount 0..1 maps to HDR
// radiance up to EMISSIVE_PEAK, so fully glowing bolts and flashes bloom.

import { FRAME_WGSL } from "./frame.ts";
import { LIGHTING_WGSL } from "./lighting.ts";

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
};

fn quat_rotate(q: vec4f, v: vec3f) -> vec3f {
  let t = 2.0 * cross(q.xyz, v);
  return v + q.w * t + cross(q.xyz, t);
}

// Lit colour blended toward glowing radiance by the emissive amount (see emissiveGain in objects.ts).
fn shade(albedo: vec3f, a: f32, n: vec3f, world_pos: vec3f) -> vec3f {
  let lit = albedo * light(n, world_pos);
  let glow = albedo * (1.0 + ${EMISSIVE_PEAK - 1}.0 * a * a);
  return mix(lit, glow, a);
}

@vertex
fn vs_main(
  @location(0) pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) offset_scale: vec4f, // xyz world position, w = uniform scale
  @location(3) rotation: vec4f,     // unit quaternion (x, y, z, w)
  @location(4) color: vec4f,        // rgb display colour, a = emissive amount 0..1
  @location(5) extra: vec4f,        // x = stretch along local Z, y = opacity
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
  return out;
}
`;

export const INSTANCED_WGSL = /* wgsl */ `
${FRAME_WGSL}
${LIGHTING_WGSL}
${INSTANCED_VS}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let n = normalize(in.normal);
  let color = shade(in.color.rgb, in.color.a, n, in.world_pos);
  return vec4f(apply_fog(color, in.world_pos), 1.0);
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

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4f {
  var n = normalize(in.normal);
  if (!front) { n = -n; }
  let v = normalize(frame.camera_pos.xyz - in.world_pos);
  let facing = abs(dot(n, v));
  let color = shade(in.color.rgb, in.color.a, n, in.world_pos);
  let alpha = clamp(in.opacity * (0.2 + 0.8 * facing * facing), 0.0, 1.0);
  return vec4f(apply_fog(color, in.world_pos), alpha);
}
`;
