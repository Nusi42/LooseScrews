// Scene lighting shared by terrain and objects, in linear HDR units: sun,
// hemisphere ambient, the camera headlight (a soft spot light along the
// view) and up to MAX_LIGHTS dynamic point lights (VQ6) from the clustered
// grid built by the "lights" compute pass (shaders/lights.ts). Terrain and
// objects call `light(n, world_pos)` (diffuse); materials with a highlight
// call `light_spec(n, world_pos, v, power, &spec)`, which also returns the
// dynamic lights' Blinn-Phong highlight in `spec`.
//
// Bindings 1-3 of group 0 (next to the Frame uniform): renderer/lights.ts
// LIGHT_BIND_ENTRIES. Point light falloff: intensity / (1 + d^2), windowed
// to 0 at the light's range by (1 - (d / range)^4)^2.

import { CLUSTER_COUNT, CLUSTER_X, CLUSTER_Y, LIGHT_STRUCTS_WGSL, MAX_LIGHTS } from "./lights.ts";
import { SUN_SHADOW_WGSL } from "./shadow.ts";

export const LIGHTING_WGSL = /* wgsl */ `
${LIGHT_STRUCTS_WGSL}

@group(0) @binding(1) var<uniform> light_info: LightInfo;
@group(0) @binding(2) var<storage, read> point_lights: array<PointLight, ${MAX_LIGHTS}>;
@group(0) @binding(3) var<storage, read> light_masks: array<vec4<u32>, ${CLUSTER_COUNT}>;
${SUN_SHADOW_WGSL}

// Sun visibility (VQ9 shadow map) of the last static_light() call, for
// materials that add their own sun highlight after lighting (terrain GGX,
// model_spec).
var<private> sun_vis_last: f32 = 1.0;

// Froxel of a world position (CPU mirror: renderer/lights.ts clusterOf).
fn cluster_index(world_pos: vec3f) -> u32 {
  let clip = frame.view_proj * vec4f(world_pos, 1.0);
  let depth = max(clip.w, 1e-4);
  let ndc = clip.xy / depth;
  let tx = u32(clamp(floor((ndc.x * 0.5 + 0.5) * ${CLUSTER_X}.0), 0.0, ${CLUSTER_X - 1}.0));
  let ty = u32(clamp(floor((ndc.y * 0.5 + 0.5) * ${CLUSTER_Y}.0), 0.0, ${CLUSTER_Y - 1}.0));
  let s = cluster_slice(depth, light_info);
  return (s * CLUSTER_Y + ty) * CLUSTER_X + tx;
}

// Diffuse (return value) and, when power > 0, Blinn-Phong highlight (*spec)
// of the dynamic point lights at a surface point.
fn dynamic_lights(n: vec3f, world_pos: vec3f, v: vec3f, power: f32, spec: ptr<function, vec3f>) -> vec3f {
  var sum = vec3f(0.0);
  if (light_info.params.x < 0.5) { return sum; }
  let mask = light_masks[cluster_index(world_pos)];
  for (var w = 0u; w < 4u; w++) {
    var bits = mask[w];
    loop {
      if (bits == 0u) { break; }
      let i = w * 32u + firstTrailingBit(bits);
      bits = bits & (bits - 1u);
      let lp = point_lights[i].pos;
      let to = lp.xyz - world_pos;
      let d2 = dot(to, to);
      let x = d2 / (lp.w * lp.w);
      if (x >= 1.0) { continue; }
      let win = 1.0 - x * x;
      let att = win * win / (1.0 + d2);
      let l = to * inverseSqrt(max(d2, 1e-6));
      // Slightly wrapped diffuse: lights grazing a wall still tint it.
      let ndl = clamp((dot(n, l) + 0.2) / 1.2, 0.0, 1.0);
      let c = point_lights[i].color.rgb * att;
      sum += c * ndl;
      if (power > 0.0) {
        let h = normalize(l + v);
        *spec += c * pow(max(dot(n, h), 0.0), power) * ndl;
      }
    }
  }
  return sum;
}

// Sun (blocked by rock: the VQ9 shadow map), ambient and the headlight (no dynamic lights).
fn static_light(n: vec3f, world_pos: vec3f) -> vec3f {
  // Rock blocks the sun (VQ9).
  sun_vis_last = sun_visibility(world_pos, n);
  let sun = max(dot(n, frame.light_dir.xyz), 0.0) * vec3f(1.0, 0.93, 0.8) * sun_vis_last;
  // Hemisphere ambient: cool sky light from above, dim bounce from below.
  let ambient = mix(vec3f(0.035, 0.035, 0.05), vec3f(0.11, 0.12, 0.16), n.y * 0.5 + 0.5);
  // Headlight so tunnels stay readable: a soft spot light along the view
  // (a dimmer fill outside the cone), falling off with distance.
  let to_cam = frame.camera_pos.xyz - world_pos;
  let d = length(to_cam);
  let l = to_cam / max(d, 1e-4);
  var head = frame.light_dir.w * max(dot(n, l), 0.0);
  if (light_info.params.w > 0.5) {
    let cone = smoothstep(light_info.spot.w, light_info.spot2.x, dot(-l, light_info.spot.xyz));
    head *= mix(light_info.spot2.y, light_info.spot2.w, cone) / (1.0 + light_info.spot2.z * d * d);
  } else {
    head /= 1.0 + 0.004 * d * d;
  }
  return ambient + sun * 0.9 + vec3f(1.0, 0.97, 0.92) * head;
}

fn light(n: vec3f, world_pos: vec3f) -> vec3f {
  var spec = vec3f(0.0);
  return static_light(n, world_pos) + dynamic_lights(n, world_pos, vec3f(0.0, 0.0, 1.0), 0.0, &spec);
}

// light() plus the dynamic lights' highlight (exponent power) in *spec.
fn light_spec(n: vec3f, world_pos: vec3f, v: vec3f, power: f32, spec: ptr<function, vec3f>) -> vec3f {
  return static_light(n, world_pos) + dynamic_lights(n, world_pos, v, power, spec);
}
`;
