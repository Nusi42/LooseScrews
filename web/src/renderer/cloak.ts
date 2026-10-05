// Cloak refraction pass (visual-quality plan VQP; shaders/cloak.ts): cloaked
// ships go into the scene's `refract` list (game/scene.ts) and are drawn
// here after the sky and light shafts, before the particles. The pass
// copies the resolved HDR scene, then draws the hulls into it sampling the
// copy through their surface. It costs nothing while no cloaked ship is in
// view (no copy, no pass).

import type { Quat } from "../math/quat.ts";
import * as q from "../math/quat.ts";
import { HDR_FORMAT } from "./post.ts";
import { CLOAK_UNIFORM_FLOATS, cloakWgsl } from "./shaders/cloak.ts";
import type { GpuTimer } from "./gpuTimer.ts";

/** Vertex layouts of the instanced meshes (renderer.ts POSITION/NORMAL/INSTANCE layouts). */
const LAYOUTS: GPUVertexBufferLayout[] = [
  { arrayStride: 12, attributes: [{ shaderLocation: 0, offset: 0, format: "float32x3" }] },
  { arrayStride: 12, attributes: [{ shaderLocation: 1, offset: 0, format: "float32x3" }] },
  {
    arrayStride: 64,
    stepMode: "instance",
    attributes: [
      { shaderLocation: 2, offset: 0, format: "float32x4" },
      { shaderLocation: 3, offset: 16, format: "float32x4" },
      { shaderLocation: 4, offset: 32, format: "float32x4" },
      { shaderLocation: 5, offset: 48, format: "float32x4" },
    ],
  },
];

/** Uniforms: camera right and up (world) and the viewport size and its inverse. */
export function packCloakUniforms(orientation: Quat, width: number, height: number, out = new Float32Array(CLOAK_UNIFORM_FLOATS)): Float32Array {
  const r = q.right(orientation);
  const u = q.up(orientation);
  out.set([r.x, r.y, r.z, 0, u.x, u.y, u.z, 0, width, height, 1 / Math.max(1, width), 1 / Math.max(1, height)]);
  return out;
}

export class CloakPass {
  private readonly device: GPUDevice;
  private readonly frameLayout: GPUBindGroupLayout;
  private readonly uniforms: GPUBuffer;
  private readonly uniformData = new Float32Array(CLOAK_UNIFORM_FLOATS);
  private readonly sampler: GPUSampler;
  private readonly pipelines = new Map<boolean, { pipeline: GPURenderPipeline; layout: GPUBindGroupLayout }>();
  private copy: GPUTexture | null = null;
  private bindGroup: GPUBindGroup | null = null;
  private bindKey: { copy: GPUTexture; depth: GPUTexture } | null = null;

  constructor(device: GPUDevice, frameLayout: GPUBindGroupLayout) {
    this.device = device;
    this.frameLayout = frameLayout;
    this.uniforms = device.createBuffer({ label: "cloak", size: CLOAK_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    this.sampler = device.createSampler({ label: "cloak", magFilter: "linear", minFilter: "linear", addressModeU: "clamp-to-edge", addressModeV: "clamp-to-edge" });
  }

  private pipelineFor(ms: boolean): { pipeline: GPURenderPipeline; layout: GPUBindGroupLayout } {
    const cached = this.pipelines.get(ms);
    if (cached) return cached;
    const device = this.device;
    const layout = device.createBindGroupLayout({
      label: "cloak",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float" } },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
        { binding: 3, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "depth", viewDimension: "2d", multisampled: ms } },
      ],
    });
    const module = device.createShaderModule({ label: "cloak", code: cloakWgsl(ms) });
    const pipeline = device.createRenderPipeline({
      label: "cloak refraction",
      layout: device.createPipelineLayout({ bindGroupLayouts: [this.frameLayout, layout] }),
      vertex: { module, entryPoint: "vs_main", buffers: LAYOUTS },
      fragment: { module, entryPoint: "fs_main", targets: [{ format: HDR_FORMAT }] },
      primitive: { topology: "triangle-list", cullMode: "back", frontFace: "ccw" },
    });
    const entry = { pipeline, layout };
    this.pipelines.set(ms, entry);
    return entry;
  }

  /**
   * Draws the cloaked hulls into `scene` (the resolved HDR scene texture,
   * which needs COPY_SRC) through a copy of it; `draw` issues the instanced
   * draws (the renderer's `refract` batches). Skipped when `count` is 0.
   */
  encode(
    encoder: GPUCommandEncoder,
    scene: GPUTexture,
    depth: GPUTexture,
    frameBindGroup: GPUBindGroup,
    orientation: Quat,
    count: number,
    draw: (pass: GPURenderPassEncoder) => void,
    timer: GpuTimer | null,
  ): void {
    if (count === 0) return;
    const device = this.device;
    const { width, height } = scene;
    if (!this.copy || this.copy.width !== width || this.copy.height !== height) {
      this.copy?.destroy();
      this.copy = device.createTexture({
        label: "cloak scene copy",
        size: { width, height },
        format: HDR_FORMAT,
        usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.TEXTURE_BINDING,
      });
    }
    const ms = depth.sampleCount > 1;
    const { pipeline, layout } = this.pipelineFor(ms);
    if (!this.bindGroup || !this.bindKey || this.bindKey.copy !== this.copy || this.bindKey.depth !== depth) {
      this.bindGroup = device.createBindGroup({
        label: "cloak",
        layout,
        entries: [
          { binding: 0, resource: { buffer: this.uniforms } },
          { binding: 1, resource: this.copy.createView() },
          { binding: 2, resource: this.sampler },
          { binding: 3, resource: depth.createView({ aspect: "depth-only" }) },
        ],
      });
      this.bindKey = { copy: this.copy, depth };
    }
    device.queue.writeBuffer(this.uniforms, 0, packCloakUniforms(orientation, width, height, this.uniformData));
    encoder.copyTextureToTexture({ texture: scene }, { texture: this.copy }, { width, height });
    const timestampWrites = timer?.pass("objects");
    const pass = encoder.beginRenderPass({
      label: "cloak",
      ...(timestampWrites ? { timestampWrites } : {}),
      colorAttachments: [{ view: scene.createView(), loadOp: "load", storeOp: "store" }],
    });
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, frameBindGroup);
    pass.setBindGroup(1, this.bindGroup);
    draw(pass);
    pass.end();
  }

  destroy(): void {
    this.copy?.destroy();
    this.uniforms.destroy();
  }
}
