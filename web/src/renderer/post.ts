// HDR post chain (VQ2): the scene renders linear radiance into an
// rgba16float target (MSAA-resolved by the renderer); this module
// downsamples it into a bloom mip chain (13-tap with a Karis average on the
// first step), upsamples it back with a tent filter (additive), and
// composites into the swapchain: exposure, colour grade, bloom, vignette,
// AgX tonemap, sRGB encoding, dither and optional film grain.
//
// The colour grade is the hook for per-world looks (VQ7: caves warm, mines
// sodium, sky cold) and for Last Color Standing's monochrome (C1): set
// `Renderer.settings.post.grade` (GRADE_PRESETS or your own).

import type { GpuTimer } from "./gpuTimer.ts";
import type { Rgb } from "./renderer.ts";
import {
  BLOOM_DOWN_WGSL,
  BLOOM_UNIFORM_FLOATS,
  BLOOM_UP_WGSL,
  COMPOSITE_WGSL,
  POST_UNIFORM_FLOATS,
} from "./shaders/post.ts";

/** Scene colour target format: linear HDR. */
export const HDR_FORMAT: GPUTextureFormat = "rgba16float";

/** Colour grade in scene-linear light, applied before the tonemap (ASC CDL plus extras). */
export interface ColorGrade {
  slope: Rgb;
  offset: Rgb;
  power: Rgb;
  /** 1 = unchanged, 0 = grey. */
  saturation: number;
  /** 0..1 blend toward monochrome (Last Color Standing). */
  monochrome: number;
  /** Colour kept in colour while monochrome (e.g. the own team's paint), display value. */
  monoTint: Rgb;
  /** 0..1 how much of `monoTint`'s hue survives the monochrome (saturated colours of that hue only). */
  monoKeep: number;
}

export const NEUTRAL_GRADE: ColorGrade = {
  slope: [1, 1, 1],
  offset: [0, 0, 0],
  power: [1, 1, 1],
  saturation: 1,
  monochrome: 0,
  monoTint: [1, 1, 1],
  monoKeep: 0,
};

/**
 * Per world-type looks: atmosphere.ts (`worldGrade`, VQ7) blends caves,
 * mines and sky by the world's Enclosure and desaturates by its Palette
 * slider. `lastColor` is the monochrome look with one colour kept.
 */
export const GRADE_PRESETS = {
  neutral: NEUTRAL_GRADE,
  caves: { ...NEUTRAL_GRADE, slope: [1.04, 1.0, 0.94], saturation: 1.05 },
  mines: { ...NEUTRAL_GRADE, slope: [1.1, 0.99, 0.84], power: [0.98, 1.0, 1.04], saturation: 1.05 },
  sky: { ...NEUTRAL_GRADE, slope: [0.94, 0.99, 1.08], saturation: 0.95 },
  // Set monoTint (e.g. the leading team's paint colour) and monoKeep 1 to keep one colour.
  lastColor: { ...NEUTRAL_GRADE, monochrome: 1 },
} as const satisfies Record<string, ColorGrade>;

export interface PostSettings {
  /** Linear multiplier before the tonemap. */
  exposure: number;
  /** Bloom gain (the averaged bloom chain is added to the scene times this). */
  bloomStrength: number;
  /** Scene radiance where bloom starts (soft knee of half the threshold); 0 blooms everything. */
  bloomThreshold: number;
  /** Corner darkening 0..1. */
  vignette: number;
  /** Film grain amount (0 = off, the default). */
  grain: number;
  /** AgX look: power (contrast) and saturation after the base curve. */
  agxPower: number;
  agxSaturation: number;
  grade: ColorGrade;
}

export const DEFAULT_POST: PostSettings = {
  exposure: 1.4,
  bloomStrength: 1.5,
  bloomThreshold: 1.0,
  vignette: 0.35,
  grain: 0,
  agxPower: 1.3,
  agxSaturation: 1.25,
  grade: NEUTRAL_GRADE,
};

/** Packs the composite uniform (struct Post in shaders/post.ts). */
export function packPostUniforms(s: PostSettings, time: number, bloomMips: number, out = new Float32Array(POST_UNIFORM_FLOATS)): Float32Array {
  const g = s.grade;
  out.set([g.slope[0], g.slope[1], g.slope[2], s.exposure], 0);
  out.set([g.offset[0], g.offset[1], g.offset[2], bloomMips > 0 ? s.bloomStrength : 0], 4);
  out.set([g.power[0], g.power[1], g.power[2], g.saturation], 8);
  out.set([s.agxPower, s.agxSaturation, g.monochrome, s.vignette], 12);
  out.set([s.grain, time % 1000, bloomMips, bloomMips > 0 ? 1 : 0], 16);
  // The kept colour is compared in linear light.
  const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4));
  out.set([lin(g.monoTint[0]), lin(g.monoTint[1]), lin(g.monoTint[2]), g.monoKeep], 20);
  return out;
}

/**
 * Sizes of the bloom mips for a `width` x `height` scene: the first is half
 * the scene, each next one half again (rounded up), at most `mips` levels,
 * stopping before a side would drop below 2 texels.
 */
export function bloomMipSizes(width: number, height: number, mips: number): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  let w = width;
  let h = height;
  for (let i = 0; i < mips; i++) {
    w = Math.ceil(w / 2);
    h = Math.ceil(h / 2);
    if (w < 2 || h < 2) break;
    out.push([w, h]);
  }
  return out;
}

interface BloomLevel {
  texture: GPUTexture;
  view: GPUTextureView;
  width: number;
  height: number;
  /** Bind group reading the previous level (or the scene) for the downsample into this one. */
  down: GPUBindGroup;
  /** Bind group reading this level for the upsample into the previous one (null for level 0). */
  up: GPUBindGroup | null;
  uniforms: GPUBuffer[];
}

export class PostChain {
  private readonly device: GPUDevice;
  private readonly sampler: GPUSampler;
  private readonly bloomLayout: GPUBindGroupLayout;
  private readonly compositeLayout: GPUBindGroupLayout;
  private readonly downPipeline: GPURenderPipeline;
  private readonly upPipeline: GPURenderPipeline;
  private readonly compositePipeline: GPURenderPipeline;
  private readonly postBuffer: GPUBuffer;
  private readonly postData = new Float32Array(POST_UNIFORM_FLOATS);
  private readonly black: GPUTexture;
  private sceneTexture: GPUTexture | null = null;
  private levels: BloomLevel[] = [];
  private composite: GPUBindGroup | null = null;
  private size = { width: 0, height: 0, mips: -1 };

  constructor(device: GPUDevice, outputFormat: GPUTextureFormat) {
    this.device = device;
    this.sampler = device.createSampler({
      label: "post linear clamp",
      magFilter: "linear",
      minFilter: "linear",
      addressModeU: "clamp-to-edge",
      addressModeV: "clamp-to-edge",
    });
    const tex = { sampleType: "float" } as const;
    this.bloomLayout = device.createBindGroupLayout({
      label: "bloom",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: tex },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
      ],
    });
    this.compositeLayout = device.createBindGroupLayout({
      label: "composite",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "unfilterable-float" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: tex },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
        { binding: 3, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
      ],
    });
    const fullscreen = (label: string, code: string, layout: GPUBindGroupLayout, target: GPUColorTargetState): GPURenderPipeline => {
      const module = device.createShaderModule({ label, code });
      return device.createRenderPipeline({
        label,
        layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }),
        vertex: { module, entryPoint: "vs_main" },
        fragment: { module, entryPoint: "fs_main", targets: [target] },
        primitive: { topology: "triangle-list" },
      });
    };
    this.downPipeline = fullscreen("bloom down", BLOOM_DOWN_WGSL, this.bloomLayout, { format: HDR_FORMAT });
    this.upPipeline = fullscreen("bloom up", BLOOM_UP_WGSL, this.bloomLayout, {
      format: HDR_FORMAT,
      blend: {
        color: { srcFactor: "one", dstFactor: "one", operation: "add" },
        alpha: { srcFactor: "one", dstFactor: "one", operation: "add" },
      },
    });
    this.compositePipeline = fullscreen("composite", COMPOSITE_WGSL, this.compositeLayout, { format: outputFormat });
    this.postBuffer = device.createBuffer({
      label: "post uniforms",
      size: POST_UNIFORM_FLOATS * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    // Stands in for the bloom chain when bloom is off.
    this.black = device.createTexture({
      label: "no bloom",
      size: { width: 1, height: 1 },
      format: HDR_FORMAT,
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.RENDER_ATTACHMENT,
    });
  }

  /** The single-sample HDR scene texture (MSAA resolve target, or the scene target without MSAA). */
  get sceneView(): GPUTextureView {
    if (!this.sceneTexture) throw new Error("PostChain.resize was not called");
    return this.sceneTexture.createView();
  }

  /** The single-sample HDR scene texture (VQ7's light shafts add onto it). */
  get sceneHdr(): GPUTexture {
    if (!this.sceneTexture) throw new Error("PostChain.resize was not called");
    return this.sceneTexture;
  }

  get bloomLevels(): number {
    return this.levels.length;
  }

  /** (Re)creates the scene texture and the bloom chain for a canvas size and mip count. */
  resize(width: number, height: number, mips: number): void {
    if (this.size.width === width && this.size.height === height && this.size.mips === mips && this.sceneTexture) return;
    this.size = { width, height, mips };
    this.sceneTexture?.destroy();
    for (const l of this.levels) {
      l.texture.destroy();
      for (const b of l.uniforms) b.destroy();
    }
    const device = this.device;
    this.sceneTexture = device.createTexture({
      label: "hdr scene",
      size: { width, height },
      format: HDR_FORMAT,
      // COPY_SRC: the cloak refraction copies it (cloak.ts).
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_SRC,
    });
    const sceneView = this.sceneTexture.createView();
    const uniform = (texel: [number, number], mode: number): GPUBuffer => {
      const b = device.createBuffer({ label: "bloom pass", size: BLOOM_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
      device.queue.writeBuffer(b, 0, new Float32Array([texel[0], texel[1], mode, 0]));
      return b;
    };
    const group = (view: GPUTextureView, buffer: GPUBuffer): GPUBindGroup =>
      device.createBindGroup({
        layout: this.bloomLayout,
        entries: [
          { binding: 0, resource: view },
          { binding: 1, resource: this.sampler },
          { binding: 2, resource: { buffer } },
        ],
      });
    this.levels = [];
    let srcView = sceneView;
    let srcSize: [number, number] = [width, height];
    for (const [w, h] of bloomMipSizes(width, height, mips)) {
      const texture = device.createTexture({
        label: `bloom ${this.levels.length}`,
        size: { width: w, height: h },
        format: HDR_FORMAT,
        usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
      });
      const view = texture.createView();
      const downU = uniform([1 / srcSize[0], 1 / srcSize[1]], this.levels.length === 0 ? 1 : 0);
      const upU = uniform([1 / w, 1 / h], 0);
      this.levels.push({
        texture,
        view,
        width: w,
        height: h,
        down: group(srcView, downU),
        up: this.levels.length === 0 ? null : group(view, upU),
        uniforms: [downU, upU],
      });
      srcView = view;
      srcSize = [w, h];
    }
    this.composite = device.createBindGroup({
      layout: this.compositeLayout,
      entries: [
        { binding: 0, resource: sceneView },
        { binding: 1, resource: (this.levels[0]?.texture ?? this.black).createView() },
        { binding: 2, resource: this.sampler },
        { binding: 3, resource: { buffer: this.postBuffer } },
      ],
    });
  }

  /** Bloom chain plus composite into `target` (timed as "post"). */
  encode(encoder: GPUCommandEncoder, target: GPUTextureView, settings: PostSettings, time: number, timer: GpuTimer | null): void {
    if (!this.composite) throw new Error("PostChain.resize was not called");
    this.device.queue.writeBuffer(this.postBuffer, 0, packPostUniforms(settings, time, this.levels.length, this.postData));
    const first = this.levels[0];
    if (first && first.uniforms[0]) {
      this.device.queue.writeBuffer(first.uniforms[0], 0, new Float32Array([1 / this.size.width, 1 / this.size.height, 1, Math.max(0, settings.bloomThreshold)]));
    }
    const pass = (label: string, view: GPUTextureView, load: boolean): GPURenderPassEncoder => {
      const timestampWrites = timer?.pass("post");
      return encoder.beginRenderPass({
        label,
        colorAttachments: [{ view, loadOp: load ? "load" : "clear", clearValue: { r: 0, g: 0, b: 0, a: 1 }, storeOp: "store" }],
        ...(timestampWrites ? { timestampWrites } : {}),
      });
    };
    const levels = this.levels;
    for (const l of levels) {
      const p = pass("bloom down", l.view, false);
      p.setPipeline(this.downPipeline);
      p.setBindGroup(0, l.down);
      p.draw(3);
      p.end();
    }
    for (let i = levels.length - 1; i > 0; i--) {
      const src = levels[i];
      const dst = levels[i - 1];
      if (!src?.up || !dst) continue;
      const p = pass("bloom up", dst.view, true);
      p.setPipeline(this.upPipeline);
      p.setBindGroup(0, src.up);
      p.draw(3);
      p.end();
    }
    const p = pass("composite", target, false);
    p.setPipeline(this.compositePipeline);
    p.setBindGroup(0, this.composite);
    p.draw(3);
    p.end();
  }

  destroy(): void {
    this.sceneTexture?.destroy();
    for (const l of this.levels) {
      l.texture.destroy();
      for (const b of l.uniforms) b.destroy();
    }
    this.black.destroy();
    this.postBuffer.destroy();
  }
}
