// Dynamic point lights (VQ6): the light type the game and the effect
// systems hand to the renderer (`Renderer.setLights`), the choice of the
// lights that matter this frame (at most RenderQuality.maxLights, scored by
// brightness at the camera and culled to the view), the hot-rock hot-spots
// read from terrain chunk meshes, and the GPU side: the light buffers, the
// LightInfo uniform and the clustered-forward compute pass
// (shaders/lights.ts). The scene shaders read the result through bindings
// 1-3 of group 0 (shaders/lighting.ts).
//
// Light colours are display (sRGB) values like every other authored colour;
// `intensity` is the light's HDR radiance multiplier 1 u from it (falloff
// intensity / (1 + d^2), windowed to 0 at `range`). Sun is 0.9, the
// headlight 0.9: a muzzle flash is about 6, an explosion 20-60.
//
// Lights are cosmetic: game code may use Math.random for them and must
// never feed them back into the simulation.

import type { Vec3 } from "../math/vec3.ts";
import type { Frustum } from "./frustum.ts";
import type { MeshData } from "./mesh.ts";
import { SURFACE_CHARGED, SURFACE_GLOW, SURFACE_HOT } from "./mesh.ts";
import { srgbToLinear } from "./shaders/frame.ts";
import {
  CLUSTER_COMPUTE_WGSL,
  CLUSTER_COUNT,
  CLUSTER_FAR,
  CLUSTER_NEAR,
  CLUSTER_WORKGROUP,
  CLUSTER_X,
  CLUSTER_Y,
  CLUSTER_Z,
  LIGHT_FLOATS,
  LIGHT_INFO_FLOATS,
  LIGHT_VIEW_FLOATS,
  MASK_WORDS,
  MAX_LIGHTS,
} from "./shaders/lights.ts";

export { MAX_LIGHTS } from "./shaders/lights.ts";

/** One dynamic point light for this frame. */
export interface PointLight {
  position: Vec3;
  /** Display (sRGB) colour. */
  color: readonly [number, number, number];
  /** HDR radiance multiplier 1 u from the light (sun = 0.9). */
  intensity: number;
  /** Range (u): no light beyond it. Default `lightRange(intensity)`. */
  range?: number;
  /** Extra weight when more lights compete than fit (default 1; objectives use more). */
  priority?: number;
}

/** Light below which a point light's falloff is cut off when no range is given. */
export const RANGE_CUTOFF = 0.02;
/** Ranges are clamped to this (u): the froxel test stays tight and caves do not light through walls far away. */
export const MAX_RANGE = 40;

/** Default range of a light: where intensity / (1 + d^2) drops to RANGE_CUTOFF, clamped to 2..MAX_RANGE. */
export function lightRange(intensity: number): number {
  return Math.min(MAX_RANGE, Math.max(2, Math.sqrt(Math.max(0, intensity) / RANGE_CUTOFF)));
}

/** Light reaching distance d (u) from a light (the shader's falloff, without the surface angle). */
export function falloff(intensity: number, range: number, d: number): number {
  const x = (d * d) / (range * range);
  if (x >= 1) return 0;
  const win = 1 - x * x;
  return (intensity * win * win) / (1 + d * d);
}

/** How much a light matters for this view: its light at the camera, softened so nearby walls count. */
export function lightScore(light: PointLight, camera: Vec3): number {
  const range = light.range ?? lightRange(light.intensity);
  const dx = light.position.x - camera.x, dy = light.position.y - camera.y, dz = light.position.z - camera.z;
  const d = Math.hypot(dx, dy, dz);
  // Light on the walls around it still shows when the light itself is far:
  // score by the distance to the nearest point of its sphere of influence.
  const near = Math.max(0, d - range * 0.5);
  return ((light.priority ?? 1) * light.intensity * Math.min(range, 12)) / (1 + near * near * 0.05);
}

/** True when the light's sphere of influence touches the view frustum (normalised inward planes). */
export function sphereInFrustum(planes: Float32Array, c: Vec3, r: number): boolean {
  for (let i = 0; i < 24; i += 4) {
    const d = (planes[i] ?? 0) * c.x + (planes[i + 1] ?? 0) * c.y + (planes[i + 2] ?? 0) * c.z + (planes[i + 3] ?? 0);
    if (d < -r) return false;
  }
  return true;
}

/**
 * The lights to upload: the in-view lights with a positive intensity, the
 * `max` best by lightScore when there are more. Order is by score (best
 * first), so a frame never flickers between equal lights at the cap.
 */
export function selectLights(lights: readonly PointLight[], camera: Vec3, max: number, frustum?: Frustum | null): PointLight[] {
  const scored: Array<{ light: PointLight; score: number }> = [];
  for (const light of lights) {
    if (!(light.intensity > 0)) continue;
    const range = light.range ?? lightRange(light.intensity);
    if (frustum && !sphereInFrustum(frustum.planes, light.position, range)) continue;
    scored.push({ light, score: lightScore(light, camera) });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.slice(0, Math.max(0, Math.min(max, MAX_LIGHTS))).map((s) => s.light);
}

/** Camera data the light pass needs: the view matrix (column-major), the frustum tangents and the forward axis. */
export interface LightView {
  view: Float32Array;
  tanX: number;
  tanY: number;
  forward: Vec3;
}

/** Headlight spot: cone half-angles (deg), light outside the cone, gain inside, distance falloff k. */
export const HEADLIGHT_SPOT = { inner: 14, outer: 40, outside: 0.3, gain: 1.35, falloff: 0.0025 } as const;

/**
 * Packs selected lights into the shading layout (`data`, LIGHT_FLOATS each:
 * world position + range, linear colour x intensity) and the froxel
 * builder's layout (`views`, LIGHT_VIEW_FLOATS each: view-space x right, y
 * up, depth forward + range).
 */
export function packLights(
  lights: readonly PointLight[],
  view: Float32Array,
  out?: { data: Float32Array; views: Float32Array },
): { data: Float32Array; views: Float32Array } {
  const data = out && out.data.length >= MAX_LIGHTS * LIGHT_FLOATS ? out.data : new Float32Array(MAX_LIGHTS * LIGHT_FLOATS);
  const views = out && out.views.length >= MAX_LIGHTS * LIGHT_VIEW_FLOATS ? out.views : new Float32Array(MAX_LIGHTS * LIGHT_VIEW_FLOATS);
  const n = Math.min(lights.length, MAX_LIGHTS);
  for (let i = 0; i < n; i++) {
    const l = lights[i]!;
    const o = i * LIGHT_FLOATS;
    const range = l.range ?? lightRange(l.intensity);
    const { x, y, z } = l.position;
    data[o] = x;
    data[o + 1] = y;
    data[o + 2] = z;
    data[o + 3] = range;
    data[o + 4] = srgbToLinear(l.color[0]) * l.intensity;
    data[o + 5] = srgbToLinear(l.color[1]) * l.intensity;
    data[o + 6] = srgbToLinear(l.color[2]) * l.intensity;
    data[o + 7] = 0;
    // View space: x right, y up, depth forward (-z of the view matrix).
    const v = i * LIGHT_VIEW_FLOATS;
    views[v] = (view[0] ?? 0) * x + (view[4] ?? 0) * y + (view[8] ?? 0) * z + (view[12] ?? 0);
    views[v + 1] = (view[1] ?? 0) * x + (view[5] ?? 0) * y + (view[9] ?? 0) * z + (view[13] ?? 0);
    views[v + 2] = -((view[2] ?? 0) * x + (view[6] ?? 0) * y + (view[10] ?? 0) * z + (view[14] ?? 0));
    views[v + 3] = range;
  }
  return { data, views };
}

/** The LightInfo uniform (shaders/lights.ts) for `count` lights. */
export function lightInfo(count: number, v: LightView, spot = true, out?: Float32Array): Float32Array {
  const d = out ?? new Float32Array(LIGHT_INFO_FLOATS);
  const rad = Math.PI / 180;
  d.set([count, v.tanX, v.tanY, spot ? 1 : 0], 0);
  d.set([CLUSTER_NEAR, CLUSTER_FAR, CLUSTER_Z / Math.log(CLUSTER_FAR / CLUSTER_NEAR), Math.log(CLUSTER_NEAR)], 4);
  d.set([v.forward.x, v.forward.y, v.forward.z, Math.cos(HEADLIGHT_SPOT.outer * rad)], 8);
  d.set([Math.cos(HEADLIGHT_SPOT.inner * rad), HEADLIGHT_SPOT.outside, HEADLIGHT_SPOT.falloff, HEADLIGHT_SPOT.gain], 12);
  return d;
}

/** Depth slice of a view depth (mirror of the WGSL `cluster_slice`). */
export function sliceOf(depth: number): number {
  const scale = CLUSTER_Z / Math.log(CLUSTER_FAR / CLUSTER_NEAR);
  const s = Math.floor((Math.log(Math.max(depth, 1e-4)) - Math.log(CLUSTER_NEAR)) * scale);
  return Math.min(CLUSTER_Z - 1, Math.max(0, s));
}

/** Froxel index of a view-space point (x right, y up, depth forward) (mirror of the WGSL `cluster_index`). */
export function clusterOf(p: Vec3, tanX: number, tanY: number): number {
  const depth = Math.max(p.z, 1e-4);
  const nx = p.x / (depth * tanX), ny = p.y / (depth * tanY);
  const clamp = (v: number, hi: number): number => Math.min(hi, Math.max(0, v));
  const tx = clamp(Math.floor((nx * 0.5 + 0.5) * CLUSTER_X), CLUSTER_X - 1);
  const ty = clamp(Math.floor((ny * 0.5 + 0.5) * CLUSTER_Y), CLUSTER_Y - 1);
  return (sliceOf(depth) * CLUSTER_Y + ty) * CLUSTER_X + tx;
}

/** View-space box of a froxel (mirror of the compute shader). */
export function froxelBounds(c: number, tanX: number, tanY: number): { min: Vec3; max: Vec3 } {
  const tx = c % CLUSTER_X;
  const ty = Math.floor(c / CLUSTER_X) % CLUSTER_Y;
  const s = Math.floor(c / (CLUSTER_X * CLUSTER_Y));
  const scale = CLUSTER_Z / Math.log(CLUSTER_FAR / CLUSTER_NEAR);
  const depthAt = (k: number): number => Math.exp(Math.log(CLUSTER_NEAR) + k / scale);
  const d0 = s === 0 ? 0 : depthAt(s);
  const d1 = s === CLUSTER_Z - 1 ? 1e6 : depthAt(s + 1);
  const x0 = ((tx / CLUSTER_X) * 2 - 1) * tanX, x1 = (((tx + 1) / CLUSTER_X) * 2 - 1) * tanX;
  const y0 = ((ty / CLUSTER_Y) * 2 - 1) * tanY, y1 = (((ty + 1) / CLUSTER_Y) * 2 - 1) * tanY;
  return {
    min: { x: Math.min(x0 * d0, x0 * d1), y: Math.min(y0 * d0, y0 * d1), z: d0 },
    max: { x: Math.max(x1 * d0, x1 * d1), y: Math.max(y1 * d0, y1 * d1), z: d1 },
  };
}

/** CPU reference of the compute pass: the froxels a view-space light sphere touches. */
export function froxelsOf(center: Vec3, range: number, tanX: number, tanY: number): number[] {
  const out: number[] = [];
  for (let c = 0; c < CLUSTER_COUNT; c++) {
    const b = froxelBounds(c, tanX, tanY);
    const qx = Math.min(Math.max(center.x, b.min.x), b.max.x) - center.x;
    const qy = Math.min(Math.max(center.y, b.min.y), b.max.y) - center.y;
    const qz = Math.min(Math.max(center.z, b.min.z), b.max.z) - center.z;
    if (qx * qx + qy * qy + qz * qz <= range * range) out.push(c);
  }
  return out;
}

// ----- hot rock -------------------------------------------------------------

/** Grid cell (u) hot-spot vertices are gathered in. */
export const HOTSPOT_CELL = 6;
/** Hot-spot lights per surface state: colour (display), intensity per sqrt(vertex), cap. */
const HOTSPOT_KINDS: ReadonlyArray<{ bit: number; color: readonly [number, number, number]; perVertex: number; cap: number; flicker: number }> = [
  { bit: SURFACE_HOT, color: [1.0, 0.45, 0.12], perVertex: 0.9, cap: 10, flicker: 0.25 },
  { bit: SURFACE_CHARGED, color: [0.45, 0.75, 1.0], perVertex: 0.7, cap: 7, flicker: 0.5 },
  { bit: SURFACE_GLOW, color: [0.4, 1.0, 0.6], perVertex: 0.45, cap: 5, flicker: 0 },
];

/** A light at the centre of a hot (or charged, glowing) rock patch; `flicker` 0..1 animates it. */
export interface HotSpot extends PointLight {
  flicker: number;
  /** Per-spot phase (rad). */
  phase: number;
}

/**
 * Hot-spot lights of a chunk mesh: vertices with a glowing surface state,
 * gathered per HOTSPOT_CELL grid cell and kind, one light at each cell's
 * centroid pushed 0.6 u off the rock along the mean normal.
 */
export function hotSpots(mesh: MeshData): HotSpot[] {
  const surface = mesh.surface;
  if (!surface || surface.length === 0) return [];
  const out: HotSpot[] = [];
  for (const kind of HOTSPOT_KINDS) {
    const cells = new Map<string, { n: number; p: [number, number, number]; nrm: [number, number, number] }>();
    for (let i = 0; i < surface.length; i++) {
      if (((surface[i] ?? 0) & kind.bit) === 0) continue;
      const x = mesh.positions[i * 3] ?? 0, y = mesh.positions[i * 3 + 1] ?? 0, z = mesh.positions[i * 3 + 2] ?? 0;
      const key = `${Math.floor(x / HOTSPOT_CELL)},${Math.floor(y / HOTSPOT_CELL)},${Math.floor(z / HOTSPOT_CELL)}`;
      let cell = cells.get(key);
      if (!cell) cells.set(key, (cell = { n: 0, p: [0, 0, 0], nrm: [0, 0, 0] }));
      cell.n++;
      cell.p[0] += x;
      cell.p[1] += y;
      cell.p[2] += z;
      cell.nrm[0] += mesh.normals[i * 3] ?? 0;
      cell.nrm[1] += mesh.normals[i * 3 + 1] ?? 0;
      cell.nrm[2] += mesh.normals[i * 3 + 2] ?? 0;
    }
    for (const cell of [...cells.values()].sort((a, b) => b.n - a.n)) {
      if (cell.n < 3) continue;
      const len = Math.hypot(cell.nrm[0], cell.nrm[1], cell.nrm[2]) || 1;
      const off = 0.6 / len;
      const position = {
        x: cell.p[0] / cell.n + cell.nrm[0] * off,
        y: cell.p[1] / cell.n + cell.nrm[1] * off,
        z: cell.p[2] / cell.n + cell.nrm[2] * off,
      };
      const intensity = Math.min(kind.cap, kind.perVertex * Math.sqrt(cell.n));
      out.push({
        position,
        color: kind.color,
        intensity,
        range: Math.min(16, lightRange(intensity)),
        priority: 0.6,
        flicker: kind.flicker,
        phase: (position.x * 1.7 + position.y * 2.3 + position.z * 0.9) % (Math.PI * 2),
      });
    }
  }
  return out;
}

/** A hot spot's light at time `t` (s): hot rock breathes, charged rock crackles. */
export function hotSpotAt(h: HotSpot, t: number): PointLight {
  if (h.flicker === 0) return h;
  const wave = 0.5 + 0.5 * Math.sin(t * 3 + h.phase) * Math.sin(t * 7.3 + h.phase * 2.1);
  return { position: h.position, color: h.color, intensity: h.intensity * (1 - h.flicker + h.flicker * wave), range: h.range ?? 0, priority: h.priority ?? 1 };
}

// ----- GPU ------------------------------------------------------------------

/** Bind group layout entries 1-3 of the scene's group 0 (light info, lights, froxel masks). */
export function lightBindLayoutEntries(): GPUBindGroupLayoutEntry[] {
  return [
    { binding: 1, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
    { binding: 2, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "read-only-storage" } },
    { binding: 3, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "read-only-storage" } },
  ];
}

/** Hands out timestamp writes for a pass label (renderer/gpuTimer.ts GpuTimer.pass). */
export type PassTimer = { pass(label: string): GPURenderPassTimestampWrites | undefined } | null;

/**
 * GPU side of the dynamic lights: uploads the chosen lights and the
 * LightInfo uniform, and builds the froxel masks in a compute pass timed as
 * "lights". With no lights the pass is skipped (the shaders see count 0).
 */
export class LightSystem {
  readonly infoBuffer: GPUBuffer;
  readonly lightBuffer: GPUBuffer;
  readonly viewBuffer: GPUBuffer;
  readonly maskBuffer: GPUBuffer;
  private readonly device: GPUDevice;
  private readonly pipeline: GPUComputePipeline;
  private readonly bindGroup: GPUBindGroup;
  private readonly packed = {
    data: new Float32Array(MAX_LIGHTS * LIGHT_FLOATS),
    views: new Float32Array(MAX_LIGHTS * LIGHT_VIEW_FLOATS),
  };
  private readonly infoData = new Float32Array(LIGHT_INFO_FLOATS);
  private readonly hot = new Map<string, HotSpot[]>();
  private count = 0;
  /** Lights uploaded in the last frame (debug overlay, gallery). */
  lastCount = 0;
  /** Candidates offered in the last frame (game lights plus hot spots). */
  lastCandidates = 0;

  constructor(device: GPUDevice) {
    this.device = device;
    this.infoBuffer = device.createBuffer({
      label: "light info",
      size: LIGHT_INFO_FLOATS * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.lightBuffer = device.createBuffer({
      label: "point lights",
      size: MAX_LIGHTS * LIGHT_FLOATS * 4,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST,
    });
    this.viewBuffer = device.createBuffer({
      label: "point light views",
      size: MAX_LIGHTS * LIGHT_VIEW_FLOATS * 4,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST,
    });
    this.maskBuffer = device.createBuffer({
      label: "light froxel masks",
      size: CLUSTER_COUNT * MASK_WORDS * 4,
      usage: GPUBufferUsage.STORAGE,
    });
    const layout = device.createBindGroupLayout({
      label: "light clusters",
      entries: [
        { binding: 0, visibility: GPUShaderStage.COMPUTE, buffer: { type: "uniform" } },
        { binding: 1, visibility: GPUShaderStage.COMPUTE, buffer: { type: "read-only-storage" } },
        { binding: 2, visibility: GPUShaderStage.COMPUTE, buffer: { type: "storage" } },
      ],
    });
    this.pipeline = device.createComputePipeline({
      label: "light clusters",
      layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }),
      compute: { module: device.createShaderModule({ label: "light clusters", code: CLUSTER_COMPUTE_WGSL }), entryPoint: "cs_main" },
    });
    this.bindGroup = device.createBindGroup({
      layout,
      entries: [
        { binding: 0, resource: { buffer: this.infoBuffer } },
        { binding: 1, resource: { buffer: this.viewBuffer } },
        { binding: 2, resource: { buffer: this.maskBuffer } },
      ],
    });
  }

  /** Entries 1-3 of the scene bind group (layout: lightBindLayoutEntries). */
  bindGroupEntries(): GPUBindGroupEntry[] {
    return [
      { binding: 1, resource: { buffer: this.infoBuffer } },
      { binding: 2, resource: { buffer: this.lightBuffer } },
      { binding: 3, resource: { buffer: this.maskBuffer } },
    ];
  }

  /** Records a terrain chunk's hot spots (null removes them). */
  setChunk(key: string, mesh: MeshData | null): void {
    const spots = mesh ? hotSpots(mesh) : [];
    if (spots.length > 0) this.hot.set(key, spots);
    else this.hot.delete(key);
  }

  clearChunks(): void {
    this.hot.clear();
  }

  /** Chooses and uploads this frame's lights (game lights plus hot spots), writes LightInfo. */
  prepare(lights: readonly PointLight[], camera: Vec3, view: LightView, max: number, frustum: Frustum, time: number): void {
    let all: PointLight[] = lights as PointLight[];
    if (this.hot.size > 0) {
      all = [...lights];
      for (const spots of this.hot.values()) for (const h of spots) all.push(hotSpotAt(h, time));
    }
    this.lastCandidates = all.length;
    const chosen = max > 0 ? selectLights(all, camera, max, frustum) : [];
    this.count = chosen.length;
    this.lastCount = chosen.length;
    if (this.count > 0) {
      packLights(chosen, view.view, this.packed);
      this.device.queue.writeBuffer(this.lightBuffer, 0, this.packed.data.buffer, 0, this.count * LIGHT_FLOATS * 4);
      this.device.queue.writeBuffer(this.viewBuffer, 0, this.packed.views.buffer, 0, this.count * LIGHT_VIEW_FLOATS * 4);
    }
    lightInfo(this.count, view, true, this.infoData);
    this.device.queue.writeBuffer(this.infoBuffer, 0, this.infoData);
  }

  /** Builds the froxel masks (skipped without lights). */
  encode(encoder: GPUCommandEncoder, timer: PassTimer): void {
    if (this.count === 0) return;
    const timestampWrites = timer?.pass("lights");
    const pass = encoder.beginComputePass({ label: "lights", ...(timestampWrites ? { timestampWrites } : {}) });
    pass.setPipeline(this.pipeline);
    pass.setBindGroup(0, this.bindGroup);
    pass.dispatchWorkgroups(Math.ceil(CLUSTER_COUNT / CLUSTER_WORKGROUP));
    pass.end();
  }

  destroy(): void {
    this.infoBuffer.destroy();
    this.lightBuffer.destroy();
    this.viewBuffer.destroy();
    this.maskBuffer.destroy();
  }
}
