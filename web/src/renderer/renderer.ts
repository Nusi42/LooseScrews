// WebGPU renderer: world-space terrain chunks (frustum-culled per chunk
// AABB) plus instanced objects (ships, projectiles, pickups, debris, wrecks,
// ...), one batch per mesh kind (primitives.ts MESH_BUILDERS) in an opaque
// and a translucent pass, all lit and fogged by the shaders in shaders/.
// The scene renders linear HDR radiance into an rgba16float target with
// MSAA (quality.ts presets); in open levels sky.ts draws the sky under the
// resolved scene and adds light shafts (VQ7); particles.ts draws the GPU
// particles and dust motes on top (VQ5); post.ts adds bloom, grades, tonemaps
// (AgX) and encodes sRGB into the swapchain. Passes are timed with timestamp queries
// when the device has them (gpuTimer.ts).
//
// The game calls `setChunk(key, mesh)` with meshes produced by ls-wasm and
// `setScene(...)` with the instances built by game/scene.ts each frame.

import * as mat4 from "../math/mat4.ts";
import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import { stripRoomWalls, type Atmosphere } from "./atmosphere.ts";
import { CanvasSizer } from "./canvas.ts";
import { CloakPass } from "./cloak.ts";
import { Frustum } from "./frustum.ts";
import { initGpu, type GpuContext } from "./gpu.ts";
import { GpuTimer, type GpuTimings } from "./gpuTimer.ts";
import { LightSystem, lightBindLayoutEntries, type PointLight } from "./lights.ts";
import { HullFrame } from "./hullFrame.ts";
import { ModeFxPass, type ModeVolume } from "./modeFx.ts";
import { GpuMesh, type MeshData } from "./mesh.ts";
import { LEGACY_MESH_BUILDERS, MESH_BUILDERS, MESH_KINDS, type MeshKind } from "./models/index.ts";
import { ParticleSystem } from "./particles.ts";
import { DEFAULT_POST, HDR_FORMAT, PostChain, type PostSettings } from "./post.ts";
import { TerrainMaterial } from "./rockTextures.ts";
import { terrainVertexData } from "./terrainBlend.ts";
import { QUALITY_PRESETS, QualityAuto, qualityFromSearch, type QualityLevel, type RenderQuality } from "./quality.ts";
import { FRAME_UNIFORM_FLOATS, ROOM_FADE, srgbToLinear, TEAM_SLOTS } from "./shaders/frame.ts";
import { Sky } from "./sky.ts";
import { SunShadow, sunShadowBindLayoutEntries } from "./sunShadow.ts";
import { VOLUMETRIC_LOOKS, VolumetricShafts, volumetricLook } from "./volumetric.ts";
import { INSTANCED_WGSL, TRANSLUCENT_WGSL } from "./shaders/objects.ts";
import { TERRAIN_WGSL } from "./shaders/terrain.ts";

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
  /** Per-world terrain look (strata tilt, rock pattern offsets; rockTextures.ts `hashSeed` of the share code); default 0. */
  terrainSeed?: number;
  /** Height fog (VQ7): density per u at world height `baseY`, falling off by `falloff` per u upwards; density 0 = off. */
  heightFog: { density: number; falloff: number; baseY: number };
  /** 0..1 how much the fog takes the sky's haze colour (the environment cube) instead of `fogColor` (VQ7). */
  skyHaze: number;
  /** Exposure, bloom, vignette, grain, AgX look and colour grade (post.ts). */
  post: PostSettings;
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
  heightFog: { density: 0, falloff: 0.1, baseY: 0 },
  skyHaze: 0,
  post: DEFAULT_POST,
};

/** One instanced object (ship, projectile, pickup, debris, ...). */
export interface Instance {
  position: Vec3;
  rotation: Quat;
  scale: number;
  color: Rgb;
  /** 0 = fully lit, 1 = fully emissive (glowing bolts). */
  emissive: number;
  /** Extra length factor along the local Z axis (trails); default 1. */
  stretch?: number;
  /** Opacity in the translucent pass (ignored when opaque); default 1. */
  opacity?: number;
  /** Engine glow of model meshes: 0 idle .. 1 full boost (VQ4); default 0. */
  glow?: number;
  /** Nav light blink phase in seconds (VQ4), so ships do not blink in lockstep; default 0. */
  blink?: number;
  /**
   * Opaque pass only: 0..1 how much of its colour survives a monochrome
   * grade (Last Color Standing, M3 C1; post.ts KEEP_ALPHA). Shares the
   * opacity slot, which the opaque pass does not use; default 0.
   */
  keepColor?: number;
}

/**
 * Floats per instance in the instance vertex buffer: offset+scale,
 * rotation, colour+emissive, stretch+opacity+glow+blink.
 */
export const INSTANCE_FLOATS = 16;

/** Packs instances into the GPU layout used by INSTANCED_WGSL / TRANSLUCENT_WGSL. */
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
    data[o + 12] = inst.stretch ?? 1;
    data[o + 13] = inst.keepColor ? 1 + Math.min(1, Math.max(0, inst.keepColor)) : (inst.opacity ?? 1);
    data[o + 14] = inst.glow ?? 0;
    data[o + 15] = inst.blink ?? 0;
  });
  return data;
}

/**
 * Opaque pass (depth-written), translucent pass (blended, drawn after), or
 * the cloak refraction (cloak.ts: drawn through a copy of the finished
 * scene; `opacity` is the cloak's strength).
 */
export type RenderPass = "opaque" | "translucent" | "refract";

/** Instances per mesh kind for each pass (built by game/scene.ts). */
export interface SceneInstances {
  opaque: Partial<Record<MeshKind, Instance[]>>;
  translucent: Partial<Record<MeshKind, Instance[]>>;
  /** Cloaked ships (VQP); optional, missing = none. */
  refract?: Partial<Record<MeshKind, Instance[]>>;
}

export function emptyScene(): SceneInstances {
  return { opaque: {}, translucent: {} };
}

export interface FrameStats {
  chunksTotal: number;
  chunksDrawn: number;
  triangles: number;
  instances: number;
}

const DEPTH_FORMAT: GPUTextureFormat = "depth24plus";

/** Sun shadow map size (VQ9): 4096² depth16 (32 MB) covers the 256 x 128 x 256 u level at about 0.09 u per texel. */
const SUN_SHADOW_SIZE = 4096;

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
/** Terrain AO and blend weights (terrainBlend.ts: two unorm8x4 per vertex). */
const EXTRA_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 8,
  attributes: [
    { shaderLocation: 3, offset: 0, format: "unorm8x4" },
    { shaderLocation: 4, offset: 4, format: "unorm8x4" },
  ],
};
/** Per-vertex material part of instanced meshes (the mesh's expanded `team` buffer; models/kit.ts PART_*). */
const PART_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 4,
  attributes: [{ shaderLocation: 6, offset: 0, format: "uint32" }],
};
const INSTANCE_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: INSTANCE_FLOATS * 4,
  stepMode: "instance",
  attributes: [
    { shaderLocation: 2, offset: 0, format: "float32x4" },
    { shaderLocation: 3, offset: 16, format: "float32x4" },
    { shaderLocation: 4, offset: 32, format: "float32x4" },
    { shaderLocation: 5, offset: 48, format: "float32x4" },
  ],
};

interface ScenePipelines {
  terrain: GPURenderPipeline;
  instanced: GPURenderPipeline;
  translucent: GPURenderPipeline;
}

/** Instances of one shared mesh (the batch does not own the mesh). */
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
    pass.setVertexBuffer(3, this.mesh.teamBuffer);
    pass.setIndexBuffer(this.mesh.indexBuffer, "uint32");
    pass.drawIndexed(this.mesh.indexCount, this.count);
  }

  destroy(): void {
    this.buffer?.destroy();
  }
}

export class Renderer {
  readonly device: GPUDevice;
  settings: RenderSettings = { ...DEFAULT_SETTINGS, post: { ...DEFAULT_POST } };

  private readonly canvas: HTMLCanvasElement;
  private readonly context: GPUCanvasContext;
  private readonly sizer: CanvasSizer;
  private readonly frameBuffer: GPUBuffer;
  private readonly frameData = new Float32Array(FRAME_UNIFORM_FLOATS);
  private readonly bindGroup: GPUBindGroup;
  private readonly pipelineLayout: GPUPipelineLayout;
  private readonly terrainMaterial: TerrainMaterial;
  private readonly terrainLayout: GPUPipelineLayout;
  private readonly modules: { terrain: GPUShaderModule; instanced: GPUShaderModule; translucent: GPUShaderModule };
  /** Scene pipelines per MSAA sample count and terrain detail (built when a quality level first needs them). */
  private readonly scenePipelines = new Map<string, ScenePipelines>();
  private readonly post: PostChain;
  /** GPU particles and dust motes (VQ5); null when switched off (`particles: false`, `?vfx=off`). */
  readonly particles: ParticleSystem | null;
  private readonly sky: Sky;
  /** Sun occlusion (VQ9): the shadow map over the level; `setEnabled(false)` (or `?shadows=off`) lets the sun through rock. */
  readonly sunShadow: SunShadow;
  /** Volumetric sun shafts through the shadow map (VQ9). */
  readonly volumetric: VolumetricShafts;
  private readonly chunks = new Map<string, GpuMesh>();
  private readonly meshes: Record<MeshKind, GpuMesh>;
  private readonly batches: Record<RenderPass, Map<MeshKind, InstanceBatch>> = {
    opaque: new Map(),
    translucent: new Map(),
    refract: new Map(),
  };
  /** Cloaked ships drawn as refraction (VQP). */
  private readonly cloak: CloakPass;
  private readonly frustum = new Frustum();
  /** Dynamic point lights (VQ6, lights.ts): this frame's lights from `setLights`. */
  private readonly lightSystem: LightSystem;
  private lights: readonly PointLight[] = [];
  /** Mode effects (M3 C1, modeFx.ts): this frame's volumes from `setModeVolumes`. */
  private readonly modeFx: ModeFxPass;
  private modeVolumes: readonly ModeVolume[] = [];
  private depthTexture: GPUTexture | null = null;
  private msaaTexture: GPUTexture | null = null;
  private startTime = performance.now();
  private deviceLost = false;
  private lastGpuError: string | null = null;
  private readonly timer: GpuTimer | null;
  private readonly auto: QualityAuto;
  private qualityLevel: RenderQuality;
  /**
   * Time per pass: with `timestamp-query`, true times terrain and objects as
   * separate render passes (the debug overlay and the visual gallery switch
   * it on); false draws the scene in one pass, timed as "scene".
   */
  detailedTiming = false;
  /** Shader time (s) for this and later frames instead of the wall clock (fixed-pose screenshots). */
  timeOverride: number | null = null;
  /** Open levels: rock fades into the sky before the hidden walls (VQP); false = cut off sharply (before shots). */
  roomFade = true;
  /** The own ship's cockpit frame (VQ1), drawn last in the scene pass; set its `state` per frame. */
  readonly hullFrame: HullFrame;

  /**
   * Creates the renderer; throws GpuUnavailableError when WebGPU cannot be
   * used. `quality` forces a level (default: `?quality=`, else picked
   * automatically from the GPU time of the first seconds). `legacyModels`
   * draws the pre-VQ4 primitives (before/after shots).
   */
  static async create(
    canvas: HTMLCanvasElement,
    options: { quality?: QualityLevel; legacyModels?: boolean; particles?: boolean; shadows?: boolean } = {},
  ): Promise<Renderer> {
    const search = typeof location === "undefined" ? "" : location.search;
    const forced = options.quality ?? qualityFromSearch(search);
    const params = new URLSearchParams(search);
    const particles = options.particles ?? params.get("vfx") !== "off";
    const shadows = options.shadows ?? params.get("shadows") !== "off";
    const renderer = new Renderer(canvas, await initGpu(canvas), forced, options.legacyModels ?? false, particles);
    renderer.sunShadow.setEnabled(shadows);
    const bench = params.get("shadowbench");
    if (bench === "full" || bench === "chunk") renderer.sunShadow.bench = bench;
    return renderer;
  }

  private constructor(canvas: HTMLCanvasElement, gpu: GpuContext, forced: QualityLevel | null, legacyModels: boolean, particles: boolean) {
    this.canvas = canvas;
    this.device = gpu.device;
    this.context = gpu.context;
    const device = this.device;
    this.sizer = new CanvasSizer(canvas, device.limits.maxTextureDimension2D);
    this.qualityLevel = QUALITY_PRESETS[forced ?? "high"];
    this.auto = new QualityAuto(forced);

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
    this.sky = new Sky(device);
    const bindGroupLayout = device.createBindGroupLayout({
      label: "frame",
      entries: [
        {
          binding: 0,
          visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
          buffer: { type: "uniform" },
        },
        ...lightBindLayoutEntries(),
        // The sky's environment cube for the fog (VQ7, sky.ts; frame.ts ENV_BINDING).
        { binding: 6, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, texture: { viewDimension: "cube" } },
        { binding: 7, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
        // The sun shadow map (VQ9, sunShadow.ts; shaders/shadow.ts).
        ...sunShadowBindLayoutEntries(),
      ],
    });
    this.lightSystem = new LightSystem(device);
    this.sunShadow = new SunShadow(device, Math.min(SUN_SHADOW_SIZE, device.limits.maxTextureDimension2D));
    this.volumetric = new VolumetricShafts(device, this.sunShadow);
    this.bindGroup = device.createBindGroup({
      layout: bindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: this.frameBuffer } },
        ...this.lightSystem.bindGroupEntries(),
        { binding: 6, resource: this.sky.envView },
        { binding: 7, resource: this.sky.envSampler },
        ...this.sunShadow.bindGroupEntries(),
      ],
    });
    this.pipelineLayout = device.createPipelineLayout({ bindGroupLayouts: [bindGroupLayout] });
    this.terrainMaterial = new TerrainMaterial(device);
    this.terrainLayout = device.createPipelineLayout({ bindGroupLayouts: [bindGroupLayout, this.terrainMaterial.layout] });
    this.hullFrame = new HullFrame(device, bindGroupLayout);
    this.cloak = new CloakPass(device, bindGroupLayout);
    this.modules = {
      terrain: device.createShaderModule({ label: "terrain", code: TERRAIN_WGSL }),
      instanced: device.createShaderModule({ label: "instanced", code: INSTANCED_WGSL }),
      translucent: device.createShaderModule({ label: "translucent", code: TRANSLUCENT_WGSL }),
    };
    this.scenePipelinesFor(this.qualityLevel);
    this.post = new PostChain(device, gpu.format);
    this.particles = particles ? new ParticleSystem(device, this.frameBuffer, this.qualityLevel.particles) : null;
    this.modeFx = new ModeFxPass(device);

    this.timer = GpuTimer.create(device);

    const meshes = {} as Record<MeshKind, GpuMesh>;
    const builders = legacyModels ? LEGACY_MESH_BUILDERS : MESH_BUILDERS;
    for (const kind of MESH_KINDS) meshes[kind] = new GpuMesh(device, builders[kind](), kind);
    this.meshes = meshes;
  }

  /** Terrain, instanced and translucent pipelines rendering into the HDR target at the quality's MSAA and terrain detail. */
  private scenePipelinesFor(quality: RenderQuality): ScenePipelines {
    const samples = quality.msaa;
    const key = `${samples}:${quality.terrainDetail ? 1 : 0}`;
    const cached = this.scenePipelines.get(key);
    if (cached) return cached;
    const device = this.device;
    const layout = this.pipelineLayout;
    const multisample: GPUMultisampleState = { count: samples };
    const depthStencil: GPUDepthStencilState = { format: DEPTH_FORMAT, depthWriteEnabled: true, depthCompare: "less" };
    const { terrain, instanced, translucent } = this.modules;
    const pipelines: ScenePipelines = {
      terrain: device.createRenderPipeline({
        label: "terrain",
        layout: this.terrainLayout,
        vertex: { module: terrain, entryPoint: "vs_main", buffers: [POSITION_LAYOUT, NORMAL_LAYOUT, TEAM_LAYOUT, EXTRA_LAYOUT] },
        fragment: {
          module: terrain,
          entryPoint: "fs_main",
          targets: [{ format: HDR_FORMAT }],
          constants: { DETAIL: quality.terrainDetail ? 1 : 0 },
        },
        primitive: { topology: "triangle-list", cullMode: TERRAIN_CULL_MODE, frontFace: "ccw" },
        depthStencil,
        multisample,
      }),
      instanced: device.createRenderPipeline({
        label: "instanced",
        layout,
        vertex: { module: instanced, entryPoint: "vs_main", buffers: [POSITION_LAYOUT, NORMAL_LAYOUT, INSTANCE_LAYOUT, PART_LAYOUT] },
        fragment: { module: instanced, entryPoint: "fs_main", targets: [{ format: HDR_FORMAT }] },
        primitive: { topology: "triangle-list", cullMode: "back", frontFace: "ccw" },
        depthStencil,
        multisample,
      }),
      translucent: device.createRenderPipeline({
        label: "translucent",
        layout,
        vertex: { module: translucent, entryPoint: "vs_main", buffers: [POSITION_LAYOUT, NORMAL_LAYOUT, INSTANCE_LAYOUT, PART_LAYOUT] },
        fragment: {
          module: translucent,
          entryPoint: "fs_main",
          targets: [
            {
              format: HDR_FORMAT,
              blend: {
                color: { srcFactor: "src-alpha", dstFactor: "one-minus-src-alpha", operation: "add" },
                alpha: { srcFactor: "one", dstFactor: "one-minus-src-alpha", operation: "add" },
              },
            },
          ],
        },
        // Both faces: the inside of a dust cloud is visible from within.
        primitive: { topology: "triangle-list", cullMode: "none", frontFace: "ccw" },
        depthStencil: { format: DEPTH_FORMAT, depthWriteEnabled: false, depthCompare: "less" },
        multisample,
      }),
    };
    this.scenePipelines.set(key, pipelines);
    return pipelines;
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

  /** Per-pass GPU times (window average), or null without `timestamp-query` or before the first readback. */
  gpuTimings(): GpuTimings | null {
    return this.timer?.timings() ?? null;
  }

  /** True when the device can time passes (`timestamp-query`). */
  get canTime(): boolean {
    return this.timer !== null;
  }

  /** The active quality preset. */
  get quality(): RenderQuality {
    return this.qualityLevel;
  }

  /** Switches the quality preset (pipelines and targets follow on the next frame). */
  setQuality(level: QualityLevel): void {
    this.qualityLevel = QUALITY_PRESETS[level];
    this.particles?.setQuality(this.qualityLevel.particles);
  }

  get aspect(): number {
    return this.sizer.aspect;
  }

  /**
   * The world's atmosphere (atmosphere.ts `atmosphereFor`; VQ7): sky, sun,
   * fog and colour grade. Call it before the world's chunks arrive: in open
   * levels `setChunk` hides the level box's sealing walls. Null restores the
   * defaults (no sky, flat fog, neutral grade).
   */
  setAtmosphere(a: Atmosphere | null): void {
    this.sky.setAtmosphere(a);
    const s = this.settings;
    this.volumetric.look = a ? volumetricLook(a.weights) : VOLUMETRIC_LOOKS.cave;
    // VQ9: without a sky the level box is sealed and the sun never gets in.
    this.sunShadow.setSealed(a !== null && a.open <= 0);
    if (!a) {
      this.settings = { ...s, fogColor: DEFAULT_SETTINGS.fogColor, fogDensity: DEFAULT_SETTINGS.fogDensity, sunDirection: DEFAULT_SETTINGS.sunDirection, heightFog: DEFAULT_SETTINGS.heightFog, skyHaze: 0, post: { ...s.post, grade: DEFAULT_POST.grade } };
      return;
    }
    this.settings = { ...s, fogColor: a.fogColor, fogDensity: a.fogDensity, sunDirection: a.sunDirection, heightFog: a.heightFog, skyHaze: 1, post: { ...s.post, grade: a.grade } };
  }

  /** The active atmosphere, or null. */
  get atmosphere(): Atmosphere | null {
    return this.sky.atmosphere;
  }

  /** Adds or replaces the terrain chunk `key`; `null` or an empty mesh removes it. */
  setChunk(key: string, mesh: MeshData | null): void {
    const old = this.chunks.get(key);
    const before = old?.aabb ?? null;
    old?.destroy();
    this.chunks.delete(key);
    // Open levels: the sky shows where the level box's walls would be.
    const room = this.sky.visible ? this.sky.atmosphere?.room : null;
    if (mesh && room) mesh = stripRoomWalls(mesh, room);
    this.lightSystem.setChunk(key, mesh);
    if (mesh && mesh.indices.length > 0) {
      this.chunks.set(key, new GpuMesh(this.device, mesh, `chunk ${key}`, terrainVertexData(mesh)));
    }
    this.sunShadow.chunkChanged(before, this.chunks.get(key)?.aabb ?? null);
  }

  clearChunks(): void {
    this.lightSystem.clearChunks();
    for (const mesh of this.chunks.values()) mesh.destroy();
    this.chunks.clear();
    this.sunShadow.clear();
  }

  get chunkCount(): number {
    return this.chunks.size;
  }

  /** Replaces the instances of one mesh kind in one pass. */
  setInstances(kind: MeshKind, instances: readonly Instance[], pass: RenderPass = "opaque"): void {
    const batches = this.batches[pass];
    let batch = batches.get(kind);
    if (!batch) {
      if (instances.length === 0) return;
      batch = new InstanceBatch(this.meshes[kind]);
      batches.set(kind, batch);
    }
    batch.update(this.device, instances);
  }

  /** Replaces every instance: kinds missing from `scene` are cleared. */
  setScene(scene: SceneInstances): void {
    for (const pass of ["opaque", "translucent", "refract"] as const) {
      const wanted = scene[pass] ?? {};
      for (const kind of MESH_KINDS) this.setInstances(kind, wanted[kind] ?? [], pass);
    }
  }

  /**
   * This frame's dynamic point lights (shots, explosions, engines, pickups,
   * objectives; game/lightFx.ts builds them). Hot-rock lights come from the
   * chunk meshes on their own. Kept until the next call.
   */
  setLights(lights: readonly PointLight[]): void {
    this.lights = lights;
  }

  /**
   * This frame's mode effect volumes (glowing Squeeze walls, zones, beams,
   * halos; game/modeVisuals.ts builds them). Kept until the next call.
   */
  setModeVolumes(volumes: readonly ModeVolume[]): void {
    this.modeVolumes = volumes;
  }

  /** Lights uploaded in the last frame and the candidates they were chosen from. */
  get lightStats(): { lights: number; candidates: number } {
    return { lights: this.lightSystem.lastCount, candidates: this.lightSystem.lastCandidates };
  }

  /** Removes every instance (before a new match). */
  clearInstances(): void {
    this.setScene(emptyScene());
    this.lights = [];
    this.modeVolumes = [];
  }

  /** View-projection matrix of `camera` at the current canvas aspect (screen-space labels). */
  viewProjection(camera: CameraView): mat4.Mat4 {
    const proj = mat4.perspective(camera.fovY, this.sizer.aspect, camera.near, camera.far);
    return mat4.multiply(proj, mat4.viewFromPose(camera.position, camera.orientation));
  }

  render(camera: CameraView): FrameStats {
    this.pickQuality();
    this.sizer.update();
    const quality = this.qualityLevel;
    const samples = quality.msaa;
    this.ensureTargets(samples, quality.bloomMips);
    const pipelines = this.scenePipelinesFor(quality);

    const view = mat4.viewFromPose(camera.position, camera.orientation);
    const viewProj = mat4.multiply(mat4.perspective(camera.fovY, this.sizer.aspect, camera.near, camera.far), view);
    this.frustum.setFromViewProj(viewProj);
    const time = this.timeOverride ?? (performance.now() - this.startTime) / 1000;
    this.writeFrameUniforms(viewProj, camera.position, time);
    const tanY = Math.tan(camera.fovY / 2);
    const lightView = { view, tanX: tanY * this.sizer.aspect, tanY, forward: q.forward(camera.orientation) };
    this.lightSystem.prepare(this.lights, camera.position, lightView, quality.maxLights, this.frustum, time);

    const fog = this.frameData.subarray(24, 27);
    const encoder = this.device.createCommandEncoder({ label: "frame" });
    const timer = this.timer;
    timer?.beginFrame();
    this.lightSystem.encode(encoder, timer);
    this.sunShadow.setSun(this.settings.sunDirection);
    this.sunShadow.encode(encoder, this.chunks, timer);
    const split = this.detailedTiming && timer !== null;
    // With MSAA the scene renders into the multisampled target and the last
    // scene pass resolves it into the post chain's HDR texture.
    const sceneView = this.post.sceneView;
    const colorView = samples > 1 ? this.msaaTexture!.createView() : sceneView;
    const depthView = this.depthTexture!.createView();
    // Open levels clear to transparent black: the sky is drawn under the scene after the resolve (sky.ts).
    const sky = this.sky.visible;
    const clearValue = sky ? { r: 0, g: 0, b: 0, a: 0 } : { r: fog[0] ?? 0, g: fog[1] ?? 0, b: fog[2] ?? 0, a: 1 };
    let refracted = 0;
    for (const batch of this.batches.refract.values()) refracted += batch.count;
    const beginPass = (label: string, first: boolean, last: boolean): GPURenderPassEncoder => {
      const timestampWrites = timer?.pass(label);
      const color: GPURenderPassColorAttachment = {
        view: colorView,
        clearValue,
        loadOp: first ? "clear" : "load",
        storeOp: samples > 1 && last ? "discard" : "store",
      };
      if (samples > 1 && last) color.resolveTarget = sceneView;
      const pass = encoder.beginRenderPass({
        label,
        ...(timestampWrites ? { timestampWrites } : {}),
        colorAttachments: [color],
        depthStencilAttachment: {
          view: depthView,
          depthClearValue: 1,
          depthLoadOp: first ? "clear" : "load",
          // The particle pass (soft particles), the volumetric shafts, the mode effects and the cloak pass read the depth after the scene.
          depthStoreOp: last && !this.particles && quality.volumetric.divisor === 0 && refracted === 0 && this.modeVolumes.length === 0 ? "discard" : "store",
        },
      });
      pass.setBindGroup(0, this.bindGroup);
      return pass;
    };

    const stats: FrameStats = { chunksTotal: this.chunks.size, chunksDrawn: 0, triangles: 0, instances: 0 };
    let pass = beginPass(split ? "terrain" : "scene", true, !split);
    pass.setPipeline(pipelines.terrain);
    pass.setBindGroup(1, this.terrainMaterial.bindGroup);
    for (const mesh of this.chunks.values()) {
      if (!mesh.extraBuffer || (mesh.aabb && !this.frustum.intersectsAabb(mesh.aabb))) continue;
      pass.setVertexBuffer(0, mesh.positionBuffer);
      pass.setVertexBuffer(1, mesh.normalBuffer);
      pass.setVertexBuffer(2, mesh.teamBuffer);
      pass.setVertexBuffer(3, mesh.extraBuffer);
      pass.setIndexBuffer(mesh.indexBuffer, "uint32");
      pass.drawIndexed(mesh.indexCount);
      stats.chunksDrawn++;
      stats.triangles += mesh.indexCount / 3;
    }
    if (split) {
      pass.end();
      pass = beginPass("objects", false, true);
    }

    const passes: Array<[RenderPass, GPURenderPipeline]> = [
      ["opaque", pipelines.instanced],
      ["translucent", pipelines.translucent],
    ];
    for (const [kind, pipeline] of passes) {
      pass.setPipeline(pipeline);
      for (const batch of this.batches[kind].values()) {
        batch.draw(pass);
        stats.instances += batch.count;
        stats.triangles += (batch.mesh.indexCount / 3) * batch.count;
      }
    }
    stats.triangles += this.hullFrame.draw(pass, samples, camera, this.sizer.aspect);
    pass.end();

    // VQ9: sunlit air between the scene and the camera (reads the scene depth).
    this.volumetric.encode(encoder, this.post.sceneHdr, this.depthTexture!, camera, this.sizer.aspect, quality.volumetric, timer);
    if (sky) this.sky.composite(encoder, this.post.sceneHdr, camera, time, timer);
    if (quality.shafts) this.sky.encodeShafts(encoder, this.post.sceneHdr, viewProj, timer);
    // Mode effects (walls, zones, beams) over the backdrop, under the particles.
    const size = { width: this.canvas.width, height: this.canvas.height };
    this.modeFx.encode(encoder, sceneView, this.depthTexture!, camera, size, this.modeVolumes, time, this.settings.fogDensity, timer);
    // Cloaked ships bend the finished backdrop (VQP); particles draw over them.
    this.cloak.encode(encoder, this.post.sceneHdr, this.depthTexture!, this.bindGroup, camera.orientation, refracted, (p) => {
      for (const batch of this.batches.refract.values()) {
        batch.draw(p);
        stats.instances += batch.count;
        stats.triangles += (batch.mesh.indexCount / 3) * batch.count;
      }
    }, timer);
    // Particles after the sky and shafts: smoke and glows draw over the finished backdrop.
    this.particles?.encode(encoder, sceneView, this.depthTexture!, camera, { width: this.canvas.width, height: this.canvas.height }, timer);
    this.post.encode(encoder, this.context.getCurrentTexture().createView(), this.settings.post, time, timer);
    timer?.endFrame(encoder);
    this.device.queue.submit([encoder.finish()]);
    timer?.afterSubmit();
    return stats;
  }

  destroy(): void {
    this.clearChunks();
    this.hullFrame.destroy();
    this.cloak.destroy();
    for (const batches of Object.values(this.batches)) for (const batch of batches.values()) batch.destroy();
    for (const mesh of Object.values(this.meshes)) mesh.destroy();
    this.depthTexture?.destroy();
    this.msaaTexture?.destroy();
    this.post.destroy();
    this.terrainMaterial.destroy();
    this.lightSystem.destroy();
    this.sunShadow.destroy();
    this.volumetric.destroy();
    this.particles?.destroy();
    this.modeFx.destroy();
    this.sky.destroy();
    this.timer?.destroy();
    this.frameBuffer.destroy();
    this.sizer.dispose();
    this.device.destroy();
  }

  /** Automatic quality: steps down once if the GPU time of the first seconds exceeds the budget. */
  private pickQuality(): void {
    if (this.auto.decided) return;
    const gpu = this.timer?.timings() ?? null;
    const picked = this.auto.sample(gpu && gpu.frames >= 10 ? gpu.total : null, performance.now(), this.qualityLevel.level);
    if (picked && picked !== this.qualityLevel.level) {
      console.info(`renderer: GPU ${gpu?.total.toFixed(2)} ms per frame, quality ${this.qualityLevel.level} -> ${picked}`);
      this.setQuality(picked);
    }
  }

  /** Depth (and MSAA colour) targets at the canvas size, and the post chain's HDR texture and bloom mips. */
  private ensureTargets(samples: number, bloomMips: number): void {
    const { width, height } = this.canvas;
    const fits = (t: GPUTexture | null): boolean =>
      t !== null && t.width === width && t.height === height && t.sampleCount === samples;
    if (!fits(this.depthTexture)) {
      this.depthTexture?.destroy();
      this.depthTexture = this.device.createTexture({
        label: "depth",
        size: { width, height },
        format: DEPTH_FORMAT,
        sampleCount: samples,
        // TEXTURE_BINDING: the particle pass reads it (soft particles).
        usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING,
      });
    }
    if (samples > 1 && !fits(this.msaaTexture)) {
      this.msaaTexture?.destroy();
      this.msaaTexture = this.device.createTexture({
        label: "hdr scene msaa",
        size: { width, height },
        format: HDR_FORMAT,
        sampleCount: samples,
        usage: GPUTextureUsage.RENDER_ATTACHMENT,
      });
    } else if (samples === 1 && this.msaaTexture) {
      this.msaaTexture.destroy();
      this.msaaTexture = null;
    }
    this.post.resize(width, height, bloomMips);
  }

  private writeFrameUniforms(viewProj: mat4.Mat4, cameraPos: Vec3, time: number): void {
    const d = this.frameData;
    const s = this.settings;
    d.set(viewProj, 0);
    d.set([cameraPos.x, cameraPos.y, cameraPos.z, time], 16);
    const sun = s.sunDirection;
    const len = Math.hypot(sun.x, sun.y, sun.z) || 1;
    d.set([sun.x / len, sun.y / len, sun.z / len, s.headlight], 20);
    // Colours are authored as display values; the scene lights in linear.
    const lin = (c: Rgb): number[] => [srgbToLinear(c[0]), srgbToLinear(c[1]), srgbToLinear(c[2])];
    d.set([...lin(s.fogColor), s.fogDensity], 24);
    d.set([...lin(s.rockColor), 1], 28);
    this.terrainMaterial.setSeed(s.terrainSeed ?? 0);
    for (let i = 0; i < TEAM_SLOTS; i++) {
      const c = s.teamColors[i] ?? s.rockColor;
      d.set([...lin(c), 1], 32 + i * 4);
    }
    const h = s.heightFog;
    d.set([h.density, h.falloff, h.baseY, s.skyHaze], 48);
    // Open levels: rock fades out before the hidden walls (VQP; frame.ts room_fade).
    const room = this.sky.visible && this.roomFade ? this.sky.atmosphere?.room : null;
    d.set(room ? [room.min.x, room.min.y, room.min.z, ROOM_FADE] : [0, 0, 0, 0], 52);
    d.set(room ? [room.max.x, room.max.y, room.max.z, 0] : [0, 0, 0, 0], 56);
    this.device.queue.writeBuffer(this.frameBuffer, 0, d);
  }
}
