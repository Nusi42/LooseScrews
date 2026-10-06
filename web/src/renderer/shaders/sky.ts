// Sky and atmosphere shaders (VQ7), driven by renderer/sky.ts:
// - ENV_BAKE_WGSL bakes the world's environment cube once per world: the
//   haze gradient, the sun's wide halo, the nebula (domain-warped gradient
//   noise with dust lanes, concentrated in a galactic band) and unresolved
//   star dust. Each mip is evaluated directly with fewer octaves, so the
//   high mips are a smooth version of the sky; the fog (frame.ts
//   `apply_fog`) samples one of them, so distant rock fades into the sky
//   behind it, tinted toward the sun and the nebula.
// - PLANET_BAKE_WGSL bakes the planet's surface (equirectangular) once per
//   world.
// - SKY_WGSL draws the sky under the resolved scene in open levels (blend
//   one-minus-dst-alpha: the scene's alpha is 1 on geometry, 0 on the
//   clear, which the light shafts also use as the sky mask): the baked
//   cube, two procedural star layers, the sun disc and corona, a planet
//   (gas giant, ocean or dry world, lit by the sun, atmosphere rim, optional
//   rings with the planet's shadow) and the boundary grid where the camera
//   nears the level box.
// - SHAFT_*_WGSL: screen-space light shafts (radial blur toward the sun at
//   quarter resolution, added onto the HDR scene before bloom).
//
// Output is linear HDR radiance like the rest of the scene.

/** Fullscreen triangle; uv (0,0) is the top-left texel. */
const FULLSCREEN_VS = /* wgsl */ `
struct FsOut {
  @builtin(position) clip: vec4f,
  @location(0) uv: vec2f,
  @location(1) @interpolate(flat) inst: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32, @builtin(instance_index) inst: u32) -> FsOut {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
  var out: FsOut;
  out.clip = vec4f(p, 0.0, 1.0);
  out.uv = vec2f(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
  out.inst = inst;
  return out;
}
`;

/** Integer hash (pcg3d) and 3D gradient noise with fbm; no sin hashes, no tiling. */
export const NOISE_WGSL = /* wgsl */ `
fn pcg3d(v0: vec3u) -> vec3u {
  var v = v0 * 1664525u + 1013904223u;
  v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
  v ^= v >> vec3u(16u);
  v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
  return v;
}

fn hash33(p: vec3f) -> vec3f {
  return vec3f(pcg3d(bitcast<vec3u>(vec3i(p)))) * (1.0 / 4294967295.0);
}

fn grad_corner(i: vec3f, f: vec3f, o: vec3f) -> f32 {
  let g = hash33(i + o) * 2.0 - 1.0;
  return dot(g, f - o);
}

// Gradient noise, roughly in [-1, 1].
fn gnoise(p: vec3f) -> f32 {
  let i = floor(p);
  let f = p - i;
  let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  let x00 = mix(grad_corner(i, f, vec3f(0.0, 0.0, 0.0)), grad_corner(i, f, vec3f(1.0, 0.0, 0.0)), u.x);
  let x10 = mix(grad_corner(i, f, vec3f(0.0, 1.0, 0.0)), grad_corner(i, f, vec3f(1.0, 1.0, 0.0)), u.x);
  let x01 = mix(grad_corner(i, f, vec3f(0.0, 0.0, 1.0)), grad_corner(i, f, vec3f(1.0, 0.0, 1.0)), u.x);
  let x11 = mix(grad_corner(i, f, vec3f(0.0, 1.0, 1.0)), grad_corner(i, f, vec3f(1.0, 1.0, 1.0)), u.x);
  return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z) * 1.6;
}

// Fractal sum with a rotation per octave (no axis-aligned artefacts).
fn fbm(p0: vec3f, octaves: i32) -> f32 {
  let rot = mat3x3f(0.0, 0.8, 0.6, -0.8, 0.36, -0.48, -0.6, -0.48, 0.64);
  var p = p0;
  var sum = 0.0;
  var amp = 0.5;
  for (var i = 0; i < octaves; i++) {
    sum += amp * gnoise(p);
    p = rot * p * 2.03;
    amp *= 0.5;
  }
  return sum;
}
`;

/** Floats of the environment bake uniform (struct Env). */
export const ENV_UNIFORM_FLOATS = 40;

/**
 * Bakes one face and mip of the environment cube per draw: instance index =
 * face + 6 x mip (faces +X, -X, +Y, -Y, +Z, -Z in WebGPU's cube layout).
 */
export const ENV_BAKE_WGSL = /* wgsl */ `
${FULLSCREEN_VS}
${NOISE_WGSL}
struct Env {
  zenith: vec4f,      // rgb, w = nebula strength
  horizon: vec4f,     // rgb, w = star dust strength
  nadir: vec4f,       // rgb, w = base size of the cube (texels)
  neb_a: vec4f,       // rgb, w = band width
  neb_b: vec4f,
  neb_core: vec4f,
  band: vec4f,        // xyz galactic band normal
  offset: vec4f,      // xyz noise offset (seed)
  sun_dir: vec4f,     // xyz towards the sun
  sun_color: vec4f,   // rgb (linear), w = halo strength
};
@group(0) @binding(0) var<uniform> env: Env;

// Cube face texel (s, t in [-1, 1], t down) to direction (OpenGL/D3D/WebGPU table).
fn face_dir(face: u32, s: f32, t: f32) -> vec3f {
  switch face {
    case 0u: { return vec3f(1.0, -t, -s); }
    case 1u: { return vec3f(-1.0, -t, s); }
    case 2u: { return vec3f(s, 1.0, t); }
    case 3u: { return vec3f(s, -1.0, -t); }
    case 4u: { return vec3f(s, -t, 1.0); }
    default: { return vec3f(-s, -t, -1.0); }
  }
}

fn env_radiance(d: vec3f, mip: f32, texel: vec3f) -> vec3f {
  // Haze gradient: nadir below, horizon, zenith above.
  let y = d.y;
  var c = mix(env.horizon.rgb, env.zenith.rgb, smoothstep(0.0, 0.9, y));
  c = mix(c, env.nadir.rgb, smoothstep(0.0, -0.9, y));
  // The sun's wide halo (forward scattering in the haze).
  let cs = max(dot(d, env.sun_dir.xyz), 0.0);
  c += env.sun_color.rgb * env.sun_color.w * (0.006 * pow(cs, 4.0) + 0.05 * pow(cs, 40.0) + 0.5 * pow(cs, 600.0));
  let strength = env.zenith.w;
  if (strength > 0.0) {
    let octaves = i32(clamp(8.0 - mip, 2.0, 8.0));
    let p = d * 2.1 + env.offset.xyz;
    let bd = dot(d, env.band.xyz);
    let band = exp(-bd * bd / env.neb_a.w);
    let warp = vec3f(fbm(p, 4), fbm(p + vec3f(5.2, 1.3, 2.8), 4), fbm(p + vec3f(9.7, 4.1, 7.3), 4));
    let n = fbm(p * 1.4 + warp * 1.6, octaves) * 0.5 + 0.5;
    // Large-scale mask: distinct clouds with empty space between them.
    let mask = smoothstep(0.4, 0.75, fbm(p * 0.45 + vec3f(11.0), 3) * 0.5 + 0.5 + 0.25 * band);
    let dens = pow(smoothstep(0.42, 0.92, n), 1.4) * mask * (0.2 + 0.8 * band);
    let hue = smoothstep(0.25, 0.75, fbm(p * 0.6 + vec3f(3.3), 3) * 0.5 + 0.5);
    var neb = mix(env.neb_a.rgb, env.neb_b.rgb, hue) * dens * 0.22;
    neb += env.neb_core.rgb * pow(dens, 3.0) * 0.55;
    // Dark dust lanes (ridged noise) in the band.
    let ridge = 1.0 - abs(fbm(p * 1.9 + warp + vec3f(7.7), octaves));
    let dust = smoothstep(0.72, 0.95, ridge) * (0.3 + 0.7 * band);
    neb *= 1.0 - 0.85 * dust;
    // Diffuse glow of the band itself (unresolved stars).
    neb += mix(env.neb_a.rgb, vec3f(0.9, 0.9, 1.0), 0.6) * band * 0.012 * (1.0 - 0.7 * dust);
    c = c * (1.0 - 0.5 * dust * band * strength) + neb * strength;
    // Star dust: faint unresolved stars, dense in the band (base mip only).
    if (mip < 0.5 && env.horizon.w > 0.0) {
      let h = hash33(texel + vec3f(17.0, 31.0, 7.0));
      let star = step(0.988 - 0.01 * band, h.x) * (0.02 + 0.12 * h.y * h.y) * (0.4 + band);
      c += mix(vec3f(1.0, 0.82, 0.66), vec3f(0.75, 0.85, 1.0), h.z) * star * env.horizon.w;
    }
  }
  return c;
}

@fragment
fn fs_main(in: FsOut) -> @location(0) vec4f {
  let face = in.inst % 6u;
  let mip = f32(in.inst / 6u);
  let size = env.nadir.w / exp2(mip);
  let st = in.uv * 2.0 - 1.0;
  let d = normalize(face_dir(face, st.x, st.y));
  let texel = vec3f(floor(in.uv * size), f32(face));
  return vec4f(env_radiance(d, mip, texel), 1.0);
}
`;

/** Floats of the sky pass uniform (struct Sky). */
export const SKY_UNIFORM_FLOATS = 68;

export const SKY_WGSL = /* wgsl */ `
${FULLSCREEN_VS}
${NOISE_WGSL}
struct Sky {
  right: vec4f,       // camera right x tan(half fov x), w = angular size of a pixel (rad)
  up: vec4f,          // camera up x tan(half fov y), w = time
  fwd: vec4f,         // camera forward, w = star brightness
  cam: vec4f,         // camera position, w = boundary grid strength
  sun: vec4f,         // xyz towards the sun, w = disc angular radius
  sun_color: vec4f,   // rgb linear
  planet: vec4f,      // xyz direction, w = angular radius (0 = none)
  axis: vec4f,        // xyz spin axis, w = ring outer radius in planet radii (0 = none)
  c0: vec4f,          // rgb, w = kind
  c1: vec4f,          // rgb, w = seed
  c2: vec4f,          // rgb, w = cosine of the cone around the planet that holds its disc, halo and rings
  air: vec4f,
  room_min: vec4f,
  room_max: vec4f,
  viewport: vec4f,    // xy size in pixels
  band: vec4f,        // xyz galactic band normal, w = band width
  turn: vec4f,        // unit quaternion from world directions to the sky's frame (M4 B5: Bored level turns the sky)
};
@group(0) @binding(0) var<uniform> sky: Sky;

fn sky_frame(d: vec3f) -> vec3f {
  let q = sky.turn;
  return d + 2.0 * cross(q.xyz, cross(q.xyz, d) + q.w * d);
}
@group(0) @binding(1) var env_cube: texture_cube<f32>;
@group(0) @binding(2) var env_samp: sampler;
@group(0) @binding(3) var planet_tex: texture_2d<f32>;
@group(0) @binding(4) var planet_samp: sampler;

// Direction of a cell centre on a cube face (any consistent layout works here).
fn cube_point(face: u32, uv: vec2f) -> vec3f {
  switch face {
    case 0u: { return vec3f(1.0, uv.x, uv.y); }
    case 1u: { return vec3f(-1.0, uv.x, uv.y); }
    case 2u: { return vec3f(uv.x, 1.0, uv.y); }
    case 3u: { return vec3f(uv.x, -1.0, uv.y); }
    case 4u: { return vec3f(uv.x, uv.y, 1.0); }
    default: { return vec3f(uv.x, uv.y, -1.0); }
  }
}

// One layer of point stars: at most one star per cube-face cell, kept away
// from the cell border so a single cell lookup per pixel suffices; the
// Gaussian footprint is about a pixel wide (narrower where cells are small,
// with the flux kept) so stars neither alias nor get cut.
fn star_layer(d: vec3f, cells: f32, prob: f32, layer: u32, bright: f32, power: f32, density: f32) -> vec3f {
  let a = abs(d);
  var face: u32;
  var uv: vec2f;
  if (a.x >= a.y && a.x >= a.z) { face = select(1u, 0u, d.x > 0.0); uv = d.yz / a.x; }
  else if (a.y >= a.z) { face = select(3u, 2u, d.y > 0.0); uv = d.xz / a.y; }
  else { face = select(5u, 4u, d.z > 0.0); uv = d.xy / a.z; }
  let g = (uv * 0.5 + 0.5) * cells;
  let cell = floor(g);
  let h = vec3f(pcg3d(vec3u(vec2u(cell), face + 8u * layer + 977u))) * (1.0 / 4294967295.0);
  let p = prob * density;
  if (h.x >= p) { return vec3f(0.0); }
  let r = h.x / p; // uniform 0..1 for the stars that exist
  let suv = ((cell + 0.25 + 0.5 * h.yz) / cells) * 2.0 - 1.0;
  let dist = length(d - normalize(cube_point(face, suv)));
  let cell_ang = (2.0 / cells) / (1.0 + dot(uv, uv));
  let px = sky.right.w;
  let sigma = min(px * 0.65, cell_ang * 0.08);
  let flux = (px * 0.65 / sigma) * (px * 0.65 / sigma);
  let e = dist / sigma;
  let mag = bright * pow(fract(r * 7.31 + h.y * 3.7), power);
  let tint = mix(vec3f(1.0, 0.72, 0.5), vec3f(0.72, 0.84, 1.0), smoothstep(0.15, 0.85, r));
  return mix(vec3f(1.0), tint, 0.35 + 0.5 * fract(h.z * 5.3)) * mag * flux * exp(-0.5 * e * e);
}

struct PlanetHit { color: vec3f, cover: f32 };

fn planet_frame(axis: vec3f) -> mat3x3f {
  let t = normalize(cross(axis, select(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 0.0, 1.0), abs(axis.x) > 0.9)));
  let b = cross(t, axis);
  return mat3x3f(t, axis, b);
}

fn planet(d: vec3f) -> PlanetHit {
  var hit: PlanetHit;
  hit.color = vec3f(0.0);
  hit.cover = 0.0;
  let ang_r = sky.planet.w;
  if (ang_r <= 0.0) { return hit; }
  let pc = sky.planet.xyz;
  let px = sky.right.w;
  let b = dot(d, pc);
  if (b < sky.c2.w) { return hit; }
  let ang = acos(clamp(b, -1.0, 1.0));
  let rs = sin(ang_r);
  let sun = sky.sun.xyz;
  // Atmosphere glow just outside the limb, on the sunlit side.
  let perp = d - pc * b;
  let limb_n = perp / max(length(perp), 1e-6);
  let edge = ang - ang_r;
  var col = vec3f(0.0);
  if (edge > 0.0) {
    col = sky.air.rgb * exp(-edge / (0.035 * ang_r)) * smoothstep(-0.35, 0.5, dot(limb_n, sun)) * 0.7;
  }
  let cover = clamp((ang_r - ang) / px + 0.5, 0.0, 1.0);
  // Ray from the viewer (the planet centre sits at distance 1).
  let disc = max(b * b - (1.0 - rs * rs), 0.0);
  let t = b - sqrt(disc);
  let n = normalize(d * t - pc);
  var surf = vec3f(0.0);
  if (cover > 0.0) {
    let ndl = dot(n, sun);
    let lit = pow(clamp(ndl * 1.05 + 0.04, 0.0, 1.0), 1.15);
    let f = planet_frame(sky.axis.xyz);
    let l = vec3f(dot(n, f[0]), dot(n, f[1]), dot(n, f[2]));
    let puv = vec2f(atan2(l.z, l.x) * (0.5 / 3.14159265) + 0.5, 0.5 - asin(clamp(l.y, -1.0, 1.0)) / 3.14159265);
    let albedo = textureSampleLevel(planet_tex, planet_samp, puv, 0.0).rgb;
    // Faint light on the night side (nebula and ring shine), so it is not a hole.
    surf = albedo * lit * sky.sun_color.rgb * 1.6 + (albedo * 0.012 + sky.air.rgb * 0.004);
    let mu = max(dot(n, -d), 0.0);
    let fres = pow(1.0 - mu, 3.0);
    surf += sky.air.rgb * fres * smoothstep(-0.25, 0.45, ndl) * 1.4;
  }
  col = mix(col, surf, cover);
  // Rings in the equatorial plane, in front of or behind the disc.
  let rings = sky.axis.w;
  if (rings > 0.0) {
    let ax = sky.axis.xyz;
    let denom = dot(d, ax);
    if (abs(denom) > 1e-4) {
      let tr = dot(pc, ax) / denom;
      let r = length(d * tr - pc) / rs;
      if (tr > 0.0 && r > 1.25 && r < rings) {
        let rr = (r - 1.25) / (rings - 1.25);
        let bands = 0.5 + 0.45 * gnoise(vec3f(rr * 19.0, sky.c1.w, 0.5)) + 0.3 * gnoise(vec3f(rr * 63.0, 1.0, sky.c1.w));
        let gap = smoothstep(0.015, 0.035, abs(rr - 0.62)) * smoothstep(0.004, 0.012, abs(rr - 0.33));
        let alpha = clamp(bands, 0.0, 1.0) * gap * smoothstep(0.0, 0.06, rr) * smoothstep(1.0, 0.9, rr) * 0.85;
        // Planet shadow on the rings.
        let oc = d * tr - pc;
        let sb = dot(oc, sun);
        let shadowed = sb < 0.0 && sb * sb - (dot(oc, oc) - rs * rs) > 0.0;
        let dusty = mix(vec3f(0.62, 0.6, 0.57), mix(sky.c2.rgb, sky.c0.rgb, rr), 0.35);
        let ring_col = dusty * (0.8 + 0.4 * gnoise(vec3f(rr * 140.0, 7.0, sky.c1.w))) * sky.sun_color.rgb * select(1.3, 0.03, shadowed);
        let in_front = cover <= 0.0 || tr < t;
        let a = alpha * select(1.0 - cover, 1.0, in_front);
        col = mix(col, ring_col, a);
        hit.cover = max(cover, a);
        hit.color = col;
        return hit;
      }
    }
  }
  hit.cover = cover;
  hit.color = col;
  return hit;
}

// Boundary grid on the level box where the camera comes near it (the
// sealing walls are hidden in open levels, the collision is not).
fn boundary(d: vec3f) -> vec3f {
  let strength = sky.cam.w;
  let o = sky.cam.xyz;
  let lo = sky.room_min.xyz;
  let hi = sky.room_max.xyz;
  let inv = vec3f(1.0) / select(d, vec3f(1e-6), abs(d) < vec3f(1e-6));
  let tf = (select(lo, hi, d > vec3f(0.0)) - o) * inv;
  let t = min(tf.x, min(tf.y, tf.z));
  let p = o + d * t;
  var uv = p.xz;
  var facing = abs(d.y);
  if (t == tf.x) { uv = p.yz; facing = abs(d.x); } else if (t == tf.z) { uv = p.xy; facing = abs(d.z); }
  let g = uv * 0.5;
  let w = max(fwidth(g), vec2f(1e-4));
  let a = abs(fract(g - 0.5) - 0.5) / w;
  let line = 1.0 - clamp(min(a.x, a.y) - 0.25, 0.0, 1.0);
  // A patch of field near the camera; grazing views (long, dense lines) fade out.
  let fade = (1.0 - smoothstep(1.5, 9.0, t)) * select(0.0, 1.0, t > 0.0) * strength * smoothstep(0.12, 0.55, facing);
  let density = clamp(1.4 - max(w.x, w.y) * 3.0, 0.0, 1.0);
  return vec3f(0.25, 0.72, 1.0) * (line * density * 0.7 + 0.025) * fade * fade;
}

@fragment
fn fs_main(in: FsOut) -> @location(0) vec4f {
  let ndc = vec2f(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
  let dw = normalize(sky.fwd.xyz + ndc.x * sky.right.xyz + ndc.y * sky.up.xyz);
  // Derivatives first (uniform control flow: the grid strength is a uniform).
  // The boundary grid stays on the level box; everything else turns with the sky.
  var grid = vec3f(0.0);
  if (sky.cam.w > 0.0) { grid = boundary(dw); }
  let d = sky_frame(dw);
  var c = textureSampleLevel(env_cube, env_samp, d, 0.0).rgb;
  // More stars along the galactic band.
  let bd = dot(d, sky.band.xyz);
  let band = exp(-bd * bd / sky.band.w);
  var stars = star_layer(d, 150.0, 0.5, 0u, 0.55, 4.0, 0.25 + 0.75 * band);
  stars += star_layer(d, 44.0, 0.3, 1u, 8.0, 9.0, 0.6 + 0.4 * band);
  c += stars * sky.fwd.w;
  // Sun disc with limb darkening and corona.
  let ds = length(d - sky.sun.xyz);
  let rs = sky.sun.w;
  let px = sky.right.w;
  let disc = smoothstep(rs + px, rs - px, ds);
  let mu = sqrt(max(1.0 - (ds / rs) * (ds / rs), 0.0));
  let x = ds / rs;
  var sun = sky.sun_color.rgb * (disc * 40.0 * (0.45 + 0.55 * mu));
  sun += sky.sun_color.rgb * (1.6 * exp(-max(x - 1.0, 0.0) * 1.8) * (1.0 - disc) + 0.35 * exp(-x * 0.18));
  c += sun;
  let p = planet(d);
  // The planet hides what lies behind it (stars, sun, nebula).
  c = mix(c, p.color, p.cover) + select(p.color, vec3f(0.0), p.cover > 0.0);
  c += grid;
  return vec4f(c, 0.0);
}
`;

/** Floats of the planet bake uniform (struct PlanetBake). */
export const PLANET_UNIFORM_FLOATS = 12;

/**
 * Bakes the planet's surface albedo (linear) into an equirectangular
 * texture once per world: u = longitude around the spin axis, v = latitude
 * (top = north). The sky pass samples it, so the planet costs one texture
 * read per pixel instead of a stack of noise octaves.
 */
export const PLANET_BAKE_WGSL = /* wgsl */ `
${FULLSCREEN_VS}
${NOISE_WGSL}
struct PlanetBake {
  c0: vec4f,          // rgb, w = kind
  c1: vec4f,          // rgb, w = seed
  c2: vec4f,
};
@group(0) @binding(0) var<uniform> sky: PlanetBake;

fn planet_surface(l: vec3f) -> vec3f {
  let seed = vec3f(sky.c1.w * 0.37, sky.c1.w * 0.11, 3.0);
  let kind = sky.c0.w;
  if (kind < 0.5) {
    // Gas giant: latitude bands, turbulent edges, one storm.
    let turb = fbm(l * vec3f(2.5, 7.0, 2.5) + seed, 5);
    let lat = l.y * 6.0 + turb * 0.5;
    let b1 = 0.5 + 0.5 * sin(lat * 2.9);
    let b2 = 0.5 + 0.5 * sin(lat * 5.3 + 1.7);
    var c = mix(sky.c1.rgb, sky.c0.rgb, b1);
    c = mix(c, sky.c2.rgb, smoothstep(0.65, 0.95, b2) * 0.6);
    let storm_at = normalize(vec3f(0.6, -0.35, 0.72));
    let sd = length(l - storm_at) / 0.12;
    c = mix(c, sky.c1.rgb * 1.25, smoothstep(1.0, 0.4, sd) * 0.7);
    return c;
  }
  if (kind < 1.5) {
    // Ocean world: continents and clouds.
    let land = smoothstep(0.04, 0.1, fbm(l * 2.2 + seed, 6));
    var c = mix(sky.c0.rgb, sky.c1.rgb * (0.8 + 0.4 * fbm(l * 9.0, 3)), land);
    let polar = smoothstep(0.82, 0.9, abs(l.y));
    c = mix(c, sky.c2.rgb, polar);
    let cloud = smoothstep(0.05, 0.45, fbm(l * vec3f(3.0, 6.0, 3.0) + seed * 1.7 + fbm(l * 2.0, 3), 5));
    return mix(c, sky.c2.rgb, cloud * 0.85);
  }
  // Dry or icy world: warped dark and pale terrains, ridges, polar caps.
  let warp = vec3f(fbm(l * 1.6 + seed, 3), fbm(l * 1.6 + seed.yzx + vec3f(4.1), 3), 0.0);
  let n1 = fbm(l * 2.4 + seed + warp * 0.9, 7) * 0.5 + 0.5;
  var c = mix(sky.c1.rgb, sky.c0.rgb, smoothstep(0.35, 0.6, n1));
  c = mix(c, sky.c2.rgb, smoothstep(0.62, 0.8, n1) * 0.7);
  let ridge = 1.0 - abs(fbm(l * 6.0 + seed.zxy + warp, 4));
  c *= 0.85 + 0.3 * smoothstep(0.7, 0.98, ridge);
  c *= 0.9 + 0.2 * (fbm(l * 18.0 + seed, 3) * 0.5 + 0.5);
  return mix(c, sky.c2.rgb, smoothstep(0.78, 0.86, abs(l.y) + 0.06 * warp.x));
}

@fragment
fn fs_main(in: FsOut) -> @location(0) vec4f {
  let lon = (in.uv.x - 0.5) * 6.2831853;
  let lat = (0.5 - in.uv.y) * 3.14159265;
  let l = vec3f(cos(lat) * cos(lon), sin(lat), cos(lat) * sin(lon));
  return vec4f(planet_surface(l), 1.0);
}
`;

/** Floats of the shaft uniform (struct Shaft). */
export const SHAFT_UNIFORM_FLOATS = 12;

const SHAFT_HEADER = /* wgsl */ `
${FULLSCREEN_VS}
struct Shaft {
  sun_uv: vec4f,      // xy sun position in uv, z = aspect (w / h), w = fade
  color: vec4f,       // rgb sun colour x strength
  texel: vec4f,       // xy texel of the source, z = decay, w = samples
};
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> sh: Shaft;
`;

/** Quarter-resolution sky mask weighted by the distance from the sun. */
export const SHAFT_PREFILTER_WGSL = /* wgsl */ `
${SHAFT_HEADER}
@fragment
fn fs_main(in: FsOut) -> @location(0) vec4f {
  let t = sh.texel.xy;
  var a = textureSampleLevel(src, samp, in.uv + vec2f(-t.x, -t.y), 0.0).a;
  a += textureSampleLevel(src, samp, in.uv + vec2f(t.x, -t.y), 0.0).a;
  a += textureSampleLevel(src, samp, in.uv + vec2f(-t.x, t.y), 0.0).a;
  a += textureSampleLevel(src, samp, in.uv + vec2f(t.x, t.y), 0.0).a;
  let open = 1.0 - clamp(a * 0.25, 0.0, 1.0);
  let dv = (in.uv - sh.sun_uv.xy) * vec2f(sh.sun_uv.z, 1.0);
  let r = length(dv);
  let w = exp(-r * 6.0) + 0.04 * exp(-r * 1.5);
  return vec4f(sh.color.rgb * open * w, 1.0);
}
`;

/** Radial blur toward the sun (decaying samples, jittered against banding). */
export const SHAFT_BLUR_WGSL = /* wgsl */ `
${SHAFT_HEADER}
@fragment
fn fs_main(in: FsOut) -> @location(0) vec4f {
  let n = i32(sh.texel.w);
  let delta = (in.uv - sh.sun_uv.xy) * (0.92 / f32(n));
  // Interleaved gradient noise.
  let jitter = fract(52.9829189 * fract(dot(in.clip.xy, vec2f(0.06711056, 0.00583715))));
  var coord = in.uv - delta * jitter;
  var illum = 1.0;
  var sum = vec3f(0.0);
  for (var i = 0; i < n; i++) {
    sum += textureSampleLevel(src, samp, coord, 0.0).rgb * illum;
    illum *= sh.texel.z;
    coord -= delta;
  }
  return vec4f(sum / f32(n), 1.0);
}
`;

/** Adds the blurred shafts onto the HDR scene (additive blend, alpha kept). */
export const SHAFT_COMPOSITE_WGSL = /* wgsl */ `
${SHAFT_HEADER}
@fragment
fn fs_main(in: FsOut) -> @location(0) vec4f {
  // Keep the shafts around the sun: the radial blur alone spreads a veil over the whole sky.
  let r = length((in.uv - sh.sun_uv.xy) * vec2f(sh.sun_uv.z, 1.0));
  return vec4f(textureSampleLevel(src, samp, in.uv, 0.0).rgb * sh.sun_uv.w * exp(-r * 2.2), 0.0);
}
`;
