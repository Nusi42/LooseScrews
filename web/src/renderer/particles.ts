// GPU particle system (visual-quality-plan.md VQ5).
//
// Particles live in one storage buffer: a ring of `capacity` slots that
// spawned particles overwrite oldest-first, followed by up to IMMEDIATE_MAX
// slots of "immediate" sprites that are rebuilt every frame (projectile
// cores, engine flames, beams: things glued to a moving object). A particle
// is written once at spawn; its motion is analytic (shaders/particles.ts:
// position and velocity from the spawn state and a drag constant), so the
// CPU only uploads the slots spawned since the last frame and the GPU needs
// no update pass. Dead or unborn slots collapse to a degenerate quad in the
// vertex shader. Dust motes are a second draw that needs no buffer at all.
//
// The passes run after the scene passes and read the scene depth for soft
// particles (no depth attachment), all timed as "particles": sprites wider
// than `halfResPx` on screen (smoke, fireballs: almost all of the fill) draw
// into a half-resolution target that is then composited onto the resolved
// HDR scene; smaller ones (sparks, cores, motes) draw at full resolution on
// top. `spawn` and `immediate` take a ParticleSpec; game/vfx.ts is the
// effect library that calls them.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { GpuTimer } from "./gpuTimer.ts";
import { HDR_FORMAT } from "./post.ts";
import {
  ORIENT,
  PARTICLE_COMPOSITE_WGSL,
  PARTICLE_FLOATS,
  PARTICLE_UNIFORM_FLOATS,
  particlesWgsl,
  PASS_KIND,
  PFLAG,
} from "./shaders/particles.ts";
import { createSpriteTexture, type SpriteId } from "./spriteAtlas.ts";

export type Rgb3 = readonly [number, number, number];

/** How a particle's quad is oriented (shaders/particles.ts ORIENT). */
export type Orient = keyof typeof ORIENT;

/** One particle at spawn. Colours are linear HDR radiance (emissive light), not display values. */
export interface ParticleSpec {
  pos: Vec3;
  /** Velocity (u/s); default at rest. For `fixed` and `plane` it gives the direction. */
  vel?: Vec3;
  /** Lifetime (s). */
  life: number;
  /** Spawn time on the particle clock (s); default the system's `now`. */
  t0?: number;
  sprite: SpriteId;
  /** Start colour (linear radiance for additive, albedo-ish for alpha). */
  color: Rgb3;
  /** End colour; default the start colour. */
  color1?: Rgb3;
  /** Start / end opacity of alpha-blended particles (ignored for additive); default 1 / 0. */
  alpha?: number;
  alpha1?: number;
  /** Start / end size (u); end defaults to start. */
  size: number;
  size1?: number;
  /** Exponential velocity drag (1/s); 0 = none. */
  drag?: number;
  /** Alpha-blended (smoke, debris) instead of additive light. */
  blend?: "add" | "alpha";
  orient?: Orient;
  /** `velocity`: seconds of travel the streak shows; `fixed`: length (u). */
  stretch?: number;
  /** Spin (rad/s) and start rotation (rad) of billboards. */
  spin?: number;
  rot?: number;
  /** Fraction of the life over which the particle fades in. */
  fadeIn?: number;
  /** Flame heat shading (white-hot core, dark red edges). */
  hot?: boolean;
}

/** The dust motes' warm grey (linear tint) before a mood colours them (M4 B5). */
export const DEFAULT_MOTE_COLOR: readonly [number, number, number] = [0.85, 0.78, 0.66];

/** Slots for immediate (per-frame) sprites after the ring. */
export const IMMEDIATE_MAX = 4096;

/**
 * The flags word of a spec (shaders/particles.ts PFLAG), stored as an
 * integer-valued f32 (exact below 2^24; a bit-cast would make small words
 * denormal floats, which GPUs may flush to zero).
 */
export function packFlags(p: Pick<ParticleSpec, "sprite" | "blend" | "orient" | "hot">): number {
  let f = p.sprite & PFLAG.spriteMask;
  if (p.blend === "alpha") f |= PFLAG.alpha;
  f |= (ORIENT[p.orient ?? "billboard"] & 3) << PFLAG.orientShift;
  if (p.hot) f |= PFLAG.hot;
  return f;
}

/** Writes one particle (PARTICLE_FLOATS floats) at `offset` of `out`; `now` stands in for a missing t0. */
export function packParticle(out: Float32Array, offset: number, p: ParticleSpec, now: number): void {
  const v = p.vel;
  const c0 = p.color;
  const c1 = p.color1 ?? c0;
  const o = offset;
  out[o] = p.pos.x;
  out[o + 1] = p.pos.y;
  out[o + 2] = p.pos.z;
  out[o + 3] = p.t0 ?? now;
  out[o + 4] = v ? v.x : 0;
  out[o + 5] = v ? v.y : 0;
  out[o + 6] = v ? v.z : 0;
  out[o + 7] = p.life;
  out[o + 8] = c0[0];
  out[o + 9] = c0[1];
  out[o + 10] = c0[2];
  out[o + 11] = p.alpha ?? 1;
  out[o + 12] = c1[0];
  out[o + 13] = c1[1];
  out[o + 14] = c1[2];
  out[o + 15] = p.alpha1 ?? 0;
  out[o + 16] = p.size;
  out[o + 17] = p.size1 ?? p.size;
  out[o + 18] = p.drag ?? 0;
  out[o + 19] = packFlags(p);
  out[o + 20] = p.stretch ?? 0;
  out[o + 21] = p.spin ?? 0;
  out[o + 22] = p.rot ?? 0;
  out[o + 23] = p.fadeIn ?? 0;
}

/** Particles living longer than this (s) go to the long-lived ring. */
export const LONG_LIFE = 0.7;
/** Share of the ring slots kept for long-lived particles (smoke, dust, debris). */
export const LONG_SHARE = 0.3;

/** One oldest-first ring of slots inside the particle buffer. */
class Region {
  readonly start: number;
  readonly capacity: number;
  head = 0;
  /** Slots written at least once. */
  used = 0;
  dirtyStart = -1;
  dirtyCount = 0;

  constructor(start: number, capacity: number) {
    this.start = start;
    this.capacity = Math.max(1, capacity);
  }

  /** The next slot (absolute index), marking it written. */
  take(): number {
    const slot = this.head;
    this.head = (slot + 1) % this.capacity;
    this.used = Math.min(this.capacity, this.used + 1);
    if (this.dirtyStart < 0) this.dirtyStart = slot;
    this.dirtyCount = Math.min(this.capacity, this.dirtyCount + 1);
    return this.start + slot;
  }

  takeDirty(out: Array<[number, number]>): void {
    if (this.dirtyStart < 0) return;
    const start = this.dirtyStart;
    const end = start + this.dirtyCount;
    this.dirtyStart = -1;
    this.dirtyCount = 0;
    if (end <= this.capacity) {
      out.push([this.start + start, end - start]);
    } else {
      out.push([this.start + start, this.capacity - start]);
      out.push([this.start, end - this.capacity]);
    }
  }

  clear(): void {
    this.head = 0;
    this.used = 0;
    this.dirtyStart = -1;
    this.dirtyCount = 0;
  }
}

/**
 * CPU side of the particle buffer: where spawns go and which slot ranges
 * must be uploaded. Two oldest-first rings, so a flood of short-lived
 * sparks and trail sections never evicts the smoke of an explosion: slots
 * [0, short) for particles living up to LONG_LIFE, [short, capacity) for
 * longer ones, then IMMEDIATE_MAX slots of this frame's immediate sprites.
 * Separate from the GPU code so Node tests cover it.
 */
export class ParticleRing {
  readonly capacity: number;
  readonly data: Float32Array;
  readonly short: Region;
  readonly long: Region;
  /** Immediate sprites this frame. */
  immCount = 0;
  /** Spawns since creation (statistics). */
  spawned = 0;
  /** Immediate sprites dropped because the frame's slots were full. */
  immDropped = 0;
  /** Particle clock (s) of the current frame. */
  now = 0;

  constructor(capacity: number) {
    this.capacity = Math.max(2, capacity | 0);
    const long = Math.max(1, Math.round(this.capacity * LONG_SHARE));
    this.short = new Region(0, this.capacity - long);
    this.long = new Region(this.capacity - long, long);
    this.data = new Float32Array((this.capacity + IMMEDIATE_MAX) * PARTICLE_FLOATS);
  }

  /** Slots written at least once (both rings). */
  get used(): number {
    return this.short.used + this.long.used;
  }

  spawn(p: ParticleSpec): void {
    const slot = (p.life > LONG_LIFE ? this.long : this.short).take();
    packParticle(this.data, slot * PARTICLE_FLOATS, p, this.now);
    this.spawned++;
  }

  immediate(p: ParticleSpec): void {
    if (this.immCount >= IMMEDIATE_MAX) {
      this.immDropped++;
      return;
    }
    packParticle(this.data, (this.capacity + this.immCount) * PARTICLE_FLOATS, { ...p, t0: this.now, life: Math.max(p.life, 1e-3) }, this.now);
    this.immCount++;
  }

  /** Slot ranges [start, count] written since the last call (up to two per ring: a ring may wrap). */
  takeDirty(): Array<[number, number]> {
    const out: Array<[number, number]> = [];
    this.short.takeDirty(out);
    this.long.takeDirty(out);
    return out;
  }

  /**
   * Buffer slot of draw instance `i` (the shader's mapping): the used slots
   * of the short ring, then of the long ring, then the immediates.
   */
  slotOf(i: number): number {
    if (i < this.short.used) return i;
    if (i < this.short.used + this.long.used) return this.long.start + (i - this.short.used);
    return this.capacity + (i - this.short.used - this.long.used);
  }

  /** Particles alive at `now` (ring slots plus immediates; a full scan, for tests and the gallery). */
  liveCount(now = this.now): number {
    let n = 0;
    const d = this.data;
    for (let i = 0; i < this.used; i++) {
      const o = this.slotOf(i) * PARTICLE_FLOATS;
      const age = now - (d[o + 3] ?? 0);
      if (age >= 0 && age < (d[o + 7] ?? 0)) n++;
    }
    return n + this.immCount;
  }

  /** Instances to draw: the used ring slots plus this frame's immediates. */
  get drawCount(): number {
    return this.used + this.immCount;
  }

  /** Forgets every particle (a new match). */
  clear(): void {
    this.short.clear();
    this.long.clear();
    this.immCount = 0;
    // Old slots must not come back to life: zero lifetimes.
    this.data.fill(0);
  }
}

/** Camera pose and lens for the particle pass. */
export interface ParticleCamera {
  position: Vec3;
  orientation: Quat;
  /** Vertical field of view (radians). */
  fovY: number;
  near: number;
  far: number;
}

/** Quality knobs of the particle system (RenderQuality.particles). */
export interface ParticleQuality {
  /** Ring slots (0 = particles off: the legacy mesh effects draw instead). */
  capacity: number;
  /** Spawn-rate factor for game/vfx.ts (1 at "high"). */
  density: number;
  /** Dust motes around the camera. */
  motes: number;
  /** Sprites at least this wide (px) draw at half resolution; 0 = everything at full resolution. */
  halfResPx: number;
}

/** Edge of the cube of dust motes around the camera (u). */
export const MOTE_VOLUME = 22;

export class ParticleSystem {
  private readonly device: GPUDevice;
  private readonly frameBuffer: GPUBuffer;
  /** Uniforms of the full-resolution draw and of the half-resolution one. */
  private readonly uniformBuffer: GPUBuffer;
  private readonly lowUniformBuffer: GPUBuffer;
  private readonly uniforms = new Float32Array(PARTICLE_UNIFORM_FLOATS);
  private readonly compositeLayout: GPUBindGroupLayout;
  private readonly compositePipeline: GPURenderPipeline;
  private readonly linearSampler: GPUSampler;
  private low: { texture: GPUTexture; view: GPUTextureView; group: GPUBindGroup } | null = null;
  private lowBindGroup: GPUBindGroup | null = null;
  private readonly sprites: GPUTexture;
  private readonly spriteView: GPUTextureView;
  private readonly sampler: GPUSampler;
  private readonly layouts = new Map<boolean, GPUBindGroupLayout>();
  private readonly pipelines = new Map<boolean, { particles: GPURenderPipeline; motes: GPURenderPipeline }>();
  private ring: ParticleRing;
  private buffer: GPUBuffer;
  private bindGroup: GPUBindGroup | null = null;
  private bindKey: { depth: GPUTexture; buffer: GPUBuffer } | null = null;
  /** Half-resolution split threshold (px; 0 = off). */
  halfResPx = 32;
  private lastCam: Vec3 | null = null;
  private lastTime = 0;
  private camVel: Vec3 = { x: 0, y: 0, z: 0 };
  /** Spawn-rate factor for the effect library (from the quality preset). */
  density = 1;
  /** Dust motes drawn around the camera. */
  motes = 1500;
  /** Mote size (u), brightness (radiance) and streak length (s of camera travel). */
  moteSize = 0.045;
  moteBrightness = 0.55;
  moteStreak = 0.035;
  /** Mote colour (M4 B5 mood presentation): linear tint per unit brightness, and the pastel rainbow share (0..1). */
  moteColor: readonly [number, number, number] = DEFAULT_MOTE_COLOR;
  moteRainbow = 0;

  constructor(device: GPUDevice, frameBuffer: GPUBuffer, quality: ParticleQuality) {
    this.device = device;
    this.frameBuffer = frameBuffer;
    this.ring = new ParticleRing(Math.max(1, quality.capacity));
    this.buffer = this.createBuffer(this.ring);
    this.density = quality.density;
    this.motes = quality.motes;
    this.halfResPx = quality.halfResPx;
    const uniform = (label: string): GPUBuffer =>
      device.createBuffer({ label, size: PARTICLE_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    this.uniformBuffer = uniform("particle uniforms");
    this.lowUniformBuffer = uniform("particle uniforms (half resolution)");
    this.linearSampler = device.createSampler({ label: "particle composite", magFilter: "linear", minFilter: "linear" });
    this.compositeLayout = device.createBindGroupLayout({
      label: "particle composite",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
      ],
    });
    const composite = device.createShaderModule({ label: "particle composite", code: PARTICLE_COMPOSITE_WGSL });
    this.compositePipeline = device.createRenderPipeline({
      label: "particle composite",
      layout: device.createPipelineLayout({ bindGroupLayouts: [this.compositeLayout] }),
      vertex: { module: composite, entryPoint: "vs_main" },
      fragment: {
        module: composite,
        entryPoint: "fs_main",
        targets: [
          {
            format: HDR_FORMAT,
            blend: {
              color: { srcFactor: "one", dstFactor: "one-minus-src-alpha", operation: "add" },
              alpha: { srcFactor: "zero", dstFactor: "one", operation: "add" },
            },
          },
        ],
      },
      primitive: { topology: "triangle-list" },
    });
    this.sprites = createSpriteTexture(device);
    this.spriteView = this.sprites.createView({ dimension: "2d-array" });
    this.sampler = device.createSampler({
      label: "particle sprites",
      magFilter: "linear",
      minFilter: "linear",
      mipmapFilter: "linear",
      addressModeU: "clamp-to-edge",
      addressModeV: "clamp-to-edge",
    });
  }

  private createBuffer(ring: ParticleRing): GPUBuffer {
    return this.device.createBuffer({
      label: "particles",
      size: ring.data.byteLength,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST,
    });
  }

  /** Particle clock (s): spawns without t0 start now; set once per frame before spawning. */
  get now(): number {
    return this.ring.now;
  }

  set now(t: number) {
    this.ring.now = t;
  }

  get capacity(): number {
    return this.ring.capacity;
  }

  /** Live slots in use (upper bound on live particles) plus this frame's immediates. */
  get drawCount(): number {
    return this.ring.drawCount;
  }

  /** Ring statistics for the debug overlay and tests. */
  get stats(): { capacity: number; used: number; immediates: number; spawned: number } {
    const r = this.ring;
    return { capacity: r.capacity, used: r.used, immediates: r.immCount, spawned: r.spawned };
  }

  spawn(p: ParticleSpec): void {
    this.ring.spawn(p);
  }

  /** Particles alive now (a CPU scan of the ring; gallery and tests). */
  liveCount(): number {
    return this.ring.liveCount();
  }

  immediate(p: ParticleSpec): void {
    this.ring.immediate(p);
  }

  clear(): void {
    this.ring.clear();
    this.device.queue.writeBuffer(this.buffer, 0, this.ring.data);
  }

  /** Applies a quality preset: a new ring size drops the live particles. */
  setQuality(quality: ParticleQuality): void {
    this.density = quality.density;
    this.motes = quality.motes;
    this.halfResPx = quality.halfResPx;
    const capacity = Math.max(1, quality.capacity);
    if (capacity === this.ring.capacity) return;
    this.ring = new ParticleRing(capacity);
    this.buffer.destroy();
    this.buffer = this.createBuffer(this.ring);
    this.bindGroup = null;
    this.bindKey = null;
  }

  private layout(ms: boolean): GPUBindGroupLayout {
    let l = this.layouts.get(ms);
    if (!l) {
      l = this.device.createBindGroupLayout({
        label: `particles${ms ? " msaa" : ""}`,
        entries: [
          { binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
          { binding: 1, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
          { binding: 2, visibility: GPUShaderStage.VERTEX, buffer: { type: "read-only-storage" } },
          { binding: 3, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float", viewDimension: "2d-array" } },
          { binding: 4, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
          { binding: 5, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "depth", viewDimension: "2d", multisampled: ms } },
        ],
      });
      this.layouts.set(ms, l);
    }
    return l;
  }

  private pipelinesFor(ms: boolean): { particles: GPURenderPipeline; motes: GPURenderPipeline } {
    let p = this.pipelines.get(ms);
    if (!p) {
      const device = this.device;
      const module = device.createShaderModule({ label: "particles", code: particlesWgsl(ms) });
      const layout = device.createPipelineLayout({ bindGroupLayouts: [this.layout(ms)] });
      // Premultiplied "over": additive sprites write alpha 0. The alpha
      // channel accumulates coverage (it matters in the half-resolution
      // target, whose composite attenuates the scene by it).
      const target: GPUColorTargetState = {
        format: HDR_FORMAT,
        blend: {
          color: { srcFactor: "one", dstFactor: "one-minus-src-alpha", operation: "add" },
          alpha: { srcFactor: "one", dstFactor: "one-minus-src-alpha", operation: "add" },
        },
      };
      const make = (label: string, entryPoint: string): GPURenderPipeline =>
        device.createRenderPipeline({
          label,
          layout,
          vertex: { module, entryPoint },
          fragment: { module, entryPoint: "fs_main", targets: [target] },
          primitive: { topology: "triangle-list", cullMode: "none" },
        });
      p = { particles: make("particles", "vs_main"), motes: make("dust motes", "vs_mote") };
      this.pipelines.set(ms, p);
    }
    return p;
  }

  /**
   * Uploads this frame's spawns and immediates and draws particles and motes
   * into `target` (the resolved HDR scene), reading `depth` (multisampled
   * when its sample count is above 1) for soft edges and occlusion.
   */
  encode(
    encoder: GPUCommandEncoder,
    target: GPUTextureView,
    depth: GPUTexture,
    camera: ParticleCamera,
    size: { width: number; height: number },
    timer: GpuTimer | null,
  ): void {
    const ring = this.ring;
    const ms = depth.sampleCount > 1;
    const device = this.device;
    for (const [start, count] of ring.takeDirty()) {
      device.queue.writeBuffer(this.buffer, start * PARTICLE_FLOATS * 4, ring.data, start * PARTICLE_FLOATS, count * PARTICLE_FLOATS);
    }
    if (ring.immCount > 0) {
      const off = ring.capacity * PARTICLE_FLOATS;
      device.queue.writeBuffer(this.buffer, off * 4, ring.data, off, ring.immCount * PARTICLE_FLOATS);
    }
    const split = this.halfResPx > 0 && ring.drawCount > 0;
    this.writeUniforms(camera, size, split ? PASS_KIND.small : PASS_KIND.all, 1, this.uniformBuffer);
    if (split) this.writeUniforms(camera, size, PASS_KIND.big, 2, this.lowUniformBuffer);
    if (!this.bindGroup || !this.lowBindGroup || !this.bindKey || this.bindKey.depth !== depth || this.bindKey.buffer !== this.buffer) {
      const group = (uniforms: GPUBuffer): GPUBindGroup =>
        device.createBindGroup({
          label: "particles",
          layout: this.layout(ms),
          entries: [
            { binding: 0, resource: { buffer: this.frameBuffer } },
            { binding: 1, resource: { buffer: uniforms } },
            { binding: 2, resource: { buffer: this.buffer } },
            { binding: 3, resource: this.spriteView },
            { binding: 4, resource: this.sampler },
            { binding: 5, resource: depth.createView({ aspect: "depth-only" }) },
          ],
        });
      this.bindGroup = group(this.uniformBuffer);
      this.lowBindGroup = group(this.lowUniformBuffer);
      this.bindKey = { depth, buffer: this.buffer };
    }
    const pipelines = this.pipelinesFor(ms);
    const begin = (label: string, view: GPUTextureView, clear: boolean): GPURenderPassEncoder => {
      const timestampWrites = timer?.pass("particles");
      return encoder.beginRenderPass({
        label,
        ...(timestampWrites ? { timestampWrites } : {}),
        colorAttachments: [{ view, loadOp: clear ? "clear" : "load", clearValue: { r: 0, g: 0, b: 0, a: 0 }, storeOp: "store" }],
      });
    };
    if (split) {
      // Big sprites into the half-resolution target, then onto the scene.
      const low = this.lowTarget(size);
      const pass = begin("particles (half resolution)", low.view, true);
      pass.setBindGroup(0, this.lowBindGroup);
      pass.setPipeline(pipelines.particles);
      pass.draw(6, ring.drawCount);
      pass.end();
      const comp = begin("particle composite", target, false);
      comp.setPipeline(this.compositePipeline);
      comp.setBindGroup(0, low.group);
      comp.draw(3);
      comp.end();
    }
    const pass = begin("particles", target, false);
    pass.setBindGroup(0, this.bindGroup);
    if (this.motes > 0) {
      pass.setPipeline(pipelines.motes);
      pass.draw(6, this.motes);
    }
    if (ring.drawCount > 0) {
      pass.setPipeline(pipelines.particles);
      pass.draw(6, ring.drawCount);
    }
    pass.end();
  }

  /** The half-resolution particle target for a canvas size. */
  private lowTarget(size: { width: number; height: number }): { texture: GPUTexture; view: GPUTextureView; group: GPUBindGroup } {
    const w = Math.max(1, Math.ceil(size.width / 2));
    const h = Math.max(1, Math.ceil(size.height / 2));
    if (this.low && this.low.texture.width === w && this.low.texture.height === h) return this.low;
    this.low?.texture.destroy();
    const texture = this.device.createTexture({
      label: "particles (half resolution)",
      size: { width: w, height: h },
      format: HDR_FORMAT,
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
    });
    const view = texture.createView();
    const group = this.device.createBindGroup({
      label: "particle composite",
      layout: this.compositeLayout,
      entries: [
        { binding: 0, resource: view },
        { binding: 1, resource: this.linearSampler },
      ],
    });
    this.low = { texture, view, group };
    return this.low;
  }

  /**
   * Starts a frame of immediate sprites: the previous frame's are dropped.
   * Without a call they are drawn again (fixed-pose gallery frames).
   */
  beginFrame(): void {
    this.ring.immCount = 0;
  }

  private writeUniforms(camera: ParticleCamera, size: { width: number; height: number }, kind: number, depthScale: number, buffer: GPUBuffer): void {
    const u = this.uniforms;
    const r = q.right(camera.orientation);
    const up = q.up(camera.orientation);
    const f = q.forward(camera.orientation);
    // Camera velocity for the mote streaks, smoothed; teleports (respawn) reset it.
    const now = this.ring.now;
    const dt = now - this.lastTime;
    if (this.lastCam && dt > 1e-4 && dt < 0.25) {
      const v = {
        x: (camera.position.x - this.lastCam.x) / dt,
        y: (camera.position.y - this.lastCam.y) / dt,
        z: (camera.position.z - this.lastCam.z) / dt,
      };
      // A jump (respawn, spectator switch) is not motion.
      const k = Math.hypot(v.x, v.y, v.z) > 200 ? 0 : 0.3;
      const c = this.camVel;
      this.camVel = k === 0 ? { x: 0, y: 0, z: 0 } : { x: c.x + (v.x - c.x) * k, y: c.y + (v.y - c.y) * k, z: c.z + (v.z - c.z) * k };
    }
    if (dt !== 0 && kind !== PASS_KIND.big) {
      this.lastCam = { ...camera.position };
      this.lastTime = now;
    }
    const cv = this.camVel;
    u.set([r.x, r.y, r.z, now], 0);
    u.set([up.x, up.y, up.z, camera.near], 4);
    u.set([f.x, f.y, f.z, camera.far], 8);
    const ring = this.ring;
    u.set([ring.short.used, ring.long.start, ring.long.used, ring.capacity], 12);
    u.set([cv.x, cv.y, cv.z, MOTE_VOLUME], 16);
    u.set([this.motes, this.moteSize, this.moteBrightness, this.moteStreak], 20);
    const focal = size.height / 2 / Math.tan(camera.fovY / 2);
    u.set([kind, this.halfResPx, depthScale, focal], 24);
    u.set([this.moteColor[0], this.moteColor[1], this.moteColor[2], this.moteRainbow], 28);
    this.device.queue.writeBuffer(buffer, 0, u);
  }

  destroy(): void {
    this.buffer.destroy();
    this.uniformBuffer.destroy();
    this.lowUniformBuffer.destroy();
    this.low?.texture.destroy();
    this.sprites.destroy();
  }
}
