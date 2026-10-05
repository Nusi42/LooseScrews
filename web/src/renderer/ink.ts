// Ink look (M4 A7, milestone-4-plan.md §3.3 "Palette"): the Palette slider
// turns the world from full colour into black-and-white ink.
//
//   0-49    the colour grade's saturation falls to 50 % (no ink)
//   50-79   ink fades in over the colour: outlines from depth, toon-banded
//           light, paper grain
//   80-100  a monochrome level: black ink on paper, hatching from light and
//           AO, an ink-wash sky and mist; more contrast toward 100
//
// Paint and pigment keep their colour in every level: the scene's alpha
// carries a keep mask (objects.ts `Instance.keepColor`, the terrain's paint
// coverage; shaders/post.ts KEEP_ALPHA), which removes the monochrome and the
// hatching (not the outlines) from those pixels.
//
// `inkForPalette` is pure data (Node tests: ink.test.ts). `InkPass` draws the
// line work (outlines and hatching) into an rgba8 target from the scene depth
// and colour (shaders/ink.ts); the composite (shaders/post.ts) applies the
// tone, paper, grain and lines. Below Palette 50 nothing of this runs.

import * as q from "../math/quat.ts";
import type { GpuTimer } from "./gpuTimer.ts";
import type { CameraView, Rgb } from "./renderer.ts";
import { INK_UNIFORM_FLOATS, inkWgsl } from "./shaders/ink.ts";

/** Palette from which ink fades in. */
export const INK_PALETTE = 50;
/** Palette from which a level is monochrome (ink on paper, pigment pickups; `MatchSetup::monochrome`). */
export const MONOCHROME_PALETTE = 80;

/** Paper (display colour): warm off-white. */
export const PAPER: Rgb = [0.93, 0.9, 0.83];
/** Ink (display colour): blue-black. */
export const INK: Rgb = [0.045, 0.045, 0.06];

export interface InkLook {
  /** 0..1 outline strength (depth creases and silhouettes). */
  outline: number;
  /** 0..1 toon banding of the light (soft steps in display lightness). */
  toon: number;
  /** 0..1 paper grain. */
  grain: number;
  /** 0..1 hatching strength (monochrome levels). */
  hatch: number;
  /** 0..1 blend from the colour image to ink on paper. */
  mono: number;
  /** 0..1 contrast of the monochrome tone (more paper and solid ink toward 1). */
  contrast: number;
  paper: Rgb;
  ink: Rgb;
}

export const NO_INK: InkLook = { outline: 0, toon: 0, grain: 0, hatch: 0, mono: 0, contrast: 0, paper: PAPER, ink: INK };

const clamp01 = (x: number): number => Math.min(Math.max(x, 0), 1);
const smooth = (t: number): number => {
  const c = clamp01(t);
  return c * c * (3 - 2 * c);
};

/** True when a Palette value makes a monochrome level (§3.4). */
export function isMonochromePalette(palette: number): boolean {
  return palette >= MONOCHROME_PALETTE;
}

/**
 * Saturation factor of the world's grade at a Palette value: 1 at 0, 0.5 at
 * 49, then down to 0.4 while ink fades in (79); the monochrome levels take the
 * colour out through `InkLook.mono` and keep 0.4 for kept colours.
 */
export function paletteSaturation(palette: number): number {
  const p = Math.min(Math.max(palette, 0), 100);
  if (p < INK_PALETTE) return 1 - (0.5 * p) / (INK_PALETTE - 1);
  if (p < MONOCHROME_PALETTE) return 0.5 - (0.1 * (p - INK_PALETTE)) / (MONOCHROME_PALETTE - 1 - INK_PALETTE);
  return 0.4;
}

/** The ink look at a Palette value (0..100). */
export function inkForPalette(palette: number): InkLook {
  const p = Math.min(Math.max(palette, 0), 100);
  if (p < INK_PALETTE) return NO_INK;
  if (p < MONOCHROME_PALETTE) {
    const t = (p - INK_PALETTE) / (MONOCHROME_PALETTE - 1 - INK_PALETTE);
    return { ...NO_INK, outline: 0.35 + 0.55 * smooth(t), toon: 0.3 + 0.6 * t, grain: 0.25 + 0.35 * t };
  }
  const t = (p - MONOCHROME_PALETTE) / (100 - MONOCHROME_PALETTE);
  return { ...NO_INK, outline: 1, toon: 1, grain: 1, hatch: 0.75 + 0.25 * t, mono: 1, contrast: t };
}

/** True when the composite applies any of the look. */
export function inkActive(look: InkLook): boolean {
  return look.outline > 0 || look.hatch > 0 || look.toon > 0 || look.grain > 0 || look.mono > 0;
}

/** True when the line pass (outlines, hatching) has to run. */
export function needsLinePass(look: InkLook): boolean {
  return look.outline > 0 || look.hatch > 0;
}

/** Hatch line spacing at 1080p (px; scaled with the height). */
export const HATCH_SPACING_PX = 6;

/** Line scale for a target height: 1 at 1080p and below, thicker on bigger targets. */
export function lineScale(height: number): number {
  return Math.max(1, height / 1080);
}

/** Packs the ink pass uniform (struct Ink in shaders/ink.ts). */
export function packInkUniforms(
  camera: CameraView,
  aspect: number,
  size: { width: number; height: number },
  look: InkLook,
  exposure: number,
  fogDensity: number,
  out = new Float32Array(INK_UNIFORM_FLOATS),
): Float32Array {
  const f = q.forward(camera.orientation);
  const r = q.right(camera.orientation);
  const u = q.up(camera.orientation);
  const tanY = Math.tan(camera.fovY / 2);
  const tanX = tanY * aspect;
  out.set([camera.position.x, camera.position.y, camera.position.z, camera.near], 0);
  out.set([f.x, f.y, f.z, camera.far], 4);
  out.set([r.x * tanX, r.y * tanX, r.z * tanX, (2 * tanY) / Math.max(size.height, 1)], 8);
  out.set([u.x * tanY, u.y * tanY, u.z * tanY, lineScale(size.height)], 12);
  out.set([look.outline, look.hatch, exposure, fogDensity], 16);
  out.set([size.width, size.height, HATCH_SPACING_PX, tanY], 20);
  return out;
}

/** The line pass: outlines and hatching into an rgba8 target the composite reads (timed as "ink"). */
export class InkPass {
  private readonly device: GPUDevice;
  private readonly uniform: GPUBuffer;
  private readonly data = new Float32Array(INK_UNIFORM_FLOATS);
  private readonly pipelines = new Map<number, { layout: GPUBindGroupLayout; pipeline: GPURenderPipeline }>();
  private target: GPUTexture | null = null;
  private bind: { group: GPUBindGroup; depth: GPUTexture; scene: GPUTexture; target: GPUTexture } | null = null;

  constructor(device: GPUDevice) {
    this.device = device;
    this.uniform = device.createBuffer({ label: "ink", size: INK_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
  }

  private pipelineFor(samples: number): { layout: GPUBindGroupLayout; pipeline: GPURenderPipeline } {
    const cached = this.pipelines.get(samples);
    if (cached) return cached;
    const device = this.device;
    const layout = device.createBindGroupLayout({
      label: "ink",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "depth", multisampled: samples > 1 } },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "unfilterable-float" } },
      ],
    });
    const module = device.createShaderModule({ label: "ink", code: inkWgsl(samples > 1) });
    const pipeline = device.createRenderPipeline({
      label: "ink",
      layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }),
      vertex: { module, entryPoint: "vs_main" },
      fragment: { module, entryPoint: "fs_main", targets: [{ format: "rgba8unorm" }] },
      primitive: { topology: "triangle-list" },
    });
    const entry = { layout, pipeline };
    this.pipelines.set(samples, entry);
    return entry;
  }

  /**
   * Draws the line work of `look` for the finished HDR `scene` (single
   * sampled) and its `depth`; returns the target for the composite.
   */
  encode(
    encoder: GPUCommandEncoder,
    scene: GPUTexture,
    depth: GPUTexture,
    camera: CameraView,
    aspect: number,
    look: InkLook,
    exposure: number,
    fogDensity: number,
    timer: GpuTimer | null,
  ): GPUTexture {
    const size = { width: scene.width, height: scene.height };
    if (!this.target || this.target.width !== size.width || this.target.height !== size.height) {
      this.target?.destroy();
      this.target = this.device.createTexture({
        label: "ink lines",
        size,
        format: "rgba8unorm",
        usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
      });
    }
    const target = this.target;
    this.device.queue.writeBuffer(this.uniform, 0, packInkUniforms(camera, aspect, size, look, exposure, fogDensity, this.data));
    const { layout, pipeline } = this.pipelineFor(depth.sampleCount);
    if (!this.bind || this.bind.depth !== depth || this.bind.scene !== scene || this.bind.target !== target) {
      this.bind = {
        depth,
        scene,
        target,
        group: this.device.createBindGroup({
          layout,
          entries: [
            { binding: 0, resource: { buffer: this.uniform } },
            { binding: 1, resource: depth.createView({ aspect: "depth-only" }) },
            { binding: 2, resource: scene.createView() },
          ],
        }),
      };
    }
    const tw = timer?.pass("ink");
    const pass = encoder.beginRenderPass({
      label: "ink",
      ...(tw ? { timestampWrites: tw } : {}),
      colorAttachments: [{ view: target.createView(), clearValue: { r: 0, g: 0, b: 0, a: 0 }, loadOp: "clear", storeOp: "store" }],
    });
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, this.bind.group);
    pass.draw(3);
    pass.end();
    return target;
  }

  destroy(): void {
    this.target?.destroy();
    this.uniform.destroy();
  }
}
