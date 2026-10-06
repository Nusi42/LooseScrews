// Particle shaders (visual-quality-plan.md VQ5): camera-facing billboards,
// velocity-stretched sprites, fixed-length ribbon sections and surface-
// oriented quads, pulled from a storage buffer of particles whose motion is
// analytic (position and velocity at any age from the spawn state and a
// drag constant, so no per-frame update pass is needed), plus ambient dust
// motes generated from the instance index in a volume that wraps around the
// camera.
//
// The pass draws into the resolved single-sample HDR scene texture after the
// scene passes, without a depth attachment: each fragment reads the scene
// depth (texture_depth_2d, or the first sample of the multisampled depth)
// and fades against it (soft particles), which also hides particles behind
// rock. One blend state (premultiplied alpha: one, one-minus-src-alpha)
// serves both kinds: additive particles write alpha 0, alpha-blended smoke
// writes its coverage.

import { FRAME_WGSL } from "./frame.ts";

/** f32 per particle in the storage buffer (six vec4f). */
export const PARTICLE_FLOATS = 24;

/** Flag bits in the particle's `flags` word (packed by particles.ts). */
export const PFLAG = {
  /** Bits 0-3: sprite layer (spriteAtlas.ts SPRITE). */
  spriteMask: 0xf,
  /** Alpha-blended (smoke, debris); otherwise additive light. */
  alpha: 1 << 4,
  /** Bits 5-6: orientation mode (ORIENT). */
  orientShift: 5,
  /** Flame heat shading: the sprite's heat channel shifts the colour to white-hot. */
  hot: 1 << 7,
} as const;

/** Orientation modes (flags bits 5-6). */
export const ORIENT = {
  /** Camera-facing quad, rotated by rot + spin * age. */
  billboard: 0,
  /** Stretched along the current velocity: length = max(size, speed * stretch). */
  velocity: 1,
  /** Stretched along the velocity direction to a fixed length `stretch` (u): trail and beam sections. */
  fixed: 2,
  /** Quad in the plane whose normal is the spawn velocity direction (shield ripples, ground rings). */
  plane: 3,
} as const;

/** f32 in the particle uniform block (PU). */
export const PARTICLE_UNIFORM_FLOATS = 32;

/** PU.pass_info.x: which particles a draw takes (half-resolution split, particles.ts). */
export const PASS_KIND = { all: 0, big: 1, small: 2 } as const;

/** WGSL of the particle and dust-mote pipelines; `multisampledDepth` picks the scene depth binding type. */
export function particlesWgsl(multisampledDepth: boolean): string {
  const depthType = multisampledDepth ? "texture_depth_multisampled_2d" : "texture_depth_2d";
  return /* wgsl */ `
${FRAME_WGSL}

struct Particle {
  a: vec4f, // xyz spawn position, w spawn time (s)
  b: vec4f, // xyz spawn velocity (u/s), w lifetime (s)
  c: vec4f, // rgb start radiance (linear), a start opacity
  d: vec4f, // rgb end radiance (linear), a end opacity
  e: vec4f, // x start size, y end size (u), z drag (1/s), w flags (an integer stored as f32: small bit patterns bit-cast would be denormals, which GPUs may flush)
  f: vec4f, // x stretch, y spin (rad/s), z start rotation (rad), w fade-in fraction of life
};

struct PU {
  cam_right: vec4f, // xyz camera right, w particle clock now (s)
  cam_up: vec4f,    // xyz camera up, w near plane
  cam_fwd: vec4f,   // xyz camera forward, w far plane
  counts: vec4f,    // x short-ring slots in use, y long-ring start, z long-ring slots in use, w capacity (immediates follow)
  cam_vel: vec4f,   // xyz camera velocity (u/s), w mote volume edge (u)
  mote: vec4f,      // x count, y size (u), z brightness, w streak seconds
  // x PASS_KIND (0 all, 1 only sprites at least y px wide, 2 only smaller ones),
  // y size threshold (px), z scene-depth texel per target pixel, w focal length (px)
  pass_info: vec4f,
  // Dust mote colour (M4 B5, the level's mood): rgb tint (radiance per unit brightness),
  // w how much of a pastel rainbow the motes take (giddy; a hue per mote).
  mote_color: vec4f,
};

@group(0) @binding(1) var<uniform> pu: PU;
@group(0) @binding(2) var<storage, read> parts: array<Particle>;
@group(0) @binding(3) var sprites: texture_2d_array<f32>;
@group(0) @binding(4) var samp: sampler;
@group(0) @binding(5) var scene_depth: ${depthType};

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) uv: vec2f,
  // rgb radiance (linear HDR), a opacity (alpha-blended) or 1 (additive)
  @location(1) color: vec4f,
  // x sprite layer, y 1 = alpha-blended, z soft-fade distance (u), w 1 = hot flame shading
  @location(2) @interpolate(flat) misc: vec4f,
  // view depth of the vertex (u along the camera's forward axis)
  @location(3) depth: f32,
};

const CORNERS = array<vec2f, 6>(
  vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
  vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0),
);

fn degenerate() -> VsOut {
  var o: VsOut;
  o.clip = vec4f(2.0, 2.0, 2.0, 1.0);
  o.uv = vec2f(0.0);
  o.color = vec4f(0.0);
  o.misc = vec4f(0.0);
  o.depth = 0.0;
  return o;
}

fn fog_amount(world_pos: vec3f) -> f32 {
  let d = distance(world_pos, frame.camera_pos.xyz);
  let fd = frame.fog_color.w * d;
  return clamp(1.0 - exp2(-fd * fd), 0.0, 1.0);
}

// Corner of a quad around pos (billboard, stretched or plane), in world space.
fn corner_world(mode: u32, corner: vec2f, pos: vec3f, vel: vec3f, spawn_vel: vec3f, size: f32, stretch: f32, ang: f32) -> vec3f {
  let cam = frame.camera_pos.xyz;
  if (mode == 0u) {
    let c = cos(ang);
    let s = sin(ang);
    let rc = vec2f(corner.x * c - corner.y * s, corner.x * s + corner.y * c);
    return pos + (pu.cam_right.xyz * rc.x + pu.cam_up.xyz * rc.y) * (size * 0.5);
  }
  if (mode == 3u) {
    let sl = length(spawn_vel);
    let n = select(pu.cam_fwd.xyz, spawn_vel / max(sl, 1e-9), sl > 1e-9);
    let helper = select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    let t = normalize(cross(n, helper));
    let b = cross(n, t);
    return pos + (t * corner.x + b * corner.y) * (size * 0.5);
  }
  let speed = length(vel);
  let dir = select(pu.cam_up.xyz, vel / max(speed, 1e-9), speed > 1e-9);
  var len = max(size, stretch);
  if (mode == 1u) {
    len = max(size, speed * stretch);
  }
  let to_cam = normalize(cam - pos);
  let sv = cross(dir, to_cam);
  let sl = length(sv);
  let side = select(pu.cam_right.xyz, sv / max(sl, 1e-9), sl > 1e-4);
  let stretched = pos + dir * (corner.y * len * 0.5) + side * (corner.x * size * 0.5);
  // Seen along its axis a stretched quad collapses to a line: turn it into a billboard.
  let billboard = pos + (pu.cam_right.xyz * corner.x + pu.cam_up.xyz * corner.y) * (size * 0.5);
  return mix(stretched, billboard, smoothstep(0.8, 0.97, abs(dot(dir, to_cam))));
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
  // Draw instance -> buffer slot (ParticleRing.slotOf): short ring, long ring, immediates.
  let short_used = u32(pu.counts.x);
  let long_used = u32(pu.counts.z);
  var idx = ii;
  if (ii >= short_used + long_used) {
    idx = u32(pu.counts.w) + (ii - short_used - long_used);
  } else if (ii >= short_used) {
    idx = u32(pu.counts.y) + (ii - short_used);
  }
  let p = parts[idx];
  let now = pu.cam_right.w;
  let age = now - p.a.w;
  let life = p.b.w;
  if (life <= 0.0 || age < 0.0 || age >= life) {
    return degenerate();
  }
  let u = age / life;
  let flags = u32(p.e.w + 0.5);
  let drag = p.e.z;
  var pos = p.a.xyz + p.b.xyz * age;
  var vel = p.b.xyz;
  if (drag > 0.0) {
    let k = exp(-drag * age);
    pos = p.a.xyz + p.b.xyz * ((1.0 - k) / drag);
    vel = p.b.xyz * k;
  }
  // Sizes ease out (fast growth first), colours and opacity are linear in age.
  let grow = 1.0 - (1.0 - u) * (1.0 - u);
  let size = mix(p.e.x, p.e.y, grow);
  // Flames cool quickly (white-hot only at the start), other colours fade linearly.
  let hot_flag = (flags & ${PFLAG.hot}u) != 0u;
  var col = mix(p.c.rgb, p.d.rgb, select(u, pow(u, 0.7), hot_flag));
  var alpha = mix(p.c.a, p.d.a, u);
  if (p.f.w > 0.0) {
    alpha = alpha * clamp(u / p.f.w, 0.0, 1.0);
    col = col * clamp(u / p.f.w, 0.0, 1.0);
  }
  let mode = (flags >> ${PFLAG.orientShift}u) & 3u;
  let corner = CORNERS[vi];
  let world = corner_world(mode, corner, pos, vel, p.b.xyz, size, p.f.x, p.f.z + p.f.y * age);

  if (mode == 2u) {
    // A trail or beam section seen end-on would stack dozens of sections on
    // the same pixels: dim sections that point at the camera.
    let to_cam = normalize(frame.camera_pos.xyz - pos);
    let al = abs(dot(normalize(p.b.xyz + vec3f(1e-9)), to_cam));
    let k = clamp(1.0 - al * al, 0.06, 1.0);
    col = col * k;
    alpha = alpha * k;
  }
  let is_alpha = (flags & ${PFLAG.alpha}u) != 0u;
  let view_center = dot(pos - frame.camera_pos.xyz, pu.cam_fwd.xyz);
  // Big sprites (most of the fill) draw into the half-resolution target, small ones at full resolution.
  let px = size * pu.pass_info.w / max(view_center, 0.05);
  let big = px >= pu.pass_info.y;
  if ((pu.pass_info.x == ${PASS_KIND.big}.0 && !big) || (pu.pass_info.x == ${PASS_KIND.small}.0 && big)) {
    return degenerate();
  }
  // Sprites closer than their own size fade out instead of filling the screen.
  let near_fade = smoothstep(0.08, 0.08 + 0.5 * size, view_center);
  let fog = fog_amount(pos);
  if (is_alpha) {
    col = mix(col, frame.fog_color.rgb, fog);
    alpha = alpha * near_fade;
  } else {
    col = col * (1.0 - fog) * near_fade;
    alpha = 1.0;
  }

  var o: VsOut;
  o.clip = frame.view_proj * vec4f(world, 1.0);
  o.uv = corner * 0.5 + 0.5;
  o.color = vec4f(col, alpha);
  let hot = select(0.0, 1.0, (flags & ${PFLAG.hot}u) != 0u);
  o.misc = vec4f(f32(flags & ${PFLAG.spriteMask}u), select(0.0, 1.0, is_alpha), max(0.05, size * 0.35), hot);
  o.depth = dot(world - frame.camera_pos.xyz, pu.cam_fwd.xyz);
  return o;
}

fn pcg(v: u32) -> u32 {
  let s = v * 747796405u + 2891336453u;
  let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (w >> 22u) ^ w;
}

fn hash3(i: u32) -> vec3f {
  let a = pcg(i);
  let b = pcg(a);
  let c = pcg(b);
  return vec3f(f32(a), f32(b), f32(c)) * (1.0 / 4294967296.0);
}

// Ambient dust motes: instance i sits at a fixed world position inside a
// cube of edge L that wraps around the camera, so motes stay put while the
// camera flies through them (motion parallax) and streak with its speed.
@vertex
fn vs_mote(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
  let h = hash3(ii * 3u + 17u);
  let l = pu.cam_vel.w;
  let cam = frame.camera_pos.xyz;
  let pos = cam + l * (fract(h - cam / l + vec3f(0.5)) - vec3f(0.5));
  let d = distance(pos, cam);
  let fade = (1.0 - smoothstep(0.3 * l, 0.5 * l, d)) * smoothstep(0.25, 1.2, d);
  let now = pu.cam_right.w;
  let twinkle = 0.55 + 0.45 * sin(now * (0.7 + 1.6 * h.x) + h.y * 6.2831);
  let size = pu.mote.y * (0.55 + 0.9 * h.z);
  // The camera's motion relative to the mote stretches it into a short streak.
  let rel = -pu.cam_vel.xyz;
  let corner = CORNERS[vi];
  let world = corner_world(1u, corner, pos, rel, rel, size, pu.mote.w, 0.0);
  let speed = length(rel);
  // A streak spreads the same light over a longer sprite.
  let spread = size / max(size, speed * pu.mote.w);
  let fog = fog_amount(pos);
  // The mood's tint, or a pastel hue per mote (giddy).
  let hue = h.x * 6.2831853;
  let rainbow = vec3f(0.62) + 0.38 * cos(vec3f(hue, hue - 2.094, hue + 2.094));
  let tint = mix(pu.mote_color.rgb, rainbow * 1.15, pu.mote_color.w);
  let col = tint * (pu.mote.z * twinkle * fade * spread * (1.0 - fog));
  var o: VsOut;
  o.clip = frame.view_proj * vec4f(world, 1.0);
  o.uv = corner * 0.5 + 0.5;
  o.color = vec4f(col, 1.0);
  o.misc = vec4f(0.0, 0.0, 0.3, 0.0);
  o.depth = dot(world - cam, pu.cam_fwd.xyz);
  return o;
}

// Coverage-weighted opacity of flame sprites (the rest of their light adds).
const FLAME_OPACITY = 0.45;

fn linear_depth(d: f32) -> f32 {
  let n = pu.cam_up.w;
  let f = pu.cam_fwd.w;
  return n * f / max(f - d * (f - n), 1e-6);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let t = textureSample(sprites, samp, in.uv, i32(in.misc.x + 0.5));
  let sd = textureLoad(scene_depth, vec2i(in.clip.xy * pu.pass_info.z), 0);
  // Soft particles: fade where the sprite meets rock, nothing behind it.
  let soft = clamp((linear_depth(sd) - in.depth) / in.misc.z, 0.0, 1.0);
  if (in.misc.y > 0.5) {
    let a = clamp(t.r * in.color.a * soft, 0.0, 1.0);
    return vec4f(in.color.rgb * t.g * a, a);
  }
  var rgb = in.color.rgb * (t.r * soft);
  if (in.misc.w > 0.5) {
    // Flame: the heat channel darkens the edges to deep red and whitens the core.
    let heat = t.g;
    let lum = dot(in.color.rgb, vec3f(0.2126, 0.7152, 0.0722));
    let h2 = heat * heat;
    rgb = rgb * (0.3 + 0.8 * heat) + vec3f(1.0, 0.78, 0.5) * (lum * t.r * soft * h2 * h2 * 0.7);
    // Flames are partly opaque: each puff hides some of the fire behind it, so
    // a fireball keeps its billows instead of adding up to a flat white disc.
    return vec4f(rgb, t.r * soft * FLAME_OPACITY);
  }
  return vec4f(rgb, 0.0);
}
`;
}

/**
 * Composites the half-resolution particle target (premultiplied colour and
 * coverage) onto the HDR scene: blend one, one-minus-src-alpha, bilinear.
 */
export const PARTICLE_COMPOSITE_WGSL = /* wgsl */ `
@group(0) @binding(0) var low: texture_2d<f32>;
@group(0) @binding(1) var low_sampler: sampler;

struct Out {
  @builtin(position) pos: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> Out {
  let p = vec2f(f32((vi << 1u) & 2u), f32(vi & 2u));
  var o: Out;
  o.pos = vec4f(p * 2.0 - 1.0, 0.0, 1.0);
  o.uv = vec2f(p.x, 1.0 - p.y);
  return o;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4f {
  return textureSample(low, low_sampler, in.uv);
}
`;
