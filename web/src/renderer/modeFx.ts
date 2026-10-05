// Mode effects pass (M3 C1; shaders/modeFx.ts): The Squeeze's glowing walls
// and next zone, Heist's extraction zones, core beacon and carrier halo,
// Hot Reactor's fuse sphere and holder halo, drawn analytically per pixel
// against the scene depth. The game lists this frame's volumes
// (`Renderer.setModeVolumes`, built by game/modeVisuals.ts); with none the
// pass is skipped.

import type { Vec3 } from "../math/vec3.ts";
import * as q from "../math/quat.ts";
import type { GpuTimer } from "./gpuTimer.ts";
import { HDR_FORMAT } from "./post.ts";
import type { CameraView, Rgb } from "./renderer.ts";
import { srgbToLinear } from "./shaders/frame.ts";
import {
  MODE_FX_HEADER_FLOATS,
  MODE_FX_KIND,
  MODE_FX_MAX_VOLUMES,
  MODE_FX_UNIFORM_FLOATS,
  MODE_FX_VOLUME_FLOATS,
  modeFxWgsl,
} from "./shaders/modeFx.ts";

/** The Squeeze's shell: rock at the sphere glows; `warn` and `grind` 0..1. */
export interface WallVolume {
  kind: "wall";
  center: Vec3;
  radius: number;
  /** Display colour of the warning glow (grinding shifts it to amber). */
  color: Rgb;
  intensity: number;
  /** 0..1: the walls are about to move (pulsing glow). */
  warn: number;
  /** 0..1: the walls are moving (amber, molten cracks, faster flicker). */
  grind: number;
  /** Half width of the glowing band on the rock (u); default 0.9. */
  band?: number;
}

/** A holographic sphere with a contour on the rock (zones, the fuse radius). */
export interface ZoneVolume {
  kind: "zone";
  center: Vec3;
  radius: number;
  color: Rgb;
  intensity: number;
  /** Fresnel film strength where the view crosses the sphere in open space. */
  film: number;
  /** Grid line strength on the film. */
  grid: number;
  /** Contour strength where the sphere cuts the rock. */
  contour: number;
  /** Contour half width (u); default 0.3. */
  contourWidth?: number;
  /** Equator ring lit up to this fraction (0..1); absent = no ring. */
  ring?: number;
  /** Pulse rate (Hz); 0 / absent = steady. */
  pulse?: number;
  /** Speed of the contour's dashes (u/s); default 1. */
  dashSpeed?: number;
}

/** A vertical pillar of light rising from `center`. */
export interface BeamVolume {
  kind: "beam";
  center: Vec3;
  /** Glow width (u). */
  width: number;
  height: number;
  color: Rgb;
  intensity: number;
  /** Bright core strength; default 1. */
  core?: number;
}

/** A rim-lit bubble around a ship. */
export interface HaloVolume {
  kind: "halo";
  center: Vec3;
  radius: number;
  color: Rgb;
  intensity: number;
  /** Rim falloff power; default 3. */
  power?: number;
}

export type ModeVolume = WallVolume | ZoneVolume | BeamVolume | HaloVolume;

const lin = (c: Rgb): [number, number, number] => [srgbToLinear(c[0]), srgbToLinear(c[1]), srgbToLinear(c[2])];

/**
 * Packs the volumes (at most MODE_FX_MAX_VOLUMES, in order) after the
 * header; returns how many were written.
 */
export function packVolumes(volumes: readonly ModeVolume[], out: Float32Array, offset = MODE_FX_HEADER_FLOATS): number {
  let n = 0;
  for (const v of volumes) {
    if (n >= MODE_FX_MAX_VOLUMES) break;
    const o = offset + n * MODE_FX_VOLUME_FLOATS;
    const [r, g, b] = lin(v.color);
    out.fill(0, o, o + MODE_FX_VOLUME_FLOATS);
    if (v.kind === "wall") {
      out.set([v.center.x, v.center.y, v.center.z, v.radius, r, g, b, v.intensity, MODE_FX_KIND.wall, v.warn, v.grind, v.band ?? 0.9], o);
    } else if (v.kind === "zone") {
      out.set([v.center.x, v.center.y, v.center.z, v.radius, r, g, b, v.intensity, MODE_FX_KIND.zone, v.film, v.grid, v.contour], o);
      out.set([v.contourWidth ?? 0.3, v.ring ?? -1, v.pulse ?? 0, v.dashSpeed ?? 1], o + 12);
    } else if (v.kind === "beam") {
      out.set([v.center.x, v.center.y, v.center.z, v.width, r, g, b, v.intensity, MODE_FX_KIND.beam, v.height, v.core ?? 1, 0], o);
    } else {
      out.set([v.center.x, v.center.y, v.center.z, v.radius, r, g, b, v.intensity, MODE_FX_KIND.halo, v.power ?? 3, 0, 0], o);
    }
    n++;
  }
  return n;
}

/** Writes the uniform header: camera basis scaled by the half-FOV tangents, fog, target size. */
export function packHeader(
  camera: CameraView,
  aspect: number,
  time: number,
  fogDensity: number,
  count: number,
  size: { width: number; height: number },
  depthScale: number,
  out: Float32Array,
): void {
  const tanY = Math.tan(camera.fovY / 2);
  const tanX = tanY * aspect;
  const r = q.right(camera.orientation);
  const u = q.up(camera.orientation);
  const f = q.forward(camera.orientation);
  out.set([camera.position.x, camera.position.y, camera.position.z, time % 1000], 0);
  out.set([r.x * tanX, r.y * tanX, r.z * tanX, camera.near], 4);
  out.set([u.x * tanY, u.y * tanY, u.z * tanY, camera.far], 8);
  out.set([f.x, f.y, f.z, fogDensity], 12);
  out.set([count, size.width, size.height, depthScale], 16);
}

/** The GPU side: one fullscreen pipeline per depth sample count, one uniform buffer. */
export class ModeFxPass {
  private readonly device: GPUDevice;
  private readonly uniform: GPUBuffer;
  private readonly data = new Float32Array(MODE_FX_UNIFORM_FLOATS);
  private readonly pipelines = new Map<boolean, { pipeline: GPURenderPipeline; layout: GPUBindGroupLayout }>();
  private bind: { group: GPUBindGroup; depth: GPUTexture } | null = null;

  constructor(device: GPUDevice) {
    this.device = device;
    this.uniform = device.createBuffer({
      label: "mode fx uniforms",
      size: MODE_FX_UNIFORM_FLOATS * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
  }

  private pipelineFor(ms: boolean): { pipeline: GPURenderPipeline; layout: GPUBindGroupLayout } {
    const cached = this.pipelines.get(ms);
    if (cached) return cached;
    const device = this.device;
    const layout = device.createBindGroupLayout({
      label: `mode fx${ms ? " msaa" : ""}`,
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "depth", viewDimension: "2d", multisampled: ms } },
      ],
    });
    const module = device.createShaderModule({ label: "mode fx", code: modeFxWgsl(ms) });
    const pipeline = device.createRenderPipeline({
      label: "mode fx",
      layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }),
      vertex: { module, entryPoint: "vs_main" },
      fragment: {
        module,
        entryPoint: "fs_main",
        targets: [
          {
            format: HDR_FORMAT,
            // Additive light; the scene's alpha (sky mask, colour keep mask) stays.
            blend: {
              color: { srcFactor: "one", dstFactor: "one", operation: "add" },
              alpha: { srcFactor: "zero", dstFactor: "one", operation: "add" },
            },
          },
        ],
      },
      primitive: { topology: "triangle-list" },
    });
    const entry = { pipeline, layout };
    this.pipelines.set(ms, entry);
    return entry;
  }

  /** Draws `volumes` onto `target` (the resolved HDR scene), reading `depth`. */
  encode(
    encoder: GPUCommandEncoder,
    target: GPUTextureView,
    depth: GPUTexture,
    camera: CameraView,
    size: { width: number; height: number },
    volumes: readonly ModeVolume[],
    time: number,
    fogDensity: number,
    timer: GpuTimer | null,
  ): void {
    if (volumes.length === 0) return;
    const ms = depth.sampleCount > 1;
    const { pipeline, layout } = this.pipelineFor(ms);
    const count = packVolumes(volumes, this.data);
    packHeader(camera, size.width / Math.max(1, size.height), time, fogDensity, count, size, depth.width / Math.max(1, size.width), this.data);
    this.device.queue.writeBuffer(this.uniform, 0, this.data);
    if (!this.bind || this.bind.depth !== depth) {
      this.bind = {
        depth,
        group: this.device.createBindGroup({
          label: "mode fx",
          layout,
          entries: [
            { binding: 0, resource: { buffer: this.uniform } },
            { binding: 1, resource: depth.createView({ aspect: "depth-only" }) },
          ],
        }),
      };
    }
    const timestampWrites = timer?.pass("mode");
    const pass = encoder.beginRenderPass({
      label: "mode fx",
      ...(timestampWrites ? { timestampWrites } : {}),
      colorAttachments: [{ view: target, loadOp: "load", storeOp: "store" }],
    });
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, this.bind.group);
    pass.draw(3);
    pass.end();
  }

  destroy(): void {
    this.uniform.destroy();
  }
}
