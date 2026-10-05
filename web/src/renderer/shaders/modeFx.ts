// Mode effects pass (M3 C1): analytic volumes drawn into the resolved HDR
// scene after the sky and before the particles, additive, reading the scene
// depth like the particles do (soft against rock, hidden behind it):
//
// - WALL: The Squeeze's closing shell. Rock within a band of the shell's
//   sphere glows with a hex force-field pattern (cyan while the walls only
//   warn, amber with molten cracks while they grind inward); where the
//   sphere runs through open space ahead of the rock (a wall the chunk
//   meshes have not caught up with yet) the same pattern shows as a film.
// - ZONE: a holographic sphere: a fresnel film with a latitude/longitude
//   grid and a scan band where the view ray crosses it in open space, and
//   a dashed contour where it cuts the rock (the next safe zone, extraction
//   zones, the reactor's fuse radius). An optional equator ring shows a
//   fill fraction (the reactor's fuse).
// - BEAM: a vertical light pillar rising from a point (extraction zones,
//   the Heist core and its carrier).
// - HALO: a bright rim around a ship (the carrier, the reactor holder).
//
// Everything fades with the scene's distance fog. The pass writes alpha 0
// with an additive blend that keeps the scene's alpha (the Last Color
// Standing colour mask in objects.ts).

/** Most volumes per frame (extra ones are dropped, lowest priority first). */
export const MODE_FX_MAX_VOLUMES = 12;

/** Floats per volume in the uniform (four vec4). */
export const MODE_FX_VOLUME_FLOATS = 16;

/** Floats of the uniform header (five vec4) before the volumes. */
export const MODE_FX_HEADER_FLOATS = 20;

/** Total floats of the uniform block. */
export const MODE_FX_UNIFORM_FLOATS = MODE_FX_HEADER_FLOATS + MODE_FX_MAX_VOLUMES * MODE_FX_VOLUME_FLOATS;

/** Volume kinds (`params.x`). */
export const MODE_FX_KIND = { wall: 1, zone: 2, beam: 3, halo: 4 } as const;

/** WGSL of the mode effects pipeline; `multisampledDepth` picks the scene depth binding type. */
export function modeFxWgsl(multisampledDepth: boolean): string {
  const depthType = multisampledDepth ? "texture_depth_multisampled_2d" : "texture_depth_2d";
  return /* wgsl */ `
struct Volume {
  pos: vec4f,    // xyz centre (beam: base), w radius (beam: width)
  color: vec4f,  // rgb linear colour, w intensity (HDR multiplier)
  params: vec4f, // x kind; wall: y warn 0..1, z grind 0..1, w band width (u)
                 //         zone: y film, z grid, w contour
                 //         beam: y height (u), z core strength
                 //         halo: y rim power
  extra: vec4f,  // zone: x contour width (u), y ring fill 0..1 (< 0: no ring), z pulse rate (Hz), w dash speed (u/s)
};

struct ModeFx {
  cam: vec4f,    // xyz camera position, w time (s)
  right: vec4f,  // xyz camera right x tan(fov x / 2), w near plane
  up: vec4f,     // xyz camera up x tan(fov y / 2), w far plane
  fwd: vec4f,    // xyz camera forward, w exp2 fog density
  info: vec4f,   // x volume count, y target width (px), z target height (px), w scene depth texel per target pixel
  vols: array<Volume, ${MODE_FX_MAX_VOLUMES}>,
};

@group(0) @binding(0) var<uniform> fx: ModeFx;
@group(0) @binding(1) var scene_depth: ${depthType};

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

const PI = 3.14159265;
const TAU = 6.2831853;

fn hash21(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

fn hash31(p: vec3f) -> f32 {
  var q = fract(p * vec3f(0.1031, 0.1030, 0.0973));
  q += dot(q, q.yzx + 33.33);
  return fract((q.x + q.y) * q.z);
}

// Smooth value noise in 0..1.
fn vnoise(p: vec3f) -> f32 {
  let i = floor(p);
  let f = p - i;
  let u = f * f * (3.0 - 2.0 * f);
  let a = mix(mix(hash31(i), hash31(i + vec3f(1.0, 0.0, 0.0)), u.x), mix(hash31(i + vec3f(0.0, 1.0, 0.0)), hash31(i + vec3f(1.0, 1.0, 0.0)), u.x), u.y);
  let b = mix(mix(hash31(i + vec3f(0.0, 0.0, 1.0)), hash31(i + vec3f(1.0, 0.0, 1.0)), u.x), mix(hash31(i + vec3f(0.0, 1.0, 1.0)), hash31(i + vec3f(1.0, 1.0, 1.0)), u.x), u.y);
  return mix(a, b, u.z);
}

fn hash33(p: vec3f) -> vec3f {
  var q = fract(p * vec3f(0.1031, 0.1030, 0.0973));
  q += dot(q, q.yxz + 33.33);
  return fract((q.xxy + q.yxx) * q.zyx);
}

// 3D cell pattern (Voronoi): x = distance to the nearest cell border (F2 - F1, about), y = cell hash.
// Feature points jitter within the middle 70 % of their cell, so the 2x2x2 cells around the
// nearer corner hold the nearest point (8 lookups instead of 27; F2 is close enough for lines).
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

// Force-field pattern of the wall at world point p: cell borders (x),
// cells lighting up one by one (y) and molten cracks while grinding (z).
// px = world size of a pixel at p.
fn wall_pattern(p: vec3f, px: f32, grind: f32, t: f32) -> vec3f {
  let k = 1.0 / 3.2;
  let c = cells(p * k);
  let lw = max(0.05, px * k * 2.0);
  // Lines thinner than a pixel fade instead of turning into a flat wash far away.
  let far = 1.0 / (1.0 + px * k * 12.0);
  let edge = (1.0 - smoothstep(0.0, lw, c.x)) * far;
  let halo = exp(-c.x * 7.0) * far;
  let rate = 0.7 + 2.2 * grind;
  let cell = pow(0.5 + 0.5 * sin(t * rate * (1.0 + c.y * 2.0) + c.y * 40.0), 48.0);
  // Molten cracks (grinding only): ridged noise drifting slowly, opening in
  // patches (a slow mask) and fading with distance like the cell lines.
  var crack = 0.0;
  if (grind > 0.0) {
    let patches = smoothstep(0.5, 0.8, vnoise(p * 0.07 + vec3f(t * 0.05, 0.0, 0.0)));
    if (patches > 0.0) {
      let nz = vnoise(p * 0.22 + vec3f(0.0, t * 0.25, 0.0)) * 0.7 + vnoise(p * 0.6 - vec3f(t * 0.15)) * 0.3;
      let ridge = 1.0 - abs(nz * 2.0 - 1.0);
      crack = pow(ridge, 28.0) * grind * patches * far;
    }
  }
  return vec3f(edge * 0.16 + halo * 0.05, cell, crack);
}

// Energy curtain: soft vertical streaks drifting upward (faster while grinding), 0..1.
fn streaks(p: vec3f, t: f32, grind: f32) -> f32 {
  let speed = 0.5 + 1.2 * grind;
  let n = vnoise(vec3f(p.x * 0.7, p.y * 0.07 - t * speed, p.z * 0.7));
  return smoothstep(0.5, 0.95, n);
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

// Ray / sphere: (near t, far t), both < 0 on a miss.
fn sphere_hits(o: vec3f, d: vec3f, c: vec3f, r: f32) -> vec2f {
  let oc = o - c;
  let b = dot(oc, d);
  let cc = dot(oc, oc) - r * r;
  let disc = b * b - cc;
  if (disc <= 0.0) { return vec2f(-1.0, -1.0); }
  let s = sqrt(disc);
  return vec2f(-b - s, -b + s);
}

fn wall_color(v: Volume, warn: f32, grind: f32) -> vec3f {
  // Cyan while warning, amber while grinding.
  return mix(v.color.rgb, vec3f(1.0, 0.42, 0.08), grind);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let size = fx.info.yz;
  let ndc = vec2f(in.clip.x / size.x * 2.0 - 1.0, 1.0 - in.clip.y / size.y * 2.0);
  let dv = fx.fwd.xyz + ndc.x * fx.right.xyz + ndc.y * fx.up.xyz;
  let dl = length(dv);
  let dir = dv / dl;
  let cam = fx.cam.xyz;
  let t = fx.cam.w;
  let raw = textureLoad(scene_depth, vec2i(in.clip.xy * fx.info.w), 0);
  let hit = raw < 1.0;
  let z = linear_depth(raw);
  var t_scene = 1e9;
  var p = cam + dv * z;
  // Surface normal from the depth (screen derivatives, taken in uniform control flow).
  let pn_raw = cross(dpdx(p), dpdy(p));
  let pn = pn_raw / max(length(pn_raw), 1e-12);
  if (hit) {
    t_scene = z * dl;
  }
  // World size of one pixel per unit of distance.
  let px_per_t = 2.0 * length(fx.up.xyz) / size.y;
  var acc = vec3f(0.0);
  let count = u32(fx.info.x);
  for (var i = 0u; i < ${MODE_FX_MAX_VOLUMES}u; i++) {
    if (i >= count) { break; }
    let v = fx.vols[i];
    let kind = u32(v.params.x + 0.5);
    let c = v.pos.xyz;
    let r = v.pos.w;
    let gain = v.color.w;
    if (kind == ${MODE_FX_KIND.wall}u) {
      let warn = v.params.y;
      let grind = v.params.z;
      let band_w = v.params.w;
      let col = wall_color(v, warn, grind);
      let pulse = 0.75 + 0.25 * sin(t * TAU * (0.8 + 1.4 * grind));
      if (hit) {
        let q = p - c;
        let s = length(q) - r;
        let band = exp(-(s * s) / (band_w * band_w));
        // Light the wall spills onto the rock in front of it, and a bright seam where the cave rock meets it.
        // Faded in over the band (D5): the meshed wall lies a little inside
        // or outside the true sphere from cell to cell, and a hard switch at
        // s = 0 drew that as rings of spill light all over the wall.
        if (s < 0.0 && s > -6.0) {
          let seam = exp(-(s + 0.85) * (s + 0.85) / 0.09);
          let off_wall = smoothstep(0.0, 1.0, -s / band_w);
          acc += col * ((0.05 + 0.08 * grind) * exp(s / 1.8) + seam * (0.9 + 0.8 * grind)) * off_wall * pulse * gain * fog_at(t_scene);
        }
        if (band > 0.003) {
          let pat = wall_pattern(p, t_scene * px_per_t, grind, t);
          // A scan band sweeping along the wall.
          let sweep = pow(0.5 + 0.5 * sin(q.y * 0.09 - t * 1.6), 24.0);
          let curtain = streaks(p, t, grind);
          let base = 0.03 + 0.22 * curtain + 0.05 * pat.y + 0.2 * sweep;
          // Cell borders show where the scan band passes and faintly where the curtain is bright.
          let e = col * (base + pat.x * (0.15 + 0.8 * curtain + 2.4 * sweep)) * pulse + vec3f(1.6, 0.55, 0.12) * pat.z * 2.5;
          acc += e * band * gain * fog_at(t_scene);
        }
      }
      // Open space ahead of the rock: the shell's surface as a film (camera inside).
      let h = sphere_hits(cam, dir, c, r);
      let tf = h.y;
      if (h.x < 0.0 && tf > 0.0 && tf < t_scene - band_w * 1.5) {
        let wp = cam + dir * tf;
        let n = (wp - c) / r;
        let facing = abs(dot(n, dir));
        let pat = wall_pattern(wp, tf * px_per_t, grind, t);
        let rim = 0.015 + 0.3 * pow(max(1.0 - facing, 0.0), 4.0);
        let e = col * (rim + streaks(wp, t, grind) * 0.1 + pat.x * 0.2) * pulse + vec3f(1.6, 0.55, 0.12) * pat.z * 1.5;
        acc += e * gain * 0.6 * fog_at(tf);
      }
    } else if (kind == ${MODE_FX_KIND.zone}u) {
      let film = v.params.y;
      let grid = v.params.z;
      let contour = v.params.w;
      let cw = v.extra.x;
      let fill = v.extra.y;
      let pulse = select(1.0, 0.7 + 0.3 * sin(t * TAU * v.extra.z), v.extra.z > 0.0);
      let col = v.color.rgb;
      // Film where the view ray crosses the sphere in open space.
      let h = sphere_hits(cam, dir, c, r);
      for (var k = 0; k < 2; k++) {
        let th = select(h.y, h.x, k == 0);
        if (th <= 0.0 || th >= t_scene) { continue; }
        let wp = cam + dir * th;
        let n = (wp - c) / r;
        let facing = abs(dot(n, dir));
        let rim = pow(max(1.0 - facing, 0.0), 5.0);
        // Latitude / longitude grid (12 meridians, a parallel every 15 degrees), drifting.
        let lon = atan2(n.z, n.x) + t * 0.05;
        let lat = asin(clamp(n.y, -1.0, 1.0));
        let pxa = th * px_per_t / r;
        let dm = abs(fract(lon / (TAU / 12.0) + 0.5) - 0.5) * (TAU / 12.0) * max(cos(lat), 0.05);
        let dp = abs(fract(lat / (PI / 12.0)) - 0.5) * (PI / 12.0);
        let lw = max(pxa * 1.2, 0.0035);
        let lines = max(1.0 - smoothstep(0.0, lw, dm), 1.0 - smoothstep(0.0, lw, dp));
        let scan = pow(0.5 + 0.5 * sin(n.y * 6.0 - t * 1.8), 40.0);
        var e = col * (film * (0.02 + rim) + grid * lines * (0.25 + rim) + film * scan * 0.25);
        if (fill >= 0.0) {
          // Fuse ring on the equator: lit up to the fill fraction.
          let ring = exp(-(wp.y - c.y) * (wp.y - c.y) / 0.16);
          let a01 = fract(atan2(n.z, n.x) / TAU + 0.25);
          let lit = select(0.12, 1.0, a01 <= fill);
          e += col * ring * lit * 3.0;
        }
        acc += e * gain * pulse * fog_at(th);
      }
      // Contour where the sphere cuts the rock, dashed and moving, with a soft inner rim.
      if (hit && contour > 0.0) {
        let q = p - c;
        let ql = max(length(q), 1e-4);
        // Distance to the contour along the surface: a wall nearly tangent to the sphere does not smear it.
        let cosn = abs(dot(pn, q / ql));
        let s = (ql - r) / max(sqrt(max(1.0 - cosn * cosn, 0.0)), 0.3);
        let cwp = max(cw, t_scene * px_per_t * 1.5);
        let line = exp(-(s * s) / (cwp * cwp));
        let inner = select(0.0, (1.0 + s / 1.5) * (1.0 + s / 1.5) * 0.04, s < 0.0 && s > -1.5);
        if (line > 0.003 || inner > 0.0) {
          let n = q / max(length(q), 1e-4);
          let along = (atan2(n.z, n.x) + PI) * r * max(length(n.xz), 0.2) + (n.y + 1.0) * r * 0.5;
          let dash = 0.55 + 0.45 * step(0.45, fract(along / 1.6 - t * v.extra.w / 1.6));
          var e = col * (line * dash + inner) * contour;
          if (fill >= 0.0) {
            let a01 = fract(atan2(n.z, n.x) / TAU + 0.25);
            e *= select(0.35, 1.0, a01 <= fill);
          }
          acc += e * gain * pulse * fog_at(t_scene);
        }
      }
    } else if (kind == ${MODE_FX_KIND.beam}u) {
      // Distance between the view ray and the vertical line through c, in xz.
      let height = v.params.y;
      let core = v.params.z;
      let oc = cam.xz - c.xz;
      let dxz = dir.xz;
      let dd = dot(dxz, dxz);
      if (dd > 1e-6) {
        let tb = -dot(oc, dxz) / dd;
        if (tb > 0.0 && tb < t_scene) {
          let y = cam.y + dir.y * tb - c.y;
          let dist = length(oc + dxz * tb);
          let wpx = max(r, tb * px_per_t * 1.5);
          if (y > -1.0 && y < height && dist < wpx * 4.0) {
            let along = smoothstep(-1.0, 0.5, y) * (1.0 - smoothstep(height * 0.35, height, y));
            let ripple = 0.75 + 0.25 * sin(y * 0.8 - t * 4.0);
            let glow = exp(-dist * dist / (wpx * wpx)) * 0.5 + exp(-dist * dist / (wpx * wpx * 0.08)) * core;
            // Thin beams far away stay visible but do not grow brighter per pixel.
            let thin = clamp(r / wpx, 0.25, 1.0);
            acc += v.color.rgb * glow * along * ripple * thin * gain * fog_at(tb);
          }
        }
      }
    } else if (kind == ${MODE_FX_KIND.halo}u) {
      let h = sphere_hits(cam, dir, c, r);
      let power = max(v.params.y, 1.0);
      for (var k = 0; k < 2; k++) {
        let th = select(h.y, h.x, k == 0);
        // Hidden behind rock or the ship itself (the back half shows only around it).
        if (th <= 0.0 || th >= t_scene) { continue; }
        let wp = cam + dir * th;
        let n = (wp - c) / r;
        let facing = abs(dot(n, dir));
        // A thin bright rim with a faint inner sheen.
        let rim = pow(max(1.0 - facing, 0.0), power) + 0.008;
        acc += v.color.rgb * rim * gain * select(1.0, 0.3, k == 1) * fog_at(th);
      }
    }
  }
  return vec4f(acc, 0.0);
}
`;
}
