// Belt Up's world look pass (M4 C7; shaders/belt.ts): the dust band, the
// aurora, the gates, the exit portal and sign and the ice glints, drawn
// analytically into the HDR scene after the sky (additive, reading the
// scene depth), and the screen-edge radiation over a copy of the finished
// scene after the particles. The game hands `Renderer.setBelt(look)` the
// belt and its state every frame (game/race/raceFx.ts); `null` (every
// classic level) skips both passes. Timed as `belt`.

import * as q from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { GpuTimer } from "./gpuTimer.ts";
import { HDR_FORMAT } from "./post.ts";
import type { CameraView, Rgb } from "./renderer.ts";
import { srgbToLinear } from "./shaders/frame.ts";
import { BELT_SLOT, BELT_UNIFORM_FLOATS, BELT_VOLUME_DIVISOR, beltVolumeWgsl, beltWgsl, RADIATION_WGSL, SIGN_HEIGHT, SIGN_UNIFORM_FLOATS, SIGN_WIDTH, SIGN_WORLD_WIDTH, signWgsl } from "./shaders/belt.ts";
import * as v3 from "../math/vec3.ts";
import type { Mat4 } from "../math/mat4.ts";

/** One gate's look: display colour, intensity (0 hides it), the number on its top bar, the pulse of the next gate. */
export interface BeltGateLook {
  color: Rgb;
  intensity: number;
  /** Number shown on the top bar (1-8), 0 = none. */
  number: number;
  /** 0..1: the next gate breathes and a sweep runs across its film. */
  pulse: number;
  /** 0 later, 1 next, 2 passed. */
  state: number;
}

/** What the belt pass draws this frame. */
export interface BeltLook {
  center: Vec3;
  innerRadius: number;
  outerRadius: number;
  halfThickness: number;
  exitRadius: number;
  cocoonRadius: number;
  gateInnerRadius: number;
  gateOuterRadius: number;
  /** Dust band strength (0 off). */
  dust: number;
  /** Display colour of the sunlit dust. */
  dustColor: Rgb;
  /** Aurora strength (0 off). */
  aurora: number;
  /** Ice glint strength (0 off). */
  glints: number;
  /** The 8 gates (missing entries are hidden). */
  gates: readonly BeltGateLook[];
  /** Gate strength overall (0 hides every gate). */
  gateStrength: number;
  /** Exit portal energy (0 hides the portal and the cocoon glow). */
  portal: number;
  /** Exit sign strength (0 hides it). */
  sign: number;
  /** Height of the sign's centre above the belt centre (u). */
  signHeight: number;
  /** Swirl speed of the portal (rad/s scale). */
  swirl: number;
  /** Screen-edge radiation 0..1 (the own ship's dose); 0 skips that pass. */
  radiation: number;
}

const lin = (c: Rgb): [number, number, number] => [srgbToLinear(c[0]), srgbToLinear(c[1]), srgbToLinear(c[2])];

/** Sign height in world units for SIGN_WORLD_WIDTH (the texture's aspect). */
export const SIGN_WORLD_HEIGHT = (SIGN_WORLD_WIDTH * SIGN_HEIGHT) / SIGN_WIDTH;

/** World corners of the exit sign (top left, top right, bottom left, bottom right), turned toward `cam`. */
export function signCorners(look: Pick<BeltLook, "center" | "signHeight">, cam: Vec3): Vec3[] {
  const sc = { x: look.center.x, y: look.center.y + look.signHeight, z: look.center.z };
  let n = v3.sub(cam, sc);
  n = v3.length(n) > 1e-4 ? v3.normalize(n) : { x: 0, y: 0, z: 1 };
  // Hangs upright, tilting halfway toward a viewer above or below.
  n = v3.normalize({ x: n.x, y: n.y * 0.5, z: n.z });
  let rgt = v3.cross({ x: 0, y: 1, z: 0 }, n);
  rgt = v3.length(rgt) > 1e-4 ? v3.normalize(rgt) : { x: 1, y: 0, z: 0 };
  const up = v3.cross(n, rgt);
  const hw = SIGN_WORLD_WIDTH / 2;
  const hh = SIGN_WORLD_HEIGHT / 2;
  const at = (x: number, y: number): Vec3 => v3.add(sc, v3.add(v3.scale(rgt, x), v3.scale(up, y)));
  return [at(-hw, hh), at(hw, hh), at(-hw, -hh), at(hw, -hh)];
}

/** Packs the uniform block (pure, tested). */
export function packBelt(
  look: BeltLook,
  camera: CameraView,
  aspect: number,
  time: number,
  fogDensity: number,
  size: { width: number; height: number },
  depthScale: number,
  sun: Vec3,
  sunColor: Rgb,
  out: Float32Array,
  low: { width: number; height: number } = { width: Math.ceil(size.width / BELT_VOLUME_DIVISOR), height: Math.ceil(size.height / BELT_VOLUME_DIVISOR) },
): void {
  const tanY = Math.tan(camera.fovY / 2);
  const tanX = tanY * aspect;
  const r = q.right(camera.orientation);
  const u = q.up(camera.orientation);
  const f = q.forward(camera.orientation);
  out.fill(0);
  out.set([camera.position.x, camera.position.y, camera.position.z, time % 1000], BELT_SLOT.cam);
  out.set([r.x * tanX, r.y * tanX, r.z * tanX, camera.near], BELT_SLOT.right);
  out.set([u.x * tanY, u.y * tanY, u.z * tanY, camera.far], BELT_SLOT.up);
  out.set([f.x, f.y, f.z, fogDensity], BELT_SLOT.fwd);
  out.set([size.width, size.height, depthScale, look.dust], BELT_SLOT.info);
  out.set([look.center.x, look.center.y, look.center.z, look.innerRadius], BELT_SLOT.center);
  out.set([look.outerRadius, look.halfThickness, look.exitRadius, look.cocoonRadius], BELT_SLOT.shape);
  out.set([look.gateInnerRadius, look.gateOuterRadius, look.aurora, look.glints], BELT_SLOT.gateShape);
  const sl = Math.hypot(sun.x, sun.y, sun.z) || 1;
  out.set([sun.x / sl, sun.y / sl, sun.z / sl, look.portal], BELT_SLOT.sun);
  out.set([...lin(sunColor), look.sign], BELT_SLOT.sunColor);
  out.set([...lin(look.dustColor), look.gateStrength], BELT_SLOT.dustColor);
  out.set([look.signHeight, SIGN_WORLD_WIDTH, SIGN_WORLD_HEIGHT, look.swirl], BELT_SLOT.extra);
  out.set([low.width, low.height, BELT_VOLUME_DIVISOR, 0], BELT_SLOT.low);
  for (let k = 0; k < 8; k++) {
    const g = look.gates[k];
    if (!g) continue;
    out.set([...lin(g.color), Math.max(0, g.intensity)], BELT_SLOT.gateColor + 4 * k);
    out.set([g.number, g.pulse, 0, g.state], BELT_SLOT.gateInfo + 4 * k);
  }
}

/**
 * Draws the emergency-exit sign into a 2D canvas context (SIGN_WIDTH x
 * SIGN_HEIGHT): a green panel with a white border, the running man leaving
 * through a door, "EXIT" and an arrow down to the portal.
 */
export function drawExitSign(g: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D): void {
  const w = SIGN_WIDTH;
  const h = SIGN_HEIGHT;
  g.clearRect(0, 0, w, h);
  const green = "#14b04f";
  const r = 22;
  const rr = (x: number, y: number, ww: number, hh: number, rad: number): void => {
    g.beginPath();
    g.moveTo(x + rad, y);
    g.arcTo(x + ww, y, x + ww, y + hh, rad);
    g.arcTo(x + ww, y + hh, x, y + hh, rad);
    g.arcTo(x, y + hh, x, y, rad);
    g.arcTo(x, y, x + ww, y, rad);
    g.closePath();
  };
  // Panel with a lighter centre (a lit lightbox).
  const grad = g.createLinearGradient(0, 0, 0, h);
  grad.addColorStop(0, "#19c25a");
  grad.addColorStop(0.5, green);
  grad.addColorStop(1, "#0f9442");
  g.fillStyle = grad;
  rr(4, 4, w - 8, h - 8, r);
  g.fill();
  g.lineWidth = 7;
  g.strokeStyle = "#f4fff6";
  rr(16, 16, w - 32, h - 32, r - 8);
  g.stroke();
  // The door (a white frame with the dark green opening) and the running man.
  const dx = 40;
  const dy = 34;
  const dw = 92;
  const dh = 124;
  g.fillStyle = "#f4fff6";
  g.fillRect(dx, dy, dw, dh);
  g.fillStyle = "#0b6a2f";
  g.fillRect(dx + 10, dy + 10, dw - 20, dh - 10);
  const man = (x: number, y: number, s: number): void => {
    g.save();
    g.translate(x, y);
    g.scale(s, s);
    g.strokeStyle = "#f4fff6";
    g.fillStyle = "#f4fff6";
    g.lineCap = "round";
    g.lineJoin = "round";
    g.lineWidth = 13;
    // Head.
    g.beginPath();
    g.arc(14, -52, 10, 0, Math.PI * 2);
    g.fill();
    // Torso leaning into the run.
    g.beginPath();
    g.moveTo(8, -36);
    g.lineTo(-6, 0);
    g.stroke();
    // Arms: one forward, one back.
    g.beginPath();
    g.moveTo(5, -30);
    g.lineTo(24, -22);
    g.lineTo(36, -34);
    g.moveTo(5, -30);
    g.lineTo(-14, -24);
    g.lineTo(-24, -10);
    g.stroke();
    // Legs: a long stride.
    g.beginPath();
    g.moveTo(-6, 0);
    g.lineTo(14, 14);
    g.lineTo(10, 36);
    g.moveTo(-6, 0);
    g.lineTo(-22, 14);
    g.lineTo(-40, 12);
    g.stroke();
    g.restore();
  };
  man(dx + dw + 34, dy + 92, 1.15);
  // "EXIT" in heavy letters.
  g.fillStyle = "#f4fff6";
  g.font = "900 104px 'Arial Black', 'Helvetica Neue', Arial, sans-serif";
  g.textBaseline = "middle";
  g.textAlign = "left";
  g.fillText("EXIT", 236, h / 2 + 6);
  // An arrow down to the portal under the sign.
  g.beginPath();
  g.moveTo(w - 58, 58);
  g.lineTo(w - 58, 118);
  g.lineWidth = 12;
  g.strokeStyle = "#f4fff6";
  g.lineCap = "round";
  g.stroke();
  g.beginPath();
  g.moveTo(w - 80, 112);
  g.lineTo(w - 58, 142);
  g.lineTo(w - 36, 112);
  g.closePath();
  g.fill();
}

/** The GPU side: the volume and surface pipelines per depth sample count, the radiation pipeline, the sign texture. */
export class BeltPass {
  private readonly device: GPUDevice;
  private readonly uniform: GPUBuffer;
  private readonly radUniform: GPUBuffer;
  private readonly data = new Float32Array(BELT_UNIFORM_FLOATS);
  private readonly radData = new Float32Array(4);
  private readonly signUniform: GPUBuffer;
  private readonly signData = new Float32Array(SIGN_UNIFORM_FLOATS);
  private signBind: { group: GPUBindGroup; depth: GPUTexture } | null = null;
  private readonly pipelines = new Map<string, { pipeline: GPURenderPipeline; layout: GPUBindGroupLayout }>();
  private volumeBind: { group: GPUBindGroup; depth: GPUTexture } | null = null;
  private surfaceBind: { group: GPUBindGroup; depth: GPUTexture; volume: GPUTexture } | null = null;
  private volume: GPUTexture | null = null;
  private readonly signTexture: GPUTexture;
  private readonly sampler: GPUSampler;
  private radPipeline: { pipeline: GPURenderPipeline; layout: GPUBindGroupLayout } | null = null;
  private copy: GPUTexture | null = null;
  private radBind: { group: GPUBindGroup; copy: GPUTexture } | null = null;

  constructor(device: GPUDevice) {
    this.device = device;
    this.uniform = device.createBuffer({ label: "belt uniforms", size: BELT_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    this.radUniform = device.createBuffer({ label: "radiation uniforms", size: 16, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    this.signUniform = device.createBuffer({ label: "exit sign uniforms", size: SIGN_UNIFORM_FLOATS * 4, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
    this.sampler = device.createSampler({ label: "belt linear", magFilter: "linear", minFilter: "linear", mipmapFilter: "linear", addressModeU: "clamp-to-edge", addressModeV: "clamp-to-edge" });
    // sRGB: the canvas paints display colours; the shader reads linear light.
    this.signTexture = device.createTexture({
      label: "exit sign",
      size: { width: SIGN_WIDTH, height: SIGN_HEIGHT },
      format: "rgba8unorm-srgb",
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST | GPUTextureUsage.RENDER_ATTACHMENT,
    });
    this.paintSign();
  }

  /** Paints the sign into its texture (a 2D canvas, copied once). */
  private paintSign(): void {
    let canvas: OffscreenCanvas | HTMLCanvasElement | null = null;
    if (typeof OffscreenCanvas !== "undefined") canvas = new OffscreenCanvas(SIGN_WIDTH, SIGN_HEIGHT);
    else if (typeof document !== "undefined") {
      const c = document.createElement("canvas");
      c.width = SIGN_WIDTH;
      c.height = SIGN_HEIGHT;
      canvas = c;
    }
    const g = canvas?.getContext("2d") as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null | undefined;
    if (!canvas || !g) return;
    drawExitSign(g);
    this.device.queue.copyExternalImageToTexture({ source: canvas }, { texture: this.signTexture }, { width: SIGN_WIDTH, height: SIGN_HEIGHT });
  }

  private pipelineFor(kind: "volume" | "surface" | "sign", ms: boolean): { pipeline: GPURenderPipeline; layout: GPUBindGroupLayout } {
    const key = `${kind}${ms ? " msaa" : ""}`;
    const cached = this.pipelines.get(key);
    if (cached) return cached;
    const device = this.device;
    const entries: GPUBindGroupLayoutEntry[] = [
      { binding: 0, visibility: GPUShaderStage.FRAGMENT | GPUShaderStage.VERTEX, buffer: { type: "uniform" } },
      { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "depth", viewDimension: "2d", multisampled: ms } },
    ];
    if (kind === "surface") {
      entries.push(
        { binding: 3, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
        { binding: 4, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float" } },
      );
    } else if (kind === "sign") {
      entries.push(
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float" } },
        { binding: 3, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
      );
    }
    const layout = device.createBindGroupLayout({ label: `belt ${key}`, entries });
    const module = device.createShaderModule({ label: `belt ${kind}`, code: kind === "surface" ? beltWgsl(ms) : kind === "sign" ? signWgsl(ms) : beltVolumeWgsl(ms) });
    // The volume target is written whole. The surface pass adds light; the
    // sign is drawn over the scene. Both raise the scene's alpha only to the
    // colour keep mask (max), so the sky mask and other keep marks stay.
    const keepAlpha: GPUBlendComponent = { srcFactor: "one", dstFactor: "one", operation: "max" };
    const target: GPUColorTargetState =
      kind === "surface"
        ? { format: HDR_FORMAT, blend: { color: { srcFactor: "one", dstFactor: "one", operation: "add" }, alpha: keepAlpha } }
        : kind === "sign"
          ? { format: HDR_FORMAT, blend: { color: { srcFactor: "one", dstFactor: "one-minus-src-alpha", operation: "add" }, alpha: keepAlpha } }
          : { format: HDR_FORMAT };
    const pipeline = device.createRenderPipeline({
      label: `belt ${kind}`,
      layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }),
      vertex: { module, entryPoint: "vs_main" },
      fragment: { module, entryPoint: "fs_main", targets: [target] },
      primitive: { topology: "triangle-list" },
    });
    const entry = { pipeline, layout };
    this.pipelines.set(key, entry);
    return entry;
  }

  /** Draws the belt look onto `target` (the resolved HDR scene), reading `depth`. */
  encode(
    encoder: GPUCommandEncoder,
    target: GPUTextureView,
    depth: GPUTexture,
    camera: CameraView,
    size: { width: number; height: number },
    look: BeltLook | null,
    time: number,
    fogDensity: number,
    sun: Vec3,
    sunColor: Rgb,
    viewProj: Mat4,
    timer: GpuTimer | null,
  ): void {
    if (!look) return;
    const ms = depth.sampleCount > 1;
    const low = { width: Math.max(1, Math.ceil(size.width / BELT_VOLUME_DIVISOR)), height: Math.max(1, Math.ceil(size.height / BELT_VOLUME_DIVISOR)) };
    if (!this.volume || this.volume.width !== low.width || this.volume.height !== low.height) {
      this.volume?.destroy();
      this.volume = this.device.createTexture({ label: "belt volume", size: low, format: HDR_FORMAT, usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING });
      this.surfaceBind = null;
    }
    packBelt(look, camera, size.width / Math.max(1, size.height), time, fogDensity, size, depth.width / Math.max(1, size.width), sun, sunColor, this.data, low);
    this.device.queue.writeBuffer(this.uniform, 0, this.data);
    const vol = this.pipelineFor("volume", ms);
    const surf = this.pipelineFor("surface", ms);
    if (!this.volumeBind || this.volumeBind.depth !== depth) {
      this.volumeBind = {
        depth,
        group: this.device.createBindGroup({
          label: "belt volume",
          layout: vol.layout,
          entries: [
            { binding: 0, resource: { buffer: this.uniform } },
            { binding: 1, resource: depth.createView({ aspect: "depth-only" }) },
          ],
        }),
      };
    }
    if (!this.surfaceBind || this.surfaceBind.depth !== depth || this.surfaceBind.volume !== this.volume) {
      this.surfaceBind = {
        depth,
        volume: this.volume,
        group: this.device.createBindGroup({
          label: "belt surface",
          layout: surf.layout,
          entries: [
            { binding: 0, resource: { buffer: this.uniform } },
            { binding: 1, resource: depth.createView({ aspect: "depth-only" }) },
            { binding: 3, resource: this.sampler },
            { binding: 4, resource: this.volume.createView() },
          ],
        }),
      };
    }
    const volWrites = timer?.pass("belt");
    const vp = encoder.beginRenderPass({
      label: "belt volume",
      ...(volWrites ? { timestampWrites: volWrites } : {}),
      colorAttachments: [{ view: this.volume.createView(), loadOp: "clear", clearValue: { r: 0, g: 0, b: 0, a: 0 }, storeOp: "store" }],
    });
    vp.setPipeline(vol.pipeline);
    vp.setBindGroup(0, this.volumeBind.group);
    vp.draw(3);
    vp.end();
    const timestampWrites = timer?.pass("belt");
    const pass = encoder.beginRenderPass({
      label: "belt",
      ...(timestampWrites ? { timestampWrites } : {}),
      colorAttachments: [{ view: target, loadOp: "load", storeOp: "store" }],
    });
    pass.setPipeline(surf.pipeline);
    pass.setBindGroup(0, this.surfaceBind.group);
    pass.draw(3);
    if (look.sign > 0) {
      const sign = this.pipelineFor("sign", ms);
      if (!this.signBind || this.signBind.depth !== depth) {
        this.signBind = {
          depth,
          group: this.device.createBindGroup({
            label: "exit sign",
            layout: sign.layout,
            entries: [
              { binding: 0, resource: { buffer: this.signUniform } },
              { binding: 1, resource: depth.createView({ aspect: "depth-only" }) },
              { binding: 2, resource: this.signTexture.createView() },
              { binding: 3, resource: this.sampler },
            ],
          }),
        };
      }
      const d = this.signData;
      d.set(viewProj, 0);
      signCorners(look, camera.position).forEach((c, i) => d.set([c.x, c.y, c.z, 1], 16 + 4 * i));
      d.set([look.sign, time % 1000, depth.width / Math.max(1, size.width), fogDensity], 32);
      d.set([camera.position.x, camera.position.y, camera.position.z, 0], 36);
      this.device.queue.writeBuffer(this.signUniform, 0, d);
      pass.setPipeline(sign.pipeline);
      pass.setBindGroup(0, this.signBind.group);
      pass.draw(6);
    }
    pass.end();
  }

  private radiationPipeline(): { pipeline: GPURenderPipeline; layout: GPUBindGroupLayout } {
    if (this.radPipeline) return this.radPipeline;
    const device = this.device;
    const layout = device.createBindGroupLayout({
      label: "radiation",
      entries: [
        { binding: 0, visibility: GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: "float" } },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, sampler: { type: "filtering" } },
      ],
    });
    const module = device.createShaderModule({ label: "radiation", code: RADIATION_WGSL });
    const pipeline = device.createRenderPipeline({
      label: "radiation",
      layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }),
      vertex: { module, entryPoint: "vs_main" },
      fragment: { module, entryPoint: "fs_main", targets: [{ format: HDR_FORMAT }] },
      primitive: { topology: "triangle-list" },
    });
    this.radPipeline = { pipeline, layout };
    return this.radPipeline;
  }

  /**
   * The screen-edge radiation over the finished scene (`scene` needs
   * COPY_SRC): a copy, then a fullscreen pass writing the fringed, static
   * covered picture back. Skipped while `look.radiation` is 0.
   */
  encodeRadiation(encoder: GPUCommandEncoder, scene: GPUTexture, look: BeltLook | null, time: number, timer: GpuTimer | null): void {
    if (!look || !(look.radiation > 0.001)) return;
    const { width, height } = scene;
    if (!this.copy || this.copy.width !== width || this.copy.height !== height) {
      this.copy?.destroy();
      this.copy = this.device.createTexture({ label: "radiation scene copy", size: { width, height }, format: HDR_FORMAT, usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.TEXTURE_BINDING });
      this.radBind = null;
    }
    const { pipeline, layout } = this.radiationPipeline();
    if (!this.radBind || this.radBind.copy !== this.copy) {
      this.radBind = {
        copy: this.copy,
        group: this.device.createBindGroup({
          label: "radiation",
          layout,
          entries: [
            { binding: 0, resource: { buffer: this.radUniform } },
            { binding: 1, resource: this.copy.createView() },
            { binding: 2, resource: this.sampler },
          ],
        }),
      };
    }
    this.radData.set([Math.min(1, look.radiation), time % 1000, width, height]);
    this.device.queue.writeBuffer(this.radUniform, 0, this.radData);
    encoder.copyTextureToTexture({ texture: scene }, { texture: this.copy }, { width, height });
    const timestampWrites = timer?.pass("belt");
    const pass = encoder.beginRenderPass({
      label: "radiation",
      ...(timestampWrites ? { timestampWrites } : {}),
      colorAttachments: [{ view: scene.createView(), loadOp: "load", storeOp: "store" }],
    });
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, this.radBind.group);
    pass.draw(3);
    pass.end();
  }

  destroy(): void {
    this.uniform.destroy();
    this.radUniform.destroy();
    this.signUniform.destroy();
    this.volume?.destroy();
    this.signTexture.destroy();
    this.copy?.destroy();
  }
}
