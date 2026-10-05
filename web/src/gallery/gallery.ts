// Visual gallery page (`?gallery=<scene id>`, docs/visual-quality-plan.md VQ8):
// renders one fixed scene of scenes.ts at a fixed pose and time, for
// before/after screenshots and the GPU budget check driven by
// tools/browser-smoke/gallery.mjs.
//
// The world comes from a deterministic share code (`randomShareCode(seed, 0,
// enclosure, density)`) in offline practice with no dummies; the camera pose
// is picked from the player's spawn point by CPU ray casts against the chunk
// meshes (raycast.ts), so it follows the generator without hand-tuned
// coordinates. Ships, shots and the explosion are synthetic and drawn
// through the normal scene builder and effect timeline (game/scene.ts,
// game/mechanics.ts), so later packages that change those show up here.
//
// Progress and results are published on `window.__gallery` (GalleryStatus).
// The page keeps rendering the same frame after it is ready, so the GPU
// timing window fills.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { boundsCenter, DEFAULT_CAMERA } from "../game/camera.ts";
import { ChunkLoader, LOADING_BUDGET } from "../game/chunks.ts";
import { CUE, EffectsState, NO_SHIP, type Cue, type EffectsContext, type Flash } from "../game/mechanics.ts";
import { buildScene } from "../game/scene.ts";
import { parsePose, parseVisuals, type ProjectileState, type ShipState, type VisualDef } from "../game/views.ts";
import type { GpuTimings } from "../renderer/gpuTimer.ts";
import { GRADE_PRESETS } from "../renderer/post.ts";
import { Renderer, type CameraView, type SceneInstances } from "../renderer/renderer.ts";
import { CONTROLS_LEN, type GameHandle } from "../wasm/api.ts";
import { loadWasm } from "../wasm/load.ts";
import { pickCloseup, pickLongest, pickVista, probeAround, raycast, rayMesh, sphereDirections, type Probe, type RayMesh } from "./raycast.ts";
import { sceneById, type GalleryScene } from "./scenes.ts";
import { explosionAt, simulateVfx } from "./vfxScenes.ts";

/** Frames rendered before the page reports ready. */
const WARMUP_FRAMES = 20;
/** Probe range for pose picking (u). */
const PROBE_RANGE = 160;
/** Probe directions around the spawn point. */
const PROBES = 160;

export interface GalleryStatus {
  scene: string;
  phase: "loading" | "ready" | "error";
  error: string | null;
  shareCode: string;
  camera: { position: Vec3; orientation: Quat } | null;
  frames: number;
  chunksDrawn: number;
  triangles: number;
  instances: number;
  canTime: boolean;
  gpu: GpuTimings | null;
  gpuError: string | null;
  /** Live particles in the frame (VQ5), null without particles. */
  particles?: number | null;
}

declare global {
  interface Window {
    __gallery?: GalleryStatus;
  }
}

export function wantsGallery(loc: { search: string }): boolean {
  return new URLSearchParams(loc.search).has("gallery");
}

const nextFrame = (): Promise<void> => new Promise((r) => requestAnimationFrame(() => r()));

/** Camera pose for `scene` from the probes around `origin` (spawn orientation as the fallback). */
export function pickPose(
  scene: GalleryScene,
  origin: Vec3,
  fallback: Quat,
  probes: readonly Probe[],
): { position: Vec3; orientation: Quat } {
  const up = { x: 0, y: 1, z: 0 };
  const look = (dir: Vec3): Quat => q.lookRotation(dir, Math.abs(dir.y) > 0.95 ? { x: 1, y: 0, z: 0 } : up);
  if (scene.pose === "closeup") {
    const p = pickCloseup(probes, 9, 5);
    if (!p || !p.hit) return { position: origin, orientation: fallback };
    // Stand 3.5 u off the wall and look at it obliquely (25° to the side),
    // so the relief and the material borders read.
    const position = v3.addScaled(origin, p.dir, p.hit.t - 3.5);
    const side = v3.normalize(v3.cross(p.dir, Math.abs(p.dir.y) > 0.95 ? { x: 1, y: 0, z: 0 } : up));
    const a = (25 * Math.PI) / 180;
    const dir = v3.normalize(v3.add(v3.scale(p.dir, Math.cos(a)), v3.scale(side, Math.sin(a))));
    return { position, orientation: look(dir) };
  }
  const p = scene.pose === "vista" ? pickVista(probes, 40, 150) : pickLongest(probes, PROBE_RANGE);
  return p ? { position: origin, orientation: look(p.dir) } : { position: origin, orientation: fallback };
}

function shapeId(visuals: readonly VisualDef[], shape: string): number {
  return visuals.findIndex((v) => v.shape === shape);
}

/** Hitbox radius per ship type (Skitter, Tinker, Hauler, Tug), data/ships. */
export const LINEUP_RADII: readonly number[] = [0.4, 0.5, 0.65, 0.55];

/** Synthetic ships, shots and effects in front of `cam` for the scene's content. */
export function sceneContent(
  scene: GalleryScene,
  cam: { position: Vec3; orientation: Quat },
  visuals: readonly VisualDef[],
  impact: Vec3 | null = null,
  vfx = false,
): SceneInstances {
  const f = q.forward(cam.orientation);
  const r = q.right(cam.orientation);
  const u = q.up(cam.orientation);
  const at = (fw: number, rt: number, upw: number): Vec3 =>
    v3.add(cam.position, v3.add(v3.scale(f, fw), v3.add(v3.scale(r, rt), v3.scale(u, upw))));
  const ships: ShipState[] = [];
  const shots: ProjectileState[] = [];
  const time = scene.time;
  let gadgets = new Set<number>();
  let flashes: readonly Flash[] = [];
  if (scene.content === "lineup") {
    LINEUP_RADII.forEach((radius, i) => {
      // Three-quarter view: noses toward the camera and to the right.
      const nose = v3.normalize(v3.add(v3.scale(f, -1), v3.scale(r, 0.9)));
      ships.push({
        id: 100 + i,
        team: i % 2,
        alive: true,
        boosting: i % 2 === 0,
        position: at(4.2, (i - 1.5) * 1.7, -0.35),
        orientation: q.lookRotation(nose, u),
        shipType: i,
        radius,
        gadgetActive: false,
      });
    });
  } else if (scene.content === "fight") {
    const a = at(7, -1.6, 0.35);
    const b = at(13, 0.4, 1.9);
    ships.push({
      id: 1, team: 1, alive: true, boosting: true, position: a,
      // Banking toward the camera, its side in view.
      orientation: q.lookRotation(v3.add(v3.normalize(v3.sub(cam.position, a)), v3.scale(r, 1.2)), u),
      shipType: 1, radius: 0.5, gadgetActive: false,
    });
    ships.push({
      id: 2, team: 1, alive: true, boosting: false, position: b,
      orientation: q.lookRotation(v3.add(v3.scale(f, -1), v3.scale(r, 0.8)), u), shipType: 2, radius: 0.65, gadgetActive: true,
    });
    gadgets = new Set([2]);
    const bolt = shapeId(visuals, "bolt");
    const rocket = shapeId(visuals, "rocket");
    const toCam = v3.normalize(v3.sub(cam.position, a));
    if (bolt >= 0) {
      for (const d of [2.2, 4.6]) {
        shots.push({ kind: 0, owner: 1, position: v3.addScaled(a, toCam, d), velocity: v3.scale(toCam, 90), visual: bolt, state: 0 });
      }
    }
    // The own rocket on its way, and the previous one exploding against the
    // rock ahead and to the right (0.15 s ago); ship A's muzzle flash is fresh.
    const blast = impact ?? at(16, 4, -1);
    if (rocket >= 0) {
      const from = at(3.5, 0.6, -0.5);
      const towards = v3.normalize(v3.sub(blast, from));
      shots.push({ kind: 0, owner: 0, position: from, velocity: v3.scale(towards, 45), visual: rocket, state: 0 });
    }
    const effects = new EffectsState();
    const ctx: EffectsContext = {
      player: 0,
      camera: cam.position,
      shipPos: (id) => ships.find((s) => s.id === id)?.position,
    };
    const cue = (kind: number, source: number, position: Vec3, value: number): Cue => ({
      kind, source, target: NO_SHIP, position, value, tick: 0,
    });
    effects.apply([cue(CUE.shockwave, 0, blast, 5)], time - 0.15, ctx);
    effects.apply([cue(CUE.muzzleFlash, 1, v3.addScaled(a, toCam, 0.7), 0)], time - 0.06, ctx);
    flashes = effects.frame(time, ctx).flashes;
  } else if (scene.content === "explosion") {
    // VQ5: a ship blowing up (gone at this time), a burning one; the effects are particles (vfxScenes.ts).
    ships.push(...explosionAt(cam, time, time).ships);
  }
  const teamOf = (id: number): number | undefined => (id === 0 ? 0 : ships.find((s) => s.id === id)?.team);
  return buildScene(ships, shots, [], visuals, teamOf, time, {
    gadgetOf: (id) => (gadgets.has(id) ? "shield" : ""),
    flashes,
    vfx,
  });
}

async function loadAround(
  loader: ChunkLoader,
  game: GameHandle,
  renderer: Renderer,
  view: CameraView,
  status: GalleryStatus,
): Promise<void> {
  for (let i = 0; i < 2000; i++) {
    const s = loader.update(game, view.position, LOADING_BUDGET);
    const stats = renderer.render(view);
    status.chunksDrawn = stats.chunksDrawn;
    if (s.pending === 0) return;
    await nextFrame();
  }
  throw new Error("terrain did not finish loading");
}

export async function runGallery(canvas: HTMLCanvasElement, ui: HTMLElement, loc: { search: string }): Promise<void> {
  const id = new URLSearchParams(loc.search).get("gallery") ?? "";
  const status: GalleryStatus = {
    scene: id,
    phase: "loading",
    error: null,
    shareCode: "",
    camera: null,
    frames: 0,
    chunksDrawn: 0,
    triangles: 0,
    instances: 0,
    canTime: false,
    gpu: null,
    gpuError: null,
  };
  window.__gallery = status;
  ui.replaceChildren();
  try {
    const scene = sceneById(id);
    if (!scene) throw new Error(`unknown gallery scene "${id}"`);
    const api = await loadWasm();
    const w = scene.world;
    status.shareCode = api.randomShareCode(w.seed, 0, w.enclosure, w.density);
    const game = api.Game.practice(status.shareCode, 0);
    game.frame(0, new Float32Array(CONTROLS_LEN));
    const spawn = parsePose(game.localPose());
    const origin = spawn?.position ?? boundsCenter(game.bounds());
    const renderer = await Renderer.create(canvas);
    renderer.detailedTiming = true;
    renderer.timeOverride = scene.time;
    // `?grade=<preset>`: try a colour grade (post.ts GRADE_PRESETS).
    const grade = new URLSearchParams(loc.search).get("grade");
    if (grade && grade in GRADE_PRESETS) renderer.settings.post.grade = GRADE_PRESETS[grade as keyof typeof GRADE_PRESETS];
    status.canTime = renderer.canTime;

    const rayMeshes = new Map<string, RayMesh>();
    const loader = new ChunkLoader((key, mesh) => {
      renderer.setChunk(key, mesh);
      if (mesh && mesh.indices.length > 0) rayMeshes.set(key, rayMesh(mesh));
      else rayMeshes.delete(key);
    });
    let view: CameraView = { position: origin, orientation: spawn?.orientation ?? q.identity(), ...DEFAULT_CAMERA };
    await loadAround(loader, game, renderer, view, status);

    const near = [...rayMeshes.values()].filter((m) => {
      const dx = Math.max(m.min.x - origin.x, 0, origin.x - m.max.x);
      const dy = Math.max(m.min.y - origin.y, 0, origin.y - m.max.y);
      const dz = Math.max(m.min.z - origin.z, 0, origin.z - m.max.z);
      return Math.hypot(dx, dy, dz) < PROBE_RANGE;
    });
    const probes = probeAround(near, origin, sphereDirections(PROBES), PROBE_RANGE);
    const pose = pickPose(scene, origin, view.orientation, probes);
    view = { ...view, ...pose };
    status.camera = pose;
    await loadAround(loader, game, renderer, view, status);

    const visuals = parseVisuals(game.visuals(), api.visualShapeNames());
    // Fight: the explosion sits where a ray ahead and to the right meets rock.
    const f = q.forward(pose.orientation);
    const rt = q.right(pose.orientation);
    const impactDir = v3.normalize(v3.add(f, v3.add(v3.scale(rt, 0.35), v3.scale(q.up(pose.orientation), -0.08))));
    const hit = raycast(rayMeshes.values(), pose.position, impactDir, 40);
    const impact = hit ? v3.addScaled(pose.position, impactDir, Math.max(4, hit.t - 0.6)) : null;
    const particles = renderer.particles;
    renderer.setScene(sceneContent(scene, pose, visuals, impact, particles !== null));
    if (particles) {
      // VQ5: the particles of the scene's synthetic history up to its time (vfxScenes.ts).
      const fallback = v3.addScaled(pose.position, impactDir, 16);
      simulateVfx(particles, scene, pose, visuals, impact ?? fallback, LINEUP_RADII, (id) => (scene.content === "fight" && id === 2 ? "shield" : ""));
      status.particles = particles.liveCount();
    }

    const frame = (): void => {
      const stats = renderer.render(view);
      status.frames++;
      status.chunksDrawn = stats.chunksDrawn;
      status.triangles = stats.triangles;
      status.instances = stats.instances;
      status.gpu = renderer.gpuTimings();
      status.gpuError = renderer.lastError;
      if (status.frames === WARMUP_FRAMES) status.phase = "ready";
      if (!renderer.isLost) requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  } catch (err) {
    status.phase = "error";
    status.error = err instanceof Error ? err.message : String(err);
    console.error("gallery:", status.error);
  }
}
