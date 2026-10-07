// Belt Up's world look (M4 C7, docs/milestone-4-plan.md §3.8 "Visuals"):
// one fullscreen pass drawn into the resolved HDR scene after the sky and
// before the particles, additive, reading the scene depth (like modeFx.ts):
//
// - the dust band: a warm, sunlit haze in the belt's annulus slab (marched
//   along the view ray up to the rock), thickest in the belt plane, wispy;
//   it shows where the belt shelters you;
// - aurora curtains above and below the belt (the radiation): three wavy
//   curtains each side, green at their foot, violet higher up, brighter the
//   farther from the belt;
// - the 8 gates: holographic arches across the belt (vertical half planes
//   from the gate's inner to its outer radius), a seven-segment sector
//   number in the top bar; the next gate bright and pulsing, later gates
//   faint, passed gates dim in the pilot's colour;
// - the exit: a swirling green portal with a bright rim inside the dust
//   cocoon, and the giant emergency-exit sign over it (a texture: the
//   running man through a door, "EXIT"), turned toward the viewer;
// - ice glints: sparse sparkles on sunlit rock in the belt.
//
// A second pipeline (RADIATION_WGSL) is the screen-edge static and
// chromatic fringe of an irradiated ship, drawn over a copy of the scene
// after the particles.

import { KEEP_ALPHA } from "./post.ts";

/** Floats of the belt uniform block (29 vec4). */
export const BELT_UNIFORM_FLOATS = 29 * 4;

/** The volume pass (dust, aurora, cocoon) runs at 1/BELT_VOLUME_DIVISOR of the resolution per axis. */
export const BELT_VOLUME_DIVISOR = 4;

/** Offsets (floats) of the uniform's vec4 slots. */
export const BELT_SLOT = {
  cam: 0,
  right: 4,
  up: 8,
  fwd: 12,
  info: 16,
  center: 20,
  shape: 24,
  gateShape: 28,
  sun: 32,
  sunColor: 36,
  dustColor: 40,
  extra: 44,
  low: 48,
  gateColor: 52,
  gateInfo: 84,
} as const;

/** Exit sign texture size (px). */
export const SIGN_WIDTH = 640;
export const SIGN_HEIGHT = 192;

/** Exit sign size in the world (u): width; the height follows the texture's aspect. */
export const SIGN_WORLD_WIDTH = 46;

const COMMON = /* wgsl */ `
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

fn hash33(p: vec3f) -> vec3f {
  var q = fract(p * vec3f(0.1031, 0.1030, 0.0973));
  q += dot(q, q.yxz + 33.33);
  return fract((q.xxy + q.yxx) * q.zyx);
}

// Smooth 2D value noise in 0..1.
fn vnoise2(p: vec2f) -> f32 {
  let i = floor(p);
  let f = p - i;
  let u = f * f * (3.0 - 2.0 * f);
  let a = hash21(i);
  let b = hash21(i + vec2f(1.0, 0.0));
  let c = hash21(i + vec2f(0.0, 1.0));
  let d = hash21(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Smooth 3D value noise in 0..1.
fn vnoise3(p: vec3f) -> f32 {
  let i = floor(p);
  let f = p - i;
  let u = f * f * (3.0 - 2.0 * f);
  let a = mix(mix(hash31(i), hash31(i + vec3f(1.0, 0.0, 0.0)), u.x), mix(hash31(i + vec3f(0.0, 1.0, 0.0)), hash31(i + vec3f(1.0, 1.0, 0.0)), u.x), u.y);
  let b = mix(mix(hash31(i + vec3f(0.0, 0.0, 1.0)), hash31(i + vec3f(1.0, 0.0, 1.0)), u.x), mix(hash31(i + vec3f(0.0, 1.0, 1.0)), hash31(i + vec3f(1.0, 1.0, 1.0)), u.x), u.y);
  return mix(a, b, u.z);
}

// Interleaved gradient noise (per pixel, stable): ray march jitter.
fn ign(px: vec2f) -> f32 {
  return fract(52.9829189 * fract(dot(px, vec2f(0.06711056, 0.00583715))));
}

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
`;

function beltCommon(multisampledDepth: boolean, surface: boolean): string {
  const depthType = multisampledDepth ? "texture_depth_multisampled_2d" : "texture_depth_2d";
  return /* wgsl */ `
struct Belt {
  cam: vec4f,        // xyz camera position, w time (s)
  right: vec4f,      // xyz camera right x tan(fov x / 2), w near plane
  up: vec4f,         // xyz camera up x tan(fov y / 2), w far plane
  fwd: vec4f,        // xyz camera forward, w exp2 fog density
  info: vec4f,       // x target width (px), y height (px), z scene depth texel per target pixel, w dust strength
  center: vec4f,     // xyz belt centre, w inner radius
  shape: vec4f,      // x outer radius, y half thickness, z exit radius, w cocoon radius
  gate_shape: vec4f, // x gate inner radius, y gate outer radius, z aurora strength, w glint strength
  sun: vec4f,        // xyz unit direction toward the sun, w exit portal energy 0..1
  sun_color: vec4f,  // rgb linear sun colour, w exit sign strength
  dust_color: vec4f, // rgb linear dust tint, w gate strength
  extra: vec4f,      // x sign height above the centre (u), y sign width (u), z sign height (u), w portal swirl speed
  low: vec4f,        // x volume target width (px), y height (px), z full-resolution pixels per volume pixel
  gate_color: array<vec4f, 8>, // rgb linear colour, w intensity (0 = hidden)
  gate_info: array<vec4f, 8>,  // x number shown (0 = none), y pulse 0..1, z fill 0..1, w state (0 later, 1 next, 2 passed)
};

@group(0) @binding(0) var<uniform> bu: Belt;
@group(0) @binding(1) var scene_depth: ${depthType};
${
  surface
    ? `@group(0) @binding(3) var lin_sampler: sampler;
@group(0) @binding(4) var volume_tex: texture_2d<f32>;`
    : ""
}

${COMMON}

fn fog_at(t: f32) -> f32 {
  let fd = bu.fwd.w * t;
  return exp2(-fd * fd);
}

fn linear_depth(d: f32) -> f32 {
  let n = bu.right.w;
  let f = bu.up.w;
  return n * f / max(f - d * (f - n), 1e-6);
}

// Ray against the horizontal slab y in [y0, y1]: (t enter, t exit), enter > exit on a miss.
fn slab(o: vec3f, d: vec3f, y0: f32, y1: f32) -> vec2f {
  if (abs(d.y) < 1e-5) {
    if (o.y >= y0 && o.y <= y1) { return vec2f(0.0, 1e9); }
    return vec2f(1.0, 0.0);
  }
  let ta = (y0 - o.y) / d.y;
  let tb = (y1 - o.y) / d.y;
  return vec2f(max(min(ta, tb), 0.0), max(ta, tb));
}

// Ray against the vertical cylinder of radius r about the belt axis: (t enter, t exit).
fn cylinder(o: vec3f, d: vec3f, c: vec3f, r: f32) -> vec2f {
  let oc = vec2f(o.x - c.x, o.z - c.z);
  let dd = vec2f(d.x, d.z);
  let a = dot(dd, dd);
  if (a < 1e-8) {
    if (dot(oc, oc) <= r * r) { return vec2f(0.0, 1e9); }
    return vec2f(1.0, 0.0);
  }
  let b = dot(oc, dd);
  let cc = dot(oc, oc) - r * r;
  let disc = b * b - a * cc;
  if (disc < 0.0) { return vec2f(1.0, 0.0); }
  let s = sqrt(disc);
  return vec2f(max((-b - s) / a, 0.0), (-b + s) / a);
}

// Dust density of the band at p (0..about 1.4).
fn dust_density(p: vec3f, t: f32) -> f32 {
  let c = bu.center.xyz;
  let d = p - c;
  let r = length(d.xz);
  let inner = bu.center.w;
  let outer = bu.shape.x;
  let h = bu.shape.y;
  let radial = smoothstep(inner - 12.0, inner + 10.0, r) * (1.0 - smoothstep(outer - 10.0, outer + 14.0, r));
  let vert = exp(-(d.y * d.y) / (h * h * 0.8));
  let base = radial * vert;
  if (base < 0.004) { return 0.0; }
  // Wisps drifting prograde with the belt: noise in a frame turning slowly about the axis.
  let ang = t * 0.004;
  let cs = cos(ang);
  let sn = sin(ang);
  let q = vec3f(d.x * cs - d.z * sn, d.y, d.x * sn + d.z * cs);
  let n1 = vnoise3(q * vec3f(0.035, 0.09, 0.035));
  let n2 = vnoise3(q * vec3f(0.11, 0.2, 0.11) + vec3f(7.1, 1.3, 3.7));
  return base * (0.25 + 1.1 * n1 * n1 + 0.35 * n2);
}

// Henyey-Greenstein phase, scaled so isotropic is 1.
fn hg(cos_t: f32, g: f32) -> f32 {
  let g2 = g * g;
  return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * cos_t, 1e-4), 1.5);
}

// Aurora emission at p (both sides of the belt: mirrored in its plane).
fn aurora_at(p: vec3f, t: f32) -> vec3f {
  let c = bu.center.xyz;
  let d = p - c;
  let above = abs(d.y) - bu.shape.y;
  if (above < 2.0) { return vec3f(0.0); }
  let r = length(d.xz);
  let az = atan2(-d.z, d.x);
  let side = select(1.7, 0.0, d.y > 0.0);
  var col = vec3f(0.0);
  for (var k = 0; k < 3; k++) {
    let fk = f32(k);
    let rk = 98.0 + 26.0 * fk + 7.0 * sin(3.0 * az + fk * 2.1 + side + t * 0.13) + 3.0 * sin(8.0 * az - t * 0.37 + fk * 1.7 + side);
    let w = 1.6 + 0.9 * fk;
    let dr = r - rk;
    let sheet = exp(-(dr * dr) / (w * w));
    if (sheet < 0.01) { continue; }
    // The curtain's foot waves; it rises higher on the outer curtains.
    let foot = 5.0 + 4.0 * fk + 3.0 * sin(5.0 * az + fk + side + t * 0.21);
    let hh = above - foot;
    let rise = smoothstep(-2.5, 1.5, hh);
    let fall = exp(-max(hh, 0.0) / (12.0 + 7.0 * fk));
    // Vertical rays along the curtain, shimmering.
    let s = vnoise2(vec2f(az * rk * 0.32, t * 0.5 + fk * 13.0 + side * 5.0));
    let s2 = vnoise2(vec2f(az * rk * 0.09 - t * 0.12, fk * 7.0 + side));
    let rays = 0.25 + 1.4 * s * s * (0.4 + 0.9 * s2);
    // Green at the foot, violet higher up; brighter farther out from the belt.
    let hue = smoothstep(2.0, 26.0, hh);
    let tint = mix(vec3f(0.12, 1.0, 0.42), vec3f(0.62, 0.16, 1.0), hue);
    let gain = 0.55 + 0.7 * smoothstep(0.0, 40.0, above) + 0.25 * fk;
    col += tint * sheet * rise * fall * rays * gain;
  }
  return col;
}

// Seven-segment digit 0-9 in a cell (uv 0..1, y down); returns coverage 0..1.
fn seg(uv: vec2f, a: vec2f, b: vec2f, w: f32) -> f32 {
  let pa = uv - a;
  let ba = b - a;
  let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
  return 1.0 - smoothstep(w * 0.6, w, length(pa - ba * h));
}

fn digit(n: i32, uv: vec2f) -> f32 {
  if (uv.x < -0.1 || uv.x > 1.1 || uv.y < -0.1 || uv.y > 1.1) { return 0.0; }
  // Segment bits: a top, b top right, c bottom right, d bottom, e bottom left, f top left, g middle.
  var bits = 0u;
  switch (n) {
    case 0: { bits = 0x3fu; }
    case 1: { bits = 0x06u; }
    case 2: { bits = 0x5bu; }
    case 3: { bits = 0x4fu; }
    case 4: { bits = 0x66u; }
    case 5: { bits = 0x6du; }
    case 6: { bits = 0x7du; }
    case 7: { bits = 0x07u; }
    case 8: { bits = 0x7fu; }
    default: { bits = 0x6fu; }
  }
  let w = 0.13;
  let x0 = 0.15;
  let x1 = 0.85;
  let y0 = 0.08;
  let y1 = 0.5;
  let y2 = 0.92;
  var c = 0.0;
  if ((bits & 1u) != 0u) { c = max(c, seg(uv, vec2f(x0, y0), vec2f(x1, y0), w)); }
  if ((bits & 2u) != 0u) { c = max(c, seg(uv, vec2f(x1, y0), vec2f(x1, y1), w)); }
  if ((bits & 4u) != 0u) { c = max(c, seg(uv, vec2f(x1, y1), vec2f(x1, y2), w)); }
  if ((bits & 8u) != 0u) { c = max(c, seg(uv, vec2f(x0, y2), vec2f(x1, y2), w)); }
  if ((bits & 16u) != 0u) { c = max(c, seg(uv, vec2f(x0, y1), vec2f(x0, y2), w)); }
  if ((bits & 32u) != 0u) { c = max(c, seg(uv, vec2f(x0, y0), vec2f(x0, y1), w)); }
  if ((bits & 64u) != 0u) { c = max(c, seg(uv, vec2f(x0, y1), vec2f(x1, y1), w)); }
  return c;
}

// One gate's hologram at local coordinates (rr = distance from the axis
// along the gate, yy = height above the belt plane); px = world size of a pixel.
fn gate_look(k: i32, rr: f32, yy: f32, px: f32, t: f32) -> vec3f {
  let gi = bu.gate_shape.x;
  let go = bu.gate_shape.y;
  let hh = bu.shape.y + 9.0;
  let col = bu.gate_color[k].rgb;
  let inten = bu.gate_color[k].w;
  let info = bu.gate_info[k];
  let next = info.y;
  // An arch: a rectangle with big rounded corners; distance to its border.
  let cx = 0.5 * (gi + go);
  let hx = 0.5 * (go - gi);
  let cr = 10.0;
  let q = vec2f(abs(rr - cx) - hx + cr, abs(yy) - hh + cr);
  let sd = length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - cr;
  let lw = max(0.35, 1.1 * px);
  let core = exp(-(sd * sd) / (lw * lw));
  let soft = exp(-(sd * sd) / (16.0 * lw * lw + 1.5));
  let inside = 1.0 - smoothstep(-0.3, 0.3, sd);
  // Light running along the frame of the next gate (dashes travelling round).
  let along = select(rr, yy, abs(q.x) > abs(q.y)) * sign(yy + 0.001) * sign(rr - cx + 0.001);
  let dash = smoothstep(0.35, 0.5, abs(fract(along * 0.12 - t * 0.9) - 0.5)) * next;
  // A thin glow just inside the frame, and for the next gate a faint shimmer and a sweep across.
  let rim_in = inside * exp(sd / 2.2) * 0.28;
  let sw = (rr - (gi + fract(t * 0.3) * (go - gi))) / 2.0;
  let sweep = exp(-sw * sw) * 0.08 * next;
  let shimmer = 0.012 * next * (0.5 + 0.5 * sin(yy * 0.9 + rr * 0.35 - t * 2.0));
  var light = core * (1.0 + 1.6 * dash) + soft * 0.32 + rim_in + inside * (sweep + shimmer);
  // The number in a plate on the top bar; the own last gate gets a checkered band.
  let n = i32(info.x + 0.5);
  if (n > 0) {
    let size = 8.0;
    let py = hh - 0.5 * size - 1.6;
    let uv = vec2f((rr - cx) / (size * 0.62) + 0.5, (py - yy) / size + 0.5);
    let plate_d = max(abs(rr - cx) - size * 0.62, abs(yy - py) - size * 0.66);
    let plate = 1.0 - smoothstep(0.0, max(px, 0.15), plate_d);
    let plate_edge = exp(-(plate_d * plate_d) / (lw * lw));
    light += plate * 0.12 + plate_edge * 0.6 + digit(n, uv) * 1.1;
    if (n == 8) {
      let band = (1.0 - smoothstep(0.0, max(px, 0.1), abs(yy - (hh - 1.0)) - 1.0)) * inside;
      let chk = step(0.5, fract(rr * 0.5)) != step(0.5, fract(yy * 0.5));
      light += band * select(0.06, 0.9, chk) * (1.0 - plate);
    }
  }
  // The next gate breathes; passed gates stay dim.
  let pulse = 0.8 + 0.2 * sin(t * 6.0) * next;
  return col * inten * pulse * light;
}

`;
}

/** WGSL of the belt volume pass (dust band, aurora, cocoon) at 1/BELT_VOLUME_DIVISOR resolution. */
export function beltVolumeWgsl(multisampledDepth: boolean): string {
  return /* wgsl */ `
${beltCommon(multisampledDepth, false)}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let size = bu.low.xy;
  let ndc = vec2f(in.clip.x / size.x * 2.0 - 1.0, 1.0 - in.clip.y / size.y * 2.0);
  let dv = bu.fwd.xyz + ndc.x * bu.right.xyz + ndc.y * bu.up.xyz;
  let dl = length(dv);
  let dir = dv / dl;
  let cam = bu.cam.xyz;
  let t = bu.cam.w;
  // The nearest scene depth of the footprint (fog never leaks in front of rock).
  // (the four corners and the centre of the footprint: five loads, not div x div).
  let div = bu.low.z;
  let dims = vec2i(textureDimensions(scene_depth)) - 1;
  let base = floor(in.clip.xy) * div;
  let last = div - 1.0;
  var raw = 1.0;
  for (var k = 0; k < 5; k++) {
    let o = select(vec2f(f32(k & 1), f32((k >> 1u) & 1)) * last, vec2f(0.5 * div), k == 4);
    let at = clamp(vec2i((base + o) * bu.info.z), vec2i(0), dims);
    raw = min(raw, textureLoad(scene_depth, at, 0));
  }
  var t_scene = 1e9;
  if (raw < 1.0) { t_scene = linear_depth(raw) * dl; }
  let c = bu.center.xyz;
  let sun = bu.sun.xyz;
  // White-noise jitter (a structured one shows as a grid through the up-sampling).
  let jitter = hash21(floor(in.clip.xy) + 0.5);
  var acc = vec3f(0.0);

  // Dust band: march the annulus slab up to the rock.
  let dust_k = bu.info.w;
  if (dust_k > 0.0) {
    let h = bu.shape.y * 2.0;
    let sl = slab(cam, dir, c.y - h, c.y + h);
    let cy = cylinder(cam, dir, c, bu.shape.x + 16.0);
    let t0 = max(sl.x, cy.x);
    let t1 = min(min(sl.y, cy.y), min(t_scene, t0 + 320.0));
    if (t1 > t0) {
      let steps = 8;
      let dt = (t1 - t0) / f32(steps);
      let ph = min(2.2, 0.45 + 0.55 * hg(dot(dir, sun), 0.45));
      var light = 0.0;
      for (var i = 0; i < steps; i++) {
        let ts = t0 + (f32(i) + jitter) * dt;
        let dens = dust_density(cam + dir * ts, t);
        light += dens * fog_at(ts);
      }
      // Optical depth of the dust seen: thin from above, saturating along the ring.
      let tau = light * dt * 0.02;
      acc += bu.dust_color.rgb * bu.sun_color.rgb * (1.0 - exp(-tau)) * ph * dust_k * 0.22;
    }
  }

  // Aurora: march the slabs above and below the belt.
  let aur_k = bu.gate_shape.z;
  if (aur_k > 0.0) {
    let h0 = bu.shape.y + 2.0;
    let h1 = bu.shape.y + 60.0;
    let cy = cylinder(cam, dir, c, 172.0);
    for (var s = 0; s < 2; s++) {
      let sl = slab(cam, dir, select(c.y - h1, c.y + h0, s == 0), select(c.y - h0, c.y + h1, s == 0));
      let t0 = max(sl.x, cy.x);
      let t1 = min(min(sl.y, cy.y), min(t_scene, t0 + 360.0));
      if (t1 <= t0) { continue; }
      let steps = 12;
      let dt = (t1 - t0) / f32(steps);
      var em = vec3f(0.0);
      for (var i = 0; i < steps; i++) {
        let ts = t0 + (f32(i) + jitter) * dt;
        em += aurora_at(cam + dir * ts, t) * fog_at(ts * 0.7);
      }
      acc += em * dt * aur_k * 0.011;
    }
  }

  // The cocoon: a faint green-white haze through the dust sphere around the exit.
  let portal = bu.sun.w;
  let ocv = c - cam;
  let tc = dot(ocv, dir);
  let b2 = max(dot(ocv, ocv) - tc * tc, 0.0);
  let rc = bu.shape.w;
  if (b2 < rc * rc && portal > 0.0) {
    let half = sqrt(rc * rc - b2);
    let ta = max(tc - half, 0.0);
    let tb = min(tc + half, t_scene);
    if (tb > ta) {
      let thick = (tb - ta) / (2.0 * rc);
      acc += vec3f(0.3, 0.85, 0.55) * thick * thick * 0.022 * (0.6 + 0.4 * portal) * fog_at(ta);
    }
  }
  return vec4f(acc, 1.0);
}
`;
}

/** WGSL of the belt surface pass: the volume composited, gates, portal, sign, glints (full resolution). */
export function beltWgsl(multisampledDepth: boolean): string {
  return /* wgsl */ `
${beltCommon(multisampledDepth, true)}

const H = 0.70710678;
const GATE_DIRS = array<vec2f, 8>(vec2f(1.0, 0.0), vec2f(H, H), vec2f(0.0, 1.0), vec2f(-H, H), vec2f(-1.0, 0.0), vec2f(-H, -H), vec2f(0.0, -1.0), vec2f(H, -H));

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let size = bu.info.xy;
  let ndc = vec2f(in.clip.x / size.x * 2.0 - 1.0, 1.0 - in.clip.y / size.y * 2.0);
  let dv = bu.fwd.xyz + ndc.x * bu.right.xyz + ndc.y * bu.up.xyz;
  let dl = length(dv);
  let dir = dv / dl;
  let cam = bu.cam.xyz;
  let t = bu.cam.w;
  let raw = textureLoad(scene_depth, vec2i(in.clip.xy * bu.info.z), 0);
  let hit = raw < 1.0;
  let z = linear_depth(raw);
  let p = cam + dv * z;
  // Surface normal from the depth (screen derivatives, uniform control flow).
  let pn_raw = cross(dpdx(p), dpdy(p));
  let pn = pn_raw / max(length(pn_raw), 1e-12);
  var t_scene = 1e9;
  if (hit) { t_scene = z * dl; }
  let px_per_t = 2.0 * length(bu.up.xyz) / size.y;
  let c = bu.center.xyz;
  let sun = bu.sun.xyz;
  // The dust, aurora and cocoon from the volume pass: a tent of four
  // bilinear taps 1.5 volume texels apart (smooth, hides the march jitter's
  // mottle; D5 widened it from 1 texel).
  let vt = 1.0 / vec2f(textureDimensions(volume_tex));
  let vuv = in.clip.xy / size;
  var acc = 0.25 * (textureSampleLevel(volume_tex, lin_sampler, vuv + vec2f(-0.75, -0.75) * vt, 0.0).rgb
    + textureSampleLevel(volume_tex, lin_sampler, vuv + vec2f(0.75, -0.75) * vt, 0.0).rgb
    + textureSampleLevel(volume_tex, lin_sampler, vuv + vec2f(-0.75, 0.75) * vt, 0.0).rgb
    + textureSampleLevel(volume_tex, lin_sampler, vuv + vec2f(0.75, 0.75) * vt, 0.0).rgb);

  // Gates, portal and sign glow are race markers: they keep their colour in ink levels.
  var marks = vec3f(0.0);
  // Gates: holographic arches in the vertical half planes through the axis.
  let gate_k = bu.dust_color.w;
  if (gate_k > 0.0) {
    var dirs = GATE_DIRS;
    for (var k = 0; k < 8; k++) {
      if (bu.gate_color[k].w <= 0.0) { continue; }
      // cos and sin of k x 45 degrees.
      let cs = dirs[k];
      let radial = vec3f(cs.x, 0.0, -cs.y);
      let nrm = vec3f(-cs.y, 0.0, -cs.x);
      let den = dot(dir, nrm);
      if (abs(den) < 1e-4) { continue; }
      let tg = dot(c - cam, nrm) / den;
      if (tg <= 0.0 || tg >= t_scene) { continue; }
      let q = cam + dir * tg - c;
      let rr = dot(q, radial);
      if (rr < bu.gate_shape.x - 8.0 || rr > bu.gate_shape.y + 8.0) { continue; }
      let yy = q.y;
      if (abs(yy) > bu.shape.y + 17.0) { continue; }
      // Close by, the film fades so the gate never fogs the view; seen edge-on it thins out.
      let near = smoothstep(2.0, 14.0, tg);
      let edge_on = smoothstep(0.02, 0.15, abs(den));
      marks += gate_look(k, rr, yy, tg * px_per_t, t) * mix(0.5, 1.0, near) * edge_on * fog_at(tg * 0.6) * gate_k;
    }
  }

  // The exit portal: a swirling disc facing the viewer at the centre.
  let portal = bu.sun.w;
  let sign_k = bu.sun_color.w;
  let ocv = c - cam;
  let tc = dot(ocv, dir);
  let b2 = max(dot(ocv, ocv) - tc * tc, 0.0);
  let b = sqrt(b2);
  if (tc > 0.0 && portal > 0.0) {
    let re = bu.shape.z * 1.25;
    let rho = b / re;
    if (tc < t_scene + re * 0.5 && rho < 6.0) {
      let side = normalize(cross(dir, select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(dir.y) > 0.9)));
      let upv = cross(side, dir);
      let off = cam + dir * tc - c;
      let phi = atan2(dot(off, upv), dot(off, side));
      let spin = t * bu.extra.w;
      let lr = log(rho + 0.06);
      let arms = 0.5 + 0.5 * sin(5.0 * phi - 7.0 * lr + spin * 2.4);
      let arms2 = 0.5 + 0.5 * sin(3.0 * phi + 11.0 * lr - spin * 1.6);
      let disc = 1.0 - smoothstep(0.92, 1.0, rho);
      let core = exp(-rho * rho * 14.0);
      let ra = (rho - 1.0) / 0.045;
      let rb = (rho - 1.0) / 0.16;
      let rim = exp(-ra * ra) + 0.4 * exp(-rb * rb);
      let halo = 0.08 / (1.0 + pow(max(rho - 1.0, 0.0) * 2.5, 2.0)) * step(0.98, rho);
      let green = vec3f(0.1, 1.0, 0.36);
      let deep = vec3f(0.01, 0.07, 0.05);
      var col = deep * disc + green * disc * (0.55 * arms * arms + 0.3 * arms2) * (0.25 + 0.75 * rho) * 1.6;
      col += vec3f(0.8, 1.0, 0.88) * core * 1.6 + green * rim * 5.0 + green * halo;
      marks += col * portal * fog_at(tc * 0.5);
    }
  }
  // The emergency-exit sign over the portal, turned toward the viewer.
  if (sign_k > 0.0) {
    let sc = c + vec3f(0.0, bu.extra.x, 0.0);
    let to_cam = cam - sc;
    var n = to_cam / max(length(to_cam), 1e-4);
    // Hangs upright, tilting halfway toward a viewer above or below.
    n = normalize(vec3f(n.x, n.y * 0.5, n.z));
    let rgt = normalize(cross(vec3f(0.0, 1.0, 0.0), n));
    let upn = cross(n, rgt);
    let den = dot(dir, n);
    if (den < -1e-4) {
      let ts = dot(sc - cam, n) / den;
      if (ts > 0.0 && ts < t_scene) {
        let q = cam + dir * ts - sc;
        let w = bu.extra.y;
        let hgt = bu.extra.z;
        let u = dot(q, rgt) / w + 0.5;
        let v = 0.5 - dot(q, upn) / hgt;
        let ex = max(abs(u - 0.5) * w - 0.5 * w, 0.0);
        let ey = max(abs(v - 0.5) * hgt - 0.5 * hgt, 0.0);
        let e = length(vec2f(ex, ey));
        let fog = fog_at(ts * 0.35);
        // A soft green glow around the light box (the box itself is drawn opaque by SIGN_WGSL).
        marks += vec3f(0.08, 0.75, 0.3) * exp(-e * 0.3) * 0.22 * sign_k * fog * step(1e-4, e);
      }
    }
  }

  // Ice glints on sunlit rock in the belt.
  let glint_k = bu.gate_shape.w;
  if (glint_k > 0.0 && hit && t_scene < 120.0) {
    let d = p - c;
    let r = length(d.xz);
    let inbelt = smoothstep(bu.center.w - 14.0, bu.center.w - 2.0, r) * (1.0 - smoothstep(bu.shape.x + 2.0, bu.shape.x + 14.0, r)) * (1.0 - smoothstep(bu.shape.y + 4.0, bu.shape.y + 14.0, abs(d.y)));
    let lit = dot(pn, sun);
    if (inbelt > 0.0 && lit > 0.05) {
      let cell_size = 0.55;
      let cell = floor(p / cell_size);
      let hsh = hash33(cell);
      let spot = (cell + 0.2 + 0.6 * hsh) * cell_size;
      let rad = max(0.035, 1.1 * t_scene * px_per_t);
      let dd = length(p - spot);
      let fac = normalize(pn + (hash33(cell + 17.0) - 0.5) * 0.9);
      let spec = pow(max(dot(reflect(-sun, fac), -dir), 0.0), 60.0);
      let rare = step(0.55, hsh.z);
      let tw = 0.6 + 0.4 * sin(t * 7.0 + hsh.x * 40.0);
      acc += vec3f(0.75, 0.9, 1.0) * exp(-(dd * dd) / (rad * rad)) * spec * rare * tw * 14.0 * glint_k * inbelt * fog_at(t_scene);
    }
  }
  // Alpha (blended with max): marker pixels write the colour keep mask (1 + keep x KEEP_ALPHA).
  let ml = dot(marks, vec3f(0.2126, 0.7152, 0.0722));
  let keep = select(0.0, 1.0 + clamp(ml * 6.0, 0.0, 1.0) * ${KEEP_ALPHA}, ml > 0.02);
  return vec4f(acc + marks, keep);
}
`;
}

/** Floats of the sign quad's uniform: view-projection, four corners, params. */
export const SIGN_UNIFORM_FLOATS = 16 + 16 + 8;

/**
 * The exit sign as an opaque light box (SIGN_WGSL): a quad turned toward
 * the viewer, depth-tested against the scene by hand (the scene depth is
 * multisampled), drawn "over" the scene with premultiplied alpha; it keeps
 * its colour in ink levels (the keep mask in alpha, blended with max).
 * Bindings: 0 uniform, 1 scene depth, 2 sign texture, 3 sampler.
 */
export function signWgsl(multisampledDepth: boolean): string {
  const depthType = multisampledDepth ? "texture_depth_multisampled_2d" : "texture_depth_2d";
  return /* wgsl */ `
struct Sign {
  view_proj: mat4x4f,
  corners: array<vec4f, 4>, // xyz world corners: top left, top right, bottom left, bottom right
  params: vec4f,            // x strength, y time (s), z scene depth texel per target pixel, w fog density
  cam: vec4f,               // xyz camera position
};

@group(0) @binding(0) var<uniform> sg: Sign;
@group(0) @binding(1) var scene_depth: ${depthType};
@group(0) @binding(2) var sign_tex: texture_2d<f32>;
@group(0) @binding(3) var lin_sampler: sampler;

fn hash21(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) uv: vec2f,
  @location(1) world: vec3f,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
  // Two triangles: TL TR BL, BL TR BR.
  var idx = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
  let k = idx[i];
  let uv = vec2f(f32(k & 1u), f32(k >> 1u));
  let p = sg.corners[k].xyz;
  var out: VsOut;
  out.clip = sg.view_proj * vec4f(p, 1.0);
  out.uv = uv;
  out.world = p;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let raw = textureLoad(scene_depth, vec2i(in.clip.xy * sg.params.z), 0);
  if (in.clip.z > raw) { discard; }
  let texel = textureSampleLevel(sign_tex, lin_sampler, in.uv, 0.0);
  if (texel.a < 0.01) { discard; }
  let d = length(in.world - sg.cam.xyz);
  let fd = sg.params.w * d * 0.35;
  let fog = exp2(-fd * fd);
  // The odd flicker of an old emergency light.
  let flick = 1.0 - 0.3 * step(0.988, hash21(vec2f(floor(sg.params.y * 12.0), 3.0)));
  let col = texel.rgb * 1.25 * sg.params.x * flick * fog;
  return vec4f(col * texel.a, select(0.0, 1.0 + ${KEEP_ALPHA}, texel.a > 0.5));
}
`;
}

/**
 * Screen-edge radiation (RADIATION_WGSL): bindings 0 uniform (x dose 0..1,
 * y time, z width px, w height px), 1 the scene copy, 2 a linear sampler.
 */
export const RADIATION_WGSL = /* wgsl */ `
struct Rad {
  info: vec4f,
};

@group(0) @binding(0) var<uniform> rad: Rad;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var lin_sampler: sampler;

${COMMON}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let size = rad.info.zw;
  let uv = in.clip.xy / size;
  let k = clamp(rad.info.x, 0.0, 1.0);
  let t = rad.info.y;
  let ctr = uv - 0.5;
  let aspect = size.x / max(size.y, 1.0);
  let e = length(vec2f(ctr.x * aspect, ctr.y)) / length(vec2f(0.5 * aspect, 0.5));
  // The rim: nothing in the middle of the view (the course stays readable);
  // any dose shows there at once and the band widens a little with it (D5:
  // danger at the edges, no colour static over the view).
  let rim = smoothstep(0.5 - 0.12 * k, 1.0, e);
  let s = 0.4 + 0.6 * k;
  let edge = rim * s;
  // The radiation's pulse, quicker the hotter it is (with the Geiger crackle).
  let pulse = 0.5 + 0.5 * sin(t * (3.0 + 7.0 * k));
  // Chromatic fringe: red and blue pulled apart along the radius, a few
  // pixels at the rim (a wide split doubles every star and edge).
  let off = ctr * 0.008 * edge;
  let base = textureSampleLevel(scene, lin_sampler, uv, 0.0);
  let r = textureSampleLevel(scene, lin_sampler, uv + off, 0.0).r;
  let bl = textureSampleLevel(scene, lin_sampler, uv - off, 0.0).b;
  let split = min(1.0, edge * 2.0);
  var col = vec3f(mix(base.r, r, split), base.g, mix(base.b, bl, split));
  // Burnt edges: the picture darkens and turns sickly toward the rim ...
  col = mix(col, col * vec3f(0.5, 0.7, 0.42), edge * 0.7);
  // ... under a toxic glow that breathes with the pulse.
  let glow = vec3f(0.55, 1.0, 0.18);
  col += glow * edge * edge * (0.08 + 0.14 * pulse);
  // Sparse static: a few 3 px flecks right at the rim (not snow over the
  // view), jumping 20 times a second.
  let frame = floor(t * 20.0);
  let cell = floor(in.clip.xy / 3.0);
  let fleck = step(1.0 - 0.02 * edge * edge * edge, hash31(vec3f(cell, frame)));
  col += glow * fleck * 0.35;
  return vec4f(col, base.a);
}
`;
