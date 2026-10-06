// Sun occlusion (VQ9): one orthographic depth map from the sun over the
// whole level, so rock blocks the sun. Cave interiors fall into shadow (lit
// by the headlight, shots, explosions and engines), openings let the sun in
// as patches on the rock, and the volumetric pass (volumetric.ts) turns the
// lit air into shafts.
//
// The map is rendered from the terrain chunks' back faces as seen from the
// sun (cull "front"): a lit surface lies in front of the back face of its
// own rock, so there is no shadow acne on lit rock, and a point is in shadow
// exactly when a ray toward the sun enters rock above it. Terrain is static
// between edits, so the map is not redrawn every frame: a changed chunk
// marks its projected rectangle dirty (old and new extent), and the next
// frame clears and redraws only those rectangles (scissored) with the
// chunks that overlap them. The region covers every chunk seen so far
// (snapped to the chunk grid); when it grows, or the sun moves, the whole
// map is redrawn. Ships do not cast (open item); they receive.
//
// Pure maths (matrix, rectangles) is exported for the unit tests.

import type { Vec3 } from "../math/vec3.ts";
import type { Aabb } from "./frustum.ts";
import type { PassTimer } from "./lights.ts";
import type { GpuMesh } from "./mesh.ts";
import { SHADOW_CASTER_WGSL, SUN_SHADOW_UNIFORM_FLOATS } from "./shaders/shadow.ts";

/** Chunk edge (u): the region snaps to it (ls-worldgen CHUNK_CELLS). */
export const SHADOW_REGION_SNAP = 32;
/** Extra depth range beyond the region's box (u), so nothing clips at the near and far planes. */
export const SHADOW_DEPTH_MARGIN = 4;
/** Receiver offset along the normal (u) and depth bias (u) against acne at the terminator. */
export const SHADOW_NORMAL_OFFSET = 0.12;
export const SHADOW_BIAS = 0.06;
/** PCF radius in texels. */
export const SHADOW_PCF_TEXELS = 1.6;
/**
 * Ship shadows (D5): a SHIP_SHADOW_SIZE² map from the sun over a square
 * SHIP_SHADOW_RANGE u to each side of the camera (about 0.08 u per texel),
 * redrawn every frame from the ship and wreck instances. Hulls farther away
 * cast none (their shadow would be a few pixels).
 */
export const SHIP_SHADOW_SIZE = 1024;
export const SHIP_SHADOW_RANGE = 40;
/** Ship map depth bias (u) and PCF radius (texels). */
export const SHIP_SHADOW_BIAS = 0.04;
export const SHIP_SHADOW_PCF_TEXELS = 1.2;
/** Dirty rectangles kept apart before they are merged into one. */
export const MAX_DIRTY_RECTS = 6;

export const SHADOW_FORMAT: GPUTextureFormat = "depth16unorm";
/** Bytes per instance of the objects pass (renderer.ts INSTANCE_FLOATS x 4); the ship caster reads its transform. */
export const SHIP_INSTANCE_STRIDE = 64;

/** Pixel rectangle [x0, y0, x1, y1) in the shadow map. */
export type Rect = [number, number, number, number];

/** The orthographic sun view over a box. */
export interface ShadowFrame {
  /** World to shadow clip, column-major (x, y in -1..1, z 0..1 with 0 nearest the sun). */
  clip: Float32Array;
  /** Depth range (u) that maps to 0..1. */
  depthRange: number;
  /** World size of one texel along x and y at `size` texels. */
  texel: [number, number];
}

function normalize(v: Vec3): Vec3 {
  const l = Math.hypot(v.x, v.y, v.z) || 1;
  return { x: v.x / l, y: v.y / l, z: v.z / l };
}

function cross(a: Vec3, b: Vec3): Vec3 {
  return { x: a.y * b.z - a.z * b.y, y: a.z * b.x - a.x * b.z, z: a.x * b.y - a.y * b.x };
}

const dot = (a: Vec3, b: Vec3): number => a.x * b.x + a.y * b.y + a.z * b.z;

function corners(b: Aabb): Vec3[] {
  const out: Vec3[] = [];
  for (let i = 0; i < 8; i++) {
    out.push({ x: i & 1 ? b.max.x : b.min.x, y: i & 2 ? b.max.y : b.min.y, z: i & 4 ? b.max.z : b.min.z });
  }
  return out;
}

/** Light-space axes: right, up and toward the sun. */
export function sunAxes(sun: Vec3): { r: Vec3; u: Vec3; s: Vec3 } {
  const s = normalize(sun);
  const helper = Math.abs(s.y) < 0.99 ? { x: 0, y: 1, z: 0 } : { x: 1, y: 0, z: 0 };
  const r = normalize(cross(helper, s));
  const u = cross(s, r);
  return { r, u, s };
}

/** Orthographic sun view tightly around `box` (its eight corners) at `size` texels. */
export function shadowFrame(sun: Vec3, box: Aabb, size: number): ShadowFrame {
  const { r, u, s } = sunAxes(sun);
  let x0 = Infinity, x1 = -Infinity, y0 = Infinity, y1 = -Infinity, s0 = Infinity, s1 = -Infinity;
  for (const c of corners(box)) {
    const x = dot(r, c), y = dot(u, c), z = dot(s, c);
    x0 = Math.min(x0, x); x1 = Math.max(x1, x);
    y0 = Math.min(y0, y); y1 = Math.max(y1, y);
    s0 = Math.min(s0, z); s1 = Math.max(s1, z);
  }
  // One texel of margin so the PCF taps at the edge stay inside.
  const padX = (x1 - x0) / size, padY = (y1 - y0) / size;
  x0 -= padX; x1 += padX; y0 -= padY; y1 += padY;
  s0 -= SHADOW_DEPTH_MARGIN; s1 += SHADOW_DEPTH_MARGIN;
  const hx = (x1 - x0) / 2, hy = (y1 - y0) / 2, cx = (x0 + x1) / 2, cy = (y0 + y1) / 2;
  const dz = s1 - s0;
  const m = new Float32Array(16);
  // Row 0: x, row 1: y, row 2: depth = (s1 - s.p) / dz, row 3: w = 1.
  m[0] = r.x / hx; m[4] = r.y / hx; m[8] = r.z / hx; m[12] = -cx / hx;
  m[1] = u.x / hy; m[5] = u.y / hy; m[9] = u.z / hy; m[13] = -cy / hy;
  m[2] = -s.x / dz; m[6] = -s.y / dz; m[10] = -s.z / dz; m[14] = s1 / dz;
  m[15] = 1;
  return { clip: m, depthRange: dz, texel: [(2 * hx) / size, (2 * hy) / size] };
}

/**
 * The ship map's sun view: SHIP_SHADOW_RANGE u to each side of `center`
 * across the sun and along it, the centre snapped to whole texels in light
 * space so shadow edges do not crawl as the camera moves.
 */
export function shipShadowFrame(sun: Vec3, center: Vec3, size = SHIP_SHADOW_SIZE, range = SHIP_SHADOW_RANGE): ShadowFrame {
  const { r, u, s } = sunAxes(sun);
  const texel = (2 * range) / size;
  const snap = (v: number): number => Math.round(v / texel) * texel;
  const cx = snap(dot(r, center)), cy = snap(dot(u, center)), cs = dot(s, center);
  const s0 = cs - range, s1 = cs + range, dz = s1 - s0;
  const m = new Float32Array(16);
  m[0] = r.x / range; m[4] = r.y / range; m[8] = r.z / range; m[12] = -cx / range;
  m[1] = u.x / range; m[5] = u.y / range; m[9] = u.z / range; m[13] = -cy / range;
  m[2] = -s.x / dz; m[6] = -s.y / dz; m[10] = -s.z / dz; m[14] = s1 / dz;
  m[15] = 1;
  return { clip: m, depthRange: dz, texel: [texel, texel] };
}

/** Shadow clip coordinates of a world point (CPU mirror of the shader's `sun_shadow.clip * p`). */
export function shadowClip(m: Float32Array, p: Vec3): [number, number, number] {
  const e = (i: number): number => m[i] ?? 0;
  return [
    e(0) * p.x + e(4) * p.y + e(8) * p.z + e(12),
    e(1) * p.x + e(5) * p.y + e(9) * p.z + e(13),
    e(2) * p.x + e(6) * p.y + e(10) * p.z + e(14),
  ];
}

/** Pixel rectangle a box covers in the map, padded by `pad` texels and clamped; null if empty. */
export function texelRect(m: Float32Array, box: Aabb, size: number, pad = 2): Rect | null {
  let x0 = Infinity, x1 = -Infinity, y0 = Infinity, y1 = -Infinity;
  for (const c of corners(box)) {
    const [x, y] = shadowClip(m, c);
    const px = (x * 0.5 + 0.5) * size, py = (0.5 - y * 0.5) * size;
    x0 = Math.min(x0, px); x1 = Math.max(x1, px);
    y0 = Math.min(y0, py); y1 = Math.max(y1, py);
  }
  const r: Rect = [
    Math.max(0, Math.floor(x0) - pad),
    Math.max(0, Math.floor(y0) - pad),
    Math.min(size, Math.ceil(x1) + pad),
    Math.min(size, Math.ceil(y1) + pad),
  ];
  return r[2] > r[0] && r[3] > r[1] ? r : null;
}

export const rectsOverlap = (a: Rect, b: Rect): boolean => a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
const union = (a: Rect, b: Rect): Rect => [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[2], b[2]), Math.max(a[3], b[3])];

/** Adds a rectangle to a dirty list: merged with every rectangle it overlaps; past `max` all collapse into one. */
export function addDirty(list: Rect[], rect: Rect, max = MAX_DIRTY_RECTS): Rect[] {
  let merged = rect;
  let rest = list;
  for (;;) {
    const hit = rest.findIndex((r) => rectsOverlap(r, merged));
    if (hit < 0) break;
    merged = union(merged, rest[hit]!);
    rest = rest.filter((_, i) => i !== hit);
  }
  const out = [...rest, merged];
  if (out.length <= max) return out;
  return [out.reduce(union)];
}

/** A box grown to the chunk grid. */
export function snapBox(b: Aabb, snap = SHADOW_REGION_SNAP): Aabb {
  return {
    min: { x: Math.floor(b.min.x / snap) * snap, y: Math.floor(b.min.y / snap) * snap, z: Math.floor(b.min.z / snap) * snap },
    max: { x: Math.ceil(b.max.x / snap) * snap, y: Math.ceil(b.max.y / snap) * snap, z: Math.ceil(b.max.z / snap) * snap },
  };
}

export function boxContains(outer: Aabb, inner: Aabb): boolean {
  return (
    inner.min.x >= outer.min.x && inner.min.y >= outer.min.y && inner.min.z >= outer.min.z &&
    inner.max.x <= outer.max.x && inner.max.y <= outer.max.y && inner.max.z <= outer.max.z
  );
}

export function boxUnion(a: Aabb, b: Aabb): Aabb {
  return {
    min: { x: Math.min(a.min.x, b.min.x), y: Math.min(a.min.y, b.min.y), z: Math.min(a.min.z, b.min.z) },
    max: { x: Math.max(a.max.x, b.max.x), y: Math.max(a.max.y, b.max.y), z: Math.max(a.max.z, b.max.z) },
  };
}

/** Group 0 layout entries 8-11 of the scene pipelines (shaders/shadow.ts SUN_SHADOW_WGSL). */
export function sunShadowBindLayoutEntries(): GPUBindGroupLayoutEntry[] {
  const vf = GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT;
  return [
    { binding: 8, visibility: vf, buffer: { type: "uniform" } },
    { binding: 9, visibility: vf, texture: { sampleType: "depth" } },
    { binding: 10, visibility: vf, sampler: { type: "comparison" } },
    { binding: 11, visibility: vf, texture: { sampleType: "depth" } },
  ];
}

/**
 * A turning sun (M4 B5: Bored level turns the level's up, and the sun with
 * it) would redraw the whole map every frame. Instead the map for a new sun
 * direction is drawn into a second map over SPREAD_SLICES frames (one band of
 * rows each) while the scene keeps using the finished one; then it is copied
 * over and the next direction starts. Shadows lag the sun by at most twice
 * that many frames (about 3 degrees at the turn's speed).
 */
export const SPREAD_SLICES = 8;

/** Rows [y0, y1) of slice `i` of `n` in a `size`² map. */
export function sliceRows(i: number, n: number, size: number): [number, number] {
  return [Math.floor((i * size) / n), Math.floor(((i + 1) * size) / n)];
}

/**
 * Bookkeeping of a spread redraw (pure; SunShadow drives the GPU side):
 * `request` a direction; `step` gives the slice to draw this frame and says
 * when the last one is drawn (then `adopt` the pending direction). A
 * direction requested while a redraw runs waits for it to finish (only the
 * newest waiting one is kept).
 */
export class SunSweep {
  pending: Vec3 | null = null;
  next = 0;
  wanted: Vec3 | null = null;
  readonly slices: number;

  constructor(slices = SPREAD_SLICES) {
    this.slices = slices;
  }

  /** The newest direction asked for (null when idle). */
  get latest(): Vec3 | null {
    return this.wanted ?? this.pending;
  }

  request(d: Vec3): void {
    if (this.pending) {
      this.wanted = d;
    } else {
      this.pending = d;
      this.next = 0;
    }
  }

  /** The slice to draw now, and whether it is the last of the pending direction. */
  step(): { slice: number; last: boolean } | null {
    if (!this.pending) return null;
    const slice = this.next++;
    return { slice, last: this.next >= this.slices };
  }

  /** The pending direction is drawn: returns it and starts the waiting one, if any. */
  adopt(): Vec3 | null {
    const done = this.pending;
    this.pending = this.wanted;
    this.wanted = null;
    this.next = 0;
    return done;
  }

  cancel(): void {
    this.pending = null;
    this.wanted = null;
    this.next = 0;
  }
}

/** Statistics of the last update (debug overlay, gallery). */
export interface ShadowStats {
  /** Map size in texels. */
  size: number;
  /** Rectangles redrawn in the last update and the chunk draws it took. */
  rects: number;
  draws: number;
  /** Updates since start (full redraws included). */
  updates: number;
  full: number;
  /** Slices drawn for spread redraws (a turning sun) and next maps swapped in (M4 B5). */
  slices?: number;
  swaps?: number;
}

export class SunShadow {
  private readonly device: GPUDevice;
  private readonly texture: GPUTexture;
  private readonly view: GPUTextureView;
  private readonly sampler: GPUSampler;
  readonly uniform: GPUBuffer;
  private readonly data = new Float32Array(SUN_SHADOW_UNIFORM_FLOATS);
  private readonly casterLayout: GPUBindGroupLayout;
  private readonly casterBindGroup: GPUBindGroup;
  private readonly casterPipeline: GPURenderPipeline;
  private readonly clearPipeline: GPURenderPipeline;
  private readonly size: number;
  private enabled = true;
  private sealed = false;
  private sun: Vec3 = { x: 0, y: 1, z: 0 };
  private region: Aabb | null = null;
  private frame: ShadowFrame | null = null;
  private dirty: Rect[] = [];
  private full = false;
  private volumetric: [number, number, number, number] = [0, 0, 0, 0];
  private shell: [number, number, number, number] = [0, 0, 0, 0];
  private readonly shipTexture: GPUTexture;
  private readonly shipView: GPUTextureView;
  private readonly shipPipeline: GPURenderPipeline;
  private readonly shipSize: number;
  /** The ship map's view this frame; null = no ship casts (the map is not sampled). */
  private shipFrame: ShadowFrame | null = null;
  /**
   * Cost measurement (`?shadowbench=full|chunk`, the gallery): redraw the
   * whole map, or the rectangles of two chunks near the map's centre (what
   * one crater remesh costs), every frame.
   */
  bench: "full" | "chunk" | null = null;
  /** Ships and wrecks cast sun shadows (the ship map, D5); false = only rock does. */
  shipShadows = true;
  readonly stats: ShadowStats = { size: 0, rects: 0, draws: 0, updates: 0, full: 0, slices: 0, swaps: 0 };
  /** Spread redraw of a turning sun (M4 B5): the next map, its frame and its own caster uniform. */
  private readonly sweep = new SunSweep();
  private sweepFrame: ShadowFrame | null = null;
  private sweepChanged: Aabb[] = [];
  private back: GPUTexture | null = null;
  private idleFrames = 0;
  private readonly sweepUniform: GPUBuffer;
  private readonly sweepData = new Float32Array(SUN_SHADOW_UNIFORM_FLOATS);
  private readonly sweepBindGroup: GPUBindGroup;

  /**
   * `size`: the terrain map (1 = no shadows: a placeholder); the ship map is
   * SHIP_SHADOW_SIZE then, else a placeholder too.
   */
  constructor(device: GPUDevice, size: number) {
    this.device = device;
    this.size = size;
    this.texture = this.createTexture(size);
    this.view = this.texture.createView();
    this.shipSize = size > 1 ? Math.min(SHIP_SHADOW_SIZE, size) : 1;
    this.shipTexture = this.createTexture(this.shipSize, "ship shadow map");
    this.shipView = this.shipTexture.createView();
    this.stats.size = size;
    this.sampler = device.createSampler({
      label: "sun shadow",
      compare: "less-equal",
      magFilter: "linear",
      minFilter: "linear",
      addressModeU: "clamp-to-edge",
      addressModeV: "clamp-to-edge",
    });
    this.uniform = device.createBuffer({
      label: "sun shadow",
      size: SUN_SHADOW_UNIFORM_FLOATS * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.writeUniform();
    this.casterLayout = device.createBindGroupLayout({
      label: "sun shadow caster",
      entries: [{ binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } }],
    });
    this.casterBindGroup = device.createBindGroup({
      layout: this.casterLayout,
      entries: [{ binding: 0, resource: { buffer: this.uniform } }],
    });
    this.sweepUniform = device.createBuffer({
      label: "sun shadow sweep",
      size: SUN_SHADOW_UNIFORM_FLOATS * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.sweepBindGroup = device.createBindGroup({
      layout: this.casterLayout,
      entries: [{ binding: 0, resource: { buffer: this.sweepUniform } }],
    });
    const module = device.createShaderModule({ label: "sun shadow caster", code: SHADOW_CASTER_WGSL });
    const layout = device.createPipelineLayout({ bindGroupLayouts: [this.casterLayout] });
    this.casterPipeline = device.createRenderPipeline({
      label: "sun shadow caster",
      layout,
      vertex: { module, entryPoint: "vs_caster", buffers: [{ arrayStride: 12, attributes: [{ shaderLocation: 0, offset: 0, format: "float32x3" }] }] },
      // Only drops the shell's fragments (fs_caster); depth comes from the rasteriser.
      fragment: { module, entryPoint: "fs_caster", targets: [] },
      // Back faces seen from the sun: lit rock lies in front of its own rock's far side.
      primitive: { topology: "triangle-list", cullMode: "front", frontFace: "ccw" },
      depthStencil: { format: SHADOW_FORMAT, depthWriteEnabled: true, depthCompare: "less" },
    });
    // Ships: instanced hulls (renderer.ts INSTANCE_LAYOUT; positions and the
    // instance transform only), every face (thin fins and open hull parts
    // cast too; SHIP_SHADOW_BIAS keeps hulls off their own shadow).
    this.shipPipeline = device.createRenderPipeline({
      label: "ship shadow caster",
      layout,
      vertex: {
        module,
        entryPoint: "vs_ship",
        buffers: [
          { arrayStride: 12, attributes: [{ shaderLocation: 0, offset: 0, format: "float32x3" }] },
          {
            arrayStride: SHIP_INSTANCE_STRIDE,
            stepMode: "instance",
            attributes: [
              { shaderLocation: 2, offset: 0, format: "float32x4" },
              { shaderLocation: 3, offset: 16, format: "float32x4" },
              { shaderLocation: 5, offset: 48, format: "float32x4" },
            ],
          },
        ],
      },
      primitive: { topology: "triangle-list", cullMode: "none" },
      depthStencil: { format: SHADOW_FORMAT, depthWriteEnabled: true, depthCompare: "less" },
    });
    this.clearPipeline = device.createRenderPipeline({
      label: "sun shadow clear",
      layout,
      vertex: { module, entryPoint: "vs_clear" },
      primitive: { topology: "triangle-list" },
      depthStencil: { format: SHADOW_FORMAT, depthWriteEnabled: true, depthCompare: "always" },
    });
  }

  private createTexture(size: number, label = "sun shadow map"): GPUTexture {
    return this.device.createTexture({
      label,
      size: { width: size, height: size },
      format: SHADOW_FORMAT,
      // COPY_*: a spread redraw (a turning sun, M4 B5) copies the finished next map over the current one.
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_SRC | GPUTextureUsage.COPY_DST,
    });
  }

  /** Scene bind group entries 8-11 (layout: sunShadowBindLayoutEntries). */
  bindGroupEntries(): GPUBindGroupEntry[] {
    return [
      { binding: 8, resource: { buffer: this.uniform } },
      { binding: 9, resource: this.view },
      { binding: 10, resource: this.sampler },
      { binding: 11, resource: this.shipView },
    ];
  }

  /** Off: the shaders see no shadow map (the sun reaches everything, the pre-VQ9 look). */
  setEnabled(on: boolean): void {
    if (on === this.enabled) return;
    this.enabled = on;
    this.full = true;
    this.writeUniform();
  }

  get isEnabled(): boolean {
    return this.enabled;
  }

  /**
   * Sealed level (no sky: the level box's walls enclose it, Enclosure >= 50):
   * rock lies between every point and the sun, so the shaders skip the map
   * (visibility 0) and nothing is drawn into it.
   */
  setSealed(sealed: boolean): void {
    if (sealed === this.sealed) return;
    this.sealed = sealed;
    this.full = true;
    this.writeUniform();
  }

  /** True when the sun can reach into the level through the map (on, not sealed, chunks present). */
  get active(): boolean {
    return this.enabled && !this.sealed && this.frame !== null;
  }

  /** Volumetric parameters (shaders/shadow.ts SunShadow.vol). */
  setVolumetric(scatter: number, g: number, length: number, steps: number): void {
    const v: [number, number, number, number] = [scatter, g, length, steps];
    if (v.every((x, i) => x === this.volumetric[i])) return;
    this.volumetric = v;
    this.writeUniform();
  }

  /**
   * The Squeeze's shell sphere (null: none). Rock at and beyond it casts no
   * shadow (shaders/shadow.ts SHELL_CASTER_MARGIN), so the arena keeps the
   * sun. A shrinking sphere needs no redraw (the remeshed chunks mark
   * theirs dirty); a new, moved or growing one redraws the whole map.
   */
  setShell(center: Vec3 | null, radius = 0): void {
    const next: [number, number, number, number] = center && radius > 0 ? [center.x, center.y, center.z, radius] : [0, 0, 0, 0];
    const cur = this.shell;
    if (next.every((x, i) => x === cur[i])) return;
    const shrinkOnly = cur[3] > 0 && next[3] > 0 && next[0] === cur[0] && next[1] === cur[1] && next[2] === cur[2] && next[3] < cur[3];
    this.shell = next;
    if (!shrinkOnly) this.full = true;
    this.writeUniform();
  }

  /**
   * Direction toward the sun; a change redraws the whole map. With `spread`
   * (a sun that keeps turning, M4 B5) the new map is drawn over
   * SPREAD_SLICES frames while the old one stays in use (SunSweep).
   */
  setSun(dir: Vec3, spread = false): void {
    const d = normalize(dir);
    const same = (a: Vec3): boolean => Math.abs(d.x - a.x) + Math.abs(d.y - a.y) + Math.abs(d.z - a.z) < 1e-6;
    if (spread && this.frame && this.region && this.enabled && !this.sealed && this.size > 1) {
      if (same(this.sweep.latest ?? this.sun)) return;
      if (!this.sweep.pending) {
        this.sweepFrame = shadowFrame(d, this.region, this.size);
        this.sweepChanged = [];
      }
      this.sweep.request(d);
      return;
    }
    this.sweep.cancel();
    if (same(this.sun)) return;
    this.sun = d;
    this.reframe();
  }

  /** A spread redraw is under way (the map lags the requested sun). */
  get sweeping(): boolean {
    return this.sweep.pending !== null;
  }

  /** A chunk was added, replaced or removed (`before` / `after`: its old and new extent). */
  chunkChanged(before: Aabb | null, after: Aabb | null): void {
    if (after && (!this.region || !boxContains(this.region, after))) {
      this.region = snapBox(this.region ? boxUnion(this.region, after) : after);
      // A growing region starts over, at the newest direction asked for.
      const latest = this.sweep.latest;
      if (latest) this.sun = latest;
      this.sweep.cancel();
      this.reframe();
      return;
    }
    if (this.sweep.pending) {
      // Redrawn in the next map once it is swapped in.
      for (const b of [before, after]) if (b) this.sweepChanged.push(b);
    }
    if (this.full || !this.frame) return;
    for (const b of [before, after]) {
      if (!b) continue;
      const r = texelRect(this.frame.clip, b, this.size);
      if (r) this.dirty = addDirty(this.dirty, r);
    }
  }

  /** Every chunk is gone (new match): the region starts over. */
  clear(): void {
    this.sweep.cancel();
    this.region = null;
    this.frame = null;
    this.dirty = [];
    this.full = false;
    this.writeUniform();
  }

  private reframe(): void {
    if (!this.region) return;
    this.frame = shadowFrame(this.sun, this.region, this.size);
    this.full = true;
    this.dirty = [];
    this.writeUniform();
  }

  /** Unit vector toward the sun. */
  get sunDirection(): Vec3 {
    return this.sun;
  }

  /** The current sun view (tests, the volumetric pass); null before the first chunk. */
  get shadowFrame(): ShadowFrame | null {
    return this.frame;
  }

  private writeUniform(): void {
    this.packUniform(this.data, this.frame);
    this.device.queue.writeBuffer(this.uniform, 0, this.data);
  }

  /** The uniform block for the map drawn with frame `f` (the scene's, or a sweep's next map). */
  private packUniform(d: Float32Array, f: ShadowFrame | null): void {
    const on = this.enabled && f !== null;
    const mode = !this.enabled ? 0 : this.sealed ? -1 : on ? 1 : 0;
    if (f) d.set(f.clip, 0);
    const texelU = f ? Math.max(f.texel[0], f.texel[1]) : 1;
    d.set([mode, SHADOW_PCF_TEXELS / this.size, SHADOW_NORMAL_OFFSET + texelU * 0.5, f ? SHADOW_BIAS / f.depthRange : 0], 16);
    d.set(this.volumetric, 20);
    d.set(this.shell, 24);
    const ship = this.shipFrame;
    if (ship) d.set(ship.clip, 28);
    d.set([ship ? 1 : 0, SHIP_SHADOW_PCF_TEXELS / this.shipSize, ship ? SHIP_SHADOW_BIAS / ship.depthRange : 0, 0], 44);
  }

  /**
   * One slice of a spread redraw into the next map; after the last one the
   * next map is copied over the scene's and its frame takes over.
   */
  private encodeSweep(encoder: GPUCommandEncoder, chunks: ReadonlyMap<string, GpuMesh>, timer: PassTimer): void {
    const step = this.enabled && !this.sealed && this.region ? this.sweep.step() : null;
    if (!step) {
      if (this.back && ++this.idleFrames > 120) {
        this.back.destroy();
        this.back = null;
      }
      return;
    }
    this.idleFrames = 0;
    const f = this.sweepFrame;
    if (!f) {
      this.sweep.cancel();
      return;
    }
    this.back ??= this.createTexture(this.size, "sun shadow map (next)");
    this.packUniform(this.sweepData, f);
    this.device.queue.writeBuffer(this.sweepUniform, 0, this.sweepData);
    const [y0, y1] = sliceRows(step.slice, this.sweep.slices, this.size);
    const rect: Rect = [0, y0, this.size, y1];
    const timestampWrites = timer?.pass("shadow");
    const pass = encoder.beginRenderPass({
      label: "sun shadow sweep",
      ...(timestampWrites ? { timestampWrites } : {}),
      colorAttachments: [],
      depthStencilAttachment: { view: this.back.createView(), depthClearValue: 1, depthLoadOp: step.slice === 0 ? "clear" : "load", depthStoreOp: "store" },
    });
    pass.setBindGroup(0, this.sweepBindGroup);
    pass.setScissorRect(rect[0], rect[1], rect[2] - rect[0], rect[3] - rect[1]);
    pass.setPipeline(this.casterPipeline);
    for (const mesh of chunks.values()) {
      if (!mesh.aabb || mesh.indexCount === 0) continue;
      const r = texelRect(f.clip, mesh.aabb, this.size);
      if (!r || !rectsOverlap(r, rect)) continue;
      pass.setVertexBuffer(0, mesh.positionBuffer);
      pass.setIndexBuffer(mesh.indexBuffer, "uint32");
      pass.drawIndexed(mesh.indexCount);
    }
    pass.end();
    this.stats.slices = (this.stats.slices ?? 0) + 1;
    if (!step.last) return;
    // The next map is complete: it becomes the scene's map.
    encoder.copyTextureToTexture({ texture: this.back }, { texture: this.texture }, { width: this.size, height: this.size });
    this.sun = this.sweep.adopt() ?? this.sun;
    this.frame = f;
    this.full = false;
    this.dirty = [];
    for (const b of this.sweepChanged) {
      const r = texelRect(f.clip, b, this.size);
      if (r) this.dirty = addDirty(this.dirty, r);
    }
    this.sweepChanged = [];
    const waiting = this.sweep.pending;
    this.sweepFrame = waiting && this.region ? shadowFrame(waiting, this.region, this.size) : null;
    this.writeUniform();
    this.stats.swaps = (this.stats.swaps ?? 0) + 1;
  }

  /**
   * Ship shadows for this frame (D5): `draw` draws the casters (the ship
   * and wreck batches, positions in vertex buffer 0 and instances in 1) into
   * the ship map around `center` (the camera). Without casters (null), with
   * shadows off or in a sealed level the map is not sampled.
   */
  encodeShips(encoder: GPUCommandEncoder, center: Vec3, draw: ((pass: GPURenderPassEncoder) => void) | null, timer: PassTimer): void {
    const wasOn = this.shipFrame !== null;
    if (!draw || !this.shipShadows || !this.enabled || this.sealed || this.shipSize <= 1) {
      if (wasOn) {
        this.shipFrame = null;
        this.writeUniform();
      }
      return;
    }
    const frame = shipShadowFrame(this.sun, center, this.shipSize);
    // The pipeline needs the matrix before the pass runs: write it first.
    this.shipFrame = frame;
    this.writeUniform();
    const timestampWrites = timer?.pass("shadow");
    const pass = encoder.beginRenderPass({
      label: "ship shadow",
      ...(timestampWrites ? { timestampWrites } : {}),
      colorAttachments: [],
      depthStencilAttachment: { view: this.shipView, depthClearValue: 1, depthLoadOp: "clear", depthStoreOp: "store" },
    });
    pass.setBindGroup(0, this.casterBindGroup);
    pass.setPipeline(this.shipPipeline);
    draw(pass);
    pass.end();
  }

  /** Redraws the dirty parts of the map (or all of it) before the scene pass. */
  encode(encoder: GPUCommandEncoder, chunks: ReadonlyMap<string, GpuMesh>, timer: PassTimer): void {
    this.encodeSweep(encoder, chunks, timer);
    const f = this.frame;
    if (f && this.bench === "full") this.full = true;
    if (f && this.bench === "chunk" && this.region) {
      const c = { x: (this.region.min.x + this.region.max.x) / 2, y: (this.region.min.y + this.region.max.y) / 2, z: (this.region.min.z + this.region.max.z) / 2 };
      for (const dx of [-SHADOW_REGION_SNAP, 0]) {
        const r = texelRect(f.clip, { min: { x: c.x + dx, y: c.y, z: c.z }, max: { x: c.x + dx + SHADOW_REGION_SNAP, y: c.y + SHADOW_REGION_SNAP, z: c.z + SHADOW_REGION_SNAP } }, this.size);
        if (r) this.dirty = addDirty(this.dirty, r);
      }
    }
    if (!this.enabled || this.sealed || !f || (!this.full && this.dirty.length === 0)) return;
    const full = this.full;
    const rects: Rect[] = full ? [[0, 0, this.size, this.size]] : this.dirty;
    this.full = false;
    this.dirty = [];
    const timestampWrites = timer?.pass("shadow");
    const pass = encoder.beginRenderPass({
      label: "sun shadow",
      ...(timestampWrites ? { timestampWrites } : {}),
      colorAttachments: [],
      depthStencilAttachment: {
        view: this.view,
        depthClearValue: 1,
        depthLoadOp: full ? "clear" : "load",
        depthStoreOp: "store",
      },
    });
    pass.setBindGroup(0, this.casterBindGroup);
    // Chunk rectangles once per update.
    const meshes: Array<[GpuMesh, Rect]> = [];
    for (const mesh of chunks.values()) {
      if (!mesh.aabb || mesh.indexCount === 0) continue;
      const r = texelRect(f.clip, mesh.aabb, this.size);
      if (r) meshes.push([mesh, r]);
    }
    let draws = 0;
    for (const rect of rects) {
      pass.setScissorRect(rect[0], rect[1], rect[2] - rect[0], rect[3] - rect[1]);
      if (!full) {
        pass.setPipeline(this.clearPipeline);
        pass.draw(3);
      }
      pass.setPipeline(this.casterPipeline);
      for (const [mesh, r] of meshes) {
        if (!rectsOverlap(r, rect)) continue;
        pass.setVertexBuffer(0, mesh.positionBuffer);
        pass.setIndexBuffer(mesh.indexBuffer, "uint32");
        pass.drawIndexed(mesh.indexCount);
        draws++;
      }
    }
    pass.end();
    this.stats.rects = rects.length;
    this.stats.draws = draws;
    this.stats.updates++;
    if (full) this.stats.full++;
  }

  destroy(): void {
    this.back?.destroy();
    this.sweepUniform.destroy();
    this.texture.destroy();
    this.shipTexture.destroy();
    this.uniform.destroy();
  }
}
