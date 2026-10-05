// Cockpit hull frame (visual-quality-plan.md VQ1): the own ship's nose,
// side parts and canopy pillars (hullFrameModels.ts) drawn in view space at
// the end of the scene pass, so they move with the camera. It is lit by the
// scene (sun, ambient, headlight, VQ4 materials via `shade_model`) at its
// world position, but projected with its own near plane and squeezed into
// the front 1/1000 of the depth range, so terrain and ships never cut it.
//
// The renderer owns one HullFrame (`Renderer.hullFrame`); the game sets the
// ship type, team colour and engine glow each frame and `null` hides it
// (dead, spectating, riding a guided missile, or switched off).

import * as mat4 from "../math/mat4.ts";
import type { Quat } from "../math/quat.ts";
import { GpuMesh } from "./mesh.ts";
import { frameModel } from "./hullFrameModels.ts";
import { MODEL_SHADING_WGSL } from "./models/shading.ts";
import { HDR_FORMAT } from "./post.ts";
import { KEEP_ALPHA } from "./shaders/post.ts";
import { FRAME_WGSL } from "./shaders/frame.ts";
import { LIGHTING_WGSL } from "./shaders/lighting.ts";
import { EMISSIVE_PEAK } from "./shaders/objects.ts";
import type { Vec3 } from "../math/vec3.ts";

/** Near and far planes of the frame's own projection (u). */
export const FRAME_NEAR = 0.01;
export const FRAME_FAR = 4;
/** The frame's depth is squeezed into [0, FRAME_DEPTH_RANGE] of the shared depth buffer. */
export const FRAME_DEPTH_RANGE = 0.001;
/** Light kept by painted frame parts next to the eye and at the nose (cockpit shade). */
export const COCKPIT_SHADE_NEAR = 0.3;
export const COCKPIT_SHADE_FAR = 0.75;
/** Floats in the frame's uniform block. */
export const HULL_FRAME_UNIFORM_FLOATS = 32;

/** What the frame shows this frame; `null` hides it. */
export interface HullFrameState {
  shipType: number;
  /** Hitbox radius (u); the frame is drawn at radius / TINKER_RADIUS. */
  radius: number;
  /** Livery colour (display value) and engine glow 0..1. */
  color: readonly [number, number, number];
  glow?: number;
  /**
   * 0..1: share of the frame's colour a monochrome grade keeps (Last Color
   * Standing, post.ts KEEP_ALPHA; D5), like the other hulls' keepColor.
   */
  keepColor?: number;
}

export interface HullFrameView {
  position: Vec3;
  orientation: Quat;
  fovY: number;
}

/**
 * Uniforms: the frame's projection, the camera orientation (view -> world)
 * and position, the livery colour and [scale, glow, blink, 0].
 */
export function packHullFrameUniforms(
  view: HullFrameView,
  aspect: number,
  state: HullFrameState,
  out = new Float32Array(HULL_FRAME_UNIFORM_FLOATS),
): Float32Array {
  out.set(mat4.perspective(view.fovY, aspect, FRAME_NEAR, FRAME_FAR), 0);
  const q = view.orientation;
  out.set([q.x, q.y, q.z, q.w], 16);
  out.set([view.position.x, view.position.y, view.position.z, 0], 20);
  out.set([state.color[0], state.color[1], state.color[2], Math.min(1, Math.max(0, state.keepColor ?? 0))], 24);
  out.set([state.radius / 0.5, state.glow ?? 0, 0.37, 0], 28);
  return out;
}

export const HULL_FRAME_WGSL = /* wgsl */ `
${FRAME_WGSL}
${LIGHTING_WGSL}

struct HullFrame {
  proj: mat4x4f,
  cam_rot: vec4f,   // camera orientation (view -> world), quaternion
  cam_pos: vec4f,
  color: vec4f,     // livery colour (display value), w = colour kept by the monochrome grade (0..1)
  params: vec4f,    // x = scale, y = engine glow, z = nav blink phase
};
@group(1) @binding(0) var<uniform> hull: HullFrame;

fn quat_rotate(q: vec4f, v: vec3f) -> vec3f {
  let t = 2.0 * cross(q.xyz, v);
  return v + q.w * t + cross(q.xyz, t);
}

fn emissive_glow(albedo: vec3f, a: f32) -> vec3f {
  return albedo * (1.0 + ${EMISSIVE_PEAK - 1}.0 * a * a);
}

fn shade(albedo: vec3f, a: f32, n: vec3f, world_pos: vec3f) -> vec3f {
  let lit = albedo * light(n, world_pos);
  return mix(lit, emissive_glow(albedo, a), a);
}

${MODEL_SHADING_WGSL}

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) world_pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) @interpolate(flat) part: u32,
  @location(3) local_pos: vec3f,
  @location(4) local_normal: vec3f,
  @location(5) view_pos: vec3f,
};

@vertex
fn vs_main(@location(0) pos: vec3f, @location(1) normal: vec3f, @location(6) part: u32) -> VsOut {
  var out: VsOut;
  let v = pos * hull.params.x;
  var clip = hull.proj * vec4f(v, 1.0);
  // Own depth range: always in front of the scene, still depth-sorted within itself.
  clip.z = clip.z * ${FRAME_DEPTH_RANGE};
  out.clip = clip;
  out.world_pos = hull.cam_pos.xyz + quat_rotate(hull.cam_rot, v);
  out.normal = quat_rotate(hull.cam_rot, normal);
  out.part = part & 0xffu;
  out.local_pos = pos;
  out.local_normal = normal;
  out.view_pos = v;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let n = normalize(in.normal);
  let color = vec4f(srgb_to_linear(hull.color.rgb), 0.0);
  var c = shade_model(in.part, n, in.world_pos, in.local_pos, normalize(in.local_normal), color, hull.params.y, hull.params.z);
  // Cockpit shade: painted parts near the eye sit in the canopy's shadow and
  // brighten toward the nose, so the frame frames the view instead of
  // competing with it. Lamps, engines and nav lights keep their light.
  if (in.part <= PART_CANOPY || in.part == PART_SCORCH) {
    let d = length(in.view_pos) / max(hull.params.x, 1e-3);
    c = c * mix(${COCKPIT_SHADE_NEAR}, ${COCKPIT_SHADE_FAR}, smoothstep(0.18, 0.5, d));
  }
  // Alpha above 1 marks colour the monochrome grade keeps (objects.ts, post.ts KEEP_ALPHA).
  return vec4f(c, 1.0 + clamp(hull.color.w, 0.0, 1.0) * ${KEEP_ALPHA});
}
`;

const POSITION_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 12,
  attributes: [{ shaderLocation: 0, offset: 0, format: "float32x3" }],
};
const NORMAL_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 12,
  attributes: [{ shaderLocation: 1, offset: 0, format: "float32x3" }],
};
const PART_LAYOUT: GPUVertexBufferLayout = {
  arrayStride: 4,
  attributes: [{ shaderLocation: 6, offset: 0, format: "uint32" }],
};

export class HullFrame {
  /** What to draw; null hides the frame. */
  state: HullFrameState | null = null;

  private readonly device: GPUDevice;
  private readonly module: GPUShaderModule;
  private readonly layout: GPUPipelineLayout;
  private readonly uniforms: GPUBuffer;
  private readonly bindGroup: GPUBindGroup;
  private readonly data = new Float32Array(HULL_FRAME_UNIFORM_FLOATS);
  private readonly pipelines = new Map<number, GPURenderPipeline>();
  private readonly meshes = new Map<number, GpuMesh>();

  constructor(device: GPUDevice, frameLayout: GPUBindGroupLayout) {
    this.device = device;
    this.module = device.createShaderModule({ label: "hull frame", code: HULL_FRAME_WGSL });
    const own = device.createBindGroupLayout({
      label: "hull frame",
      entries: [{ binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } }],
    });
    this.layout = device.createPipelineLayout({ bindGroupLayouts: [frameLayout, own] });
    this.uniforms = device.createBuffer({
      label: "hull frame uniforms",
      size: HULL_FRAME_UNIFORM_FLOATS * 4,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.bindGroup = device.createBindGroup({ layout: own, entries: [{ binding: 0, resource: { buffer: this.uniforms } }] });
  }

  private pipeline(samples: number): GPURenderPipeline {
    let p = this.pipelines.get(samples);
    if (!p) {
      p = this.device.createRenderPipeline({
        label: "hull frame",
        layout: this.layout,
        vertex: { module: this.module, entryPoint: "vs_main", buffers: [POSITION_LAYOUT, NORMAL_LAYOUT, PART_LAYOUT] },
        fragment: { module: this.module, entryPoint: "fs_main", targets: [{ format: HDR_FORMAT }] },
        primitive: { topology: "triangle-list", cullMode: "back", frontFace: "ccw" },
        depthStencil: { format: "depth24plus", depthWriteEnabled: true, depthCompare: "less" },
        multisample: { count: samples },
      });
      this.pipelines.set(samples, p);
    }
    return p;
  }

  private mesh(shipType: number): GpuMesh {
    let m = this.meshes.get(shipType);
    if (!m) {
      m = new GpuMesh(this.device, frameModel(shipType).build(), `hull frame ${shipType}`);
      this.meshes.set(shipType, m);
    }
    return m;
  }

  /** Draws the frame into the open scene pass (group 0 = the frame uniforms is already bound). */
  draw(pass: GPURenderPassEncoder, samples: number, view: HullFrameView, aspect: number): number {
    const s = this.state;
    if (!s) return 0;
    const mesh = this.mesh(s.shipType);
    this.device.queue.writeBuffer(this.uniforms, 0, packHullFrameUniforms(view, aspect, s, this.data));
    pass.setPipeline(this.pipeline(samples));
    pass.setBindGroup(1, this.bindGroup);
    pass.setVertexBuffer(0, mesh.positionBuffer);
    pass.setVertexBuffer(1, mesh.normalBuffer);
    pass.setVertexBuffer(2, mesh.teamBuffer);
    pass.setIndexBuffer(mesh.indexBuffer, "uint32");
    pass.drawIndexed(mesh.indexCount);
    return mesh.indexCount / 3;
  }

  destroy(): void {
    for (const m of this.meshes.values()) m.destroy();
    this.meshes.clear();
    this.uniforms.destroy();
  }
}
