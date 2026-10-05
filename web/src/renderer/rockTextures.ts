// Procedural rock textures and the terrain material bind group (VQ3b,
// docs/visual-quality-plan.md §3). At start-up a compute shader fills a
// 512² two-layer rgba8unorm texture array with tiling noise and builds its
// mip chain, so the terrain shader samples smooth, filtered detail instead of
// the old `fract(sin(...))` per-cube hash (blocky checkerboard, `sin`
// precision streaks at level-scale coordinates):
//
//   layer 0 "rock"  (sampled about once per 9 u, and its height once per
//     31 u as the macro albedo blotch):
//     rg = height slope (d/du, d/dv; 0.5 = flat), b = height (fbm with
//     Worley cracks), a = mineral vein ramp (1 on thin warped lines,
//     falling off over a few texels, in patches; hard rock's glossy veins)
//   layer 1 "grain" (sampled about once per 2.7 u):
//     rg = slope of fine grain and pits, b = grain height, a = a second
//     independent grain value (speckle)
//
// Everything is periodic over the texture (gradient noise with wrapped
// lattice cells, integer PCG hashes), so the samplers repeat seamlessly.
// Group 1 of the terrain pipeline: the texture, a trilinear anisotropic
// sampler and the per-world `TerrainParams` (strata tilt, texture offsets).

/** Texture edge (texels). */
export const ROCK_TEXTURE_SIZE = 512;
/** Layers of the texture array (rock, grain). */
export const ROCK_TEXTURE_LAYERS = 2;
/** Floats of the TerrainParams uniform (3 vec4). */
export const TERRAIN_PARAM_FLOATS = 12;
/** Workgroup edge of the generator and mip compute shaders. */
const WG = 8;

/** Mip levels of a square texture of edge `size` down to 1x1. */
export function mipLevelCount(size: number): number {
  return Math.floor(Math.log2(Math.max(1, size))) + 1;
}

/** 32-bit FNV-1a of a string (world seed for the terrain look). */
export function hashSeed(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

/** mulberry32 step: deterministic floats in [0, 1) from a seed. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Largest strata tilt from the world's Y axis (radians). */
export const MAX_STRATA_TILT = (28 * Math.PI) / 180;

/**
 * TerrainParams for a world seed: `strata` = unit strata normal (tilted up
 * to MAX_STRATA_TILT from +Y) and bands per u, `offset` = texture offsets
 * (so worlds do not share the same rock pattern), `look` = x strata
 * contrast, y vein amount, zw reserved.
 */
export function terrainParams(seed: number): Float32Array {
  const r = rng(seed ^ 0x9e3779b9);
  const tilt = r() * MAX_STRATA_TILT;
  const azimuth = r() * Math.PI * 2;
  const s = Math.sin(tilt);
  const out = new Float32Array(TERRAIN_PARAM_FLOATS);
  out.set([s * Math.cos(azimuth), Math.cos(tilt), s * Math.sin(azimuth), 0.055 + 0.04 * r()], 0);
  out.set([r(), r(), r(), r()], 4);
  out.set([0.7 + 0.5 * r(), 0.6 + 0.4 * r(), 0, 0], 8);
  return out;
}

/** WGSL of the generator: one invocation per texel of one layer. */
export const ROCK_GEN_WGSL = /* wgsl */ `
@group(0) @binding(0) var dst: texture_storage_2d_array<rgba8unorm, write>;

const SIZE: f32 = ${ROCK_TEXTURE_SIZE}.0;
const TAU: f32 = 6.28318530718;

fn pcg2(v0: vec2u) -> vec2u {
  var v = v0 * 1664525u + 1013904223u;
  v.x += v.y * 1664525u;
  v.y += v.x * 1664525u;
  v = v ^ (v >> vec2u(16u));
  v.x += v.y * 1664525u;
  v.y += v.x * 1664525u;
  v = v ^ (v >> vec2u(16u));
  return v;
}

fn rand2(cell: vec2i, period: i32, seed: u32) -> vec2f {
  let c = vec2u(((cell % period) + period) % period);
  let h = pcg2(c + vec2u(seed * 7919u, seed * 104729u));
  return vec2f(h) / 4294967295.0;
}

// Periodic gradient noise (period in lattice cells), about [-0.7, 0.7].
fn gnoise(p: vec2f, period: i32, seed: u32) -> f32 {
  let i = vec2i(floor(p));
  let f = p - floor(p);
  let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  let ga = rand2(i, period, seed).x * TAU;
  let gb = rand2(i + vec2i(1, 0), period, seed).x * TAU;
  let gc = rand2(i + vec2i(0, 1), period, seed).x * TAU;
  let gd = rand2(i + vec2i(1, 1), period, seed).x * TAU;
  let a = dot(vec2f(cos(ga), sin(ga)), f);
  let b = dot(vec2f(cos(gb), sin(gb)), f - vec2f(1.0, 0.0));
  let c = dot(vec2f(cos(gc), sin(gc)), f - vec2f(0.0, 1.0));
  let d = dot(vec2f(cos(gd), sin(gd)), f - vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Fractal sum over the unit square x in [0,1)^2, base period p0 cells.
fn fbm(x: vec2f, p0: i32, octaves: i32, seed: u32) -> f32 {
  var sum = 0.0;
  var amp = 0.5;
  var period = p0;
  for (var o = 0; o < octaves; o++) {
    sum += amp * gnoise(x * f32(period), period, seed + u32(o) * 31u);
    amp *= 0.5;
    period *= 2;
  }
  return sum;
}

// Periodic Worley noise: (F1, F2) distances in cells.
fn worley(x: vec2f, period: i32, seed: u32) -> vec2f {
  let p = x * f32(period);
  let i = vec2i(floor(p));
  let f = p - floor(p);
  var f1 = 9.0;
  var f2 = 9.0;
  for (var y = -1; y <= 1; y++) {
    for (var xx = -1; xx <= 1; xx++) {
      let o = vec2i(xx, y);
      let d = length(vec2f(o) + rand2(i + o, period, seed) - f);
      if (d < f1) { f2 = f1; f1 = d; } else if (d < f2) { f2 = d; }
    }
  }
  return vec2f(f1, f2);
}

// Ridged fractal: sharp crests where the noise crosses zero.
fn ridged(x: vec2f, p0: i32, octaves: i32, seed: u32) -> f32 {
  var sum = 0.0;
  var amp = 0.5;
  var period = p0;
  for (var o = 0; o < octaves; o++) {
    let r = 1.0 - abs(gnoise(x * f32(period), period, seed + u32(o) * 17u) * 1.4);
    sum += amp * r * r;
    amp *= 0.5;
    period *= 2;
  }
  return sum;
}

// Layer 0: fractured rock relief. Domain-warped fbm (rounded lumps), ridged
// crests, and Worley cracks kept only on part of the cell edges (a mask), so
// no regular cell network shows.
fn rock_height(x: vec2f) -> f32 {
  let warp = vec2f(fbm(x, 3, 3, 11u), fbm(x + vec2f(0.31, 0.77), 3, 3, 12u)) * 0.09;
  let q = x + warp;
  let base = fbm(q, 3, 6, 1u);
  let crest = ridged(q, 5, 3, 3u);
  let w = worley(q, 6, 5u);
  let mask = smoothstep(0.0, 0.2, fbm(q, 4, 2, 9u) + 0.05);
  let crack = (1.0 - smoothstep(0.0, 0.07, w.y - w.x)) * mask;
  return base * 0.85 + (crest - 0.45) * 0.4 - crack * 0.16;
}

// Layer 1: fine grain (no regular pores).
fn grain_height(x: vec2f) -> f32 {
  return fbm(x, 12, 5, 41u) * 0.85 + (ridged(x, 20, 2, 47u) - 0.4) * 0.25;
}

// Thin bright mineral veins: zero lines of warped noise at an even width
// (the noise divided by its slope is the distance to the line), kept only
// in patches (a low-frequency mask).
fn vein_noise(x: vec2f) -> f32 {
  let warp = vec2f(fbm(x, 3, 3, 61u), fbm(x + vec2f(0.5, 0.2), 3, 3, 62u)) * 0.1;
  return gnoise((x + warp) * 3.0, 3, 63u) + 0.3 * gnoise((x + warp) * 8.0, 8, 64u);
}

fn vein(x: vec2f) -> f32 {
  let e = 0.5 / SIZE;
  let n = vein_noise(x);
  let g = vec2f(vein_noise(x + vec2f(e, 0.0)) - vein_noise(x - vec2f(e, 0.0)), vein_noise(x + vec2f(0.0, e)) - vein_noise(x - vec2f(0.0, e))) / (2.0 * e);
  let d = abs(n) / max(length(g), 1e-3);
  // A soft ramp a few texels wide (the shader thresholds it, so magnified
  // veins keep smooth edges); the width varies along the vein.
  let width = 0.004 + 0.005 * smoothstep(-0.2, 0.3, fbm(x, 6, 2, 66u));
  let line = 1.0 - smoothstep(0.0, width, d);
  let zone = smoothstep(0.02, 0.2, fbm(x, 2, 3, 65u));
  return line * zone;
}

@compute @workgroup_size(${WG}, ${WG})
fn gen(@builtin(global_invocation_id) id: vec3u) {
  if (id.x >= ${ROCK_TEXTURE_SIZE}u || id.y >= ${ROCK_TEXTURE_SIZE}u) { return; }
  let layer = id.z;
  let x = (vec2f(id.xy) + 0.5) / SIZE;
  let e = 1.0 / SIZE;
  var out: vec4f;
  if (layer == 0u) {
    let h = rock_height(x);
    // Slope per texture unit, central differences one texel apart.
    let du = (rock_height(x + vec2f(e, 0.0)) - rock_height(x - vec2f(e, 0.0))) / (2.0 * e);
    let dv = (rock_height(x + vec2f(0.0, e)) - rock_height(x - vec2f(0.0, e))) / (2.0 * e);
    out = vec4f(0.5 + 0.5 * clamp(vec2f(du, dv) / 24.0, vec2f(-1.0), vec2f(1.0)), clamp(h * 0.8 + 0.5, 0.0, 1.0), vein(x));
  } else {
    let h = grain_height(x);
    let du = (grain_height(x + vec2f(e, 0.0)) - grain_height(x - vec2f(e, 0.0))) / (2.0 * e);
    let dv = (grain_height(x + vec2f(0.0, e)) - grain_height(x - vec2f(0.0, e))) / (2.0 * e);
    out = vec4f(0.5 + 0.5 * clamp(vec2f(du, dv) / 60.0, vec2f(-1.0), vec2f(1.0)), clamp(h * 0.9 + 0.5, 0.0, 1.0), clamp(fbm(x, 32, 3, 71u) + 0.5, 0.0, 1.0));
  }
  textureStore(dst, vec2i(id.xy), i32(layer), out);
}
`;

/** WGSL of one mip step: 2x2 box filter from `src` (level n) into `dst` (level n+1). */
export const ROCK_MIP_WGSL = /* wgsl */ `
@group(0) @binding(0) var src: texture_2d_array<f32>;
@group(0) @binding(1) var dst: texture_storage_2d_array<rgba8unorm, write>;

@compute @workgroup_size(${WG}, ${WG})
fn mip(@builtin(global_invocation_id) id: vec3u) {
  let size = textureDimensions(dst);
  if (id.x >= size.x || id.y >= size.y) { return; }
  let layer = i32(id.z);
  let s = vec2i(id.xy) * 2;
  let c = textureLoad(src, s, layer, 0) + textureLoad(src, s + vec2i(1, 0), layer, 0)
        + textureLoad(src, s + vec2i(0, 1), layer, 0) + textureLoad(src, s + vec2i(1, 1), layer, 0);
  textureStore(dst, vec2i(id.xy), layer, c * 0.25);
}
`;

/** The terrain material's group-1 resources (texture array, sampler, TerrainParams). */
export class TerrainMaterial {
  readonly layout: GPUBindGroupLayout;
  readonly bindGroup: GPUBindGroup;
  private readonly texture: GPUTexture;
  private readonly params: GPUBuffer;
  private readonly device: GPUDevice;
  private seed: number | null = null;

  constructor(device: GPUDevice) {
    this.device = device;
    const size = ROCK_TEXTURE_SIZE;
    const mips = mipLevelCount(size);
    this.texture = device.createTexture({
      label: "rock textures",
      size: { width: size, height: size, depthOrArrayLayers: ROCK_TEXTURE_LAYERS },
      format: "rgba8unorm",
      mipLevelCount: mips,
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.STORAGE_BINDING,
    });
    this.generate(mips);
    this.params = device.createBuffer({
      label: "terrain params",
      size: TERRAIN_PARAM_FLOATS * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.setSeed(0);
    const sampler = device.createSampler({
      label: "rock sampler",
      addressModeU: "repeat",
      addressModeV: "repeat",
      magFilter: "linear",
      minFilter: "linear",
      mipmapFilter: "linear",
      maxAnisotropy: 8,
    });
    this.layout = device.createBindGroupLayout({
      label: "terrain material",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float", viewDimension: "2d-array" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
      ],
    });
    this.bindGroup = device.createBindGroup({
      label: "terrain material",
      layout: this.layout,
      entries: [
        { binding: 0, resource: this.texture.createView({ dimension: "2d-array" }) },
        { binding: 1, resource: sampler },
        { binding: 2, resource: { buffer: this.params } },
      ],
    });
  }

  /** Per-world look (strata tilt, texture offsets); cheap, re-uploads only on change. */
  setSeed(seed: number): void {
    if (seed === this.seed) return;
    this.seed = seed;
    this.device.queue.writeBuffer(this.params, 0, terrainParams(seed));
  }

  destroy(): void {
    this.texture.destroy();
    this.params.destroy();
  }

  private generate(mips: number): void {
    const device = this.device;
    const size = ROCK_TEXTURE_SIZE;
    const layers = ROCK_TEXTURE_LAYERS;
    const genModule = device.createShaderModule({ label: "rock gen", code: ROCK_GEN_WGSL });
    const mipModule = device.createShaderModule({ label: "rock mip", code: ROCK_MIP_WGSL });
    const gen = device.createComputePipeline({ label: "rock gen", layout: "auto", compute: { module: genModule, entryPoint: "gen" } });
    const mip = device.createComputePipeline({ label: "rock mip", layout: "auto", compute: { module: mipModule, entryPoint: "mip" } });
    const level = (n: number): GPUTextureView =>
      this.texture.createView({ dimension: "2d-array", baseMipLevel: n, mipLevelCount: 1 });
    const encoder = device.createCommandEncoder({ label: "rock textures" });
    const pass = encoder.beginComputePass({ label: "rock textures" });
    pass.setPipeline(gen);
    pass.setBindGroup(0, device.createBindGroup({ layout: gen.getBindGroupLayout(0), entries: [{ binding: 0, resource: level(0) }] }));
    pass.dispatchWorkgroups(Math.ceil(size / WG), Math.ceil(size / WG), layers);
    pass.setPipeline(mip);
    for (let n = 1; n < mips; n++) {
      const edge = Math.max(1, size >> n);
      pass.setBindGroup(
        0,
        device.createBindGroup({
          layout: mip.getBindGroupLayout(0),
          entries: [
            { binding: 0, resource: level(n - 1) },
            { binding: 1, resource: level(n) },
          ],
        }),
      );
      pass.dispatchWorkgroups(Math.ceil(edge / WG), Math.ceil(edge / WG), layers);
    }
    pass.end();
    device.queue.submit([encoder.finish()]);
  }
}
