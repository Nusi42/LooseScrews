// Sky and atmosphere on the GPU (docs/visual-quality-plan.md VQ7):
// - the environment cube (ENV_SIZE per face, full mip chain, rgba16float),
//   baked once per world by `setAtmosphere`; the frame bind group exposes it
//   to every scene shader for the fog (frame.ts `apply_fog`);
// - the planet's surface texture, baked with the cube;
// - the sky pass (`composite`) in open levels: after the scene is resolved
//   (single-sampled, no MSAA cost), drawn *under* it with the scene's alpha
//   (geometry writes 1, the clear 0): out = scene + sky x (1 - alpha);
// - the light shafts (`encodeShafts`), three small passes after that and
//   before bloom, at "high" quality only.
// All passes are timed as "sky". The pure per-world logic is in atmosphere.ts.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import { shaftFade, sunScreen, type Atmosphere } from "./atmosphere.ts";
import type { GpuTimer } from "./gpuTimer.ts";
import { HDR_FORMAT } from "./post.ts";
import { srgbToLinear } from "./shaders/frame.ts";
import {
  ENV_BAKE_WGSL,
  ENV_UNIFORM_FLOATS,
  PLANET_BAKE_WGSL,
  PLANET_UNIFORM_FLOATS,
  SHAFT_BLUR_WGSL,
  SHAFT_COMPOSITE_WGSL,
  SHAFT_PREFILTER_WGSL,
  SHAFT_UNIFORM_FLOATS,
  SKY_UNIFORM_FLOATS,
  SKY_WGSL,
} from "./shaders/sky.ts";

/** Texels per side of the environment cube's base mip. */
export const ENV_SIZE = 512;
/** Mip levels of the environment cube (down to 1x1). */
export const ENV_MIPS = Math.log2(ENV_SIZE) + 1;
/** Width of the galactic band (squared cosine scale of its Gaussian). */
const BAND_WIDTH = 0.12;
/** Size of the planet's surface texture (equirectangular). */
export const PLANET_TEX_WIDTH = 1024;
export const PLANET_TEX_HEIGHT = 512;
/** Radial blur samples of the light shafts. */
export const SHAFT_SAMPLES = 40;

type Rgb = readonly [number, number, number];
const lin = (c: Rgb): [number, number, number] => [srgbToLinear(c[0]), srgbToLinear(c[1]), srgbToLinear(c[2])];

/** Packs the environment bake uniform (struct Env in shaders/sky.ts). */
export function packEnvUniforms(a: Atmosphere, out = new Float32Array(ENV_UNIFORM_FLOATS)): Float32Array {
  const n = a.nebula;
  out.set([...lin(a.zenith), n.strength], 0);
  out.set([...lin(a.horizon), a.stars], 4);
  out.set([...lin(a.nadir), ENV_SIZE], 8);
  out.set([...lin(n.a), BAND_WIDTH], 12);
  out.set([...lin(n.b), 0], 16);
  out.set([...lin(n.core), 0], 20);
  out.set([n.band.x, n.band.y, n.band.z, 0], 24);
  out.set([n.offset.x, n.offset.y, n.offset.z, 0], 28);
  out.set([a.sunDirection.x, a.sunDirection.y, a.sunDirection.z, 0], 32);
  // The halo is weaker in enclosed worlds (the sun is outside the rock).
  out.set([...lin(a.sunColor), 0.25 + 0.75 * a.open], 36);
  return out;
}

/** Distance from the camera within which the boundary grid shows (u). */
export const GRID_RANGE = 10;

/** Boundary grid strength: 1 within GRID_RANGE of a face of the room box, else 0 (the sky pass skips it). */
export function gridStrength(room: { min: Vec3; max: Vec3 } | null, p: Vec3): number {
  if (!room) return 0;
  const d = Math.min(p.x - room.min.x, room.max.x - p.x, p.y - room.min.y, room.max.y - p.y, p.z - room.min.z, room.max.z - p.z);
  return d < GRID_RANGE ? 1 : 0;
}

/** Angular radius (rad) of a cone around the planet direction holding its disc, limb glow and rings. */
export function planetCone(p: { radius: number; rings: number }): number {
  const disc = p.radius * 1.3;
  if (p.rings <= 0) return disc;
  const reach = Math.sin(p.radius) * p.rings;
  return reach >= 1 ? Math.PI : Math.max(disc, Math.asin(reach) * 1.05);
}

export interface SkyCamera {
  position: Vec3;
  orientation: Quat;
  fovY: number;
}

/** Packs the sky pass uniform (struct Sky in shaders/sky.ts) for one frame. */
export function packSkyUniforms(
  a: Atmosphere,
  cam: SkyCamera,
  width: number,
  height: number,
  time: number,
  out = new Float32Array(SKY_UNIFORM_FLOATS),
  turn: Quat = q.identity(),
  level = 1,
): Float32Array {
  const tanY = Math.tan(cam.fovY / 2);
  const tanX = tanY * (width / Math.max(height, 1));
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const f = q.forward(cam.orientation);
  const px = (2 * tanY) / Math.max(height, 1);
  out.set([r.x * tanX, r.y * tanX, r.z * tanX, px], 0);
  out.set([u.x * tanY, u.y * tanY, u.z * tanY, time], 4);
  out.set([f.x, f.y, f.z, a.stars], 8);
  out.set([cam.position.x, cam.position.y, cam.position.z, gridStrength(a.room, cam.position)], 12);
  const s = a.sunDirection;
  out.set([s.x, s.y, s.z, a.sunSize], 16);
  out.set([...lin(a.sunColor), 0], 20);
  const p = a.planet;
  if (p) {
    out.set([p.direction.x, p.direction.y, p.direction.z, p.radius], 24);
    out.set([p.axis.x, p.axis.y, p.axis.z, p.rings], 28);
    out.set([...lin(p.c0), p.kind], 32);
    out.set([...lin(p.c1), p.seed], 36);
    out.set([...lin(p.c2), Math.cos(planetCone(p))], 40);
    out.set([...lin(p.air), 0], 44);
  } else {
    out.fill(0, 24, 48);
  }
  const room = a.room;
  out.set(room ? [room.min.x, room.min.y, room.min.z, 0] : [0, 0, 0, 0], 48);
  out.set(room ? [room.max.x, room.max.y, room.max.z, 0] : [0, 0, 0, 0], 52);
  // z: the level's light (RenderSettings.lightLevel, M4 C3 Disco inferno dims the sky).
  out.set([width, height, level, 0], 56);
  const b = a.nebula.band;
  out.set([b.x, b.y, b.z, BAND_WIDTH], 60);
  // World to sky frame (the inverse of the level's turn, M4 B5).
  out.set([-turn.x, -turn.y, -turn.z, turn.w], 64);
  return out;
}

interface ShaftTargets {
  width: number;
  height: number;
  mask: GPUTexture;
  blur: GPUTexture;
}

export class Sky {
  private readonly device: GPUDevice;
  private readonly env: GPUTexture;
  private readonly sampler: GPUSampler;
  private readonly clampSampler: GPUSampler;
  private readonly envBuffer: GPUBuffer;
  private readonly bakePipeline: GPURenderPipeline;
  private readonly bakeGroup: GPUBindGroup;
  private readonly skyLayout: GPUBindGroupLayout;
  private readonly skyModule: GPUShaderModule;
  private readonly skyBuffer: GPUBuffer;
  private readonly skyData = new Float32Array(SKY_UNIFORM_FLOATS);
  private readonly skyGroup: GPUBindGroup;
  private readonly skyPipeline: GPURenderPipeline;
  private readonly planetTex: GPUTexture;
  private readonly planetBuffer: GPUBuffer;
  private readonly planetPipeline: GPURenderPipeline;
  private readonly planetGroup: GPUBindGroup;
  private readonly shaftLayout: GPUBindGroupLayout;
  private readonly shaftPipelines: { prefilter: GPURenderPipeline; blur: GPURenderPipeline; composite: GPURenderPipeline };
  private readonly shaftBuffer: GPUBuffer;
  private readonly shaftData = new Float32Array(SHAFT_UNIFORM_FLOATS);
  private shaftTargets: ShaftTargets | null = null;
  private atmosphereValue: Atmosphere | null = null;
  /** The level's turn (M4 B5, turn.ts): the sky, its sun and the shafts turn with it. */
  turn: Quat = q.identity();
  /** The level's light (RenderSettings.lightLevel, M4 C3): scales the sky and the shafts. */
  level = 1;

  /** The environment cube (sampled by the fog of every scene shader). */
  readonly envView: GPUTextureView;
  /** Trilinear sampler for the environment cube. */
  get envSampler(): GPUSampler {
    return this.sampler;
  }

  constructor(device: GPUDevice) {
    this.device = device;
    this.env = device.createTexture({
      label: "environment cube",
      size: { width: ENV_SIZE, height: ENV_SIZE, depthOrArrayLayers: 6 },
      mipLevelCount: ENV_MIPS,
      format: HDR_FORMAT,
      dimension: "2d",
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.RENDER_ATTACHMENT,
    });
    this.envView = this.env.createView({ dimension: "cube" });
    this.sampler = device.createSampler({
      label: "environment",
      magFilter: "linear",
      minFilter: "linear",
      mipmapFilter: "linear",
    });
    this.clampSampler = device.createSampler({
      label: "shafts",
      magFilter: "linear",
      minFilter: "linear",
      addressModeU: "clamp-to-edge",
      addressModeV: "clamp-to-edge",
    });

    this.envBuffer = device.createBuffer({ label: "env bake", size: ENV_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    const bakeModule = device.createShaderModule({ label: "env bake", code: ENV_BAKE_WGSL });
    const bakeLayout = device.createBindGroupLayout({
      label: "env bake",
      entries: [{ binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } }],
    });
    this.bakePipeline = device.createRenderPipeline({
      label: "env bake",
      layout: device.createPipelineLayout({ bindGroupLayouts: [bakeLayout] }),
      vertex: { module: bakeModule, entryPoint: "vs_main" },
      fragment: { module: bakeModule, entryPoint: "fs_main", targets: [{ format: HDR_FORMAT }] },
      primitive: { topology: "triangle-list" },
    });
    this.bakeGroup = device.createBindGroup({ layout: bakeLayout, entries: [{ binding: 0, resource: { buffer: this.envBuffer } }] });

    // Planet surface: linear albedo in an sRGB-encoded texture (no banding in the darks).
    const PLANET_FORMAT: GPUTextureFormat = "rgba8unorm-srgb";
    this.planetTex = device.createTexture({
      label: "planet surface",
      size: { width: PLANET_TEX_WIDTH, height: PLANET_TEX_HEIGHT },
      format: PLANET_FORMAT,
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.RENDER_ATTACHMENT,
    });
    const planetSampler = device.createSampler({
      label: "planet",
      magFilter: "linear",
      minFilter: "linear",
      addressModeU: "repeat",
      addressModeV: "clamp-to-edge",
    });
    this.planetBuffer = device.createBuffer({ label: "planet bake", size: PLANET_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    const planetModule = device.createShaderModule({ label: "planet bake", code: PLANET_BAKE_WGSL });
    this.planetPipeline = device.createRenderPipeline({
      label: "planet bake",
      layout: device.createPipelineLayout({ bindGroupLayouts: [bakeLayout] }),
      vertex: { module: planetModule, entryPoint: "vs_main" },
      fragment: { module: planetModule, entryPoint: "fs_main", targets: [{ format: PLANET_FORMAT }] },
      primitive: { topology: "triangle-list" },
    });
    this.planetGroup = device.createBindGroup({ layout: bakeLayout, entries: [{ binding: 0, resource: { buffer: this.planetBuffer } }] });

    this.skyBuffer = device.createBuffer({ label: "sky", size: SKY_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    this.skyLayout = device.createBindGroupLayout({
      label: "sky",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { viewDimension: "cube" } },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
        { binding: 3, visibility: GPUShaderStage.FRAGMENT, texture: {} },
        { binding: 4, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
      ],
    });
    this.skyModule = device.createShaderModule({ label: "sky", code: SKY_WGSL });
    this.skyGroup = device.createBindGroup({
      layout: this.skyLayout,
      entries: [
        { binding: 0, resource: { buffer: this.skyBuffer } },
        { binding: 1, resource: this.envView },
        { binding: 2, resource: this.sampler },
        { binding: 3, resource: this.planetTex.createView() },
        { binding: 4, resource: planetSampler },
      ],
    });
    // Under the scene: out = scene + sky x (1 - scene alpha); the alpha stays.
    this.skyPipeline = device.createRenderPipeline({
      label: "sky",
      layout: device.createPipelineLayout({ bindGroupLayouts: [this.skyLayout] }),
      vertex: { module: this.skyModule, entryPoint: "vs_main" },
      fragment: {
        module: this.skyModule,
        entryPoint: "fs_main",
        targets: [
          {
            format: HDR_FORMAT,
            blend: {
              color: { srcFactor: "one-minus-dst-alpha", dstFactor: "one", operation: "add" },
              alpha: { srcFactor: "zero", dstFactor: "one", operation: "add" },
            },
          },
        ],
      },
      primitive: { topology: "triangle-list" },
    });

    this.shaftBuffer = device.createBuffer({ label: "shafts", size: SHAFT_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    this.shaftLayout = device.createBindGroupLayout({
      label: "shafts",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: {} },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
      ],
    });
    const shaftPipeline = (label: string, code: string, blend?: GPUBlendState): GPURenderPipeline => {
      const module = device.createShaderModule({ label, code });
      return device.createRenderPipeline({
        label,
        layout: device.createPipelineLayout({ bindGroupLayouts: [this.shaftLayout] }),
        vertex: { module, entryPoint: "vs_main" },
        fragment: { module, entryPoint: "fs_main", targets: [blend ? { format: HDR_FORMAT, blend } : { format: HDR_FORMAT }] },
        primitive: { topology: "triangle-list" },
      });
    };
    this.shaftPipelines = {
      prefilter: shaftPipeline("shaft mask", SHAFT_PREFILTER_WGSL),
      blur: shaftPipeline("shaft blur", SHAFT_BLUR_WGSL),
      composite: shaftPipeline("shaft composite", SHAFT_COMPOSITE_WGSL, {
        color: { srcFactor: "one", dstFactor: "one", operation: "add" },
        alpha: { srcFactor: "zero", dstFactor: "one", operation: "add" },
      }),
    };
  }

  /** The active atmosphere (null: no sky, flat fog). */
  get atmosphere(): Atmosphere | null {
    return this.atmosphereValue;
  }

  /** True when the sky pass draws (an open level). */
  get visible(): boolean {
    return (this.atmosphereValue?.open ?? 0) > 0;
  }

  /** Sets the world's atmosphere and bakes its environment cube (every face and mip, one submit). */
  setAtmosphere(a: Atmosphere | null): void {
    this.atmosphereValue = a;
    if (!a) return;
    this.device.queue.writeBuffer(this.envBuffer, 0, packEnvUniforms(a));
    const encoder = this.device.createCommandEncoder({ label: "env bake" });
    for (let mip = 0; mip < ENV_MIPS; mip++) {
      for (let face = 0; face < 6; face++) {
        const view = this.env.createView({ dimension: "2d", baseMipLevel: mip, mipLevelCount: 1, baseArrayLayer: face, arrayLayerCount: 1 });
        const pass = encoder.beginRenderPass({
          label: "env bake",
          colorAttachments: [{ view, loadOp: "clear", clearValue: { r: 0, g: 0, b: 0, a: 1 }, storeOp: "store" }],
        });
        pass.setPipeline(this.bakePipeline);
        pass.setBindGroup(0, this.bakeGroup);
        pass.draw(3, 1, 0, face + 6 * mip);
        pass.end();
      }
    }
    const p = a.planet;
    if (p) {
      this.device.queue.writeBuffer(this.planetBuffer, 0, new Float32Array([...lin(p.c0), p.kind, ...lin(p.c1), p.seed, ...lin(p.c2), 0]));
      const pass = encoder.beginRenderPass({
        label: "planet bake",
        colorAttachments: [{ view: this.planetTex.createView(), loadOp: "clear", clearValue: { r: 0, g: 0, b: 0, a: 1 }, storeOp: "store" }],
      });
      pass.setPipeline(this.planetPipeline);
      pass.setBindGroup(0, this.planetGroup);
      pass.draw(3);
      pass.end();
    }
    this.device.queue.submit([encoder.finish()]);
  }

  /**
   * Draws the sky under the resolved HDR scene (timed as "sky"): where the
   * scene's alpha is below 1 (the clear is 0), the sky shows. No-op unless
   * `visible`.
   */
  composite(encoder: GPUCommandEncoder, scene: GPUTexture, cam: SkyCamera, time: number, timer: GpuTimer | null): void {
    const a = this.atmosphereValue;
    if (!a || a.open <= 0) return;
    this.device.queue.writeBuffer(this.skyBuffer, 0, packSkyUniforms(a, cam, scene.width, scene.height, time, this.skyData, this.turn, this.level));
    const timestampWrites = timer?.pass("sky");
    const pass = encoder.beginRenderPass({
      label: "sky",
      colorAttachments: [{ view: scene.createView(), loadOp: "load", storeOp: "store" }],
      ...(timestampWrites ? { timestampWrites } : {}),
    });
    pass.setPipeline(this.skyPipeline);
    pass.setBindGroup(0, this.skyGroup);
    pass.draw(3);
    pass.end();
  }

  /**
   * Light shafts from the sun through the openings: sky mask (scene alpha 0)
   * at quarter resolution, radial blur toward the sun, added onto `scene`
   * (the resolved HDR texture). Skipped when the sun is far off screen.
   */
  encodeShafts(encoder: GPUCommandEncoder, scene: GPUTexture, viewProj: ArrayLike<number>, timer: GpuTimer | null): void {
    const a = this.atmosphereValue;
    if (!a || a.open <= 0 || a.shafts <= 0) return;
    const ndc = sunScreen(viewProj, q.rotate(this.turn, a.sunDirection));
    const fade = shaftFade(ndc) * a.shafts * this.level;
    if (!ndc || fade <= 0.001) return;
    const width = Math.max(1, Math.ceil(scene.width / 4));
    const height = Math.max(1, Math.ceil(scene.height / 4));
    let t = this.shaftTargets;
    if (!t || t.width !== width || t.height !== height) {
      t?.mask.destroy();
      t?.blur.destroy();
      const make = (label: string): GPUTexture =>
        this.device.createTexture({ label, size: { width, height }, format: HDR_FORMAT, usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING });
      t = { width, height, mask: make("shaft mask"), blur: make("shaft blur") };
      this.shaftTargets = t;
    }
    const sun = lin(a.sunColor);
    const k = 0.7;
    this.shaftData.set([ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, scene.width / scene.height, fade], 0);
    this.shaftData.set([sun[0] * k, sun[1] * k, sun[2] * k, 0], 4);
    this.shaftData.set([1 / scene.width, 1 / scene.height, 0.955, SHAFT_SAMPLES], 8);
    this.device.queue.writeBuffer(this.shaftBuffer, 0, this.shaftData);
    const group = (view: GPUTextureView): GPUBindGroup =>
      this.device.createBindGroup({
        layout: this.shaftLayout,
        entries: [
          { binding: 0, resource: view },
          { binding: 1, resource: this.clampSampler },
          { binding: 2, resource: { buffer: this.shaftBuffer } },
        ],
      });
    const run = (label: string, pipeline: GPURenderPipeline, src: GPUTextureView, dst: GPUTextureView, load: boolean): void => {
      const timestampWrites = timer?.pass("sky");
      const pass = encoder.beginRenderPass({
        label,
        colorAttachments: [{ view: dst, loadOp: load ? "load" : "clear", clearValue: { r: 0, g: 0, b: 0, a: 0 }, storeOp: "store" }],
        ...(timestampWrites ? { timestampWrites } : {}),
      });
      pass.setPipeline(pipeline);
      pass.setBindGroup(0, group(src));
      pass.draw(3);
      pass.end();
    };
    const maskView = t.mask.createView();
    const blurView = t.blur.createView();
    run("shaft mask", this.shaftPipelines.prefilter, scene.createView(), maskView, false);
    run("shaft blur", this.shaftPipelines.blur, maskView, blurView, false);
    run("shaft composite", this.shaftPipelines.composite, blurView, scene.createView(), true);
  }

  destroy(): void {
    this.env.destroy();
    this.planetTex.destroy();
    this.planetBuffer.destroy();
    this.envBuffer.destroy();
    this.skyBuffer.destroy();
    this.shaftBuffer.destroy();
    this.shaftTargets?.mask.destroy();
    this.shaftTargets?.blur.destroy();
  }
}
