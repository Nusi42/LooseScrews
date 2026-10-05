// Model showcase (`?view=ships`): fixed scenes for reviewing the VQ4 hulls
// and props and for before/after screenshots.
//
//   ?view=ships                    ship bay: the four hulls side by side
//   ?view=ships&show=wrecks        the four wrecks
//   ?view=ships&show=props         rocket, harpoon, mine, crate, upgrade, gadget
//   ?view=ships&scene=flight       in flight: a practice world (share code
//                                  `code`, default the smoke world), seen from
//                                  the spawn, with ships flying ahead
//   &models=legacy                 the pre-VQ4 primitives ("before")
//   &boost=1                       engines at full boost
//   &focus=0..3                    bay camera close to one hull (ShipType index)
//   ?view=ships&bench=1            object cost: 8 ships + 200 props + 8 wrecks
//                                  in view, GPU time per frame minus an empty
//                                  frame (queue.onSubmittedWorkDone, no
//                                  timestamp-query needed); the scene is drawn
//                                  BENCH_COPIES times so the cost rises above
//                                  the timing noise, and divided back; result
//                                  as JSON in `document.body.dataset.bench`
//
// Sets `document.body.dataset.showcase = "ready"` once the scene is drawn
// (the screenshot script waits for it).

import * as q from "../../math/quat.ts";
import type { Quat } from "../../math/quat.ts";
import type { Vec3 } from "../../math/vec3.ts";
import { CONTROLS_LEN, type WasmApi } from "../../wasm/api.ts";
import { MeshBuilder, NO_TEAM } from "../mesh.ts";
import { appendBox } from "../primitives.ts";
import { emptyScene, type CameraView, type Instance, type Rgb, type SceneInstances, Renderer } from "../renderer.ts";
import { shipMeshKind, wreckMeshKind, type MeshKind } from "./index.ts";

/** Hitbox radii by ship type (data/ships) and the Tinker's (instance scale 1). */
const RADII = [0.4, 0.5, 0.65, 0.55];
const TINKER_RADIUS = 0.5;
const NAMES = ["Skitter", "Tinker", "Hauler", "Tug"];
/** Hull colours of teams 0 and 1 (game/scene.ts TEAM_HULL). */
const TEAM: readonly Rgb[] = [
  [0.95, 0.5, 0.2],
  [0.25, 0.6, 0.95],
];
const UP: Vec3 = { x: 0, y: 1, z: 0 };
/** Display size of the hulls in the bay (x their real size) and their spacing (u). */
const DISPLAY = 1.7;
const SPACING = 2.3;
export const SMOKE_CODE = "LS08-0000-0000-0000-8006-8001-8F9G";

export function wantsShowcase(loc: Pick<Location, "search">): boolean {
  return new URLSearchParams(loc.search).get("view") === "ships";
}

function push(scene: SceneInstances, mesh: MeshKind, inst: Instance): void {
  (scene.opaque[mesh] ??= []).push(inst);
}

function yawPitch(yaw: number, pitch = 0): Quat {
  return q.mul(q.fromAxisAngle(UP, yaw), q.fromAxisAngle({ x: 1, y: 0, z: 0 }, pitch));
}

/** A closed bay (floor, walls, ceiling) drawn as a terrain chunk, inner faces visible. */
function bayMesh() {
  const b = new MeshBuilder();
  const w = 9, h = 3.2, d = 7, t = 0.5;
  // Each slab is a solid box; the inner faces show from the bay.
  appendBox(b, 0, -t, 0, w, t, d, NO_TEAM); // floor
  appendBox(b, 0, h + t, 0, w, t, d, NO_TEAM); // ceiling
  appendBox(b, 0, h / 2, -d - t, w, h, t, NO_TEAM); // back wall
  appendBox(b, -w - t, h / 2, 0, t, h, d, NO_TEAM);
  appendBox(b, w + t, h / 2, 0, t, h, d, NO_TEAM);
  // Display plinths.
  for (let i = 0; i < 4; i++) appendBox(b, (i - 1.5) * SPACING, 0.08, -1.5, 0.85, 0.08, 0.85, NO_TEAM);
  return b.build();
}

function caption(ui: HTMLElement, text: string): void {
  const el = document.createElement("div");
  el.style.cssText =
    "position:fixed;left:0;right:0;bottom:24px;text-align:center;font:600 15px system-ui,sans-serif;color:#d8dde6;letter-spacing:.04em;text-shadow:0 1px 3px #000";
  el.textContent = text;
  ui.appendChild(el);
}

/** Ship bay, wreck row or prop row. */
function bayScene(show: string, boost: boolean, time: number, focus: number | null): { scene: SceneInstances; camera: CameraView; label: string } {
  const scene = emptyScene();
  const label: string[] = [];
  for (let t = 0; t < 4; t++) {
    const x = (t - 1.5) * SPACING;
    if (show === "props") continue;
    const scale = (DISPLAY * RADII[t]!) / TINKER_RADIUS;
    // Front three-quarter view; with boost from behind, so the engines show.
    const rotation = yawPitch((boost ? 0.75 : -2.3) + 0.2 * Math.sin(time * 0.3 + t), -0.1);
    const position = { x, y: 0.2 + DISPLAY * RADII[t]! * 0.6, z: -1.5 };
    if (show === "wrecks") {
      push(scene, wreckMeshKind(t), { position, rotation, scale: 1.2 * DISPLAY, color: [0.55, 0.52, 0.5], emissive: 0 });
      label.push(`${NAMES[t]} wreck`);
    } else {
      push(scene, shipMeshKind(t), { position, rotation, scale, color: TEAM[t % 2]!, emissive: 0, glow: boost ? 1 : 0, blink: t * 0.31 });
      label.push(NAMES[t]!);
    }
  }
  if (show === "props") {
    const props: [MeshKind, Rgb, number][] = [
      ["rocket", [1, 0.55, 0.15], 0.9],
      ["harpoon", [0.7, 0.8, 0.9], 1.2],
      ["mine", [1, 0.25, 0.2], 0.8],
      ["crate", [0.35, 0.9, 0.45], 0.9],
      ["upgrade", [0.4, 0.75, 1], 0.9],
      ["gadget", [0.95, 0.8, 0.3], 0.9],
    ];
    props.forEach(([mesh, color, size], i) => {
      const x = (i - 2.5) * 1.45;
      const rotation = mesh === "rocket" || mesh === "harpoon" ? yawPitch(-2.2, 0.15) : yawPitch(0.5 + time * 0.2, 0.2);
      push(scene, mesh, { position: { x, y: 0.9, z: -1.5 }, rotation, scale: size * 1.25, color, emissive: 0.35 });
      label.push(mesh);
    });
  }
  const camera: CameraView = {
    position: { x: 0, y: 1.9, z: 4.2 },
    orientation: q.lookRotation({ x: 0, y: -0.27, z: -1 }, UP),
    fovY: (46 * Math.PI) / 180,
    near: 0.05,
    far: 100,
  };
  if (focus !== null && focus >= 0 && focus < 4) {
    const x = (focus - 1.5) * SPACING;
    const y = 0.2 + DISPLAY * RADII[focus]! * 0.6;
    camera.position = { x: x - 0.35, y: y + 0.75, z: -1.5 + 2.1 * DISPLAY * RADII[focus]! + 0.4 };
    camera.orientation = q.lookRotation({ x: x - camera.position.x, y: y - camera.position.y, z: -1.5 - camera.position.z }, UP);
    return { scene, camera, label: label[focus] ?? "" };
  }
  return { scene, camera, label: label.join("  ·  ") };
}

/** Ships flying ahead of the spawn pose: a mixed formation, a wreck and pickups. */
function flightScene(pose: { position: Vec3; orientation: Quat }, boost: boolean, time: number): SceneInstances {
  const scene = emptyScene();
  const f = q.forward(pose.orientation), r = q.right(pose.orientation), u = q.up(pose.orientation);
  const at = (ahead: number, right: number, up: number): Vec3 => ({
    x: pose.position.x + f.x * ahead + r.x * right + u.x * up,
    y: pose.position.y + f.y * ahead + r.y * right + u.y * up,
    z: pose.position.z + f.z * ahead + r.z * right + u.z * up,
  });
  // Rotation in the camera frame: yaw about its up axis, then pitch.
  const local = (yaw: number, pitch: number): Quat =>
    q.mul(pose.orientation, q.mul(q.fromAxisAngle({ x: 0, y: 1, z: 0 }, yaw), q.fromAxisAngle({ x: 1, y: 0, z: 0 }, pitch)));
  const ships: [number, number, number, number, number, number][] = [
    // type, ahead, right, up, yaw, team
    [1, 2.4, -0.8, -0.35, 2.5, 0],
    [0, 3.8, 1.0, 0.2, -0.5, 1],
    [2, 5.6, -0.4, 0.7, 0.9, 1],
    [3, 3.2, 1.7, -0.55, 3.6, 0],
  ];
  ships.forEach(([t, ahead, right, up, yaw, team], i) => {
    push(scene, shipMeshKind(t), {
      position: at(ahead, right, up),
      rotation: local(yaw + 0.1 * Math.sin(time + i), -0.15),
      scale: RADII[t]! / TINKER_RADIUS,
      color: TEAM[team]!,
      emissive: 0,
      glow: boost || i % 2 === 1 ? 1 : 0,
      blink: i * 0.37,
    });
  });
  push(scene, wreckMeshKind(2), { position: at(5, -1.9, -1.0), rotation: local(0.7, 0.4), scale: 1.6, color: [0.6, 0.6, 0.6], emissive: 0 });
  push(scene, "crate", { position: at(4.5, 0.4, -1.0), rotation: local(time, 0), scale: 0.9, color: [0.35, 0.9, 0.45], emissive: 0.35 });
  push(scene, "rocket", { position: at(2.8, 0.45, 0.3), rotation: local(Math.PI + 0.3, 0), scale: 0.9, color: [1, 0.55, 0.15], emissive: 0.25 });
  return scene;
}

/** Copies of the bench scene drawn per frame (the reported cost is per copy). */
export const BENCH_COPIES = 10;

/**
 * The bench scene: `copies` x (8 ships (two of each type, half boosting),
 * 200 props and 8 wrecks) in blocks ahead of the camera.
 */
export function benchScene(copies = 1): SceneInstances {
  const scene = emptyScene();
  for (let c = 0; c < copies; c++) addBenchCopy(scene, -c * 1.5);
  return scene;
}

function addBenchCopy(scene: SceneInstances, dz: number): void {
  for (let i = 0; i < 8; i++) {
    const t = i % 4;
    push(scene, shipMeshKind(t), {
      position: { x: (i - 3.5) * 1.6, y: 1.2, z: -8 + dz },
      rotation: yawPitch(0.6 * i, 0.2),
      scale: RADII[t]! / TINKER_RADIUS,
      color: TEAM[i % 2]!,
      emissive: 0,
      glow: i % 2,
      blink: i * 0.3,
    });
    push(scene, wreckMeshKind(t), { position: { x: (i - 3.5) * 1.6, y: -1.2, z: -8 + dz }, rotation: yawPitch(0.4 * i), scale: 1.2, color: [0.6, 0.6, 0.6], emissive: 0 });
  }
  const kinds: MeshKind[] = ["rocket", "mine", "crate", "upgrade", "gadget", "harpoon"];
  for (let i = 0; i < 200; i++) {
    const kind = kinds[i % kinds.length]!;
    push(scene, kind, {
      position: { x: ((i % 20) - 9.5) * 0.9, y: Math.floor(i / 20) * 0.7 - 3, z: -14 + dz },
      rotation: yawPitch(i * 0.7, i * 0.3),
      scale: 0.8,
      color: [0.4 + (i % 3) * 0.2, 0.7, 0.9 - (i % 4) * 0.15],
      emissive: 0.3,
    });
  }
}

/** Mean ms per frame of `frames` renders, waiting for the GPU to finish. */
async function timeFrames(renderer: Renderer, camera: CameraView, frames: number): Promise<number> {
  await renderer.device.queue.onSubmittedWorkDone();
  const t = performance.now();
  for (let i = 0; i < frames; i++) renderer.render(camera);
  await renderer.device.queue.onSubmittedWorkDone();
  return (performance.now() - t) / frames;
}

async function bench(renderer: Renderer, legacy: boolean): Promise<void> {
  const camera: CameraView = { position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), fovY: (75 * Math.PI) / 180, near: 0.05, far: 600 };
  const scene = benchScene(BENCH_COPIES);
  const frames = 60;
  const info = (renderer.device as GPUDevice & { adapterInfo?: GPUAdapterInfo }).adapterInfo;
  const result: Record<string, number | string | boolean> = { gpu: `${info?.vendor ?? ""} ${info?.architecture ?? ""} ${info?.description ?? ""}`.trim(), legacy, width: document.querySelector("canvas")?.width ?? 0, height: document.querySelector("canvas")?.height ?? 0 };
  // Warm up both, then alternate empty and full runs and keep the best (least disturbed) of each.
  let empty = Infinity, full = Infinity;
  renderer.setScene(scene);
  await timeFrames(renderer, camera, 10);
  for (let k = 0; k < 5; k++) {
    renderer.setScene(emptyScene());
    empty = Math.min(empty, await timeFrames(renderer, camera, frames));
    renderer.setScene(scene);
    full = Math.min(full, await timeFrames(renderer, camera, frames));
  }
  let triangles = 0, instances = 0;
  const stats = renderer.render(camera);
  triangles = stats.triangles;
  instances = stats.instances;
  Object.assign(result, {
    copies: BENCH_COPIES,
    emptyMs: +empty.toFixed(3),
    fullMs: +full.toFixed(3),
    objectsMsPerCopy: +((full - empty) / BENCH_COPIES).toFixed(4),
    trianglesPerCopy: triangles / BENCH_COPIES,
    instancesPerCopy: instances / BENCH_COPIES,
  });
  document.body.dataset.bench = JSON.stringify(result);
}

export async function showShowcase(canvas: HTMLCanvasElement, ui: HTMLElement, loadApi: () => Promise<WasmApi>): Promise<void> {
  const params = new URLSearchParams(location.search);
  const legacy = params.get("models") === "legacy";
  const boost = params.get("boost") === "1";
  const show = params.get("show") ?? "ships";
  const flight = params.get("scene") === "flight";
  const focusParam = params.get("focus");
  const focus = focusParam === null ? null : Number(focusParam);
  const renderer = await Renderer.create(canvas, { legacyModels: legacy });
  const t0 = performance.now();
  if (params.get("bench") === "1") {
    renderer.render({ position: { x: 0, y: 0, z: 0 }, orientation: q.identity(), fovY: 1.3, near: 0.05, far: 600 });
    await bench(renderer, legacy);
    caption(ui, `bench: ${document.body.dataset.bench ?? ""}`);
    document.body.dataset.showcase = "ready";
    return;
  }
  if (!flight) {
    renderer.setChunk("bay", bayMesh());
    renderer.settings = { ...renderer.settings, headlight: 0.35, fogDensity: 0.004 };
    let label = "";
    const frame = (): void => {
      const time = (performance.now() - t0) / 1000;
      const s = bayScene(show, boost, time, focus);
      label = s.label;
      renderer.setScene(s.scene);
      renderer.render(s.camera);
      requestAnimationFrame(frame);
    };
    frame();
    caption(ui, `${legacy ? "Before (primitives)" : "VQ4 models"}  —  ${label}${boost ? " (boost)" : ""}`);
    document.body.dataset.showcase = "ready";
    return;
  }

  const api = await loadApi();
  const code = params.get("code") ?? SMOKE_CODE;
  const game = api.Game.practice(code, 0);
  game.frame(0, new Float32Array(CONTROLS_LEN));
  const p = game.localPose();
  if (!p || p.length < 7) throw new Error("practice world has no spawn pose");
  const pose = { position: { x: p[0]!, y: p[1]!, z: p[2]! }, orientation: { x: p[3]!, y: p[4]!, z: p[5]!, w: p[6]! } };
  // Mesh every chunk around the spawn before the first frame.
  for (let guard = 0; guard < 4096; guard++) {
    const m = game.meshNext(pose.position.x, pose.position.y, pose.position.z);
    if (!m) break;
    try {
      renderer.setChunk(m.key, {
        positions: m.positions(),
        normals: m.normals(),
        indices: m.indices(),
        team: m.teams(),
        surface: m.surface?.() ?? new Uint8Array(0),
        materials: m.materials?.() ?? new Uint8Array(0),
        materialWeights: m.materialWeights?.() ?? new Uint8Array(0),
        paintColors: m.paintColors?.() ?? new Uint8Array(0),
      });
    } finally {
      m.free();
    }
  }
  const camera: CameraView = { position: pose.position, orientation: pose.orientation, fovY: (75 * Math.PI) / 180, near: 0.05, far: 600 };
  const frame = (): void => {
    renderer.setScene(flightScene(pose, boost, (performance.now() - t0) / 1000));
    renderer.render(camera);
    requestAnimationFrame(frame);
  };
  frame();
  caption(ui, `${legacy ? "Before (primitives)" : "VQ4 models"}  —  in flight, ${code}`);
  document.body.dataset.showcase = "ready";
}
