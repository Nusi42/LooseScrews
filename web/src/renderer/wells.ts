// World effects pass (M4 B5; shaders/wells.ts): gravity wells (lensing,
// glow, the core stone), cave-in warnings (glowing cracks) and rubble (a
// dust coat), drawn per pixel against the scene depth. The game lists this
// frame's volumes (`Renderer.setWorldVolumes`, built by game/worldFx.ts);
// with none on screen the pass is skipped. Only the screen rectangle the
// volumes cover is copied and drawn (scissored), so a distant well costs a
// small rectangle, not the whole frame. Timed as "wells".

import * as mat4 from "../math/mat4.ts";
import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { GpuTimer } from "./gpuTimer.ts";
import { HDR_FORMAT } from "./post.ts";
import type { CameraView, Rgb } from "./renderer.ts";
import { srgbToLinear } from "./shaders/frame.ts";
import { WELLS_HEADER_FLOATS, WELLS_KIND, WELLS_MAX_VOLUMES, WELLS_UNIFORM_FLOATS, WELLS_VOLUME_FLOATS, wellsWgsl } from "./shaders/wells.ts";

/** A gravity well's look. */
export interface WellVolume {
  kind: "well";
  center: Vec3;
  /** Radius of the core stone (u); 0 = no stone (dynamic wells: the Paperweight, an Architect stroke). */
  core: number;
  /** Lens radius E (u): the photon ring's radius; 0 = no lensing. */
  lens: number;
  /** How far the lensing and the glow reach (u). */
  reach: number;
  /** Width of the glow cloud (u). */
  glow: number;
  /** Display colour of the glow and the veins. */
  color: Rgb;
  /** HDR multiplier of the glow (0 = dark: the coffee break). */
  intensity: number;
  /** Spin axis of the swirl (unit). */
  axis: Vec3;
  /** Phase (rad): wells do not swirl in lockstep. */
  phase: number;
}

/** A cave-in's warning: glowing cracks in the rock around `center`. */
export interface CrackVolume {
  kind: "crack";
  center: Vec3;
  radius: number;
  /** 0..1 through the warning. */
  progress: number;
  color: Rgb;
  intensity: number;
  /** The level's up (the ceiling faces down along it). */
  up: Vec3;
  seed: number;
}

/** A cave-in's rubble: a dust coat on the rock around `center`. */
export interface RubbleVolume {
  kind: "rubble";
  center: Vec3;
  radius: number;
  /** 1 just after the collapse, fading to 0 (a light coat stays). */
  fresh: number;
  color: Rgb;
  up: Vec3;
  seed: number;
}

export type WorldVolume = WellVolume | CrackVolume | RubbleVolume;

const lin = (c: Rgb): [number, number, number] => [srgbToLinear(c[0]), srgbToLinear(c[1]), srgbToLinear(c[2])];

/** Extent (u) of a volume's effect around its centre. */
export function volumeReach(v: WorldVolume): number {
  if (v.kind === "well") return Math.max(v.reach, v.glow * 3, v.core + 0.5);
  return v.radius;
}

/** Packs at most WELLS_MAX_VOLUMES volumes after the header; returns how many were written. */
export function packWorldVolumes(volumes: readonly WorldVolume[], out: Float32Array, offset = WELLS_HEADER_FLOATS): number {
  let n = 0;
  for (const v of volumes) {
    if (n >= WELLS_MAX_VOLUMES) break;
    const o = offset + n * WELLS_VOLUME_FLOATS;
    const [r, g, b] = lin(v.color);
    out.fill(0, o, o + WELLS_VOLUME_FLOATS);
    if (v.kind === "well") {
      out.set([v.center.x, v.center.y, v.center.z, volumeReach(v), r, g, b, v.intensity, WELLS_KIND.well, v.core, v.lens, v.glow, v.axis.x, v.axis.y, v.axis.z, v.phase], o);
    } else if (v.kind === "crack") {
      out.set([v.center.x, v.center.y, v.center.z, v.radius, r, g, b, v.intensity, WELLS_KIND.crack, v.progress, v.seed, 0, v.up.x, v.up.y, v.up.z, 0], o);
    } else {
      out.set([v.center.x, v.center.y, v.center.z, v.radius, r, g, b, 1, WELLS_KIND.rubble, v.fresh, v.seed, 0, v.up.x, v.up.y, v.up.z, 0], o);
    }
    n++;
  }
  return n;
}

/** Pixel rectangle [x0, y0, x1, y1). */
export type PixelRect = [number, number, number, number];

/**
 * Screen rectangle (pixels of a `width` x `height` target) a sphere covers:
 * the projected corners of its bounding box, padded by 2 px; the whole
 * target when the camera is inside or a corner lies behind it; null off
 * screen.
 */
export function sphereRect(viewProj: mat4.Mat4, center: Vec3, radius: number, eye: Vec3, width: number, height: number): PixelRect | null {
  const full: PixelRect = [0, 0, width, height];
  if (Math.hypot(center.x - eye.x, center.y - eye.y, center.z - eye.z) < radius * 1.05) return full;
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (let i = 0; i < 8; i++) {
    const c = {
      x: center.x + (i & 1 ? radius : -radius),
      y: center.y + (i & 2 ? radius : -radius),
      z: center.z + (i & 4 ? radius : -radius),
    };
    const [x, y, , w] = mat4.transformPoint(viewProj, c);
    if (w <= 1e-3) return full;
    const px = (x / w * 0.5 + 0.5) * width;
    const py = (0.5 - y / w * 0.5) * height;
    x0 = Math.min(x0, px); x1 = Math.max(x1, px);
    y0 = Math.min(y0, py); y1 = Math.max(y1, py);
  }
  const r: PixelRect = [Math.max(0, Math.floor(x0) - 2), Math.max(0, Math.floor(y0) - 2), Math.min(width, Math.ceil(x1) + 2), Math.min(height, Math.ceil(y1) + 2)];
  return r[2] > r[0] && r[3] > r[1] ? r : null;
}

/** Union of the rectangles of every volume, or null when none is on screen. */
export function volumesRect(volumes: readonly WorldVolume[], viewProj: mat4.Mat4, eye: Vec3, width: number, height: number): PixelRect | null {
  let out: PixelRect | null = null;
  for (const v of volumes.slice(0, WELLS_MAX_VOLUMES)) {
    const r = sphereRect(viewProj, v.center, volumeReach(v), eye, width, height);
    if (!r) continue;
    out = out ? [Math.min(out[0], r[0]), Math.min(out[1], r[1]), Math.max(out[2], r[2]), Math.max(out[3], r[3])] : r;
  }
  return out;
}

/** The GPU side: a scene copy, one fullscreen pipeline per depth sample count, one uniform buffer. */
export class WellsPass {
  private readonly device: GPUDevice;
  private readonly uniform: GPUBuffer;
  private readonly data = new Float32Array(WELLS_UNIFORM_FLOATS);
  private readonly sampler: GPUSampler;
  private readonly pipelines = new Map<string, GPURenderPipeline>();
  private readonly layouts = new Map<boolean, GPUBindGroupLayout>();
  private readonly modules = new Map<boolean, GPUShaderModule>();
  private copy: GPUTexture | null = null;
  private bind: { group: GPUBindGroup; depth: GPUTexture; copy: GPUTexture } | null = null;
  /** The rectangle drawn in the last frame (null: skipped), for tests and the gallery. */
  lastRect: PixelRect | null = null;

  constructor(device: GPUDevice) {
    this.device = device;
    this.uniform = device.createBuffer({ label: "wells uniforms", size: WELLS_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    this.sampler = device.createSampler({ label: "wells", magFilter: "linear", minFilter: "linear", addressModeU: "clamp-to-edge", addressModeV: "clamp-to-edge" });
  }

  private layoutFor(ms: boolean): GPUBindGroupLayout {
    let layout = this.layouts.get(ms);
    if (!layout) {
      layout = this.device.createBindGroupLayout({
        label: `wells${ms ? " msaa" : ""}`,
        entries: [
          { binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
          { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "depth", viewDimension: "2d", multisampled: ms } },
          { binding: 2, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float" } },
          { binding: 3, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
        ],
      });
      this.layouts.set(ms, layout);
    }
    return layout;
  }

  /** The pipeline for the depth sample count and the volume kinds present (unused branches compile away). */
  private pipelineFor(ms: boolean, wells: boolean, surface: boolean): GPURenderPipeline {
    const key = `${ms ? 1 : 0}${wells ? 1 : 0}${surface ? 1 : 0}`;
    const cached = this.pipelines.get(key);
    if (cached) return cached;
    const device = this.device;
    let module = this.modules.get(ms);
    if (!module) {
      module = device.createShaderModule({ label: "wells", code: wellsWgsl(ms) });
      this.modules.set(ms, module);
    }
    const pipeline = device.createRenderPipeline({
      label: "world fx",
      layout: device.createPipelineLayout({ bindGroupLayouts: [this.layoutFor(ms)] }),
      vertex: { module, entryPoint: "vs_main" },
      fragment: { module, entryPoint: "fs_main", targets: [{ format: HDR_FORMAT }], constants: { HAS_WELLS: wells ? 1 : 0, HAS_SURFACE: surface ? 1 : 0 } },
      primitive: { topology: "triangle-list" },
    });
    this.pipelines.set(key, pipeline);
    return pipeline;
  }

  /** Draws `volumes` into `scene` (the resolved HDR scene, COPY_SRC), reading `depth`. */
  encode(
    encoder: GPUCommandEncoder,
    scene: GPUTexture,
    depth: GPUTexture,
    camera: CameraView,
    viewProj: mat4.Mat4,
    volumes: readonly WorldVolume[],
    time: number,
    fogDensity: number,
    timer: GpuTimer | null,
  ): void {
    this.lastRect = null;
    if (volumes.length === 0) return;
    const { width, height } = scene;
    const rect = volumesRect(volumes, viewProj, camera.position, width, height);
    if (!rect) return;
    this.lastRect = rect;
    const device = this.device;
    if (!this.copy || this.copy.width !== width || this.copy.height !== height) {
      this.copy?.destroy();
      this.copy = device.createTexture({ label: "wells scene copy", size: { width, height }, format: HDR_FORMAT, usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.TEXTURE_BINDING });
    }
    const ms = depth.sampleCount > 1;
    const count = packWorldVolumes(volumes, this.data);
    const shown = volumes.slice(0, count);
    const pipeline = this.pipelineFor(ms, shown.some((v) => v.kind === "well"), shown.some((v) => v.kind !== "well"));
    const layout = this.layoutFor(ms);
    const tanY = Math.tan(camera.fovY / 2);
    const tanX = tanY * (width / Math.max(1, height));
    const r = q.right(camera.orientation);
    const u = q.up(camera.orientation);
    const f = q.forward(camera.orientation);
    const d = this.data;
    d.set([camera.position.x, camera.position.y, camera.position.z, time % 1000], 0);
    d.set([r.x * tanX, r.y * tanX, r.z * tanX, camera.near], 4);
    d.set([u.x * tanY, u.y * tanY, u.z * tanY, camera.far], 8);
    d.set([f.x, f.y, f.z, fogDensity], 12);
    d.set([count, width, height, depth.width / Math.max(1, width)], 16);
    device.queue.writeBuffer(this.uniform, 0, d);
    if (!this.bind || this.bind.depth !== depth || this.bind.copy !== this.copy) {
      this.bind = {
        depth,
        copy: this.copy,
        group: device.createBindGroup({
          label: "wells",
          layout,
          entries: [
            { binding: 0, resource: { buffer: this.uniform } },
            { binding: 1, resource: depth.createView({ aspect: "depth-only" }) },
            { binding: 2, resource: this.copy.createView() },
            { binding: 3, resource: this.sampler },
          ],
        }),
      };
    }
    const [x0, y0, x1, y1] = rect;
    encoder.copyTextureToTexture({ texture: scene, origin: { x: x0, y: y0 } }, { texture: this.copy, origin: { x: x0, y: y0 } }, { width: x1 - x0, height: y1 - y0 });
    const timestampWrites = timer?.pass("wells");
    const pass = encoder.beginRenderPass({
      label: "world fx",
      ...(timestampWrites ? { timestampWrites } : {}),
      colorAttachments: [{ view: scene.createView(), loadOp: "load", storeOp: "store" }],
    });
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, this.bind.group);
    pass.setScissorRect(x0, y0, x1 - x0, y1 - y0);
    pass.draw(3);
    pass.end();
  }

  destroy(): void {
    this.copy?.destroy();
    this.uniform.destroy();
  }
}
