// Thin WebGPU renderer: world-space terrain chunks (frustum-culled per chunk
// AABB) plus instanced ships and projectiles, all lit and fogged by the shaders
// in shaders.ts.
//
// Wave 3 plugs in by calling `setChunk(key, mesh)` with meshes produced by
// ls-wasm and `setInstances(...)` with interpolated ship/projectile state.

import * as mat4 from "../math/mat4.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import { CanvasSizer } from "./canvas.ts";
import { Frustum } from "./frustum.ts";
import { initGpu, type GpuContext } from "./gpu.ts";
import { GpuMesh, type MeshData } from "./mesh.ts";
import { projectileMesh, shipMesh } from "./primitives.ts";
import { FRAME_UNIFORM_FLOATS, INSTANCED_WGSL, TEAM_SLOTS, TERRAIN_WGSL } from "./shaders.ts";

export type Rgb = readonly [number, number, number];

export interface CameraView {
  position: Vec3;
  orientation: Quat;
  /** Vertical field of view in radians. */
  fovY: number;
  near: number;
  far: number;
}

export interface RenderSettings {
  fogColor: Rgb;
  /** exp2 fog density per u; 0 disables fog. */
  fogDensity: number;
  rockColor: Rgb;
  /** Paint colours indexed by 0-based team id (NO_TEAM = 255 is unpainted). */
  teamColors: readonly Rgb[];
  /** Direction towards the sun (normalised by the renderer). */
  sunDirection: Vec3;
  headlight: number;
}

export const DEFAULT_SETTINGS: RenderSettings = {
  fogColor: [0.05, 0.06, 0.09],
  fogDensity: 0.012,
  rockColor: [0.46, 0.43, 0.4],
  teamColors: [
    [1.0, 0.42, 0.1], // team 0: orange
    [0.15, 0.62, 1.0], // team 1: blue
    [0.7, 1.0, 0.2], // spare
    [0.9, 0.3, 0.9], // spare
  ],
  sunDirection: { x: 0.35, y: 0.85, z: 0.4 },
  headlight: 0.9,
};

/** One instanced object (ship or projectile). */
export interface Instance {
  position: Vec3;
  rotation: Quat;
  scale: number;
  color: Rgb;
  /** 0 = fully lit, 1 = fully emissive (glowing bolts). */
  emissive: number;
}

/** Floats per instance in the instance vertex buffer: offset+scale, rotation, colour+emissive. */
export const INSTANCE_FLOATS = 12;

/** Packs instances into the GPU layout used by INSTANCED_WGSL. */
export function packInstances(instances: readonly Instance[], out?: Float32Array): Float32Array {
  const needed = instances.length * INSTANCE_FLOATS;
  const data = out && out.length >= needed ? out : new Float32Array(needed);
  instances.forEach((inst, i) => {
    const o = i * INSTANCE_FLOATS;
    data[o] = inst.position.x;
    data[o + 1] = inst.position.y;
    data[o + 2] = inst.position.z;
    data[o + 3] = inst.scale;
    data[o + 4] = inst.rotation.x;
    data[o + 5] = inst.rotation.y;
    data[o + 6] = inst.rotation.z;
    data[o + 7] = inst.rotation.w;
    data[o + 8] = inst.color[0];
    data[o + 9] = inst.color[1];
    data[o + 10] = inst.color[2];
    data[o + 11] = inst.emissive;
  });
  return data;
}

export type InstanceKind = "ships" | "projectiles";

export interface FrameStats {
  chunksTotal: number;
  chunksDrawn: number;
  triangles: number;
  instances: number;
}

const DEPTH_FORMAT: GPUTextureFormat = "depth24plus";

/**
 * Terrain meshes must use counter-clockwise front faces as seen from open
 * space. Switch to "none" if a mesher ever emits mixed winding.
 */
const TERRAIN_CULL_MODE: GPUCullMode = "back";

const POSITION_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 12,
  attributes: [{ shaderLocation: 0, offset: 0, format: "float32x3" }],
};
const NORMAL_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 12,
  attributes: [{ shaderLocation: 1, offset: 0, format: "float32x3" }],
};
const TEAM_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 4,
  attributes: [{ shaderLocation: 2, offset: 0, format: "uint32" }],
};
const INSTANCE_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: INSTANCE_FLOATS * 4,
  stepMode: "instance",
  attributes: [
    { shaderLocation: 2, offset: 0, format: "float32x4" },
    { shaderLocation: 3, offset: 16, format: "float32x4" },
    { shaderLocation: 4, offset: 32, format: "float32x4" },
  ],
};

class InstanceBatch {
  readonly mesh: GpuMesh;
  private buffer: GPUBuffer | null = null;
  private capacity = 0;
  private scratch = new Float32Array(0);
  count = 0;

  constructor(mesh: GpuMesh) {
    this.mesh = mesh;
  }

  update(device: GPUDevice, instances: readonly Instance[]): void {
    this.count = instances.length;
    if (this.count === 0) return;
    if (this.count > this.capacity || !this.buffer) {
      this.buffer?.destroy();
      this.capacity = Math.max(64, 1 << Math.ceil(Math.log2(this.count)));
      this.buffer = device.createBuffer({
        label: "instances",
        size: this.capacity * INSTANCE_FLOATS * 4,
        usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
      });
      this.scratch = new Float32Array(this.capacity * INSTANCE_FLOATS);
    }
    packInstances(instances, this.scratch);
    device.queue.writeBuffer(this.buffer, 0, this.scratch.buffer, 0, this.count * INSTANCE_FLOATS * 4);
  }

  draw(pass: GPURenderPassEncoder): void {
    if (this.count === 0 || !this.buffer) return;
    pass.setVertexBuffer(0, this.mesh.positionBuffer);
    pass.setVertexBuffer(1, this.mesh.normalBuffer);
    pass.setVertexBuffer(2, this.buffer);
    pass.setIndexBuffer(this.mesh.indexBuffer, "uint32");
    pass.drawIndexed(this.mesh.indexCount, this.count);
  }

  destroy(): void {
    this.buffer?.destroy();
    this.mesh.destroy();
  }
}

export class Renderer {
  readonly device: GPUDevice;
  settings: RenderSettings = { ...DEFAULT_SETTINGS };

  private readonly canvas: HTMLCanvasElement;
  private readonly context: GPUCanvasContext;
  private readonly sizer: CanvasSizer;
  private readonly frameBuffer: GPUBuffer;
  private readonly frameData = new Float32Array(FRAME_UNIFORM_FLOATS);
  private readonly bindGroup: GPUBindGroup;
  private readonly terrainPipeline: GPURenderPipeline;
  private readonly instancedPipeline: GPURenderPipeline;
  private readonly chunks = new Map<string, GpuMesh>();
  private readonly batches: Record<InstanceKind, InstanceBatch>;
  private readonly frustum = new Frustum();
  private depthTexture: GPUTexture | null = null;
  private startTime = performance.now();
  private deviceLost = false;
  private lastGpuError: string | null = null;

  /** Creates the renderer; throws GpuUnavailableError when WebGPU cannot be used. */
  static async create(canvas: HTMLCanvasElement): Promise<Renderer> {
    return new Renderer(canvas, await initGpu(canvas));
  }

  private constructor(canvas: HTMLCanvasElement, gpu: GpuContext) {
    this.canvas = canvas;
    this.device = gpu.device;
    this.context = gpu.context;
    const device = this.device;
    this.sizer = new CanvasSizer(canvas, device.limits.maxTextureDimension2D);

    // Validation errors outside error scopes would otherwise only show up as a
    // console warning; log them and keep the latest for the HUD debug line.
    device.addEventListener("uncapturederror", (event) => {
      const message = (event as GPUUncapturedErrorEvent).error.message;
      this.lastGpuError = message;
      console.error("WebGPU:", message);
    });
    device.lost
      .then(() => {
        this.deviceLost = true;
      })
      .catch(() => {});

    this.frameBuffer = device.createBuffer({
      label: "frame uniforms",
      size: FRAME_UNIFORM_FLOATS * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    const bindGroupLayout = device.createBindGroupLayout({
      label: "frame",
      entries: [
        {
          binding: 0,
          visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
          buffer: { type: "uniform" },
        },
      ],
    });
    this.bindGroup = device.createBindGroup({
      layout: bindGroupLayout,
      entries: [{ binding: 0, resource: { buffer: this.frameBuffer } }],
    });
    const layout = device.createPipelineLayout({ bindGroupLayouts: [bindGroupLayout] });
    const depthStencil: GPUDepthStencilState = {
      format: DEPTH_FORMAT,
      depthWriteEnabled: true,
      depthCompare: "less",
    };

    const terrainModule = device.createShaderModule({ label: "terrain", code: TERRAIN_WGSL });
    this.terrainPipeline = device.createRenderPipeline({
      label: "terrain",
      layout,
      vertex: { module: terrainModule, entryPoint: "vs_main", buffers: [POSITION_LAYOUT, NORMAL_LAYOUT, TEAM_LAYOUT] },
      fragment: { module: terrainModule, entryPoint: "fs_main", targets: [{ format: gpu.format }] },
      primitive: { topology: "triangle-list", cullMode: TERRAIN_CULL_MODE, frontFace: "ccw" },
      depthStencil,
    });

    const instancedModule = device.createShaderModule({ label: "instanced", code: INSTANCED_WGSL });
    this.instancedPipeline = device.createRenderPipeline({
      label: "instanced",
      layout,
      vertex: {
        module: instancedModule,
        entryPoint: "vs_main",
        buffers: [POSITION_LAYOUT, NORMAL_LAYOUT, INSTANCE_LAYOUT],
      },
      fragment: { module: instancedModule, entryPoint: "fs_main", targets: [{ format: gpu.format }] },
      primitive: { topology: "triangle-list", cullMode: "back", frontFace: "ccw" },
      depthStencil,
    });

    this.batches = {
      ships: new InstanceBatch(new GpuMesh(device, shipMesh(), "ship")),
      projectiles: new InstanceBatch(new GpuMesh(device, projectileMesh(), "projectile")),
    };
  }

  /** True once the GPU device is lost; the frame loop should stop. */
  get isLost(): boolean {
    return this.deviceLost;
  }

  /** Message of the most recent uncaptured WebGPU error, if any. */
  get lastError(): string | null {
    return this.lastGpuError;
  }

  /** Resolves with a message when the GPU device is lost (driver reset, tab backgrounded, ...). */
  get lost(): Promise<string> {
    return this.device.lost.then((info) => info.message || info.reason || "unknown reason");
  }

  get aspect(): number {
    return this.sizer.aspect;
  }

  /** Adds or replaces the terrain chunk `key`; `null` or an empty mesh removes it. */
  setChunk(key: string, mesh: MeshData | null): void {
    this.chunks.get(key)?.destroy();
    this.chunks.delete(key);
    if (mesh && mesh.indices.length > 0) {
      this.chunks.set(key, new GpuMesh(this.device, mesh, `chunk ${key}`));
    }
  }

  clearChunks(): void {
    for (const mesh of this.chunks.values()) mesh.destroy();
    this.chunks.clear();
  }

  get chunkCount(): number {
    return this.chunks.size;
  }

  setInstances(kind: InstanceKind, instances: readonly Instance[]): void {
    this.batches[kind].update(this.device, instances);
  }

  render(camera: CameraView): FrameStats {
    this.sizer.update();
    this.ensureDepthTexture();

    const proj = mat4.perspective(camera.fovY, this.sizer.aspect, camera.near, camera.far);
    const view = mat4.viewFromPose(camera.position, camera.orientation);
    const viewProj = mat4.multiply(proj, view);
    this.frustum.setFromViewProj(viewProj);
    this.writeFrameUniforms(viewProj, camera.position);

    const s = this.settings;
    const encoder = this.device.createCommandEncoder({ label: "frame" });
    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view: this.context.getCurrentTexture().createView(),
          clearValue: { r: s.fogColor[0], g: s.fogColor[1], b: s.fogColor[2], a: 1 },
          loadOp: "clear",
          storeOp: "store",
        },
      ],
      depthStencilAttachment: {
        view: this.depthTexture!.createView(),
        depthClearValue: 1,
        depthLoadOp: "clear",
        depthStoreOp: "discard",
      },
    });
    pass.setBindGroup(0, this.bindGroup);

    const stats: FrameStats = { chunksTotal: this.chunks.size, chunksDrawn: 0, triangles: 0, instances: 0 };
    pass.setPipeline(this.terrainPipeline);
    for (const mesh of this.chunks.values()) {
      if (mesh.aabb && !this.frustum.intersectsAabb(mesh.aabb)) continue;
      pass.setVertexBuffer(0, mesh.positionBuffer);
      pass.setVertexBuffer(1, mesh.normalBuffer);
      pass.setVertexBuffer(2, mesh.teamBuffer);
      pass.setIndexBuffer(mesh.indexBuffer, "uint32");
      pass.drawIndexed(mesh.indexCount);
      stats.chunksDrawn++;
      stats.triangles += mesh.indexCount / 3;
    }

    pass.setPipeline(this.instancedPipeline);
    for (const batch of Object.values(this.batches)) {
      batch.draw(pass);
      stats.instances += batch.count;
      stats.triangles += (batch.mesh.indexCount / 3) * batch.count;
    }

    pass.end();
    this.device.queue.submit([encoder.finish()]);
    return stats;
  }

  destroy(): void {
    this.clearChunks();
    for (const batch of Object.values(this.batches)) batch.destroy();
    this.depthTexture?.destroy();
    this.frameBuffer.destroy();
    this.sizer.dispose();
    this.device.destroy();
  }

  private ensureDepthTexture(): void {
    const { width, height } = this.canvas;
    if (this.depthTexture && this.depthTexture.width === width && this.depthTexture.height === height) return;
    this.depthTexture?.destroy();
    this.depthTexture = this.device.createTexture({
      label: "depth",
      size: { width, height },
      format: DEPTH_FORMAT,
      usage: GPUTextureUsage.RENDER_ATTACHMENT,
    });
  }

  private writeFrameUniforms(viewProj: mat4.Mat4, cameraPos: Vec3): void {
    const d = this.frameData;
    const s = this.settings;
    d.set(viewProj, 0);
    d.set([cameraPos.x, cameraPos.y, cameraPos.z, (performance.now() - this.startTime) / 1000], 16);
    const sun = s.sunDirection;
    const len = Math.hypot(sun.x, sun.y, sun.z) || 1;
    d.set([sun.x / len, sun.y / len, sun.z / len, s.headlight], 20);
    d.set([...s.fogColor, s.fogDensity], 24);
    d.set([...s.rockColor, 1], 28);
    for (let i = 0; i < TEAM_SLOTS; i++) {
      const c = s.teamColors[i] ?? s.rockColor;
      d.set([c[0], c[1], c[2], 1], 32 + i * 4);
    }
    this.device.queue.writeBuffer(this.frameBuffer, 0, d);
  }
}
