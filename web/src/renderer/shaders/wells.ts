// World effects pass (M4 B5; renderer/wells.ts): gravity wells, cave-in
// warnings and rubble, drawn per pixel against the scene depth after the
// sky and the mode effects and before the cloak and the particles.
//
// The pass reads a copy of the resolved HDR scene (only the screen rectangle
// the volumes cover is copied and drawn), so it can bend and recolour what
// is already there, not only add light:
//
// - WELL: gravitational lensing of everything behind the well's centre (the
//   background slides toward the centre by E^2 / b at impact parameter b,
//   E the lens radius, so a ring of magnified rock or sky forms around the
//   core and the far side shows inverted inside it), a photon ring, a soft
//   violet glow integrated along the view ray up to the scene depth (rock in
//   front hides it), and the core stone: darkened to polished black with
//   slowly pulsing violet veins and a violet rim.
// - CRACK: a cave-in's 1.5 s warning: glowing cracks open in the rock around
//   the spot (strongest on the ceiling, the rock facing down along the
//   level's up), widening and brightening as the warning runs out.
// - RUBBLE: a fresh cave-in's rubble: rock around the spot dusted pale (more
//   on faces turned up), cracks between the fallen stones; the fresh dust
//   fades to a lasting light coat.
//
// Everything fades with the scene's distance fog. Alpha (the sky mask and
// the colour keep mask) is copied through unchanged.

/** Most volumes per frame (extra ones are dropped, nearest first kept by the caller). */
export const WELLS_MAX_VOLUMES = 12;

/** Floats per volume (four vec4). */
export const WELLS_VOLUME_FLOATS = 16;

/** Floats of the header (five vec4) before the volumes. */
export const WELLS_HEADER_FLOATS = 20;

export const WELLS_UNIFORM_FLOATS = WELLS_HEADER_FLOATS + WELLS_MAX_VOLUMES * WELLS_VOLUME_FLOATS;

/** Volume kinds (`params.x`). */
export const WELLS_KIND = { well: 1, crack: 2, rubble: 3 } as const;

/** WGSL of the world effects pipeline; `multisampledDepth` picks the scene depth binding type. */
export function wellsWgsl(multisampledDepth: boolean): string {
  const depthType = multisampledDepth ? "texture_depth_multisampled_2d" : "texture_depth_2d";
  return /* wgsl */ `
struct Volume {
  pos: vec4f,    // xyz centre, w reach (well: lens and glow extent; crack, rubble: radius)
  color: vec4f,  // rgb linear colour, w intensity
  params: vec4f, // x kind; well: y core radius (0 = none), z lens radius E, w glow width
                 //         crack: y progress 0..1, z seed;  rubble: y freshness 0..1, z seed
  extra: vec4f,  // well: xyz spin axis, w phase;  crack, rubble: xyz the level's up
};

struct Wells {
  cam: vec4f,    // xyz camera position, w time (s)
  right: vec4f,  // xyz camera right x tan(fov x / 2), w near plane
  up: vec4f,     // xyz camera up x tan(fov y / 2), w far plane
  fwd: vec4f,    // xyz camera forward, w exp2 fog density
  info: vec4f,   // x volume count, y target width (px), z target height (px), w scene depth texel per target pixel
  vols: array<Volume, ${WELLS_MAX_VOLUMES}>,
};

// Pipeline variants: without wells (no lensing, no glow) or without cave-in
// volumes, the unused branches compile away.
override HAS_WELLS: bool = true;
override HAS_SURFACE: bool = true;

@group(0) @binding(0) var<uniform> fx: Wells;
@group(0) @binding(1) var scene_depth: ${depthType};
@group(0) @binding(2) var scene_copy: texture_2d<f32>;
@group(0) @binding(3) var copy_samp: sampler;

struct VsOut {
  @builtin(position) clip: vec4f,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
  var out: VsOut;
  out.clip = vec4f(p, 0.0, 1.0);
  return out;
}

const TAU = 6.2831853;

fn hash31(p: vec3f) -> f32 {
  var q = fract(p * vec3f(0.1031, 0.1030, 0.0973));
  q += dot(q, q.yzx + 33.33);
  return fract((q.x + q.y) * q.z);
}

fn hash33(p: vec3f) -> vec3f {
  var q = fract(p * vec3f(0.1031, 0.1030, 0.0973));
  q += dot(q, q.yxz + 33.33);
  return fract((q.xxy + q.yxx) * q.zyx);
}

fn vnoise(p: vec3f) -> f32 {
  let i = floor(p);
  let f = p - i;
  let u = f * f * (3.0 - 2.0 * f);
  let a = mix(mix(hash31(i), hash31(i + vec3f(1.0, 0.0, 0.0)), u.x), mix(hash31(i + vec3f(0.0, 1.0, 0.0)), hash31(i + vec3f(1.0, 1.0, 0.0)), u.x), u.y);
  let b = mix(mix(hash31(i + vec3f(0.0, 0.0, 1.0)), hash31(i + vec3f(1.0, 0.0, 1.0)), u.x), mix(hash31(i + vec3f(0.0, 1.0, 1.0)), hash31(i + vec3f(1.0, 1.0, 1.0)), u.x), u.y);
  return mix(a, b, u.z);
}

// 3D cells (Voronoi): x = distance to the nearest cell border (about F2 - F1), y = cell hash.
fn cells(p: vec3f) -> vec2f {
  let i = floor(p - 0.5);
  let f = p - i;
  var d1 = 8.0;
  var d2 = 8.0;
  var id = 0.0;
  for (var z = 0; z <= 1; z++) {
    for (var y = 0; y <= 1; y++) {
      for (var x = 0; x <= 1; x++) {
        let g = vec3f(f32(x), f32(y), f32(z));
        let h = hash33(i + g);
        let r = g + 0.15 + 0.7 * h - f;
        let d = dot(r, r);
        if (d < d1) {
          d2 = d1;
          d1 = d;
          id = h.x;
        } else if (d < d2) {
          d2 = d;
        }
      }
    }
  }
  return vec2f(sqrt(d2) - sqrt(d1), id);
}

fn erf_approx(x: f32) -> f32 {
  return tanh(1.2027606 * x + 0.0997 * x * x * x);
}

fn fog_at(t: f32) -> f32 {
  let fd = fx.fwd.w * t;
  return exp2(-fd * fd);
}

fn linear_depth(d: f32) -> f32 {
  let n = fx.right.w;
  let f = fx.up.w;
  return n * f / max(f - d * (f - n), 1e-6);
}

// Distance along the view axis of the scene at a target pixel (1e9 for sky).
fn scene_z(px: vec2f) -> f32 {
  let dims = vec2i(textureDimensions(scene_depth));
  let q = clamp(vec2i(px * fx.info.w), vec2i(0), dims - 1);
  let raw = textureLoad(scene_depth, q, 0);
  return select(1e9, linear_depth(raw), raw < 1.0);
}

// Target pixel of a world point (camera basis scaled by the half-FOV tangents).
fn to_pixel(p: vec3f) -> vec2f {
  let rel = p - fx.cam.xyz;
  let z = max(dot(rel, fx.fwd.xyz), 1e-4);
  let rl = fx.right.xyz;
  let ul = fx.up.xyz;
  let nx = dot(rel, rl) / dot(rl, rl) / z;
  let ny = dot(rel, ul) / dot(ul, ul) / z;
  return vec2f((nx * 0.5 + 0.5) * fx.info.y, (0.5 - ny * 0.5) * fx.info.z);
}

// Rotation of p about a unit axis by angle a (Rodrigues).
fn spin(p: vec3f, axis: vec3f, a: f32) -> vec3f {
  let c = cos(a);
  let s = sin(a);
  return p * c + cross(axis, p) * s + axis * dot(axis, p) * (1.0 - c);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let size = fx.info.yz;
  let pix = in.clip.xy;
  let base = textureLoad(scene_copy, vec2i(pix), 0);
  let ndc = vec2f(pix.x / size.x * 2.0 - 1.0, 1.0 - pix.y / size.y * 2.0);
  let dv = fx.fwd.xyz + ndc.x * fx.right.xyz + ndc.y * fx.up.xyz;
  let dl = length(dv);
  let dir = dv / dl;
  let cam = fx.cam.xyz;
  let t = fx.cam.w;
  let z = scene_z(pix);
  let hit = z < 1e8;
  let p = cam + dv * min(z, 1e5);
  // Surface normal from the depth (screen derivatives, in uniform control flow), facing the camera.
  let pn_raw = cross(dpdx(p), dpdy(p));
  var pn = pn_raw / max(length(pn_raw), 1e-12);
  if (dot(pn, dir) > 0.0) { pn = -pn; }
  let t_scene = select(1e9, z * dl, hit);
  // World size of one pixel per unit of distance.
  let px_per_t = 2.0 * length(fx.up.xyz) / size.y;
  let px = px_per_t * min(t_scene, 1e4);

  var lens = vec2f(0.0);
  var lens_min_t = 1e9;
  var add = vec3f(0.0);
  var col = base.rgb;
  let count = u32(fx.info.x);
  // Bending first: the lensed background is what the rest is laid over.
  for (var i = 0u; i < ${WELLS_MAX_VOLUMES}u; i++) {
    if (i >= count || !HAS_WELLS) { break; }
    let v = fx.vols[i];
    if (u32(v.params.x + 0.5) != ${WELLS_KIND.well}u) { continue; }
    let c = v.pos.xyz;
    let reach = v.pos.w;
    let core = v.params.y;
    let e = v.params.z;
    let tc = dot(c - cam, dir);
    let bv = cam + dir * tc - c;
    let b = length(bv);
    if (tc > 0.0 && b < reach && t_scene > tc + core * 0.6) {
      let taper = 1.0 - smoothstep(0.4 * reach, reach, b);
      // Displacement toward the centre at the well's plane (u): E^2 / b, held
      // finite near the centre (it crosses over: the far side shows inverted).
      let defl = min(e * e / max(b, 1e-3), 2.2 * b + 0.6 * e) * taper;
      let cs = to_pixel(c);
      let pv = pix - cs;
      let pl = max(length(pv), 1e-3);
      lens -= pv / pl * (defl / (tc * px_per_t));
      lens_min_t = min(lens_min_t, tc - core * 0.6);
    }
  }
  if (dot(lens, lens) > 0.01) {
    let at = pix + lens;
    // Only background bends: a sample in front of the well (the core, rock
    // nearer than it) keeps the unbent pixel.
    let zs = scene_z(at);
    let ok = smoothstep(lens_min_t - 0.4, lens_min_t + 0.4, zs * dl);
    let uv = clamp(at / size, vec2f(0.0), vec2f(1.0));
    let bent = textureSampleLevel(scene_copy, copy_samp, uv, 0.0).rgb;
    col = mix(col, bent, ok);
  }

  for (var i = 0u; i < ${WELLS_MAX_VOLUMES}u; i++) {
    if (i >= count) { break; }
    let v = fx.vols[i];
    let kind = u32(v.params.x + 0.5);
    let c = v.pos.xyz;
    let reach = v.pos.w;
    if (HAS_WELLS && kind == ${WELLS_KIND.well}u) {
      let core = v.params.y;
      let e = v.params.z;
      let s = max(v.params.w, 0.1);
      let gain = v.color.w;
      let tc = dot(c - cam, dir);
      let b = length(cam + dir * tc - c);
      // Nothing of this well reaches the ray (the camera can be inside it: tc < 0 keeps the distance to the centre).
      if (b > reach || (tc < -reach)) { continue; }
      // Glow: a Gaussian cloud integrated along the ray up to the scene (rock in front hides it).
      let t_end = min(t_scene, 1e4);
      // (The integral of the Gaussian along the ray, with a smoothstep for the error function.)
      let g = exp(-(b * b) / (s * s)) * (smoothstep(-1.6, 1.6, (t_end - tc) / s) - smoothstep(-1.6, 1.6, -tc / s));
      add += v.color.rgb * (gain * 0.16 * g * fog_at(max(tc, 0.0)));
      // Photon ring at the lens radius (behind the well only).
      if (tc > 0.0 && t_scene > tc) {
        let w = 0.1 * e + 0.05;
        let ring = exp(-((b - e) * (b - e)) / (w * w)) * (1.0 - smoothstep(0.4 * reach, reach, b));
        add += v.color.rgb * (gain * ring * 0.12 * fog_at(tc));
      }
      // The core stone: polished black with thin violet veins and a violet rim.
      if (hit && core > 0.0) {
        let q = p - c;
        let r = length(q);
        if (r < core + 0.45) {
          let n = q / max(r, 1e-3);
          let qs = spin(q, v.extra.xyz, t * 0.1 + v.extra.w) / max(core, 0.5);
          // Veins: the ridges of two octaves of noise, thin and meandering.
          let nz = vnoise(qs * 2.1 + vec3f(v.extra.w)) * 0.65 + vnoise(qs * 4.7) * 0.35;
          let ridge = 1.0 - abs(nz * 2.0 - 1.0);
          let aa = min(0.5, px / max(core, 0.5) * 6.0);
          let vein = smoothstep(0.976 - aa, 0.995, ridge) / (1.0 + aa * 6.0);
          let flow = 0.55 + 0.45 * sin(t * 1.4 + dot(qs, vec3f(3.1, 1.7, 2.3)));
          let rim = pow(1.0 - abs(dot(n, -dir)), 4.0);
          let on = 1.0 - smoothstep(core + 0.15, core + 0.45, r);
          // Keep the lighting's shape (speculars) but make the stone near black.
          let stone = col * 0.22 + vec3f(0.002, 0.001, 0.004);
          let glow = v.color.rgb * gain * (vein * flow * 2.4 + rim * 0.35);
          col = mix(col, stone + glow, on);
        }
      }
    } else if (HAS_SURFACE && kind == ${WELLS_KIND.crack}u && hit) {
      let d = distance(p, c);
      if (d < reach) {
        let prog = clamp(v.params.y, 0.0, 1.0);
        let up = v.extra.xyz;
        // Only the ceiling cracks (rock facing down along the level's up).
        let ceiling = smoothstep(0.15, 0.6, dot(pn, -up));
        let spread = reach * (0.45 + 0.55 * prog);
        let d_edge = 1.0 - smoothstep(0.35 * spread, spread, d);
        if (ceiling * d_edge > 0.002) {
          let pp = p + vec3f(v.params.z * 17.0);
          // The cracks spread from the spot (the reach grows with the warning), with a ragged edge.
          let wob = vnoise(pp * 0.45);
          let fall = 1.0 - smoothstep(0.35 * spread, spread, d + (wob - 0.5) * 0.5 * reach);
          // Polygonal cracks (cell borders), only some of them, kinked by a cheap warp.
          let pw = pp + (vec3f(wob, fract(wob * 7.13), fract(wob * 3.71)) - 0.5) * 1.6;
          let big = cells(pw * 0.3);
          let lw1 = max(0.025, px * 0.3 * 1.5) + 0.04 * prog;
          let keep1 = smoothstep(0.32, 0.5, vnoise(pw * 0.45 + vec3f(1.3)));
          let line = (1.0 - smoothstep(0.0, lw1, big.x)) * keep1 / (1.0 + px * 0.3 * 10.0);
          var fine = 0.0;
          if (prog > 0.3) {
            let small = cells(pw * 0.9 + vec3f(4.1));
            let lw2 = max(0.03, px * 0.9 * 1.5) + 0.03 * prog;
            fine = (1.0 - smoothstep(0.0, lw2, small.x)) * smoothstep(0.3, 0.85, prog) * smoothstep(0.45, 0.6, fract(small.y * 5.3)) / (1.0 + px * 0.9 * 10.0);
          }
          let flick = 0.8 + 0.2 * sin(t * 23.0 + big.y * 40.0) * sin(t * 7.0 + big.y * 13.0);
          let heat = (line + fine * 0.7) * fall * ceiling * flick;
          // Soot along the cracks, then the glow (hotter and wider as the warning runs out).
          col *= 1.0 - 0.6 * min(1.0, heat * 1.5);
          add += v.color.rgb * v.color.w * heat * (0.25 + 1.4 * prog * prog) * fog_at(t_scene);
        }
      }
    } else if (HAS_SURFACE && kind == ${WELLS_KIND.rubble}u && hit) {
      let d = distance(p, c);
      if (d < reach) {
        let fresh = clamp(v.params.y, 0.0, 1.0);
        let up = v.extra.xyz;
        let pp = p + vec3f(v.params.z * 13.0);
        let n = vnoise(pp * 0.6);
        let fall = 1.0 - smoothstep(0.5 * reach, reach, d + (n - 0.5) * 0.35 * reach);
        let top = 0.55 + 0.55 * max(dot(pn, up), 0.0);
        let coat = clamp(fall * (0.4 + 0.5 * fresh) * (0.6 + 0.4 * n) * top, 0.0, 0.9);
        let lum = dot(col, vec3f(0.2126, 0.7152, 0.0722));
        // Pale broken-stone dust: brighter and greyer than the rock it settles on.
        let dusty = max(lum * (1.7 + 0.6 * fresh), 0.012) * vec3f(1.0, 0.96, 0.9);
        col = mix(col, dusty, coat);
        // Grit: darker specks and pale chips in the dust (fading out where a pixel is wider than a speck).
        let grit = vnoise(pp * 5.0);
        let speck = 1.0 / (1.0 + px * 25.0);
        col *= 1.0 - 0.3 * coat * smoothstep(0.62, 0.82, grit) * speck;
        col += dusty * 0.35 * coat * smoothstep(0.2, 0.05, grit) * speck;
      }
    }
  }
  return vec4f(col + add, base.a);
}
`;
}
