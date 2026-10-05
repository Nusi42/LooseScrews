// Procedural particle sprite atlas (visual-quality-plan.md VQ5), baked once at
// start-up on the CPU into an rg8unorm 2D texture array with a full mip chain
// (no aliasing when sprites shrink to a few pixels).
//
// Channel r is coverage (alpha of a smoke puff, intensity of a glow), g is a
// brightness or heat term inside the sprite (the hot centre of a flame, the
// fake self-shadowing of a smoke puff, the lit face of a debris shard). The
// particle shader (shaders/particles.ts) multiplies the particle colour by
// them. Pure maths: Node tests cover the shapes and the mip chain.

/** Sprite layers of the atlas (ParticleSpec.sprite). */
export const SPRITE = {
  /** Soft round glow with a long halo (cores, flashes, engine heat, motes). */
  glow: 0,
  /** Sharp narrow streak (sparks, embers; drawn stretched along the velocity). */
  spark: 1,
  /** Four billowy smoke puffs (pick one at random per particle). */
  smoke0: 2,
  smoke1: 3,
  smoke2: 4,
  smoke3: 5,
  /** Turbulent flame blob with a hot centre (fireballs, exhaust, muzzle fire). */
  flame: 6,
  /** Thin ring (shockwaves, shield ripples). */
  ring: 7,
  /** Four-point star flare (muzzle flashes, impact flashes, pickup glints). */
  flare: 8,
  /** Irregular rock or hull shard with a lit face (debris). */
  shard: 9,
  /** Soft-edged beam/trail section: bright core line, soft sides (ribbons, arcs). */
  streak: 10,
} as const;

export type SpriteId = (typeof SPRITE)[keyof typeof SPRITE];

/** Number of layers in the atlas. */
export const SPRITE_LAYERS = 11;
/** Edge length of a sprite layer at mip 0 (power of two). */
export const SPRITE_SIZE = 128;

/** Small deterministic PRNG (mulberry32) for the bake. */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Tileable 2D value noise lattice of `cells` x `cells`, smooth (quintic) interpolation, in 0..1. */
class ValueNoise {
  private readonly cells: number;
  private readonly v: Float32Array;

  constructor(cells: number, rnd: () => number) {
    this.cells = cells;
    this.v = new Float32Array(cells * cells);
    for (let i = 0; i < this.v.length; i++) this.v[i] = rnd();
  }

  at(x: number, y: number): number {
    const n = this.cells;
    const fx = x * n;
    const fy = y * n;
    const ix = Math.floor(fx);
    const iy = Math.floor(fy);
    const tx = fx - ix;
    const ty = fy - iy;
    const sx = tx * tx * tx * (tx * (tx * 6 - 15) + 10);
    const sy = ty * ty * ty * (ty * (ty * 6 - 15) + 10);
    const w = (i: number, j: number): number => this.v[(((j % n) + n) % n) * n + (((i % n) + n) % n)] ?? 0;
    const a = w(ix, iy) + (w(ix + 1, iy) - w(ix, iy)) * sx;
    const b = w(ix, iy + 1) + (w(ix + 1, iy + 1) - w(ix, iy + 1)) * sx;
    return a + (b - a) * sy;
  }
}

/** Fractal sum of value noise octaves (0..1, roughly centred on 0.5). */
function fbm(octaves: readonly ValueNoise[], x: number, y: number): number {
  let sum = 0;
  let amp = 0.5;
  let norm = 0;
  for (const o of octaves) {
    sum += o.at(x, y) * amp;
    norm += amp;
    amp *= 0.5;
  }
  return sum / norm;
}

const clamp01 = (x: number): number => (x < 0 ? 0 : x > 1 ? 1 : x);
const smoothstep = (a: number, b: number, x: number): number => {
  const t = clamp01((x - a) / (b - a));
  return t * t * (3 - 2 * t);
};

/** One sprite as two float channels in 0..1 (coverage, brightness), row-major, `size` x `size`. */
export interface SpriteImage {
  size: number;
  r: Float32Array;
  g: Float32Array;
}

type Shader = (x: number, y: number, u: number, v: number) => [number, number];

function render(size: number, shader: Shader): SpriteImage {
  const r = new Float32Array(size * size);
  const g = new Float32Array(size * size);
  for (let j = 0; j < size; j++) {
    for (let i = 0; i < size; i++) {
      // Centred coordinates in -1..1 (pixel centres) and 0..1 texture coordinates.
      const u = (i + 0.5) / size;
      const v = (j + 0.5) / size;
      const [a, b] = shader(u * 2 - 1, v * 2 - 1, u, v);
      r[j * size + i] = clamp01(a);
      g[j * size + i] = clamp01(b);
    }
  }
  return { size, r, g };
}

/** Fades every sprite to exactly 0 at the quad's inscribed circle, so no square edges show. */
function edge(rad: number): number {
  return 1 - smoothstep(0.82, 1.0, rad);
}

function glow(size: number): SpriteImage {
  return render(size, (x, y) => {
    const r2 = x * x + y * y;
    const a = (0.62 * Math.exp(-r2 * 14) + 0.38 * Math.exp(-r2 * 3.2)) * edge(Math.sqrt(r2));
    return [a, 1];
  });
}

function spark(size: number): SpriteImage {
  // Long axis is y (the shader stretches along it).
  return render(size, (x, y) => {
    const a = Math.exp(-x * x * 38) * Math.exp(-y * y * 2.4) * edge(Math.hypot(x, y));
    return [a, 1];
  });
}

function streak(size: number): SpriteImage {
  // A trail or arc section: constant along y (sections overlap end to end), soft across x.
  return render(size, (x, y) => {
    const across = 0.7 * Math.exp(-x * x * 40) + 0.3 * Math.exp(-x * x * 6);
    const along = 1 - smoothstep(0.7, 0.95, Math.abs(y));
    return [across * along * (1 - smoothstep(0.85, 1, Math.abs(x))), 1];
  });
}

function smoke(size: number, seed: number): SpriteImage {
  const rnd = mulberry32(seed);
  const octaves = [new ValueNoise(4, rnd), new ValueNoise(8, rnd), new ValueNoise(16, rnd), new ValueNoise(32, rnd)];
  const detail = [new ValueNoise(6, rnd), new ValueNoise(12, rnd), new ValueNoise(24, rnd)];
  return render(size, (x, y, u, v) => {
    const rad = Math.hypot(x, y);
    const n = fbm(octaves, u, v);
    // Billowy outline: the noise pushes the edge in and out.
    const shape = 1 - smoothstep(0.25, 0.95, rad + (n - 0.5) * 0.75);
    const d = fbm(detail, u + 0.37, v + 0.11);
    const density = shape * (0.55 + 0.45 * smoothstep(0.25, 0.75, d));
    // Fake self-shadowing: lit from above, darker towards the bottom and the thick core.
    const lit = 0.55 + 0.45 * clamp01(0.5 - y * 0.55 + (d - 0.5) * 0.8) - 0.15 * shape;
    return [density * edge(rad), clamp01(lit)];
  });
}

function flame(size: number): SpriteImage {
  const rnd = mulberry32(77);
  const octaves = [new ValueNoise(5, rnd), new ValueNoise(10, rnd), new ValueNoise(20, rnd), new ValueNoise(40, rnd)];
  return render(size, (x, y, u, v) => {
    const rad = Math.hypot(x, y);
    const n = fbm(octaves, u, v);
    const body = clamp01(1 - (rad + (n - 0.5) * 0.9) * 1.05);
    const a = Math.pow(body, 1.25) * edge(rad);
    // Heat: hottest in the middle, broken up by the turbulence.
    const heat = clamp01(Math.pow(body, 0.7) * (0.65 + 0.7 * (n - 0.35)));
    return [a, heat];
  });
}

function ring(size: number): SpriteImage {
  return render(size, (x, y) => {
    const rad = Math.hypot(x, y);
    const band = Math.exp(-((rad - 0.78) * (rad - 0.78)) * 260);
    const inner = 0.12 * smoothstep(0.2, 0.78, rad) * (1 - smoothstep(0.78, 0.85, rad));
    return [(band + inner) * edge(rad), 1];
  });
}

function flare(size: number): SpriteImage {
  return render(size, (x, y) => {
    const rad = Math.hypot(x, y);
    const core = Math.exp(-rad * rad * 30);
    const halo = 0.3 * Math.exp(-rad * rad * 5);
    const rays = 0.55 * (Math.exp(-Math.abs(y) * 60) * Math.exp(-x * x * 3) + Math.exp(-Math.abs(x) * 60) * Math.exp(-y * y * 3));
    const diag = 0.18 * (Math.exp(-Math.abs(x - y) * 45) + Math.exp(-Math.abs(x + y) * 45)) * Math.exp(-rad * rad * 8);
    return [(core + halo + rays + diag) * edge(rad), 1];
  });
}

function shard(size: number): SpriteImage {
  // An irregular convex-ish polygon: radius per angle from a few harmonics.
  const rnd = mulberry32(1234);
  const k = [rnd(), rnd(), rnd(), rnd(), rnd(), rnd()];
  const radius = (ang: number): number =>
    0.62 + 0.16 * Math.sin(ang * 2 + (k[0] ?? 0) * 6.28) + 0.1 * Math.sin(ang * 3 + (k[1] ?? 0) * 6.28) + 0.07 * Math.sin(ang * 5 + (k[2] ?? 0) * 6.28);
  return render(size, (x, y) => {
    const ang = Math.atan2(y, x);
    const rad = Math.hypot(x, y);
    const rr = radius(ang);
    const px = 2.2 / size;
    const a = 1 - smoothstep(rr - px, rr + px, rad);
    // Two facets: a lit upper-left face and a darker lower-right one, a bright rim.
    const facet = x + y < (k[3] ?? 0.5) * 0.4 - 0.2 ? 0.95 : 0.45;
    const rim = smoothstep(rr - 0.18, rr - 0.02, rad) * 0.35;
    return [a, clamp01(facet + rim - rad * 0.2)];
  });
}

/** Renders every layer at `size` (SPRITE order). */
export function bakeSprites(size = SPRITE_SIZE): SpriteImage[] {
  const layers: SpriteImage[] = [];
  layers[SPRITE.glow] = glow(size);
  layers[SPRITE.spark] = spark(size);
  layers[SPRITE.smoke0] = smoke(size, 11);
  layers[SPRITE.smoke1] = smoke(size, 23);
  layers[SPRITE.smoke2] = smoke(size, 37);
  layers[SPRITE.smoke3] = smoke(size, 51);
  layers[SPRITE.flame] = flame(size);
  layers[SPRITE.ring] = ring(size);
  layers[SPRITE.flare] = flare(size);
  layers[SPRITE.shard] = shard(size);
  layers[SPRITE.streak] = streak(size);
  return layers;
}

/** Number of mip levels of a `size` x `size` texture down to 1 x 1. */
export function mipCount(size: number): number {
  return Math.floor(Math.log2(size)) + 1;
}

/**
 * Packs one sprite into rg8 bytes per mip level (2x2 box filter per step;
 * coverage-weighted brightness, so dark empty texels do not darken edges).
 */
export function spriteMips(img: SpriteImage): Uint8Array[] {
  const out: Uint8Array[] = [];
  let size = img.size;
  let r = img.r;
  let g = img.g;
  for (;;) {
    const bytes = new Uint8Array(size * size * 2);
    for (let i = 0; i < size * size; i++) {
      bytes[i * 2] = Math.round(clamp01(r[i] ?? 0) * 255);
      bytes[i * 2 + 1] = Math.round(clamp01(g[i] ?? 0) * 255);
    }
    out.push(bytes);
    if (size === 1) break;
    const half = size >> 1;
    const nr = new Float32Array(half * half);
    const ng = new Float32Array(half * half);
    for (let j = 0; j < half; j++) {
      for (let i = 0; i < half; i++) {
        let sr = 0;
        let sg = 0;
        let sw = 0;
        for (const [di, dj] of [[0, 0], [1, 0], [0, 1], [1, 1]] as const) {
          const k = (j * 2 + dj) * size + (i * 2 + di);
          const a = r[k] ?? 0;
          sr += a;
          sg += (g[k] ?? 0) * (a + 1e-3);
          sw += a + 1e-3;
        }
        nr[j * half + i] = sr / 4;
        ng[j * half + i] = sg / sw;
      }
    }
    r = nr;
    g = ng;
    size = half;
  }
  return out;
}

/** Bakes the atlas into a GPU texture array (rg8unorm, mipmapped). */
export function createSpriteTexture(device: GPUDevice, size = SPRITE_SIZE): GPUTexture {
  const layers = bakeSprites(size);
  const levels = mipCount(size);
  const texture = device.createTexture({
    label: "particle sprites",
    size: { width: size, height: size, depthOrArrayLayers: layers.length },
    format: "rg8unorm",
    mipLevelCount: levels,
    dimension: "2d",
    usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
  });
  layers.forEach((img, layer) => {
    spriteMips(img).forEach((bytes, level) => {
      const s = Math.max(1, size >> level);
      device.queue.writeTexture(
        { texture, mipLevel: level, origin: { x: 0, y: 0, z: layer } },
        bytes,
        { bytesPerRow: s * 2, rowsPerImage: s },
        { width: s, height: s, depthOrArrayLayers: 1 },
      );
    });
  });
  return texture;
}
