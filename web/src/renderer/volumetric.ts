// Volumetric sun shafts (VQ9): the air the sun reaches glows faintly toward
// the camera, strongest when looking toward the sun, so an opening that lets
// the sun into a cave shows a shaft of light. Marches the sun shadow map
// (sunShadow.ts) along every view ray at reduced resolution
// (shaders/volumetric.ts), then adds the result onto the resolved HDR scene.
// Timed as "volumetric". Runs after the scene pass (it reads the scene
// depth) and before the sky, the screen-space shafts and the particles.

import * as q from "../math/quat.ts";
import type { CameraView } from "./renderer.ts";
import type { PassTimer } from "./lights.ts";
import { HDR_FORMAT } from "./post.ts";
import type { SunShadow } from "./sunShadow.ts";
import { VOLUMETRIC_COMPOSITE_WGSL, VOLUMETRIC_UNIFORM_FLOATS, volumetricWgsl } from "./shaders/volumetric.ts";

/** Shaft settings of a quality level: resolution divisor (0 = off) and march steps. */
export interface VolumetricQuality {
  divisor: number;
  steps: number;
}

/** Shaft look of a world (atmosphere weights): scattering per u, phase asymmetry, march length (u), strength. */
export interface VolumetricLook {
  scatter: number;
  g: number;
  length: number;
  strength: number;
}

/** Per world type; blended by the atmosphere's weights (renderer.ts `setAtmosphere`). */
export const VOLUMETRIC_LOOKS: Readonly<Record<"sky" | "cave" | "mine", VolumetricLook>> = {
  // Open space: no dust (black space stays black); the screen-space shafts (sky.ts) carry the sun's rays.
  sky: { scatter: 0, g: 0.6, length: 60, strength: 1 },
  // Caverns: dusty air, so light from an opening reads as a beam.
  cave: { scatter: 0.0013, g: 0.6, length: 60, strength: 1 },
  mine: { scatter: 0.0016, g: 0.55, length: 60, strength: 1 },
};

/**
 * The look of a world from its type weights. Dust only fills nearly enclosed
 * worlds: it fades in from an enclosed weight (cave + mine) of DUST_FROM
 * (Enclosure 37) to 1 (Enclosure 50), so open and half-open worlds keep their
 * black space, and caverns with a few openings to the sky get the shafts.
 */
/** Enclosed weight (cave + mine) below which the air holds no dust. */
export const DUST_FROM = 0.75;

export function volumetricLook(w: { sky: number; cave: number; mine: number }): VolumetricLook {
  const b = (k: keyof VolumetricLook): number => w.sky * VOLUMETRIC_LOOKS.sky[k] + w.cave * VOLUMETRIC_LOOKS.cave[k] + w.mine * VOLUMETRIC_LOOKS.mine[k];
  const t = Math.min(1, Math.max(0, (w.cave + w.mine - DUST_FROM) / (1 - DUST_FROM)));
  const fade = t * t * (3 - 2 * t);
  const dusty = w.cave + w.mine > 0 ? (w.cave * VOLUMETRIC_LOOKS.cave.scatter + w.mine * VOLUMETRIC_LOOKS.mine.scatter) / (w.cave + w.mine) : 0;
  return { scatter: dusty * fade, g: b("g"), length: b("length"), strength: b("strength") };
}

/** Sun radiance used by the scene shaders (lighting.ts static_light: 0.9 x (1, 0.93, 0.8)). */
export const SUN_RADIANCE: readonly [number, number, number] = [0.9, 0.837, 0.72];

/** Size of the low-resolution march target for a canvas size and divisor. */
export function marchSize(width: number, height: number, divisor: number): { width: number; height: number } {
  return { width: Math.max(1, Math.ceil(width / divisor)), height: Math.max(1, Math.ceil(height / divisor)) };
}

export class VolumetricShafts {
  private readonly device: GPUDevice;
  private readonly shadow: SunShadow;
  private readonly uniform: GPUBuffer;
  private readonly data = new Float32Array(VOLUMETRIC_UNIFORM_FLOATS);
  private readonly marchLayouts = new Map<number, { layout: GPUBindGroupLayout; pipeline: GPURenderPipeline }>();
  private readonly compositeLayout: GPUBindGroupLayout;
  private readonly compositePipeline: GPURenderPipeline;
  private readonly sampler: GPUSampler;
  private target: GPUTexture | null = null;
  private marchBind: { bind: GPUBindGroup; depth: GPUTexture; target: GPUTexture } | null = null;
  private compositeBind: { bind: GPUBindGroup; target: GPUTexture } | null = null;
  look: VolumetricLook = VOLUMETRIC_LOOKS.cave;

  constructor(device: GPUDevice, shadow: SunShadow) {
    this.device = device;
    this.shadow = shadow;
    this.uniform = device.createBuffer({
      label: "volumetric",
      size: VOLUMETRIC_UNIFORM_FLOATS * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.sampler = device.createSampler({ label: "volumetric", magFilter: "linear", minFilter: "linear", addressModeU: "clamp-to-edge", addressModeV: "clamp-to-edge" });
    this.compositeLayout = device.createBindGroupLayout({
      label: "volumetric composite",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
      ],
    });
    const module = device.createShaderModule({ label: "volumetric composite", code: VOLUMETRIC_COMPOSITE_WGSL });
    this.compositePipeline = device.createRenderPipeline({
      label: "volumetric composite",
      layout: device.createPipelineLayout({ bindGroupLayouts: [this.compositeLayout] }),
      vertex: { module, entryPoint: "vs_full" },
      fragment: {
        module,
        entryPoint: "fs_add",
        targets: [
          {
            format: HDR_FORMAT,
            blend: { color: { srcFactor: "one", dstFactor: "one", operation: "add" }, alpha: { srcFactor: "zero", dstFactor: "one", operation: "add" } },
          },
        ],
      },
      primitive: { topology: "triangle-list" },
    });
  }

  private marchPipeline(samples: number): { layout: GPUBindGroupLayout; pipeline: GPURenderPipeline } {
    const cached = this.marchLayouts.get(samples);
    if (cached) return cached;
    const device = this.device;
    const ms = samples > 1;
    const layout = device.createBindGroupLayout({
      label: "volumetric march",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "depth", multisampled: ms } },
        { binding: 8, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
        { binding: 9, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "depth" } },
        { binding: 10, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "comparison" } },
        { binding: 11, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "depth" } },
      ],
    });
    const module = device.createShaderModule({ label: "volumetric march", code: volumetricWgsl(ms) });
    const pipeline = device.createRenderPipeline({
      label: "volumetric march",
      layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }),
      vertex: { module, entryPoint: "vs_full" },
      fragment: { module, entryPoint: "fs_march", targets: [{ format: HDR_FORMAT }] },
      primitive: { topology: "triangle-list" },
    });
    const entry = { layout, pipeline };
    this.marchLayouts.set(samples, entry);
    return entry;
  }

  /**
   * Marches and adds the shafts onto `scene` (the resolved HDR texture).
   * Skipped when the quality has no shafts, the shadow map is off or the
   * world's air does not scatter.
   */
  encode(
    encoder: GPUCommandEncoder,
    scene: GPUTexture,
    depth: GPUTexture,
    camera: CameraView,
    aspect: number,
    quality: VolumetricQuality,
    timer: PassTimer,
  ): boolean {
    if (quality.divisor <= 0 || !this.shadow.active || this.look.scatter <= 0 || this.look.strength <= 0) return false;
    const size = marchSize(scene.width, scene.height, quality.divisor);
    if (!this.target || this.target.width !== size.width || this.target.height !== size.height) {
      this.target?.destroy();
      this.target = this.device.createTexture({
        label: "volumetric",
        size,
        format: HDR_FORMAT,
        usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
      });
    }
    const target = this.target;
    const look = this.look;
    this.shadow.setVolumetric(look.scatter, look.g, look.length, quality.steps);
    this.writeUniform(camera, aspect, size, scene);

    const { layout, pipeline } = this.marchPipeline(depth.sampleCount);
    if (!this.marchBind || this.marchBind.depth !== depth || this.marchBind.target !== target) {
      this.marchBind = {
        depth,
        target,
        bind: this.device.createBindGroup({
          layout,
          entries: [
            { binding: 0, resource: { buffer: this.uniform } },
            { binding: 1, resource: depth.createView({ aspect: "depth-only" }) },
            ...this.shadow.bindGroupEntries(),
          ],
        }),
      };
    }
    if (!this.compositeBind || this.compositeBind.target !== target) {
      this.compositeBind = {
        target,
        bind: this.device.createBindGroup({
          layout: this.compositeLayout,
          entries: [
            { binding: 0, resource: target.createView() },
            { binding: 1, resource: this.sampler },
          ],
        }),
      };
    }
    const tw = timer?.pass("volumetric");
    const march = encoder.beginRenderPass({
      label: "volumetric march",
      ...(tw ? { timestampWrites: tw } : {}),
      colorAttachments: [{ view: target.createView(), clearValue: { r: 0, g: 0, b: 0, a: 0 }, loadOp: "clear", storeOp: "store" }],
    });
    march.setPipeline(pipeline);
    march.setBindGroup(0, this.marchBind.bind);
    march.draw(3);
    march.end();
    const tw2 = timer?.pass("volumetric");
    const add = encoder.beginRenderPass({
      label: "volumetric composite",
      ...(tw2 ? { timestampWrites: tw2 } : {}),
      colorAttachments: [{ view: scene.createView(), loadOp: "load", storeOp: "store" }],
    });
    add.setPipeline(this.compositePipeline);
    add.setBindGroup(0, this.compositeBind.bind);
    add.draw(3);
    add.end();
    return true;
  }

  private writeUniform(camera: CameraView, aspect: number, size: { width: number; height: number }, scene: GPUTexture): void {
    const d = this.data;
    const f = q.forward(camera.orientation);
    const r = q.right(camera.orientation);
    const u = q.up(camera.orientation);
    const tanY = Math.tan(camera.fovY / 2);
    const tanX = tanY * aspect;
    const sun = this.shadow.sunDirection;
    const s = this.look.strength;
    d.set([camera.position.x, camera.position.y, camera.position.z, camera.near], 0);
    d.set([f.x, f.y, f.z, camera.far], 4);
    d.set([r.x * tanX, r.y * tanX, r.z * tanX, 0], 8);
    d.set([u.x * tanY, u.y * tanY, u.z * tanY, 0], 12);
    d.set([sun.x, sun.y, sun.z, 0], 16);
    d.set([SUN_RADIANCE[0] * s, SUN_RADIANCE[1] * s, SUN_RADIANCE[2] * s, 0], 20);
    d.set([size.width, size.height, scene.width, scene.height], 24);
    // A fixed jitter pattern: without temporal accumulation a moving one would shimmer.
    d.set([0, 0, 0, 0], 28);
    this.device.queue.writeBuffer(this.uniform, 0, d);
  }

  destroy(): void {
    this.target?.destroy();
    this.uniform.destroy();
  }
}
