// Terrain chunk shader: rock materials, team paint with the colour-blind
// paint patterns, surface states (hot, charged, glowing, scorched), lit by
// lighting.ts and fogged. Output is linear HDR radiance (post.ts tonemaps).
// VQ3b (triplanar rock) replaces the material part of this file.

import { FRAME_WGSL, MATERIAL_SHIFT, PAINT_COLOR_SHIFT, SURFACE_SHIFT, TEAM_SLOTS } from "./frame.ts";
import { LIGHTING_WGSL } from "./lighting.ts";

export const TERRAIN_WGSL = /* wgsl */ `
${FRAME_WGSL}
${LIGHTING_WGSL}

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) world_pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) @interpolate(flat) team: u32,
};

@vertex
fn vs_main(@location(0) pos: vec3f, @location(1) normal: vec3f, @location(2) team: u32) -> VsOut {
  var out: VsOut;
  out.clip = frame.view_proj * vec4f(pos, 1.0);
  out.world_pos = pos;
  out.normal = normal;
  out.team = team;
  return out;
}

fn hash3(p: vec3f) -> f32 {
  return fract(sin(dot(p, vec3f(12.9898, 78.233, 37.719))) * 43758.5453);
}

// Cheap value-noise-ish variation so flat surfaces show motion parallax.
fn grain(p: vec3f) -> f32 {
  return hash3(floor(p * 0.5));
}

// Paint effect colours (PAINT_* 1..3): red bouncy, blue slippery, yellow sticky (display values).
fn paint_effect_color(c: u32) -> vec3f {
  if (c == 1u) { return vec3f(0.95, 0.18, 0.15); }
  if (c == 2u) { return vec3f(0.2, 0.45, 1.0); }
  return vec3f(1.0, 0.85, 0.15);
}

// Surface coordinates projected along the dominant normal axis, so the
// paint patterns below show on walls, floors and ceilings alike.
fn surface_uv(p: vec3f, n: vec3f) -> vec2f {
  let a = abs(n);
  if (a.x >= a.y && a.x >= a.z) { return p.yz; }
  if (a.y >= a.z) { return p.xz; }
  return p.xy;
}

// Each paint effect has its own shape, so it never relies on hue alone
// (colour blindness): red bouncy = diagonal stripes, blue slippery = wavy
// bands, yellow sticky = dots. Returns 1 inside a pattern mark.
fn paint_pattern(c: u32, uv: vec2f) -> f32 {
  if (c == 1u) { return step(0.62, fract((uv.x + uv.y) * 0.9)); }
  if (c == 2u) { return step(0.62, fract(uv.y * 0.8 + 0.3 * sin(uv.x * 2.2))); }
  let cell = fract(uv * 0.9) - vec2f(0.5);
  return 1.0 - step(0.24, length(cell));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let n = normalize(in.normal);
  let team = in.team & 0xffu;
  let surface = (in.team >> ${SURFACE_SHIFT}u) & 0xffu;
  let material = (in.team >> ${MATERIAL_SHIFT}u) & 0xffu;
  let paint_color = (in.team >> ${PAINT_COLOR_SHIFT}u) & 0xffu;
  let t = frame.camera_pos.w;
  var base = frame.rock_color.rgb * (0.8 + 0.4 * grain(in.world_pos));
  // Rock materials: weak rock is lighter, sandy and crumbly; hard rock is
  // darker, cooler and speckled with glints.
  if (material == 1u) {
    let crumble = hash3(floor(in.world_pos * 2.0));
    base = base * vec3f(1.4, 1.15, 0.78) * (0.85 + 0.3 * crumble);
  } else if (material == 3u) {
    let glint = step(0.97, hash3(floor(in.world_pos * 4.0)));
    base = base * vec3f(0.5, 0.58, 0.78) + vec3f(0.12) * glint;
  }
  var emissive = vec3f(0.0);
  if (team < ${TEAM_SLOTS}u) {
    let tint = frame.team_colors[team].rgb;
    base = mix(base, tint, 0.85);
    emissive = tint * 0.3; // paint glows faintly so it reads in dark tunnels
    if (paint_color >= 1u && paint_color <= 3u) {
      // Coloured paint: the effect colour with the effect's pattern drawn
      // in a dark shade of the team colour. The marks contrast by
      // luminance, so the pattern (the effect) reads even where team and
      // effect hues match; the marks' hue gives the owner.
      let effect = srgb_to_linear(paint_effect_color(paint_color));
      let mark = paint_pattern(paint_color, surface_uv(in.world_pos, n));
      base = mix(effect, tint * 0.05, mark);
      emissive = effect * 0.4 * (1.0 - mark);
    }
  }
  // Surface states (M2): hot rock glows and pulses, charged rock sparks,
  // glowing rock lights up (HDR: these bloom).
  if ((surface & 1u) != 0u) {
    let pulse = 0.55 + 0.25 * sin(t * 3.0 + grain(in.world_pos) * 6.28);
    emissive += vec3f(1.0, 0.25, 0.03) * 3.0 * pulse;
  }
  if ((surface & 2u) != 0u) {
    let cell = floor(in.world_pos * 3.0);
    let spark = step(0.94, hash3(cell + vec3f(floor(t * 18.0))));
    emissive += vec3f(0.01, 0.04, 0.1) + vec3f(0.4, 0.75, 1.0) * spark * 5.0;
  }
  if ((surface & 4u) != 0u) {
    emissive += vec3f(0.3, 1.0, 0.5) * 1.2;
  }
  if ((surface & 8u) != 0u) {
    // Permanent scorch: charred and sooty.
    base = base * (0.12 + 0.08 * grain(in.world_pos * 3.0));
  }
  let color = base * light(n, in.world_pos) + emissive;
  return vec4f(apply_fog(color, in.world_pos), 1.0);
}
`;
